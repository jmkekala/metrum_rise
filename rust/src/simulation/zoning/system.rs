// SPDX-License-Identifier: GPL-2.0-only

//! Road-aligned parcel zoning system state.

mod editing;
mod occupancy;
mod preview;
mod queries;
mod restore;
mod validation;

use super::profiles::{ZoningProfileRegistry, load_builtin_profile_registry};
use super::{ParcelStore, ZoningParcel};
use crate::simulation::core::config::WorldConfig;
use godot::prelude::Vector3;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// Road-aligned parcel zoning system.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Validated built-in zoning-profile registry shared by the parcel tool, allocator, and saves.
    pub profiles: Arc<ZoningProfileRegistry>,
    /// Stable road-aligned parcel store used as zoning authority.
    pub parcels: ParcelStore,
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
            config: *config,
            overlay_revision: 0,
            overlay_occupancy_revision: 0,
        }
    }

    /// Clears all authored zoning parcels.
    pub fn clear(&mut self) {
        let had_parcels = !self.parcels.parcels().is_empty();
        self.parcels.clear();
        if had_parcels {
            self.bump_overlay_revision();
        }
    }

    /// Remaps parcel road-edge attachments after network compaction.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if self.parcels.remove_edges_not_in_mapping(mapping) {
            self.bump_overlay_revision();
        }
    }

    pub(crate) fn remove_parcels_attached_to_edge(&mut self, edge_idx: usize) -> usize {
        let removed = self.parcels.remove_attached_to_edge(edge_idx);
        if removed > 0 {
            self.bump_overlay_revision();
        }
        removed
    }

    pub(crate) fn remove_parcels_by_raw_ids(&mut self, raw_ids: &HashSet<u64>) -> usize {
        let ids = raw_ids
            .iter()
            .map(|&raw_id| crate::simulation::zoning::ParcelId::from_raw(raw_id))
            .collect();
        let removed = self.parcels.remove_ids(&ids);
        if removed > 0 {
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
    }

    pub(crate) fn restore_parcel_removal_undo(&mut self, undo: ZoningParcelRemovalUndo) {
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
