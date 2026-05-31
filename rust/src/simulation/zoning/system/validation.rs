//! Shared parcel edit validation for `ZoningSystem` operations.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{self, ParcelGeometry, ParcelPlacementError};
use crate::simulation::zoning::{
    MAX_PARCEL_DEPTH_M, MAX_PARCEL_FRONTAGE_M, MAX_PARCEL_GAP_M, MIN_PARCEL_DEPTH_M,
    MIN_PARCEL_FRONTAGE_M, MIN_PARCEL_GAP_M,
};

impl ZoningSystem {
    pub(super) fn validate_single_parcel_geometry(
        &self,
        geometry: &ParcelGeometry,
        graph: &RegionGraph,
    ) -> Result<(), ParcelPlacementError> {
        if !parcels::geometry_inside_world(geometry, self.config.width_m, self.config.height_m) {
            return Err(ParcelPlacementError::OutsideWorld);
        }
        if self.parcels.overlaps_existing(geometry) {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if parcels::geometry_overlaps_road(graph, geometry) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        Ok(())
    }

    pub(super) fn validate_parcel_dimensions(
        frontage_m: f32,
        depth_m: f32,
    ) -> Result<(), ParcelPlacementError> {
        if !frontage_m.is_finite()
            || !depth_m.is_finite()
            || !(MIN_PARCEL_FRONTAGE_M..=MAX_PARCEL_FRONTAGE_M).contains(&frontage_m)
            || !(MIN_PARCEL_DEPTH_M..=MAX_PARCEL_DEPTH_M).contains(&depth_m)
        {
            return Err(ParcelPlacementError::InvalidDimensions);
        }
        Ok(())
    }

    pub(super) fn validate_parcel_gap(gap_m: f32) -> Result<(), ParcelPlacementError> {
        if !gap_m.is_finite() || !(MIN_PARCEL_GAP_M..=MAX_PARCEL_GAP_M).contains(&gap_m) {
            return Err(ParcelPlacementError::InvalidGap);
        }
        Ok(())
    }

    pub(super) fn validate_parcel_run_geometries(
        &self,
        geometries: &[ParcelGeometry],
        graph: &RegionGraph,
    ) -> Result<(), ParcelPlacementError> {
        for geometry in geometries {
            if !parcels::geometry_inside_world(geometry, self.config.width_m, self.config.height_m)
            {
                return Err(ParcelPlacementError::OutsideWorld);
            }
        }
        if parcels::any_geometry_overlaps_road(graph, geometries) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        if self.parcels.overlaps_any_existing(geometries)
            || parcels::geometries_have_overlap(geometries)
        {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        Ok(())
    }

    pub(super) fn validate_profile_id(&self, runtime_id: u16) -> Result<(), ParcelPlacementError> {
        if runtime_id == 0 || self.profiles.profile_by_runtime_id(runtime_id).is_some() {
            Ok(())
        } else {
            Err(ParcelPlacementError::UnknownProfile)
        }
    }
}
