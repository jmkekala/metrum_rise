// SPDX-License-Identifier: GPL-2.0-only

//! Building occupancy bookkeeping for authored parcels.

use super::ZoningSystem;
use crate::simulation::zoning::ParcelId;

impl ZoningSystem {
    /// Claims one parcel for a building index.
    pub fn occupy_parcel(&mut self, parcel_id: u64, building_idx: usize) -> bool {
        if let Some(parcel) = self.parcels.get(ParcelId::from_raw(parcel_id))
            && let Some(lot) = parcel.cell_lot()
            && (self.cells.lot_profile(lot) != Some(parcel.zone_profile_runtime_id())
                || !lot
                    .cells()
                    .all(|key| self.cells.lot(key) == Some(parcel_id)))
        {
            return false;
        }
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
            self.release_invalid_empty_cell_lot(ParcelId::from_raw(parcel_id));
            self.bump_overlay_occupancy_revision();
        }
        changed
    }

    /// Restores an existing occupant without requiring paint compatibility during rezone grace.
    /// The load/undo caller validates building identity; cell ownership must still be complete.
    pub(crate) fn restore_parcel_occupancy(&mut self, parcel_id: u64, building_idx: usize) -> bool {
        if let Some(parcel) = self.parcels.get(ParcelId::from_raw(parcel_id))
            && let Some(lot) = parcel.cell_lot()
            && !lot
                .cells()
                .all(|key| self.cells.lot(key) == Some(parcel_id))
        {
            return false;
        }
        let changed = self
            .parcels
            .set_occupied_building(ParcelId::from_raw(parcel_id), building_idx);
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
