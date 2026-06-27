//! Save/load parcel restoration from road attachment data.

use super::ZoningSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ParcelId;
use crate::simulation::zoning::parcels::{self, ParcelPlacementError};

impl ZoningSystem {
    /// Restores one saved parcel from road attachment data.
    pub fn restore_parcel_from_attachment(
        &mut self,
        parcel_id: u64,
        edge_idx: usize,
        side: i8,
        frontage_center_t: f32,
        frontage_m: f32,
        depth_m: f32,
        runtime_id: u16,
        graph: &RegionGraph,
    ) -> Result<ParcelId, ParcelPlacementError> {
        self.validate_profile_id(runtime_id)?;
        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        if edge_idx >= graph.edge_count() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
            || edge.physical_geometry.len() < 2
            || edge.physical_length <= frontage_m
        {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
        if s_m < frontage_m * 0.5 || s_m > edge.physical_length - frontage_m * 0.5 {
            return Err(ParcelPlacementError::FrontageOutOfBounds);
        }
        let id = ParcelId::from_raw(parcel_id);
        if id.is_none() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let geometry = parcels::geometry_from_attachment(
            graph,
            edge_idx,
            if side >= 0 { 1 } else { -1 },
            frontage_center_t,
            frontage_m,
            depth_m,
        );
        self.validate_single_parcel_geometry(&geometry, graph)?;
        self.parcels.insert_loaded(id, geometry, runtime_id);
        Ok(id)
    }

    pub(crate) fn repair_parcel_attachment(
        &mut self,
        parcel_id: u64,
        edge_idx: usize,
        side: i8,
        frontage_center_t: f32,
        graph: &RegionGraph,
    ) -> Result<(), ParcelPlacementError> {
        let id = ParcelId::from_raw(parcel_id);
        if id.is_none() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let Some(parcel) = self.parcels.get(id) else {
            return Err(ParcelPlacementError::NoRoadAttachment);
        };
        let frontage_m = parcel.frontage_m();
        let depth_m = parcel.depth_m();

        Self::validate_parcel_dimensions(frontage_m, depth_m)?;
        if edge_idx >= graph.edge_count() {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
            || edge.physical_geometry.len() < 2
            || edge.physical_length <= frontage_m
        {
            return Err(ParcelPlacementError::NoRoadAttachment);
        }
        let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
        if s_m < frontage_m * 0.5 || s_m > edge.physical_length - frontage_m * 0.5 {
            return Err(ParcelPlacementError::FrontageOutOfBounds);
        }

        let geometry = parcels::geometry_from_attachment(
            graph,
            edge_idx,
            if side >= 0 { 1 } else { -1 },
            frontage_center_t,
            frontage_m,
            depth_m,
        );
        if !parcels::geometry_inside_world(&geometry, self.config.width_m, self.config.height_m) {
            return Err(ParcelPlacementError::OutsideWorld);
        }
        if self.parcels.overlaps_existing_except(&geometry, id) {
            return Err(ParcelPlacementError::OverlapsExistingParcel);
        }
        if parcels::geometry_overlaps_road(graph, &geometry) {
            return Err(ParcelPlacementError::OverlapsRoad);
        }
        self.parcels.replace_geometry(id, geometry);
        Ok(())
    }
}
