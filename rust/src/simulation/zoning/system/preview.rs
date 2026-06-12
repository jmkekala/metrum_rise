//! Parcel preview and drag-stroke projection for `ZoningSystem`.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{self, ParcelGeometry, ParcelPlacementError};
use crate::simulation::zoning::{DEFAULT_PARCEL_DEPTH_M, DEFAULT_PARCEL_FRONTAGE_M};
use godot::prelude::Vector2;

impl ZoningSystem {
    /// Projects a default 20 x 20 m parcel at a world position without mutating storage.
    pub fn preview_default_parcel_at(
        &self,
        world_x: f32,
        world_z: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelGeometry, ParcelPlacementError> {
        self.preview_parcel_at(
            world_x,
            world_z,
            DEFAULT_PARCEL_FRONTAGE_M,
            DEFAULT_PARCEL_DEPTH_M,
            graph,
        )
    }

    /// Projects a parcel with caller-selected dimensions without mutating storage.
    pub fn preview_parcel_at(
        &self,
        world_x: f32,
        world_z: f32,
        frontage_m: f32,
        depth_m: f32,
        graph: &RegionGraph,
    ) -> Result<ParcelGeometry, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        let geometry = parcels::project_default_parcel_at(
            graph,
            Vector2::new(world_x, world_z),
            frontage_m,
            depth_m,
        )?;
        self.validate_single_parcel_geometry(&geometry, graph)?;
        Ok(geometry)
    }

    /// Projects the legal parcels in a same-road drag run without mutating storage.
    pub fn preview_parcel_run_at(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        frontage_m: f32,
        depth_m: f32,
        gap_m: f32,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        Self::validate_parcel_gap(gap_m)?;
        let start_point = Vector2::new(start_x, start_z);
        let end_point = Vector2::new(end_x, end_z);
        let existing_start = self
            .parcels
            .find_at_point(start_point)
            .and_then(|id| self.parcels.get(id))
            .map(parcels::geometry_for_parcel);
        let geometries = if let Some(existing_geometry) = existing_start.as_ref() {
            parcels::project_parcel_run_from_existing(
                graph,
                existing_geometry,
                end_point,
                frontage_m,
                depth_m,
                gap_m,
            )?
        } else {
            parcels::project_parcel_run_at(
                graph,
                start_point,
                end_point,
                frontage_m,
                depth_m,
                gap_m,
            )?
        };
        self.valid_parcel_run_geometries(geometries, graph)
    }

    /// Returns authored parcel geometries touched by one world-space zoning paint stroke.
    pub fn preview_rezone_stroke(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
    ) -> Vec<ParcelGeometry> {
        self.parcels
            .find_touching_segment(Vector2::new(start_x, start_z), Vector2::new(end_x, end_z))
            .into_iter()
            .filter_map(|id| self.parcels.get(id).map(parcels::geometry_for_parcel))
            .collect()
    }
}
