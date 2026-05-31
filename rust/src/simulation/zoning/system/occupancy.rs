//! Building occupancy bookkeeping for authored parcels.

use super::ZoningSystem;
use crate::simulation::zoning::ParcelId;

impl ZoningSystem {
    /// Claims one parcel for a building index.
    pub fn occupy_parcel(&mut self, parcel_id: u64, building_idx: usize) -> bool {
        self.parcels
            .set_occupied_building(ParcelId::from_raw(parcel_id), building_idx)
    }

    /// Clears a parcel building claim.
    pub fn clear_parcel_occupancy(&mut self, parcel_id: u64) -> bool {
        self.parcels
            .clear_occupied_building(ParcelId::from_raw(parcel_id))
    }

    /// Remaps a building index inside parcel occupancy after allocator swap-remove.
    pub fn remap_parcel_occupancy(&mut self, old_idx: usize, new_idx: usize) {
        self.parcels.remap_occupied_building(old_idx, new_idx);
    }

    /// Clears every parcel occupancy claim.
    pub fn clear_all_parcel_occupancy(&mut self) {
        self.parcels.clear_all_occupancy();
    }
}
