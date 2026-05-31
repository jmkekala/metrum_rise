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
        }
    }

    /// Clears all authored zoning parcels.
    pub fn clear(&mut self) {
        self.parcels.clear();
    }

    /// Remaps parcel road-edge attachments after network compaction.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        self.parcels.remove_edges_not_in_mapping(mapping);
    }
}
