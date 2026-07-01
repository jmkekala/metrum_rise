//! Mutating parcel placement and rezone operations.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{ParcelGeometry, ParcelPlacementError};
use crate::simulation::zoning::{DEFAULT_PARCEL_DEPTH_M, DEFAULT_PARCEL_FRONTAGE_M, ParcelId};
use godot::prelude::Vector2;
use std::collections::HashSet;

impl ZoningSystem {
    /// Creates a new parcel or changes the profile of the parcel under the given world position.
    ///
    /// Runtime id `0` creates or assigns a free/unzoned parcel.
    pub fn place_or_rezone_default_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        runtime_id: u16,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.place_or_rezone_parcel_at(
            world_x,
            world_z,
            runtime_id,
            DEFAULT_PARCEL_FRONTAGE_M,
            DEFAULT_PARCEL_DEPTH_M,
            graph,
        )
    }

    /// Creates or rezones a parcel using caller-selected frontage and depth.
    ///
    /// Runtime id `0` creates or assigns a free/unzoned parcel.
    pub fn place_or_rezone_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        runtime_id: u16,
        frontage_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        let point = Vector2::new(world_x, world_z);
        if let Some(existing_id) = self.parcels.find_at_point(point) {
            self.parcels
                .set_zone_profile_runtime_id(existing_id, runtime_id);
            return Ok(existing_id);
        }

        let geometry = self.preview_parcel_at(world_x, world_z, frontage_m, depth_m, graph)?;
        Ok(self.parcels.insert_new(geometry, runtime_id))
    }

    /// Creates the legal parcels in a same-road drag run using the requested minimum gap.
    ///
    /// Blocked candidates are skipped so a road, world edge, or existing parcel does not cancel
    /// already legal parcels in the same drag stroke. On curved roads, Rust may widen spacing
    /// between generated parcels to preserve non-overlap, then stops when no further parcel can
    /// fit inside the drag span.
    pub fn place_parcel_run_at(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        runtime_id: u16,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        let geometries = self.preview_parcel_run_at(
            start_x, start_z, end_x, end_z, frontage_m, depth_m, gap_m, graph,
        )?;
        let mut ids = Vec::with_capacity(geometries.len());
        for geometry in geometries {
            ids.push(self.parcels.insert_new(geometry, runtime_id));
        }
        Ok(ids)
    }

    /// Creates parcels from geometries that were projected and validated by a caller-held
    /// immutable preview pass.
    pub(crate) fn place_prevalidated_parcel_geometries(
        &mut self,
        geometries: Vec<ParcelGeometry>,
        runtime_id: u16,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        if geometries.is_empty() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let mut ids = Vec::with_capacity(geometries.len());
        for geometry in geometries {
            ids.push(self.parcels.insert_new(geometry, runtime_id));
        }
        Ok(ids)
    }

    /// Changes the zoning profile of every authored parcel touched by one world-space stroke.
    pub fn rezone_stroke(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        runtime_id: u16,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        let ids = self
            .parcels
            .find_touching_segment(Vector2::new(start_x, start_z), Vector2::new(end_x, end_z));
        if ids.is_empty() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        for id in &ids {
            self.parcels.set_zone_profile_runtime_id(*id, runtime_id);
        }
        Ok(ids)
    }

    /// Changes the zoning profile of existing parcels identified by a caller-held immutable
    /// preview pass.
    pub(crate) fn rezone_prevalidated_parcel_geometries(
        &mut self,
        geometries: &[ParcelGeometry],
        runtime_id: u16,
    ) -> Result<Vec<ParcelId>, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        let mut seen = HashSet::new();
        let mut ids = Vec::with_capacity(geometries.len());
        for geometry in geometries {
            let Some(id) = self.parcels.find_at_point(geometry.center) else {
                continue;
            };
            if !seen.insert(id) {
                continue;
            }
            self.parcels.set_zone_profile_runtime_id(id, runtime_id);
            ids.push(id);
        }
        if ids.is_empty() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        Ok(ids)
    }
}
