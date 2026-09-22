// SPDX-License-Identifier: GPL-2.0-only

//! Building occupancy bookkeeping for authored parcels.

use super::ZoningSystem;
use crate::simulation::zoning::ParcelId;

impl ZoningSystem {
    /// Claims one parcel for a building index.
    pub fn occupy_parcel(&mut self, parcel_id: u64, building_idx: usize) -> bool {
        let changed = self
            .parcels
            .set_occupied_building(ParcelId::from_raw(parcel_id), building_idx);
        if changed {
            self.bump_overlay_occupancy_revision();
        }
        changed
    }

    /// Clears a parcel building claim.
    pub fn clear_parcel_occupancy(&mut self, parcel_id: u64) -> bool {
        let changed = self
            .parcels
            .clear_occupied_building(ParcelId::from_raw(parcel_id));
        if changed {
            self.bump_overlay_occupancy_revision();
        }
        changed
    }

    /// Remaps the known parcel's building index after allocator swap-remove or its undo.
    /// A missing parcel or stale old occupant leaves both state and revision unchanged.
    pub fn remap_parcel_occupancy(&mut self, parcel_id: u64, old_idx: usize, new_idx: usize) {
        if self
            .parcels
            .remap_occupied_building(ParcelId::from_raw(parcel_id), old_idx, new_idx)
        {
            self.bump_overlay_occupancy_revision();
        }
    }

    /// Restores a parcel's redevelopment generation when loading a save.
    ///
    /// Loading must not advance the counter, so this assigns rather than bumps.
    pub fn restore_parcel_build_generation(&mut self, parcel_id: u64, generation: u32) {
        if let Some(parcel) = self.parcels.get_mut(ParcelId::from_raw(parcel_id)) {
            parcel.set_build_generation(generation);
        }
    }

    /// Clears every parcel occupancy claim.
    pub fn clear_all_parcel_occupancy(&mut self) {
        if self.parcels.clear_all_occupancy() {
            self.bump_overlay_occupancy_revision();
        }
    }
}
