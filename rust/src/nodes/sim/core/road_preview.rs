// SPDX-License-Identifier: GPL-2.0-only

//! Asynchronous road-preview requests, worker snapshots, and compilation.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use super::state::SimCore;
use crate::debug_log;
use crate::nodes::sim::road_tool::{RoadGhostSnapIndex, validate_road_candidate_against_water};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::{
    RoadPreviewTopologyReuse, RoadPreviewValidation, RoadSurfaceSystem,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;

#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewSnapshot {
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    /// Forward vehicle lanes used for live parcel-clearance validation when publishing the result.
    pub(crate) fwd_lanes: u8,
    /// Backward vehicle lanes used for live parcel-clearance validation when publishing the result.
    pub(crate) bkw_lanes: u8,
    snap_to_existing_roads: bool,
    pub(crate) prepared_points: Vec<godot::prelude::Vector3>,
    pub(crate) surface_vertices: Vec<godot::prelude::Vector3>,
    pub(crate) validation: RoadPreviewValidation,
    pub(crate) is_valid: bool,
    topology_reuse: Option<Arc<Mutex<Option<RoadPreviewTopologyReuse>>>>,
}

/// Exact road validation that can be reused while its source generation and inputs still match.
#[derive(Debug)]
pub(crate) struct RoadPreviewValidationCertificate {
    surface_generation: u64,
    fwd_lanes: u8,
    bkw_lanes: u8,
    snap_to_existing_roads: bool,
    prepared_points: Vec<godot::prelude::Vector3>,
    validation: RoadPreviewValidation,
    topology_reuse: Mutex<Option<RoadPreviewTopologyReuse>>,
}

impl RoadPreviewSnapshot {
    /// Moves a successful exact preview artifact into a certificate for one authoritative commit.
    pub(crate) fn validation_certificate(&self) -> Option<RoadPreviewValidationCertificate> {
        (self.is_valid && self.surface_generation > 0).then(|| RoadPreviewValidationCertificate {
            surface_generation: self.surface_generation,
            fwd_lanes: self.fwd_lanes,
            bkw_lanes: self.bkw_lanes,
            snap_to_existing_roads: self.snap_to_existing_roads,
            prepared_points: self.prepared_points.clone(),
            validation: self.validation.clone(),
            topology_reuse: Mutex::new(self.topology_reuse.as_ref().and_then(|topology_reuse| {
                topology_reuse
                    .lock()
                    .expect("road preview topology lock poisoned")
                    .take()
            })),
        })
    }
}

impl RoadPreviewValidationCertificate {
    /// Returns the cached validation only when every authoritative input remains identical.
    pub(crate) fn validation_for(
        &self,
        surface_generation: u64,
        prepared_points: &[godot::prelude::Vector3],
        fwd_lanes: u8,
        bkw_lanes: u8,
        snap_to_existing_roads: bool,
    ) -> Option<&RoadPreviewValidation> {
        (self.surface_generation == surface_generation
            && self.fwd_lanes == fwd_lanes
            && self.bkw_lanes == bkw_lanes
            && self.snap_to_existing_roads == snap_to_existing_roads
            && self.prepared_points == prepared_points)
            .then_some(&self.validation)
    }

    /// Takes preview-produced topology after `validation_for` accepted the exact certificate.
    pub(crate) fn topology_reuse(&self) -> Option<RoadPreviewTopologyReuse> {
        self.topology_reuse
            .lock()
            .expect("road preview certificate topology lock poisoned")
            .take()
    }
}

#[derive(Clone)]
pub(crate) struct RoadPreviewWorkerContext {
    terrain: Arc<TerrainSystem>,
    region_graph: Arc<RegionGraph>,
    road_surface: Arc<RoadSurfaceSystem>,
    water: Arc<WaterSystem>,
    surface_chunk_span_m: f32,
    surface_chunk_origin_x_m: f32,
    surface_chunk_origin_z_m: f32,
    surface_generation: u64,
}

pub(crate) struct RoadPreviewRequest {
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) points: Vec<godot::prelude::Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
    pub(crate) snap_to_existing_roads: bool,
}

#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    pub(crate) terrain: Arc<TerrainSystem>,
    pub(crate) region_graph: Arc<RegionGraph>,
    pub(crate) road_surface: Arc<RoadSurfaceSystem>,
    /// Immutable authored-water state used by road placement validation.
    pub(crate) water: Arc<WaterSystem>,
    pub(crate) ghost_snap_index: Arc<RoadGhostSnapIndex>,
    pub(crate) surface_generation: u64,
}

pub(crate) fn road_tool_snapshots_from_core(
    core: &SimCore,
) -> Option<(RoadPreviewWorkerContext, RoadToolQuerySnapshot)> {
    if !core
        .transit_network
        .road_surface
        .published_generation_matches_source()
    {
        return None;
    }
    let snapshot_start = Instant::now();
    let terrain_start = Instant::now();
    let terrain = Arc::new(core.heightmap.clone());
    let terrain_ms = terrain_start.elapsed().as_secs_f64() * 1000.0;
    let graph_start = Instant::now();
    let region_graph = Arc::new(core.region_graph.clone());
    let graph_ms = graph_start.elapsed().as_secs_f64() * 1000.0;
    let surface_start = Instant::now();
    let road_surface = Arc::new(core.transit_network.road_surface.clone());
    let surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
    let water_start = Instant::now();
    let water = Arc::new(core.watermap.clone());
    let water_ms = water_start.elapsed().as_secs_f64() * 1000.0;
    let surface_chunk_span_m = road_surface.chunk_span_m();
    let (surface_chunk_origin_x_m, surface_chunk_origin_z_m) = road_surface.chunk_origin_m();
    let surface_generation = core.road_tool_surface_generation;
    let ghost_start = Instant::now();
    let ghost_snap_index = Arc::new(RoadGhostSnapIndex::from_graph(region_graph.as_ref()));
    let ghost_ms = ghost_start.elapsed().as_secs_f64() * 1000.0;
    if crate::debug::is_perf_enabled() {
        println!(
            "[DEBUG:perf] road_tool_snapshot generation={} edges={} terrain_ms={:.3} graph_ms={:.3} surface_ms={:.3} water_ms={:.3} ghost_ms={:.3} total_ms={:.3}",
            surface_generation,
            region_graph.edge_count(),
            terrain_ms,
            graph_ms,
            surface_ms,
            water_ms,
            ghost_ms,
            snapshot_start.elapsed().as_secs_f64() * 1000.0
        );
    }

    Some((
        RoadPreviewWorkerContext {
            terrain: Arc::clone(&terrain),
            region_graph: Arc::clone(&region_graph),
            road_surface: Arc::clone(&road_surface),
            water: Arc::clone(&water),
            surface_chunk_span_m,
            surface_chunk_origin_x_m,
            surface_chunk_origin_z_m,
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
    ))
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
    let preview_surface = RoadSurfaceSystem::new_with_chunk_grid(
        context.surface_chunk_span_m,
        context.surface_chunk_origin_x_m,
        context.surface_chunk_origin_z_m,
    );
    let fwd_lanes = request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8;
    let bkw_lanes = request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8;
    let (mut preview, topology_reuse) = preview_surface
        .compile_preview_surface_mesh_only_with_existing_surface_snap_and_topology_reuse(
            &request.points,
            fwd_lanes,
            bkw_lanes,
            context.terrain.as_ref(),
            context.region_graph.as_ref(),
            context.road_surface.as_ref(),
            request.snap_to_existing_roads,
        );
    preview.validation = validate_road_candidate_against_water(
        preview.edge_class,
        &preview.prepared_points,
        fwd_lanes,
        bkw_lanes,
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
    let topology_reuse = (preview.is_valid && generation_matches)
        .then_some(topology_reuse)
        .flatten();

    RoadPreviewSnapshot {
        request_id: request.request_id,
        surface_generation: generation_matches
            .then_some(context.surface_generation)
            .unwrap_or(0),
        fwd_lanes,
        bkw_lanes,
        snap_to_existing_roads: request.snap_to_existing_roads,
        prepared_points: preview.prepared_points,
        surface_vertices: preview.surface_vertices,
        validation: preview.validation,
        is_valid: preview.is_valid,
        topology_reuse: topology_reuse
            .map(|topology_reuse| Arc::new(Mutex::new(Some(topology_reuse)))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use godot::prelude::Vector3;

    #[test]
    fn validation_certificate_requires_exact_generation_and_inputs() {
        let points = vec![Vector3::ZERO, Vector3::new(8.0, 0.0, 0.0)];
        let snapshot = RoadPreviewSnapshot {
            request_id: 7,
            surface_generation: 11,
            fwd_lanes: 1,
            bkw_lanes: 1,
            snap_to_existing_roads: true,
            prepared_points: points.clone(),
            surface_vertices: Vec::new(),
            validation: RoadPreviewValidation::valid(0.0),
            is_valid: true,
            topology_reuse: None,
        };
        let certificate = snapshot
            .validation_certificate()
            .expect("valid preview should produce a certificate");

        assert!(
            certificate
                .validation_for(11, &points, 1, 1, true)
                .is_some()
        );
        assert!(
            certificate
                .validation_for(12, &points, 1, 1, true)
                .is_none()
        );
        assert!(
            certificate
                .validation_for(
                    11,
                    &[Vector3::ZERO, Vector3::new(9.0, 0.0, 0.0)],
                    1,
                    1,
                    true
                )
                .is_none()
        );
        assert!(
            certificate
                .validation_for(11, &points, 2, 1, true)
                .is_none()
        );
        assert!(
            certificate
                .validation_for(11, &points, 1, 1, false)
                .is_none()
        );
    }
}
