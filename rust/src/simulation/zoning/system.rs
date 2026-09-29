// SPDX-License-Identifier: GPL-2.0-only

//! Road-aligned parcel zoning system state.

mod cell_editing;
mod cell_lots;
mod editing;
mod occupancy;
mod preview;
mod queries;
mod reservations;
mod restore;
mod validation;

use super::profiles::{ZoningProfileRegistry, load_builtin_profile_registry};
use super::{ParcelStore, ZoningParcel};
use crate::simulation::core::config::WorldConfig;
use glam::DVec2;
use godot::prelude::Vector3;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) use cell_editing::CellZoningEdit;
pub(crate) use cell_lots::{CellLotGeneration, CellLotRemovalUndo};

/// Road-aligned parcel zoning system.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Validated built-in zoning-profile registry shared by the parcel tool, allocator, and saves.
    pub profiles: Arc<ZoningProfileRegistry>,
    /// Stable road-aligned parcel store used as zoning authority.
    pub parcels: ParcelStore,
    /// Road-generated cell paint and derived lot coverage; unpainted cells reserve no land.
    pub(crate) cells: super::cells::CellStore,
    cell_lot_dirty: HashSet<(i32, i32)>,
    cell_external_revision: u64,
    /// Cached compatible footprints, invalidated by the asset registry's explicit refresh hook.
    pub(crate) cell_lot_sizes: Option<Vec<super::cells::CellLotSize>>,
    /// World configuration used for parcel bounds validation.
    pub config: WorldConfig,
    overlay_revision: u64,
    overlay_occupancy_revision: u64,
}

/// Operation-local parcel records removed by one road bulldoze.
pub(crate) struct ZoningParcelRemovalUndo {
    original_parcel_count: usize,
    parcels: Vec<(usize, ZoningParcel)>,
}

impl ZoningParcelRemovalUndo {
    pub(crate) fn is_empty(&self) -> bool {
        self.parcels.is_empty()
    }
}

impl ZoningSystem {
    /// Creates a new, empty parcel zoning system for `config`.
    pub fn new(config: &WorldConfig) -> Self {
        let profiles = load_builtin_profile_registry()
            .unwrap_or_else(|err| panic!("could not load built-in zoning profiles: {err}"));
        Self {
            profiles,
            parcels: ParcelStore::default(),
            cells: super::cells::CellStore::default(),
            cell_lot_dirty: HashSet::new(),
            cell_external_revision: 0,
            cell_lot_sizes: None,
            config: *config,
            overlay_revision: 0,
            overlay_occupancy_revision: 0,
        }
    }

    /// Clears both authored parcels and generated cell paint.
    pub fn clear(&mut self) {
        let had_parcels = !self.parcels.parcels().is_empty();
        self.parcels.clear();
        self.cells.clear();
        self.cell_lot_dirty.clear();
        self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
        self.cell_lot_sizes = None;
        if had_parcels {
            self.bump_overlay_revision();
        }
    }

    pub(crate) fn remove_parcels_attached_to_edge(&mut self, edge_idx: usize) -> usize {
        let cells = &mut self.cells;
        let removed = self.parcels.remove_attached_to_edge(edge_idx, |parcel| {
            if let Some(lot) = parcel.cell_lot() {
                cells.release_lot(lot, parcel.id().raw());
            } else {
                cells.invalidate_generated_cells(
                    super::cells::CellBounds::from_points(
                        parcel
                            .corners()
                            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
                    )
                    .expanded(
                        (super::cells::CELL_DEPTH + 1) as f64 * f64::from(self.config.zone_cell_m),
                    ),
                );
            }
        });
        if removed > 0 {
            self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
            self.bump_overlay_revision();
        }
        removed
    }

    pub(crate) fn remove_parcels_by_raw_ids(&mut self, raw_ids: &HashSet<u64>) -> usize {
        let ids = raw_ids
            .iter()
            .map(|&raw_id| crate::simulation::zoning::ParcelId::from_raw(raw_id))
            .collect();
        let cells = &mut self.cells;
        let removed = self.parcels.remove_ids(&ids, |parcel| {
            if let Some(lot) = parcel.cell_lot() {
                cells.release_lot(lot, parcel.id().raw());
            } else {
                cells.invalidate_generated_cells(
                    super::cells::CellBounds::from_points(
                        parcel
                            .corners()
                            .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
                    )
                    .expanded(
                        (super::cells::CELL_DEPTH + 1) as f64 * f64::from(self.config.zone_cell_m),
                    ),
                );
            }
        });
        if removed > 0 {
            self.cell_external_revision = self.cell_external_revision.wrapping_add(1);
            self.bump_overlay_revision();
        }
        removed
    }

    pub(crate) fn parcel_ids_overlapping_road_corridor(
        &self,
        points: &[Vector3],
        half_width_m: f32,
    ) -> Vec<u64> {
        self.parcels
            .ids_overlapping_road_corridor(points, half_width_m)
            .into_iter()
            .map(|id| id.raw())
            .collect()
    }

    pub(crate) fn capture_parcel_removal_undo(&self, edge_idx: usize) -> ZoningParcelRemovalUndo {
        ZoningParcelRemovalUndo {
            original_parcel_count: self.parcels.parcels().len(),
            parcels: self.parcels.capture_attached_to_edge(edge_idx),
        }
    }

    pub(crate) fn can_restore_parcel_removal_undo(&self, undo: &ZoningParcelRemovalUndo) -> bool {
        self.parcels
            .can_restore_removed(undo.original_parcel_count, &undo.parcels)
            && undo.parcels.iter().all(|(_, parcel)| {
                parcel
                    .cell_lot()
                    .is_none_or(|lot| self.cells.can_restore_lot(lot))
            })
    }

    pub(crate) fn restore_parcel_removal_undo(&mut self, undo: ZoningParcelRemovalUndo) {
        debug_assert!(self.can_restore_parcel_removal_undo(&undo));
        for (_, parcel) in &undo.parcels {
            if let Some(lot) = parcel.cell_lot() {
                let restored = self.cells.restore_lot(lot, parcel.id().raw());
                debug_assert!(restored);
            } else {
                self.mark_cell_lots_dirty(super::cells::CellBounds::from_points(
                    parcel
                        .corners()
                        .map(|p| DVec2::new(f64::from(p.x), f64::from(p.y))),
                ));
            }
        }
        self.parcels
            .restore_removed(undo.original_parcel_count, undo.parcels);
        self.bump_overlay_revision();
    }

    pub(crate) fn overlay_revision(&self) -> u64 {
        self.overlay_revision
    }

    pub(crate) fn overlay_occupancy_revision(&self) -> u64 {
        self.overlay_occupancy_revision
    }

    pub(crate) fn bump_overlay_revision(&mut self) {
        self.overlay_revision = self.overlay_revision.wrapping_add(1);
    }

    pub(crate) fn bump_overlay_occupancy_revision(&mut self) {
        self.overlay_occupancy_revision = self.overlay_occupancy_revision.wrapping_add(1);
    }
}
