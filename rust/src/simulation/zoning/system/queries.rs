//! Read-only parcel queries exposed by `ZoningSystem`.

use super::ZoningSystem;
use crate::simulation::zoning::parcels::{self, ParcelGeometry};
use crate::simulation::zoning::{ParcelId, ZoningParcel};
use godot::prelude::Vector2;

impl ZoningSystem {
    /// Returns every authored zoning parcel.
    pub fn parcels(&self) -> &[ZoningParcel] {
        self.parcels.parcels()
    }

    /// Returns one authored parcel by stable raw id.
    pub fn parcel_by_raw_id(&self, parcel_id: u64) -> Option<&ZoningParcel> {
        self.parcels.get(ParcelId::from_raw(parcel_id))
    }

    /// Returns one mutable authored parcel by stable raw id.
    pub fn parcel_by_raw_id_mut(&mut self, parcel_id: u64) -> Option<&mut ZoningParcel> {
        self.parcels.get_mut(ParcelId::from_raw(parcel_id))
    }

    /// Returns true when one world-space point is inside an authored parcel.
    pub fn has_parcel_at(&self, world_x: f32, world_z: f32) -> bool {
        self.parcels
            .find_at_point(Vector2::new(world_x, world_z))
            .is_some()
    }

    /// Returns the runtime zoning-profile id of the parcel under one world-space point.
    pub fn parcel_profile_runtime_id_at(&self, world_x: f32, world_z: f32) -> Option<u16> {
        let id = self.parcels.find_at_point(Vector2::new(world_x, world_z))?;
        self.parcels
            .get(id)
            .map(|parcel| parcel.zone_profile_runtime_id())
    }

    /// Returns the authored parcel geometry under one world-space point.
    pub fn parcel_geometry_at(&self, world_x: f32, world_z: f32) -> Option<ParcelGeometry> {
        let id = self.parcels.find_at_point(Vector2::new(world_x, world_z))?;
        self.parcels.get(id).map(parcels::geometry_for_parcel)
    }
}
