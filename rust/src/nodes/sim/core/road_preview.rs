//! Asynchronous road-preview requests, worker snapshots, and compilation.

use std::sync::{Arc, RwLock};
use std::time::Instant;

use super::state::SimCore;
use crate::debug_log;
use crate::nodes::sim::road_tool::{RoadGhostSnapIndex, validate_road_candidate_against_water};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{RoadPreviewValidation, RoadSurfaceSystem};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewSnapshot {
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) prepared_points: Vec<godot::prelude::Vector3>,
    pub(crate) surface_vertices: Vec<godot::prelude::Vector3>,
    pub(crate) validation: RoadPreviewValidation,
    pub(crate) is_valid: bool,
}

#[derive(Clone)]
pub(crate) struct RoadPreviewWorkerContext {
    terrain: Arc<TerrainSystem>,
    region_graph: Arc<RegionGraph>,
    road_surface: Arc<RoadSurfaceSystem>,
    water: Arc<WaterSystem>,
    surface_chunk_span_m: f32,
    surface_generation: u64,
}

pub(crate) struct RoadPreviewRequest {
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) points: Vec<godot::prelude::Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
}

#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    pub(crate) terrain: Arc<TerrainSystem>,
    pub(crate) region_graph: Arc<RegionGraph>,
    pub(crate) road_surface: Arc<RoadSurfaceSystem>,
    /// Immutable authored-water state used by road placement validation.
    pub(crate) water: Arc<WaterSystem>,
    pub(crate) ghost_snap_index: RoadGhostSnapIndex,
    pub(crate) surface_generation: u64,
}

pub(crate) fn road_tool_snapshots_from_core(
    core: &SimCore,
) -> (RoadPreviewWorkerContext, RoadToolQuerySnapshot) {
    let terrain = Arc::new(core.heightmap.clone());
    let region_graph = Arc::new(core.region_graph.clone());
    let road_surface = Arc::new(core.transit_network.road_surface.clone());
    let water = Arc::new(core.watermap.clone());
    let surface_chunk_span_m = road_surface.chunk_span_m();
    let surface_generation = core.road_tool_surface_generation;
    let ghost_snap_index = RoadGhostSnapIndex::from_graph(region_graph.as_ref());

    (
        RoadPreviewWorkerContext {
            terrain: Arc::clone(&terrain),
            region_graph: Arc::clone(&region_graph),
            road_surface: Arc::clone(&road_surface),
            water: Arc::clone(&water),
            surface_chunk_span_m,
            surface_generation,
        },
        RoadToolQuerySnapshot {
            terrain,
            region_graph,
            road_surface,
            water,
            ghost_snap_index,
            surface_generation,
        },
    )
}

pub(crate) fn run_road_preview_worker(
    context: Arc<RwLock<RoadPreviewWorkerContext>>,
    result: Arc<RwLock<Option<RoadPreviewSnapshot>>>,
    rx: std::sync::mpsc::Receiver<RoadPreviewRequest>,
) {
    while let Ok(mut request) = rx.recv() {
        while let Ok(next) = rx.try_recv() {
            request = next;
        }

        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = request.points.len();
        let preview = {
            let context = context.read().expect("road preview context lock poisoned");
            compile_road_preview_from_context(&context, request)
        };
        let prepared_count = preview.prepared_points.len();
        let surface_vertex_count = preview.surface_vertices.len();
        let is_valid = preview.is_valid;
        if road_debug {
            let validation = &preview.validation;
            debug_log!(
                "road",
                "preview_surface_worker points={} prepared_points={} surface_vertices={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} span=({:.3},{:.3}) run={:.3} dy={:.3} span_y=({:.3},{:.3}) span_terrain=({:.3},{:.3}) span_delta=({:.3},{:.3}) endpoint_snap=({},{}) endpoint_delta=({:.3},{:.3}) total_ms={:.3}",
                point_count,
                prepared_count,
                surface_vertex_count,
                is_valid,
                validation.invalid_reason,
                validation.max_grade,
                validation.allowed_grade,
                validation.offending_span_start_m,
                validation.offending_span_end_m,
                validation.offending_span_run_m,
                validation.offending_span_height_delta_m,
                validation.offending_span_start_height_m,
                validation.offending_span_end_height_m,
                validation.offending_span_start_terrain_height_m,
                validation.offending_span_end_terrain_height_m,
                validation.offending_span_start_support_delta_m,
                validation.offending_span_end_support_delta_m,
                validation.start_endpoint_snapped_node_id,
                validation.end_endpoint_snapped_node_id,
                validation.start_endpoint_support_delta_m,
                validation.end_endpoint_support_delta_m,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        *result.write().expect("road preview result lock poisoned") = Some(preview);
    }
}

pub(crate) fn compile_road_preview_from_context(
    context: &RoadPreviewWorkerContext,
    request: RoadPreviewRequest,
) -> RoadPreviewSnapshot {
    let request_surface_generation = request.surface_generation;
    let preview_surface = RoadSurfaceSystem::new(context.surface_chunk_span_m);
    let mut preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &request.points,
        request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        context.terrain.as_ref(),
        context.region_graph.as_ref(),
        context.road_surface.as_ref(),
    );
    preview.validation = validate_road_candidate_against_water(
        preview.edge_class,
        &preview.prepared_points,
        request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        context.water.as_ref(),
        preview.validation,
    );
    preview.is_valid = preview.validation.is_valid;
    let generation_matches = request_surface_generation == context.surface_generation;
    if !generation_matches {
        preview.validation.is_valid = false;
        preview.validation.invalid_reason = "stale_surface_generation";
        preview.is_valid = false;
    }

    RoadPreviewSnapshot {
        request_id: request.request_id,
        surface_generation: generation_matches
            .then_some(context.surface_generation)
            .unwrap_or(0),
        prepared_points: preview.prepared_points,
        surface_vertices: preview.surface_vertices,
        validation: preview.validation,
        is_valid: preview.is_valid,
    }
}
