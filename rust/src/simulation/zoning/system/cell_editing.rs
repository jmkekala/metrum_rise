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

/// The paint a road stroke erased from the cells its pieces newly cover (`ZONE-09`), kept in its
/// undo record: the erase as one cell edit, whose inverse also re-inserts the cells that
/// regeneration dropped under the road since, and the paint the commit's lot plan gave the new
/// cells along the road (PLAN-RZ 2.4), which undo erases again first. Lots are not kept; undo
/// re-derives them from the restored paint.
#[derive(Clone, Debug)]
pub(crate) struct CellPaintUndo {
    erased: CellEdit,
    inherited: Vec<(CellKey, u16)>,
}

impl CellPaintUndo {
    /// The erased cells and the paint each had, sorted by key.
    pub(crate) fn erased(&self) -> &[(CellKey, u16)] {
        &self.erased.previous
    }

    /// The new cells that inherited the erased paint and the paint each took, sorted by key.
    pub(crate) fn inherited(&self) -> &[(CellKey, u16)] {
        &self.inherited
    }

    /// Adds the paint the stroke's lot plan gave the new cells along its road, when `paint`
    /// is that plan's: it names this record's erase. False, and nothing kept, for another
    /// stroke's (a stroke whose record the benchmark mode dropped). O(cells).
    pub(crate) fn record_inherited(&mut self, paint: RoadPaint) -> bool {
        if paint.erase != self.erased.revision {
            return false;
        }
        debug_assert!(self.inherited.is_empty(), "one lot plan per stroke");
        self.inherited = paint.cells;
        true
    }
}

/// Cell paint of one road stroke's land (`ZONE-09`): `erase` names the stroke's erase (the cell
/// revision it left), `cells` holds cell keys with their paint, sorted by key. As the erase's
/// sources, the painted cells the stroke erased, which the commit's lot plan hands on to the new
/// cells along the road (PLAN-RZ 2.4, [`super::CellLotPlanner::inheriting`]); as that plan's
/// product, the new cells it painted, for the stroke's undo record.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RoadPaint {
    /// The cell revision the stroke's erase left.
    pub(crate) erase: u64,
    /// Cells and their paint, sorted by key.
    pub(crate) cells: Vec<(CellKey, u16)>,
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

    /// Refreshes only cold or invalidated cell geometry before deriving demand lots, through
    /// the lot planner ([`super::CellLotPlanner::prepare_generated_cells`]); tests' entry.
    #[cfg(test)]
    pub(crate) fn prepare_generated_cells(
        &mut self,
        graph: &RegionGraph,
        bounds: CellBounds,
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> CellGeneration {
        self.cell_lot_planner()
            .prepare_generated_cells(graph, bounds, blocked)
    }

    /// Generates the cells a loaded save left out near every road, on registered frames
    /// ([`crate::simulation::zoning::cells::CellStore::warm_cold_chunks`]), with the same parcel
    /// and caller-owned blockers as direct generation. Returns the chunks generated and those
    /// left cold.
    pub(crate) fn warm_generated_cells(
        &mut self,
        graph: &RegionGraph,
        blocked: impl Fn(&[DVec2; 4]) -> bool + Sync,
    ) -> (usize, usize) {
        let parcels = &self.parcels;
        self.cells.warm_cold_chunks(graph, &self.config, |corners| {
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

    /// Erases the paint of `keys`, valid cells a committed road stroke newly covers (`ZONE-09`),
    /// after the commit removed their lots: the cells stay until the deferred lot plan's
    /// regeneration drops them, as the road's pieces block them. Queues their regions for lot
    /// derivation and leaves the erased paint to the commit's lot plan, whose new cells along
    /// the road inherit it ([`Self::take_road_paint_sources`]). Returns the erase for the
    /// stroke's undo record, or `None` when no cell was painted. O(keys).
    pub(crate) fn erase_cells_under_road(&mut self, keys: Vec<CellKey>) -> Option<CellPaintUndo> {
        let selection = CellSelection {
            revision: self.cells.revision(),
            cells: keys,
        };
        let erased = self.cells.paint(&selection, 0)?;
        if erased.previous.is_empty() {
            return None;
        }
        self.mark_cell_keys_dirty(erased.previous.iter().map(|&(key, _)| key));
        self.bump_overlay_revision();
        debug_assert!(self.road_paint_sources.is_none(), "a commit's plan took them");
        self.road_paint_sources = Some(RoadPaint {
            erase: erased.revision,
            cells: erased.previous.clone(),
        });
        Some(CellPaintUndo {
            erased,
            inherited: Vec::new(),
        })
    }

    /// Whether an erase's paint waits for the lot plan that drains it.
    #[cfg(test)]
    pub(crate) fn has_road_paint_sources(&self) -> bool {
        self.road_paint_sources.is_some()
    }

    /// The paint the last road stroke erased, for its commit's lot plan to hand on to the new
    /// cells along the road (PLAN-RZ 2.4): the plan that drains the stroke's lot regions takes
    /// it. `O(1)`.
    pub(crate) fn take_road_paint_sources(&mut self) -> Option<RoadPaint> {
        self.road_paint_sources.take()
    }

    /// Whether the road undo can give every erased cell its paint back now (`ZONE-09`, decision
    /// D5: otherwise the whole undo is refused). Every inherited cell must still carry the paint
    /// it inherited, claimed by no lot or by an empty one: undo erases that paint and removes
    /// those lots first. Then each erased cell must still stand unpainted and unclaimed or,
    /// dropped since, meet no reserved cell but the inherited ones; and no authored parcel or
    /// `blocked` (fields, explicit sites) may hold its land. A building on inherited cells
    /// therefore refuses the undo, the erased cells under it being taken. Roads are the
    /// caller's: the undo takes away the stroke that covers these cells.
    /// O((erased + inherited cells) * local candidates).
    pub(crate) fn can_restore_cell_paint_undo(
        &self,
        undo: &CellPaintUndo,
        blocked: impl Fn(&[DVec2; 4]) -> bool,
    ) -> bool {
        let inherited = undo.inherited();
        let inherited_free = inherited.iter().all(|&(key, profile)| {
            self.cells.profile(key) == Some(profile)
                && self.cells.lot(key).is_some_and(|lot| {
                    lot == 0
                        || self
                            .parcel_by_raw_id(lot)
                            .is_some_and(ZoningParcel::is_available)
                })
        });
        let is_inherited = |key: CellKey| {
            inherited
                .binary_search_by_key(&key, |&(inherited, _)| inherited)
                .is_ok()
        };
        inherited_free
            && undo.erased().iter().all(|&(key, profile)| {
                self.validate_profile_id(profile).is_ok()
                    && self.cells.frame(key.grid).is_some_and(|frame| {
                        let corners = frame.corners(key.x, key.y);
                        let free = match self.cells.profile(key) {
                            Some(0) => self.cells.lot(key) == Some(0),
                            Some(_) => false,
                            None => !self.cells.overlaps_reserved_except(&corners, is_inherited),
                        };
                        free && !overlaps_parcels(&self.parcels, &corners) && !blocked(&corners)
                    })
            })
    }

    /// Restores the erased paint after [`Self::can_restore_cell_paint_undo`] held: the empty
    /// lots on inherited cells are removed and the inherited paint erased, then the cells the
    /// regeneration dropped are inserted again, replacing unpainted competitors, and both sets'
    /// regions are queued so lots derive from the paint (decision D6). Returns how many lots it
    /// removed. O(erased + inherited cells).
    pub(crate) fn restore_cell_paint_undo(&mut self, undo: CellPaintUndo) -> usize {
        let mut lots: Vec<u64> = undo
            .inherited
            .iter()
            .filter_map(|&(key, _)| self.cells.lot(key).filter(|&lot| lot != 0))
            .collect();
        lots.sort_unstable();
        lots.dedup();
        let removed = self.remove_parcels_under_road(&lots);
        if !undo.inherited.is_empty() {
            let selection = CellSelection {
                revision: self.cells.revision(),
                cells: undo.inherited.iter().map(|&(key, _)| key).collect(),
            };
            let erased = self.cells.paint(&selection, 0);
            debug_assert!(
                erased.is_some_and(|edit| edit.previous == undo.inherited),
                "checked by can_restore_cell_paint_undo"
            );
        }
        let restored = self.cells.restore_paint_values(&undo.erased);
        debug_assert!(restored, "checked by can_restore_cell_paint_undo");
        self.mark_cell_keys_dirty(
            undo.erased()
                .iter()
                .chain(&undo.inherited)
                .map(|&(key, _)| key),
        );
        self.bump_overlay_revision();
        removed
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

pub(super) fn overlaps_parcels(
    parcels: &crate::simulation::zoning::ParcelStore,
    corners: &[DVec2; 4],
) -> bool {
    if !parcels.has_authored_parcels() {
        return false;
    }
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
