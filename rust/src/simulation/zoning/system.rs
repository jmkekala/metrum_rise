//! Road-aligned parcel zoning system state.

mod editing;
mod occupancy;
mod preview;
mod queries;
mod restore;
mod validation;

use super::ParcelStore;
use super::profiles::{ZoningProfileRegistry, load_builtin_profile_registry};
use crate::simulation::core::config::WorldConfig;
use std::collections::HashMap;
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
