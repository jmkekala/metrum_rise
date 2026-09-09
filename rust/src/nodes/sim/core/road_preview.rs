// SPDX-License-Identifier: GPL-2.0-only

//! Asynchronous road-preview requests, worker snapshots, and compilation.

mod requests;
pub(crate) use requests::{RoadPreviewSender, road_preview_channel};

use std::sync::{Arc, Mutex, RwLock, TryLockError};
use std::time::{Duration, Instant};

use super::road_edit_plan::RoadEditPlan;
use super::road_terrain_plan::RoadTerrainSiteInputs;
use super::state::SimCore;
use crate::debug_log;
use crate::nodes::sim::road_tool::validate_road_candidate_against_water;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::render::road::preview::{
    RoadJunctionPreview, RoadPreviewRetainedCache,
};
use crate::simulation::network::surface::{
    RoadPreviewValidation, RoadPreviewVisualMesh, RoadSurfaceSystem,
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
    pub(crate) prepared_points: Vec<godot::prelude::Vector3>,
    pub(crate) visual_mesh: RoadPreviewVisualMesh,
    pub(crate) junction_preview: Option<RoadJunctionPreview>,
    pub(crate) validation: RoadPreviewValidation,
    pub(crate) is_valid: bool,
    edit_plan: Option<Arc<RoadEditPlan>>,
}

impl RoadPreviewSnapshot {
    /// Shares a successful plan without consuming its products before the exact click check.
    pub(crate) fn edit_plan(&self) -> Option<Arc<RoadEditPlan>> {
        (self.is_valid && self.surface_generation > 0)
            .then(|| self.edit_plan.clone())
            .flatten()
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
    source_mesh_generation: u64,
}

#[derive(Debug)]
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
    if crate::debug::is_perf_enabled() {
        println!(
            "[DEBUG:perf] road_tool_snapshot generation={} edges={} terrain_ms={:.3} graph_ms={:.3} surface_ms={:.3} water_ms={:.3} total_ms={:.3}",
            surface_generation,
            region_graph.edge_count(),
            terrain_ms,
            graph_ms,
            surface_ms,
            water_ms,
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
            source_mesh_generation: core.cached_road_mesh_generation,
        },
        RoadToolQuerySnapshot {
            terrain,
            region_graph,
            road_surface,
            water,
            surface_generation,
        },
    ))
}

pub(crate) fn run_road_preview_worker(
    core: Arc<Mutex<SimCore>>,
    context: Arc<RwLock<RoadPreviewWorkerContext>>,
    result: Arc<RwLock<Option<RoadPreviewSnapshot>>>,
    rx: requests::RoadPreviewReceiver,
) {
    let mut ready_request = None;
    let mut retained_cache = RoadPreviewRetainedCache::default();
    'requests: loop {
        let Some(request) = ready_request.take().or_else(|| rx.recv().ok()) else {
            return;
        };

        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = request.points.len();
        // O(1) immutable Arc snapshot; compiling/exporting must not delay a newer context.
        let context = context
            .read()
            .expect("road preview context lock poisoned")
            .clone();
        let mut preview = prepare_road_preview_from_context(&context, request);
        let retained_reused = preview
            .junction_preview
            .as_mut()
            .is_some_and(|scene| retained_cache.reuse(scene));
        if preview.edit_plan.is_some() || (preview.junction_preview.is_some() && !retained_reused) {
            // Snapshot only the affected chunk Arcs. Never hold SimCore while filtering meshes
            // or doing Rayon work, and never wait for it while retaining the context read lock.
            let inputs = loop {
                match core.try_lock() {
                    Ok(core) => {
                        if core.road_tool_surface_generation != preview.surface_generation
                            || core.terrain_stroke_active
                            || preview.junction_preview.as_ref().is_some_and(|scene| {
                                core.cached_road_mesh_generation != scene.source_mesh_generation
                            })
                        {
                            break None;
                        }
                        let sites = preview
                            .edit_plan
                            .as_ref()
                            .and_then(|plan| plan.topology_for(&context.region_graph))
                            .and_then(|plan| plan.earthworks())
                            .and_then(|plan| RoadTerrainSiteInputs::capture(&core, plan));
                        let meshes = preview
                            .junction_preview
                            .as_ref()
                            .filter(|_| !retained_reused)
                            .map(|scene| {
                                scene
                                    .replacement_chunks
                                    .iter()
                                    .filter_map(|key| {
                                        core.cached_road_mesh_chunks
                                            .get(key)
                                            .map(|mesh| (*key, Arc::clone(mesh)))
                                    })
                                    .collect()
                            });
                        break Some((sites, meshes));
                    }
                    Err(TryLockError::Poisoned(_)) => panic!("simulation core lock poisoned"),
                    Err(TryLockError::WouldBlock) => {}
                }
                // Keep the completed compile while contended, but a newer pointer request wins.
                match rx.recv_timeout(Duration::from_millis(1)) {
                    Ok(next) => {
                        ready_request = Some(next);
                        continue 'requests;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            };
            if let Some((sites, meshes)) = inputs {
                compile_preview_terrain(&mut preview, &context, sites);
                if let Some(scene) = &mut preview.junction_preview
                    && let Some(meshes) = meshes
                {
                    scene.retain_existing(&meshes);
                    retained_cache.store(scene);
                }
            } else {
                preview.junction_preview = None;
                preview.surface_generation = 0;
                preview.is_valid = false;
                preview.validation.is_valid = false;
                preview.validation.invalid_reason = "stale_surface_generation";
            }
        }
        let prepared_count = preview.prepared_points.len();
        let surface_vertex_count = preview.visual_mesh.vertices.len();
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

#[cfg(test)]
/// Compiles the immutable test context without authoritative local site inputs.
pub(crate) fn compile_road_preview_from_context(
    context: &RoadPreviewWorkerContext,
    request: RoadPreviewRequest,
) -> RoadPreviewSnapshot {
    let mut preview = prepare_road_preview_from_context(context, request);
    compile_preview_terrain(&mut preview, context, None);
    preview
}

#[cfg(test)]
/// Exercises production site capture and off-lock compilation against a test core.
pub(crate) fn compile_road_preview_with_sites(
    context: &RoadPreviewWorkerContext,
    request: RoadPreviewRequest,
    core: &mut SimCore,
) -> RoadPreviewSnapshot {
    let mut preview = prepare_road_preview_from_context(context, request);
    let sites = preview
        .edit_plan
        .as_ref()
        .and_then(|plan| plan.topology_for(&context.region_graph))
        .and_then(|plan| plan.earthworks())
        .and_then(|plan| RoadTerrainSiteInputs::capture(core, plan));
    compile_preview_terrain(&mut preview, context, sites);
    preview
}

fn compile_preview_terrain(
    preview: &mut RoadPreviewSnapshot,
    context: &RoadPreviewWorkerContext,
    sites: Option<RoadTerrainSiteInputs>,
) {
    // The unpublished result is exclusively worker-owned; no shared plan is mutated after export.
    if let Some(plan) = preview.edit_plan.as_mut().and_then(Arc::get_mut) {
        plan.compile_terrain(
            &context.terrain,
            &context.region_graph,
            &context.road_surface,
            sites,
        );
    }
}

fn prepare_road_preview_from_context(
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
    let prepared = RoadSurfaceSystem::prepare_road_input_for_tool(
        &request.points,
        &context.terrain,
        &context.region_graph,
        &context.road_surface,
        request.snap_to_existing_roads,
    );
    let (mut preview, topology_reuse, render_input, topology_plan) = preview_surface
        .compile_prepared_preview_surface_with_topology_reuse(
            &prepared,
            fwd_lanes,
            bkw_lanes,
            context.terrain.as_ref(),
            context.region_graph.as_ref(),
            context.road_surface.as_ref(),
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
    let junction_preview = (preview.is_valid && generation_matches)
        .then_some(render_input)
        .flatten()
        .and_then(|input| {
            let mut scene = input.render(
                &context.terrain,
                &context.region_graph,
                &context.road_surface,
            )?;
            // Water-only query changes retain the previous, still-correct road meshes.
            scene.source_mesh_generation = context.source_mesh_generation;
            Some(scene)
        });

    // A junction scene already contains the candidate road. Do not build/export a second ribbon.
    let visual_mesh = if junction_preview.is_some() {
        RoadPreviewVisualMesh::default()
    } else {
        preview_surface.build_preview_visual_mesh(
            &preview.prepared_points,
            &preview.compiled_sections,
            fwd_lanes,
            bkw_lanes,
            context.terrain.as_ref(),
        )
    };
    let request_id = request.request_id;
    let edit_plan = (preview.is_valid && generation_matches).then(|| {
        Arc::new(RoadEditPlan::new(
            request,
            context.terrain.source_generation(),
            prepared,
            preview.validation.clone(),
            topology_reuse,
            topology_plan,
        ))
    });
    RoadPreviewSnapshot {
        request_id,
        surface_generation: generation_matches
            .then_some(context.surface_generation)
            .unwrap_or(0),
        fwd_lanes,
        bkw_lanes,
        prepared_points: preview.prepared_points,
        visual_mesh,
        junction_preview,
        validation: preview.validation,
        is_valid: preview.is_valid,
        edit_plan,
    }
}
