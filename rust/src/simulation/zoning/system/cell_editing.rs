// SPDX-License-Identifier: GPL-2.0-only

//! Cell generation and paint transactions sharing the authored-parcel reservation authority.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::cells::{
    CellBounds, CellEdit, CellGeneration, CellKey, CellSelection, CellSelectionShape,
    interiors_overlap,
};
use crate::simulation::zoning::{ParcelGeometry, ParcelId, ZoningParcel};
use glam::DVec2;
use godot::prelude::Vector2;
use std::collections::HashSet;

/// One local gesture inverse, including derived lots invalidated by changed cell paint.
#[derive(Clone, Debug)]
pub(crate) struct CellZoningEdit {
    /// Changed cells and their previous paint; used to invalidate pending spawn work locally.
    pub(crate) paint: CellEdit,
    lots: Vec<LotChange>,
    parcel_revision: u64,
    occupancy_revision: u64,
}

#[derive(Clone, Debug)]
struct LotChange {
    before: ZoningParcel,
    // None means the empty lot was removed; occupied lots retain coverage during grace.
    after_profile: Option<u16>,
}

impl CellZoningEdit {
    /// Stable parcel ids whose spawn eligibility changed in this gesture, in sorted order.
    pub(crate) fn changed_lots(&self) -> impl Iterator<Item = ParcelId> + '_ {
        self.lots.iter().map(|change| change.before.id())
    }
}

impl ZoningSystem {
    /// Suppresses newly reserved manual land on the next local generated-cell query.
    pub(super) fn invalidate_cell_geometry_for_parcel(&mut self, geometry: &ParcelGeometry) {
        self.mark_cell_lots_dirty(CellBounds::from_points(
            geometry
                .corners
                .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
        ));
    }

    /// Materializes a gesture's cells with the same mixed-workflow exclusions as generation.
    pub(crate) fn select_road_cells(
        &mut self,
        graph: &RegionGraph,
        shape: CellSelectionShape,
        path: &[DVec2],
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> CellSelection {
        let parcels = &self.parcels;
        self.cells
            .select_generated(graph, &self.config, shape, path, |corners| {
                overlaps_parcels(parcels, corners) || blocked(corners)
            })
    }

    /// Generates a local region while enforcing manual-parcel reservations and caller-owned
    /// compiled road, field and explicit-building reservations through the same predicate.
    pub(crate) fn generate_cells(
        &mut self,
        graph: &RegionGraph,
        bounds: CellBounds,
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> CellGeneration {
        let parcels = &self.parcels;
        self.cells
            .generate_in_bounds(graph, &self.config, bounds, |corners| {
                overlaps_parcels(parcels, corners) || blocked(corners)
            })
    }

    /// Revalidates every cell before applying one paint/erase delta; profile zero means Erase.
    /// Building-lot invalidation consumes the returned delta at the simulation transaction layer.
    pub(crate) fn paint_cells(
        &mut self,
        selection: &CellSelection,
        profile: u16,
        blocked: impl Fn(&[DVec2; 4]) -> bool,
    ) -> Option<CellZoningEdit> {
        if self.validate_profile_id(profile).is_err() || selection.revision != self.cells.revision()
        {
            return None;
        }
        if !selection.cells.iter().all(|key| {
            self.cells.profile(*key).is_some()
                && self.cells.frame(key.grid).is_some_and(|frame| {
                    let corners = frame.corners(key.x, key.y);
                    // Erasing remains possible even when terrain or an external edit has
                    // made the previous designation unusable. Occupied claims are retained.
                    profile == 0
                        || (!overlaps_parcels(&self.parcels, &corners) && !blocked(&corners))
                })
        }) {
            return None;
        }
        let mut edit = self.cells.paint(selection, profile)?;
        let lots = self.reconcile_cell_lot_paint(edit.previous.iter().map(|&(key, _)| key));
        edit.revision = self.cells.revision();
        self.mark_cell_keys_dirty(edit.previous.iter().map(|&(key, _)| key));
        if !edit.previous.is_empty() {
            self.bump_overlay_revision();
        }
        Some(CellZoningEdit {
            paint: edit,
            lots,
            parcel_revision: self.overlay_revision(),
            occupancy_revision: self.overlay_occupancy_revision(),
        })
    }

    /// Restores a completed gesture. Unchanged lots recover their identities; after growth or
    /// other lot changes, restore only paint and use the current building/redevelopment state.
    /// The inverse is atomic and stays local even after demand growth or unrelated edits.
    pub(crate) fn restore_cell_gesture(
        &mut self,
        edit: &CellZoningEdit,
        blocked: impl Fn(&[DVec2; 4]) -> bool,
    ) -> Option<Vec<ParcelId>> {
        let paint = &edit.paint;
        if !paint.previous.iter().all(|&(key, profile)| {
            self.validate_profile_id(profile).is_ok()
                && (self.cells.profile(key) == Some(paint.profile)
                    || (paint.profile == 0 && profile != 0 && self.cells.profile(key).is_none()))
                && self.cells.frame(key.grid).is_some_and(|frame| {
                    let corners = frame.corners(key.x, key.y);
                    profile == 0
                        || (!overlaps_parcels(&self.parcels, &corners) && !blocked(&corners))
                })
        }) {
            return None;
        }
        if self.undo_cell_paint(edit) {
            return Some(edit.changed_lots().collect());
        }
        if !self.cells.restore_paint_values(paint) {
            return None;
        }
        let lots = self.reconcile_cell_lot_paint(paint.previous.iter().map(|&(key, _)| key));
        self.mark_cell_keys_dirty(paint.previous.iter().map(|&(key, _)| key));
        if !paint.previous.is_empty() {
            self.bump_overlay_revision();
        }
        Some(lots.into_iter().map(|change| change.before.id()).collect())
    }

    fn reconcile_cell_lot_paint(&mut self, keys: impl Iterator<Item = CellKey>) -> Vec<LotChange> {
        let mut ids: Vec<_> = keys
            .filter_map(|key| self.cells.lot(key).filter(|&id| id != 0))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        ids.sort_unstable();
        let mut lots = Vec::with_capacity(ids.len());
        for raw in ids {
            let id = ParcelId::from_raw(raw);
            let Some(parcel) = self.parcels.get(id) else {
                continue;
            };
            let Some(lot) = parcel.cell_lot() else {
                continue;
            };
            let new_profile = self.cells.lot_profile(lot).unwrap_or(0);
            if new_profile == parcel.zone_profile_runtime_id() {
                continue;
            }
            let before = parcel.clone();
            let after_profile = if new_profile == 0 && parcel.is_available() {
                self.cells.release_lot(lot, raw);
                self.parcels.remove_local(id);
                None
            } else {
                self.parcels.set_zone_profile_runtime_id(id, new_profile);
                Some(new_profile)
            };
            lots.push(LotChange {
                before,
                after_profile,
            });
        }
        lots
    }

    /// Restores a gesture only while paint, parcel topology and occupancy still match.
    /// All preconditions are checked before restoring either paint or lot reservations.
    pub(crate) fn undo_cell_paint(&mut self, edit: &CellZoningEdit) -> bool {
        if edit.parcel_revision != self.overlay_revision()
            || edit.occupancy_revision != self.overlay_occupancy_revision()
            || edit.paint.revision != self.cells.revision()
            || !edit.lots.iter().all(|change| match change.after_profile {
                Some(profile) => self
                    .parcels
                    .get(change.before.id())
                    .is_some_and(|p| p.zone_profile_runtime_id() == profile),
                None => {
                    self.parcels.get(change.before.id()).is_none()
                        && change.before.cell_lot().is_some_and(|lot| {
                            lot.cells().all(|key| self.cells.lot(key) == Some(0))
                        })
                }
            })
        {
            return false;
        }
        if !self.cells.undo_paint(&edit.paint) {
            return false;
        }
        self.mark_cell_keys_dirty(edit.paint.previous.iter().map(|&(key, _)| key));
        for change in &edit.lots {
            if change.after_profile.is_some() {
                self.parcels.set_zone_profile_runtime_id(
                    change.before.id(),
                    change.before.zone_profile_runtime_id(),
                );
            } else if let Some(lot) = change.before.cell_lot() {
                let claimed = self.cells.claim_lot(
                    lot,
                    change.before.id().raw(),
                    change.before.zone_profile_runtime_id(),
                );
                debug_assert!(claimed);
                let restored = self.parcels.restore_local(change.before.clone());
                debug_assert!(restored);
            }
        }
        if !edit.paint.previous.is_empty() {
            self.bump_overlay_revision();
        }
        true
    }

    /// Rejects a manual parcel wherever full cell paint or occupied coverage reserves its land.
    pub(super) fn parcel_overlaps_cell_reservation(&self, geometry: &ParcelGeometry) -> bool {
        self.cells.overlaps_reserved(
            &geometry
                .corners
                .map(|point| DVec2::new(f64::from(point.x), f64::from(point.y))),
        )
    }
}

fn overlaps_parcels(
    parcels: &crate::simulation::zoning::ParcelStore,
    corners: &[DVec2; 4],
) -> bool {
    let bounds = CellBounds::from_points(*corners);
    // Outward conversion keeps chunk-boundary candidates when Godot's f32 representation
    // rounds a micrometre-normalized cell vertex toward the query interior.
    let min = Vector2::new(
        (bounds.min.x as f32).next_down(),
        (bounds.min.y as f32).next_down(),
    );
    let max = Vector2::new(
        (bounds.max.x as f32).next_up(),
        (bounds.max.y as f32).next_up(),
    );
    let mut overlaps = false;
    parcels.visit_in_bounds(min, max, |parcel| {
        if !overlaps && parcel.cell_lot().is_none() {
            overlaps = interiors_overlap(
                corners,
                &parcel
                    .corners()
                    .map(|point| DVec2::new(f64::from(point.x), f64::from(point.y))),
            );
        }
    });
    overlaps
}
