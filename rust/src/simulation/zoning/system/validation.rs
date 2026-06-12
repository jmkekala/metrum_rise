//! Shared parcel edit validation for `ZoningSystem` operations.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{self, ParcelGeometry, ParcelPlacementError};
use crate::simulation::zoning::{
    MAX_PARCEL_DEPTH_M, MAX_PARCEL_FRONTAGE_M, MAX_PARCEL_GAP_M, MIN_PARCEL_DEPTH_M,
    MIN_PARCEL_FRONTAGE_M, MIN_PARCEL_GAP_M, ParcelId,
};
use std::collections::{HashMap, HashSet};

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

    pub(super) fn valid_parcel_run_geometries(
        &self,
        geometries: Vec<ParcelGeometry>,
        graph: &RegionGraph,
    ) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
        let mut accepted = Vec::with_capacity(geometries.len());
        let mut accepted_chunks: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut accepted_seen = HashSet::new();
        let mut existing_seen: HashSet<ParcelId> = HashSet::new();
        let mut blocked_by_world = false;
        let mut blocked_by_road = false;
        let mut blocked_by_existing = false;

        for geometry in geometries {
            if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m)
            {
                blocked_by_world = true;
                continue;
            }
            if parcels::geometry_overlaps_road(graph, &geometry) {
                blocked_by_road = true;
                continue;
            }
            existing_seen.clear();
            if self
                .parcels
                .overlaps_existing_with_scratch(&geometry, &mut existing_seen)
            {
                blocked_by_existing = true;
                continue;
            }

            let chunks = parcels::chunks_for_aabb(geometry.aabb_min, geometry.aabb_max);
            accepted_seen.clear();
            let overlaps_accepted = chunks.iter().any(|chunk| {
                accepted_chunks.get(chunk).is_some_and(|indices| {
                    indices.iter().any(|&index| {
                        accepted_seen.insert(index)
                            && parcels::geometries_overlap(&accepted[index], &geometry)
                    })
                })
            });
            if overlaps_accepted {
                blocked_by_existing = true;
                continue;
            }

            let accepted_index = accepted.len();
            accepted.push(geometry);
            for chunk in chunks {
                accepted_chunks
                    .entry(chunk)
                    .or_default()
                    .push(accepted_index);
            }
        }

        if !accepted.is_empty() {
            return Ok(accepted);
        }

        if blocked_by_world {
            Err(ParcelPlacementError::OutsideWorld)
        } else if blocked_by_road {
            Err(ParcelPlacementError::OverlapsRoad)
        } else if blocked_by_existing {
            Err(ParcelPlacementError::OverlapsExistingParcel)
        } else {
            Err(ParcelPlacementError::NoRoadAttachment)
        }
    }

    pub(super) fn validate_profile_id(&self, runtime_id: u16) -> Result<(), ParcelPlacementError> {
        if runtime_id == 0 || self.profiles.profile_by_runtime_id(runtime_id).is_some() {
            Ok(())
        } else {
            Err(ParcelPlacementError::UnknownProfile)
        }
    }
}
