// SPDX-License-Identifier: GPL-2.0-only

//! Road-tool preview requests, the immutable road-tool query snapshot and the synchronous
//! `ROAD-68-A` preview, which validates the stroke and carries no geometry.

use std::sync::Arc;

use godot::prelude::Vector3;

use super::state::SimCore;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::road_edit::{
    RoadPreviewValidation, prepare_road_input_for_tool, validate_road_candidate_fast,
};
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

/// One road-tool stroke submitted for preview or commit planning.
#[derive(Debug)]
pub(crate) struct RoadPreviewRequest {
    pub(crate) include_terrain: bool,
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) points: Vec<Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) snap_to_existing_roads: bool,
}

/// The validated result of one preview request; `ROAD-68-E` adds its road and terrain pieces.
#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewSnapshot {
    pub(crate) include_terrain: bool,
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) prepared_points: Vec<Vector3>,
    pub(crate) validation: RoadPreviewValidation,
}

/// Corridor clearance of a stroke against live parcels and field reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoadCorridorClearance {
    /// Parcels the corridor overlaps; any overlap refuses the stroke (`parcel_overlap`).
    pub(crate) overlapping_parcels: usize,
    /// Lowest overlapping parcel id, present when any parcel overlaps.
    pub(crate) first_overlapping_parcel: Option<u64>,
    /// Whether the corridor overlaps a committed field reservation (`field_overlap`).
    pub(crate) field_overlap: bool,
    /// Zoning overlay revision the parcel query read.
    pub(crate) zoning_revision: u64,
    /// Agriculture field revision the field query read.
    pub(crate) field_revision: u64,
}

impl RoadCorridorClearance {
    /// Queries the parcel chunk index and the field reservations along the corridor that the
    /// lanes and sidewalks cover. O(corridor segments × parcels and fields in the touched chunks).
    pub(crate) fn evaluate(
        core: &SimCore,
        points: &[Vector3],
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Self {
        let lanes = fwd_lanes.clamp(0, i32::from(u8::MAX)) + bkw_lanes.clamp(0, i32::from(u8::MAX));
        let half_width = (lanes as f32 * crate::config::LANE_WIDTH).max(2.0) * 0.5
            + crate::config::SIDEWALK_WIDTH;
        let parcels = core
            .zoning
            .parcel_ids_overlapping_road_corridor(points, half_width);
        Self {
            overlapping_parcels: parcels.len(),
            first_overlapping_parcel: parcels.first().copied(),
            field_overlap: core
                .allocator
                .field_clearance
                .overlaps_road_corridor(points, half_width),
            zoning_revision: core.zoning.overlay_revision(),
            field_revision: core.agriculture.visual_revision(),
        }
    }
}

/// Immutable road-tool inputs from one published simulation generation.
#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    /// Persistent curb frames from the same publication as the graph.
    pub(crate) zoning_grid: crate::simulation::zoning::cells::RoadGridSnap,
    pub(crate) terrain: Arc<TerrainSystem>,
    pub(crate) region_graph: Arc<RegionGraph>,
    pub(crate) roadbed: Arc<RoadbedSystem>,
    /// Immutable authored-water state used by road placement validation.
    pub(crate) water: Arc<WaterSystem>,
    pub(crate) surface_generation: u64,
}

impl RoadToolQuerySnapshot {
    /// Prepares and validates one stroke against this snapshot: the profile, endpoint and water
    /// checks of the hover path. O(stroke points + endpoint snap lookups).
    pub(crate) fn preview(&self, request: &RoadPreviewRequest) -> RoadPreviewSnapshot {
        let fwd_lanes = request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let bkw_lanes = request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
        let prepared = prepare_road_input_for_tool(
            &request.points,
            &self.terrain,
            &self.region_graph,
            &self.roadbed,
            request.snap_to_existing_roads,
        );
        let validation = validate_road_candidate_fast(&prepared, &self.terrain, &self.region_graph);
        let mut validation = validate_road_candidate_against_water(
            prepared.class,
            &prepared.points,
            fwd_lanes,
            bkw_lanes,
            &self.water,
            validation,
        );
        if request.surface_generation != self.surface_generation {
            validation = validation.with_invalid_reason("stale_surface_generation");
        }
        RoadPreviewSnapshot {
            include_terrain: request.include_terrain,
            request_id: request.request_id,
            surface_generation: self.surface_generation,
            fwd_lanes: request.fwd_lanes,
            bkw_lanes: request.bkw_lanes,
            prepared_points: prepared.points,
            validation,
        }
    }
}

/// Captures the road-tool query snapshot once the roadbed matches the graph.
pub(crate) fn road_tool_snapshot_from_core(core: &SimCore) -> Option<RoadToolQuerySnapshot> {
    if !core.transit_network.roadbed.is_current() {
        return None;
    }
    Some(RoadToolQuerySnapshot {
        zoning_grid: core
            .zoning
            .cells
            .road_grid_snap(f64::from(core.config.zone_cell_m)),
        terrain: Arc::new(core.heightmap.clone()),
        region_graph: Arc::new(core.region_graph.clone()),
        roadbed: Arc::new(core.transit_network.roadbed.clone()),
        water: Arc::new(core.watermap.clone()),
        surface_generation: core.road_tool_surface_generation,
    })
}
