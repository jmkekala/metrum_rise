//! dedicated background thread. The render thread reads only from a
//! `RenderSnapshot` that the sim thread writes after each tick.
//!
//! ### Method Mapping
//!
//! | Category | Method | Godot Caller |
//! |----------|--------|--------------|
//! | **System** | `undo_action` | `input_manager.gd` |
//! | | `create_blank_world` | future world-editor UI |
//! | | `save_game` | `input_manager.gd`, `main.gd` |
//! | | `load_game` | `input_manager.gd`, `main.gd` |
//! | | `save_world_definition` | future world-editor UI |
//! | | `load_world_definition` | future world-editor UI |
//! | | `get_perf_stats` | `debug_panel.gd` |
//! | | `get_demand_pressures` | `main_ui.gd` |
//! | | `get_treasury_balance` | `main_ui.gd` |
//! | | `get_agent_count` | `main_ui.gd` |
//! | **Economy Editor** | `is_economy_editor_mode` | `economy_editor.gd` |
//! | | `load_economy_project` | `economy_editor.gd` |
//! | | `export_economy_project` | `economy_editor.gd` |
//! | | `run_economy_sandbox` | `economy_editor.gd` |
//! | **World Editor** | `is_world_editor_mode` | `world_editor.gd` |
//! | | `create_blank_world` | `world_editor.gd` |
//! | | `save_world_definition` | `world_editor.gd` |
//! | | `load_world_definition` | `world_editor.gd` |
//! | | `begin_world_lake_fill_preview` | `world_editor.gd` |
//! | | `update_world_lake_fill_preview` | `world_editor.gd` |
//! | | `get_world_lake_fill_preview` | `world_editor.gd` |
//! | | `commit_world_lake_fill_preview` | `world_editor.gd` |
//! | | `cancel_world_lake_fill_preview` | `world_editor.gd` |
//! | **Environment** | `get_pollution_image_data` | `overlay_manager.gd` |
//! | | `get_noise_image_data` | `overlay_manager.gd` |
//! | | `get_desirability_image_data` | `overlay_manager.gd` |
//! | **Terrain** | `sculpt_terrain` | `terrain_tool.gd` |
//! | | `smooth_terrain` | `world_editor.gd` |
//! | | `slope_terrain` | `world_editor.gd` |
//! | | `is_terrain_dirty` | `terrain.gd` |
//! | | `clear_terrain_dirty` | `terrain.gd` |
//! | | `get_terrain_patch_layout` | `terrain.gd`, `water.gd` |
//! | | `get_dirty_terrain_patches` | `terrain.gd` |
//! | | `get_terrain_patch` | `terrain.gd` |
//! | | `get_terrain_border_loop` | `terrain.gd`, `water.gd` |
//! | | `get_height_at` | `road_tool.gd`, `building_tool.gd` |
//! | | `intersect_terrain` | `input_manager.gd` (mouse pick) |
//! | | `get_world_surface_height` | `road_tool.gd`, `move_tool.gd` |
//! | | `intersect_world_surface` | `road_tool.gd`, `select_tool.gd` |
//! | **Water** | `add_water` | `water_tool.gd` |
//! | | `add_water_source` | `water_tool.gd` |
//! | | `is_water_dirty` | `water.gd` |
//! | | `clear_water_dirty` | `water.gd` |
//! | | `get_dirty_water_patches` | `water.gd` |
//! | | `get_water_patch` | `water.gd` |
//! | | `get_water_patch_debug` | `water.gd` |
//! | | `get_water_patch_authored_fill_debug` | `water.gd` |
//! | | `get_water_border_depths` | `water.gd` |
//! | **Network** | `add_road` | `road_tool.gd` |
//! | | `is_network_dirty` | `network_renderer.gd` |
//! | | `clear_network_dirty` | `network_renderer.gd` |
//! | | `get_road_mesh_data` | `network_renderer.gd` |
//! | | `get_preview_road_surface_immediate` | `road_tool.gd` |
//! | | `request_preview_road_surface` | `road_tool.gd` |
//! | | `get_preview_road_surface_result` | `road_tool.gd` |
//! | | `get_road_surface_debug_data` | `network_tool.gd` |
//! | | `get_road_tool_cursor_pos` | `road_tool.gd` |
//! | | `get_closest_network_point` | `road_tool.gd`, `zoning_tool.gd` |
//! | | `check_border_candidate` | `road_tool.gd` |
//! | | `set_border_connection` | `road_tool.gd` |
//! | **Zoning** | `get_zone_profiles` | `zoning_tool.gd`, `asset_editor.gd` |
//! | | `get_zoning_parcel_preview` | `zoning_tool.gd` |
//! | | `apply_zoning_parcel_at` | `zoning_tool.gd` |
//! | | `get_zoning_parcel_drag_preview` | `zoning_tool.gd` |
//! | | `get_zoning_parcel_drag_preview_packed` | `zoning_tool.gd` |
//! | | `apply_zoning_parcel_drag` | `zoning_tool.gd` |
//! | | `has_zoning_parcel_at` | `zoning_tool.gd` |
//! | | `get_zoning_parcel_profile_runtime_id_at` | `zoning_tool.gd` |
//! | | `get_zoning_parcel_rezone_drag_preview_packed` | `zoning_tool.gd` |
//! | | `apply_zoning_parcel_rezone_drag` | `zoning_tool.gd` |
//! | | `get_zoning_parcels_overlay` | `zoning_overlay.gd` |
//! | **Agents** | `get_agent_transforms` | `agent_renderer.gd` |
//! | | `get_car_transforms` | `agent_renderer.gd` |
//! | | `get_car_render_ids` | `agent_renderer.gd` |
//! | | `set_camera_aabb` | `agents.gd` (culling update) |

use godot::classes::{INode3D, Node3D};
use godot::prelude::*;

use crate::config;
use crate::nodes::sim::core::{
    CachedRefinedTerrainCdtWindow, CachedRefinedTerrainPatch, CityTreasury,
    RefinedTerrainCdtWindowBuildInput, RefinedTerrainCdtWindowKey, RefinedTerrainPatchBuildInput,
    RefinedTerrainPatchCacheKey, RenderSnapshot, RoadPreviewRequest, RoadPreviewSnapshot,
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, SimCommand, SimCore,
    compile_road_preview_from_context, run_road_preview_worker, run_sim_thread,
    terrain_cdt_local_sample_margin_m,
};
use crate::nodes::sim::core::{
    WorldLakeFillPreview, WorldLakeFillPreviewStatus, WorldWaterFillKind,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::terrain::cdt::{
    MAX_TERRAIN_TIE_IN_SLOPE_RATIO, TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass,
    TerrainCdtError, TerrainCdtInput, TerrainCdtPatch, TerrainCdtRoadBoundarySource,
    TerrainCdtRoadLoop, TerrainCdtTieInGuideSample, TerrainCdtVertex,
    build_road_touched_terrain_patch,
};
use crate::simulation::water::WaterSystem;
use crate::simulation::zoning::ZoningSystem;

use crate::debug_log;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

const TERRAIN_CDT_DIAGNOSTIC_STAGE_LABEL: &str = "cdt_triangulation";
const TERRAIN_CDT_DIAGNOSTIC_STAGE_CODE: i64 = 0;
const TERRAIN_CDT_BACKEND_NONE_LABEL: &str = "none";
const TERRAIN_CDT_BACKEND_NONE_CODE: i64 = -1;
const TERRAIN_CDT_BACKEND_SPADE_LABEL: &str = "spade";
const TERRAIN_CDT_BACKEND_SPADE_CODE: i64 = 0;
const TERRAIN_CDT_FAR_SAMPLE_MIN_STEP_M: f32 = 8.0;
const TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES: f32 = 8_192.0;
const TERRAIN_CDT_SAMPLE_KEY_SCALE: f64 = 1000.0;
const TERRAIN_CDT_TIE_IN_GUIDE_RING_MULTIPLIERS: [f32; 3] = [1.0, 2.0, 4.0];

#[derive(Default)]
struct TerrainCdtSourceExport {
    counts: Vec<i32>,
    labels: Vec<String>,
    kind_codes: Vec<i32>,
    primary_ids: Vec<i32>,
    node_kind_codes: Vec<i32>,
    edge_class_codes: Vec<i32>,
    owner_kinds: Vec<i32>,
    owner_indices: Vec<i32>,
    support_policies: Vec<i32>,
    roles: Vec<i32>,
    section_ranges: Vec<i32>,
    s_ranges: Vec<f32>,
}

impl TerrainCdtSourceExport {
    fn with_sample_capacity(sample_count: usize) -> Self {
        Self {
            counts: Vec::with_capacity(sample_count),
            labels: Vec::new(),
            kind_codes: Vec::new(),
            primary_ids: Vec::new(),
            node_kind_codes: Vec::new(),
            edge_class_codes: Vec::new(),
            owner_kinds: Vec::new(),
            owner_indices: Vec::new(),
            support_policies: Vec::new(),
            roles: Vec::new(),
            section_ranges: Vec::new(),
            s_ranges: Vec::new(),
        }
    }

    fn push_sources(&mut self, sources: &[TerrainCdtRoadBoundarySource]) {
        self.counts
            .push(i32::try_from(sources.len()).unwrap_or(i32::MAX));
        for source in sources.iter().copied() {
            self.labels.push(source.debug_label());
            self.kind_codes.push(source.source_kind_code());
            self.primary_ids.push(source.primary_id_code());
            self.node_kind_codes.push(source.node_kind_code());
            self.edge_class_codes.push(source.edge_class_code());
            self.owner_kinds.push(source.owner_kind_code());
            self.owner_indices.push(source.owner_index_code());
            self.support_policies.push(source.support_policy_code());
            self.roles.push(source.role_code());
            self.section_ranges.extend(source.section_range_codes());
            self.s_ranges.extend(source.s_range_values());
        }
    }
}

struct TerrainCdtTriangleBufferExport {
    vertices: Vec<Vector3>,
    normals: Vec<Vector3>,
    uvs: Vec<Vector2>,
    indices: Vec<i32>,
    face_sources: TerrainCdtSourceExport,
    emitted_faces: usize,
}

impl TerrainCdtTriangleBufferExport {
    fn empty() -> Self {
        Self {
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            face_sources: TerrainCdtSourceExport::default(),
            emitted_faces: 0,
        }
    }
}

#[derive(Clone, Copy)]
struct TerrainCdtWindowBounds {
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
    boundary_step_m: f32,
}

#[derive(GodotClass)]
#[class(base=Node3D)]
/// The central simulation node exposed to Godot.
///
/// All simulation state is in [`SimCore`] behind an `Arc<Mutex<>>`. This struct
/// holds only Godot-facing fields plus the handles needed to communicate with the
/// background sim thread.
pub struct SimulationNode {
    /// All simulation state — ticked by the background thread.
    pub(crate) core: Arc<Mutex<SimCore>>,
    /// Latest pre-computed rendering data from the sim thread.
    pub(crate) snapshot: Arc<RwLock<RenderSnapshot>>,
    /// Background sim thread handle.
    pub(crate) sim_thread: Option<std::thread::JoinHandle<()>>,
    /// Channel to send commands (speed changes, quit) to the sim thread.
    pub(crate) cmd_tx: std::sync::mpsc::Sender<SimCommand>,
    /// Receiver held here until `ready()` transfers it to the background thread.
    pub(crate) cmd_rx: Option<std::sync::mpsc::Receiver<SimCommand>>,
    /// Channel for road-tool preview jobs handled outside the simulation thread.
    pub(crate) road_preview_tx: std::sync::mpsc::Sender<RoadPreviewRequest>,
    /// Immutable context consumed by the road-preview worker.
    pub(crate) road_preview_context: Arc<RwLock<RoadPreviewWorkerContext>>,
    /// Latest completed road-tool preview from the dedicated preview worker.
    pub(crate) road_preview_result: Arc<RwLock<Option<RoadPreviewSnapshot>>>,
    /// Immutable road-tool query state used by cursor picking without locking SimCore.
    pub(crate) road_tool_query_snapshot: Arc<RwLock<RoadToolQuerySnapshot>>,
    /// Monotonic ids for stale-safe asynchronous road preview requests.
    pub(crate) road_preview_request_counter: AtomicU64,
    /// True when running in headless benchmark mode.
    pub(crate) benchmark_mode: bool,
    /// True when launched with `--asset-editor`. Sim thread is not started;
    /// the node runs a 500 m sandbox for preview only.
    pub(crate) asset_editor_mode: bool,
    /// True when launched with `--economy-editor`. Sim thread is not started;
    /// the node only serves the authored-economy editor shell.
    pub(crate) economy_editor_mode: bool,
    /// True when launched with `--world-editor`. The node boots the blank-world
    /// authoring shell and keeps the simulation thread available for terrain /
    /// future water authoring workflows.
    pub(crate) world_editor_mode: bool,
    /// Incremented every Godot frame in benchmark mode.
    pub(crate) benchmark_tick_count: u32,
    /// Last day for which benchmark CSV has been written.
    pub(crate) last_logged_day: u32,
    /// Accumulated Godot render time (unused by sim, kept for potential UI use).
    pub(crate) time_passed: f64,
    base: Base<Node3D>,
}

struct RoadClipLoopQuery {
    cdt_road_loops: Vec<TerrainCdtRoadLoop>,
    source_count: usize,
    clip_error_label: Option<&'static str>,
}

struct TerrainCdtLoopWindowDraft {
    bounds: (f32, f32, f32, f32),
    loops: Vec<TerrainCdtRoadLoop>,
}

impl SimulationNode {
    // ── Lifecycle ──

    /// Acquires the sim-core mutex, recovering silently if it was poisoned by a
    /// prior sim-thread panic.  Using `unwrap()` on a poisoned mutex would
    /// crash Godot on the next frame even though the sim thread has already
    /// recovered; this helper matches the recovery logic in `run_sim_thread`.
    #[inline]
    fn lock_core(&self) -> std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore> {
        match self.core.lock() {
            Ok(g) => g,
            Err(e) => {
                godot_error!("[sim] mutex poisoned — recovering in Godot main-thread call");
                e.into_inner()
            }
        }
    }

    /// Non-blocking mutex acquire. Returns `None` if the sim thread currently holds
    /// the lock (e.g., during `add_road_internal`). Used for per-frame read-only
    /// calls (terrain raycast, network snap) so the Godot main thread never stalls
    /// waiting for an in-progress road placement.
    #[inline]
    fn try_lock_core(&self) -> Option<std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore>> {
        match self.core.try_lock() {
            Ok(g) => Some(g),
            Err(std::sync::TryLockError::WouldBlock) => None,
            Err(std::sync::TryLockError::Poisoned(e)) => {
                godot_error!("[sim] mutex poisoned — recovering in try_lock_core");
                Some(e.into_inner())
            }
        }
    }

    fn road_tool_is_near_border(pos: Vector3, half_w: f32, half_h: f32, threshold: f32) -> bool {
        pos.x < -half_w + threshold
            || pos.x > half_w - threshold
            || pos.z < -half_h + threshold
            || pos.z > half_h - threshold
    }

    fn road_tool_snap_to_border(mut pos: Vector3, half_w: f32, half_h: f32) -> Vector3 {
        let d_left = pos.x + half_w;
        let d_right = half_w - pos.x;
        let d_top = pos.z + half_h;
        let d_bottom = half_h - pos.z;
        let min_d = d_left.min(d_right).min(d_top).min(d_bottom);
        if min_d == d_left {
            pos.x = -half_w;
        } else if min_d == d_right {
            pos.x = half_w;
        } else if min_d == d_top {
            pos.z = -half_h;
        } else {
            pos.z = half_h;
        }
        pos
    }

    /// Returns the dimensions of the heightmap.
    pub fn get_heightmap_size_internal(&self) -> Vector2 {
        let core = self.lock_core();
        Vector2::new(core.heightmap.width as f32, core.heightmap.height as f32)
    }

    /// Spawns the background simulation thread.
    fn start_sim_thread(&mut self) {
        if let Some(rx) = self.cmd_rx.take() {
            let core = Arc::clone(&self.core);
            let snap = Arc::clone(&self.snapshot);
            let road_preview = Arc::clone(&self.road_preview_context);
            let road_query = Arc::clone(&self.road_tool_query_snapshot);
            self.sim_thread = Some(std::thread::spawn(move || {
                run_sim_thread(core, snap, road_preview, road_query, rx);
            }));
        }
    }

    /// Rebuilds the render snapshot immediately from the current core state.
    fn refresh_snapshot_from_core(&self) {
        let (snapshot, preview_context, road_query_snapshot) = {
            let core = self.lock_core();
            (
                core.build_snapshot(),
                RoadPreviewWorkerContext::from_core(&core),
                RoadToolQuerySnapshot::from_core(&core),
            )
        };
        *self.snapshot.write().unwrap() = snapshot;
        *self.road_preview_context.write().unwrap() = preview_context;
        *self.road_tool_query_snapshot.write().unwrap() = road_query_snapshot;
    }

    fn world_lake_fill_preview_dict(
        preview: Option<WorldLakeFillPreview>,
        ok: bool,
        message: &str,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("ok", ok);
        dict.set("message", GString::from(message));
        if let Some(preview) = preview {
            dict.set("active", true);
            dict.set("valid", preview.is_valid());
            dict.set("seed_world_x", f64::from(preview.seed_world_x));
            dict.set("seed_world_z", f64::from(preview.seed_world_z));
            dict.set("seed_height_m", f64::from(preview.seed_height_m));
            dict.set(
                "surface_elevation_m",
                f64::from(preview.surface_elevation_m),
            );
            dict.set("filled_cells", preview.filled_cells as i64);
            dict.set(
                "status",
                GString::from(match preview.status {
                    WorldLakeFillPreviewStatus::Ready => "ready",
                    WorldLakeFillPreviewStatus::SurfaceBelowSeedTerrain => "below_seed",
                    WorldLakeFillPreviewStatus::EscapesWorldEdge => "edge_escape",
                    WorldLakeFillPreviewStatus::DoesNotReachWorldEdge => "not_edge_connected",
                }),
            );
            dict.set(
                "kind",
                GString::from(match preview.kind {
                    WorldWaterFillKind::Lake => "lake",
                    WorldWaterFillKind::OpenWater => "open_water",
                }),
            );
        } else {
            dict.set("active", false);
            dict.set("valid", false);
            dict.set("filled_cells", 0_i64);
            dict.set("status", GString::from("inactive"));
            dict.set("kind", GString::from("inactive"));
        }
        dict
    }

    fn world_water_authoring_marker_dict(
        kind: &str,
        world_x: f32,
        world_z: f32,
        terrain_height_m: f32,
        surface_elevation_m: Option<f32>,
        rate_m_per_tick: Option<f32>,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("kind", GString::from(kind));
        dict.set("world_x", f64::from(world_x));
        dict.set("world_z", f64::from(world_z));
        dict.set("terrain_height_m", f64::from(terrain_height_m));
        if let Some(surface_elevation_m) = surface_elevation_m {
            dict.set("surface_elevation_m", f64::from(surface_elevation_m));
        }
        if let Some(rate_m_per_tick) = rate_m_per_tick {
            dict.set("rate_m_per_tick", f64::from(rate_m_per_tick));
        }
        dict
    }

    fn terrain_patch_dict(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set(
            "sample_width",
            i64::try_from(patch.sample_width).unwrap_or(0),
        );
        dict.set(
            "sample_height",
            i64::try_from(patch.sample_height).unwrap_or(0),
        );
        dict.set(
            "texture_width",
            i64::try_from(patch.texture_width).unwrap_or(0),
        );
        dict.set(
            "texture_height",
            i64::try_from(patch.texture_height).unwrap_or(0),
        );
        dict.set(
            "inner_offset_x",
            i64::try_from(patch.inner_offset_x).unwrap_or(0),
        );
        dict.set(
            "inner_offset_z",
            i64::try_from(patch.inner_offset_z).unwrap_or(0),
        );
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict.set(
            "height_data",
            PackedFloat32Array::from_iter(patch.height_data.iter().copied()),
        );
        dict
    }

    fn refined_terrain_patch_dict(
        core: &crate::nodes::sim::core::SimCore,
        patch_x: usize,
        patch_z: usize,
        render_step_m: f32,
        include_debug: bool,
    ) -> VarDictionary {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let snapshot_start = road_debug.then(Instant::now);
        let Some(base_patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        let snapshot_ms = snapshot_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let dict_start = road_debug.then(Instant::now);
        let mut dict = Self::terrain_patch_dict(&base_patch);
        let base_dict_ms = dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let clip_start = road_debug.then(Instant::now);
        let road_clip_query = Self::road_clip_loop_query_for_bounds(
            core,
            base_patch.world_origin_x,
            base_patch.world_origin_z,
            base_patch.world_origin_x + base_patch.world_size_x,
            base_patch.world_origin_z + base_patch.world_size_z,
        );
        let clip_ms = clip_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let clip_loops = road_clip_query.cdt_road_loops.len();
        let clip_points: usize = road_clip_query
            .cdt_road_loops
            .iter()
            .map(|road_loop| road_loop.vertices.len())
            .sum();
        let clip_dict_start = road_debug.then(Instant::now);
        if include_debug {
            Self::append_road_clip_query(&mut dict, &road_clip_query);
        } else {
            Self::append_road_clip_status(&mut dict, &road_clip_query);
        }
        let clip_dict_ms = clip_dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let cdt_input_start = road_debug.then(Instant::now);
        let cdt_input = Self::terrain_cdt_input(
            &core.heightmap,
            &base_patch,
            &road_clip_query.cdt_road_loops,
            safe_render_step_m,
        );
        let cdt_input_ms = cdt_input_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let cdt_source_samples = cdt_input.source_samples.len();
        let cdt_append_start = road_debug.then(Instant::now);
        Self::append_cdt_terrain_mesh(
            &mut dict,
            &base_patch,
            cdt_input,
            safe_render_step_m,
            road_clip_query.source_count > 0,
            true,
            road_clip_query.clip_error_label,
            include_debug,
        );
        let cdt_append_ms = cdt_append_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "refined_patch key=({},{}) include_debug={} snapshot_ms={:.3} base_dict_ms={:.3} clip_query_ms={:.3} clip_dict_ms={:.3} cdt_input_ms={:.3} cdt_append_ms={:.3} total_ms={:.3} clip_loops={} clip_points={} cdt_source_samples={}",
                patch_x,
                patch_z,
                include_debug,
                snapshot_ms,
                base_dict_ms,
                clip_ms,
                clip_dict_ms,
                cdt_input_ms,
                cdt_append_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0),
                clip_loops,
                clip_points,
                cdt_source_samples
            );
        }
        dict
    }

    fn refined_patch_cache_key(
        patch_x: usize,
        patch_z: usize,
        render_step_m: f32,
    ) -> RefinedTerrainPatchCacheKey {
        RefinedTerrainPatchCacheKey {
            patch_x,
            patch_z,
            render_step_mm: (render_step_m.max(f32::EPSILON) * 1000.0).round() as u32,
        }
    }

    fn cached_refined_terrain_patch_dict(
        cached: &CachedRefinedTerrainPatch,
        include_debug: bool,
    ) -> VarDictionary {
        let mut dict = Self::terrain_patch_dict(&cached.patch);
        let road_clip_query = RoadClipLoopQuery {
            cdt_road_loops: Vec::new(),
            source_count: cached.road_clip_source_count,
            clip_error_label: cached.clip_error_label,
        };
        Self::append_road_clip_status(&mut dict, &road_clip_query);
        Self::append_cached_cdt_terrain_mesh(&mut dict, cached, include_debug);
        dict
    }

    fn append_cached_cdt_terrain_mesh(
        dict: &mut VarDictionary,
        cached: &CachedRefinedTerrainPatch,
        include_debug: bool,
    ) {
        if cached.input_road_loops == 0 {
            if let Some(error_label) = cached.clip_error_label {
                Self::append_empty_cdt_failure(dict, error_label, include_debug);
            } else if cached.road_clip_source_count > 0 {
                Self::append_empty_cdt_failure(dict, "missing_road_clip_loops", include_debug);
            }
            return;
        }

        let successful_windows = cached
            .windows
            .iter()
            .filter_map(|window| window.mesh_result.as_ref().ok().map(|mesh| (window, mesh)))
            .collect::<Vec<_>>();
        if successful_windows.is_empty() {
            let error_label = cached
                .windows
                .iter()
                .find_map(|window| window.mesh_result.as_ref().err())
                .map(Self::terrain_cdt_error_label)
                .unwrap_or("missing_road_clip_loops");
            Self::append_empty_cdt_failure(dict, error_label, include_debug);
            return;
        }

        let has_conflicts = successful_windows
            .iter()
            .any(|(_, mesh)| mesh.stats.invalid_constraint_edges > 0);
        dict.set(
            "terrain_cdt_status",
            GString::from(if has_conflicts { "conflicted" } else { "ok" }),
        );
        if include_debug {
            Self::append_cdt_diagnostic_metadata(dict, TERRAIN_CDT_BACKEND_SPADE_LABEL);
        }
        dict.set(
            "terrain_cdt_input_vertices",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.input_vertices)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_constraint_edges",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.constraint_edges)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_road_constraint_edges",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.road_constraint_edges)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_accepted_faces",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.accepted_faces)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_rejected_road_faces",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.rejected_road_faces)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_invalid_constraints",
            i64::try_from(
                successful_windows
                    .iter()
                    .map(|(_, mesh)| mesh.stats.invalid_constraint_edges)
                    .sum::<usize>(),
            )
            .unwrap_or(0),
        );
        Self::append_cdt_window_mesh_buffers(
            dict,
            &cached.patch,
            &successful_windows,
            (cached.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON),
            include_debug,
        );
    }

    fn append_cdt_window_mesh_buffers(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[(
            &CachedRefinedTerrainCdtWindow,
            &crate::simulation::terrain::cdt::TerrainCdtMesh,
        )],
        boundary_step_m: f32,
        include_debug: bool,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let terrain_buffer_start = road_debug.then(Instant::now);
        let mut terrain_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut retaining_buffers = TerrainCdtTriangleBufferExport::empty();
        let mut cdt_windows = Vec::with_capacity(windows.len());
        for (window, mesh) in windows {
            let window_terrain_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.triangles,
                &mesh.terrain_triangle_sources,
                include_debug,
            );
            Self::append_triangle_buffer_export(&mut terrain_buffers, window_terrain_buffers);
            let window_retaining_buffers = Self::terrain_cdt_triangle_buffers(
                patch,
                &mesh.vertices,
                &mesh.retaining_wall_triangles,
                &mesh.retaining_wall_triangle_sources,
                include_debug,
            );
            Self::append_triangle_buffer_export(&mut retaining_buffers, window_retaining_buffers);
            if let Some(bounds) =
                Self::terrain_cdt_window_bounds(patch, window.cdt_patch, boundary_step_m)
            {
                cdt_windows.push(bounds);
            }
        }
        Self::append_regular_terrain_mesh_outside_cdt_windows(
            &mut terrain_buffers,
            patch,
            &cdt_windows,
        );
        let terrain_buffer_ms = terrain_buffer_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let terrain_vertices = terrain_buffers.vertices.len();
        let terrain_indices = terrain_buffers.indices.len();
        let retaining_vertices = retaining_buffers.vertices.len();
        let retaining_indices = retaining_buffers.indices.len();
        let terrain_emitted_faces = terrain_buffers.emitted_faces;
        let retaining_emitted_faces = retaining_buffers.emitted_faces;

        let dict_start = road_debug.then(Instant::now);
        dict.set(
            "terrain_cdt_emitted_faces",
            i64::try_from(terrain_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_emitted_faces",
            i64::try_from(retaining_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_mesh_vertices",
            PackedVector3Array::from_iter(terrain_buffers.vertices),
        );
        dict.set(
            "terrain_mesh_normals",
            PackedVector3Array::from_iter(terrain_buffers.normals),
        );
        dict.set(
            "terrain_mesh_uvs",
            PackedVector2Array::from_iter(terrain_buffers.uvs),
        );
        dict.set(
            "terrain_mesh_indices",
            PackedInt32Array::from_iter(terrain_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_mesh",
                &terrain_buffers.face_sources,
            );
        }
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::from_iter(retaining_buffers.vertices),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::from_iter(retaining_buffers.normals),
        );
        dict.set(
            "terrain_retaining_wall_mesh_uvs",
            PackedVector2Array::from_iter(retaining_buffers.uvs),
        );
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::from_iter(retaining_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_retaining_wall_mesh",
                &retaining_buffers.face_sources,
            );
        }
        let dict_ms = dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "terrain_cdt_window_mesh_buffers key=({},{}) include_debug={} windows={} terrain_vertices={} terrain_indices={} terrain_faces={} retaining_vertices={} retaining_indices={} retaining_faces={} terrain_buffer_ms={:.3} dict_ms={:.3} total_ms={:.3}",
                patch.patch_x,
                patch.patch_z,
                include_debug,
                windows.len(),
                terrain_vertices,
                terrain_indices,
                terrain_emitted_faces,
                retaining_vertices,
                retaining_indices,
                retaining_emitted_faces,
                terrain_buffer_ms,
                dict_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    fn append_road_clip_loops_for_bounds(
        dict: &mut VarDictionary,
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let road_clip_query =
            Self::road_clip_loop_query_for_bounds(core, min_x, min_z, max_x, max_z);
        Self::append_road_clip_query(dict, &road_clip_query);
    }

    fn road_clip_loop_query_for_bounds(
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> RoadClipLoopQuery {
        match core
            .transit_network
            .road_surface
            .terrain_cdt_road_loops_for_world_bounds(&core.region_graph, min_x, min_z, max_x, max_z)
        {
            Ok((cdt_road_loops, source_count)) => RoadClipLoopQuery {
                cdt_road_loops,
                source_count,
                clip_error_label: None,
            },
            Err(err) => RoadClipLoopQuery {
                cdt_road_loops: Vec::new(),
                source_count: 1,
                clip_error_label: Some(err.debug_label()),
            },
        }
    }

    fn append_road_clip_query(dict: &mut VarDictionary, road_clip_query: &RoadClipLoopQuery) {
        Self::append_road_clip_status(dict, road_clip_query);
        dict.set(
            "road_clip_signature",
            Self::road_clip_query_signature(road_clip_query),
        );
        Self::append_road_clip_loops(dict, &road_clip_query.cdt_road_loops);
    }

    fn append_road_clip_status(dict: &mut VarDictionary, road_clip_query: &RoadClipLoopQuery) {
        let (status, error, source_count) = Self::road_clip_status_values(road_clip_query);
        dict.set("road_clip_status", GString::from(status));
        dict.set("road_clip_error", GString::from(error));
        dict.set("road_clip_source_count", source_count);
    }

    fn road_clip_status_values(
        road_clip_query: &RoadClipLoopQuery,
    ) -> (&'static str, &'static str, i64) {
        let (status, error) = if let Some(error_label) = road_clip_query.clip_error_label {
            ("failed", error_label)
        } else {
            ("ok", "none")
        };
        (
            status,
            error,
            i64::try_from(road_clip_query.source_count).unwrap_or(0),
        )
    }

    fn append_road_clip_loops(dict: &mut VarDictionary, road_clip_loops: &[TerrainCdtRoadLoop]) {
        let point_count: usize = road_clip_loops
            .iter()
            .map(|road_loop| road_loop.vertices.len())
            .sum();
        dict.set(
            "road_clip_loop_count",
            i64::try_from(road_clip_loops.len()).unwrap_or(0),
        );
        dict.set(
            "road_clip_point_count",
            i64::try_from(point_count).unwrap_or(0),
        );
        if road_clip_loops.is_empty() {
            return;
        }

        let mut group_ids = BTreeMap::<u64, i32>::new();
        let mut loop_counts = Vec::with_capacity(road_clip_loops.len());
        let mut loop_groups = Vec::with_capacity(road_clip_loops.len());
        let mut loop_roles = Vec::with_capacity(road_clip_loops.len());
        let mut loop_points = Vec::new();
        for road_loop in road_clip_loops {
            let next_group_id = i32::try_from(group_ids.len()).unwrap_or(i32::MAX);
            let group_id = *group_ids
                .entry(road_loop.footprint_group_id)
                .or_insert(next_group_id);
            loop_counts.push(i32::try_from(road_loop.vertices.len()).unwrap_or(0));
            loop_groups.push(group_id);
            loop_roles.push(if road_loop.is_hole { 1 } else { 0 });
            loop_points.extend(
                road_loop
                    .vertices
                    .iter()
                    .map(|vertex| Vector3::new(vertex.x as f32, vertex.height_m, vertex.z as f32)),
            );
        }
        dict.set(
            "road_clip_loop_counts",
            PackedInt32Array::from_iter(loop_counts),
        );
        dict.set(
            "road_clip_loop_groups",
            PackedInt32Array::from_iter(loop_groups),
        );
        dict.set(
            "road_clip_loop_roles",
            PackedInt32Array::from_iter(loop_roles),
        );
        dict.set(
            "road_clip_loop_points",
            PackedVector3Array::from_iter(loop_points),
        );
    }

    fn road_clip_query_signature(road_clip_query: &RoadClipLoopQuery) -> i64 {
        let mut hash = 0xcbf29ce484222325_u64;
        Self::mix_road_clip_signature(&mut hash, road_clip_query.source_count as u64);
        if let Some(error_label) = road_clip_query.clip_error_label {
            for byte in error_label.as_bytes() {
                Self::mix_road_clip_signature(&mut hash, u64::from(*byte));
            }
        }
        for road_loop in &road_clip_query.cdt_road_loops {
            Self::mix_road_clip_signature(&mut hash, road_loop.footprint_group_id);
            Self::mix_road_clip_signature(&mut hash, u64::from(road_loop.is_hole));
            Self::mix_road_clip_signature(&mut hash, road_loop.vertices.len() as u64);
            for vertex in &road_loop.vertices {
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.x * 1000.0).round() as i64) as u64,
                );
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.z * 1000.0).round() as i64) as u64,
                );
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.height_m * 1000.0).round() as i64) as u64,
                );
            }
        }
        i64::from_ne_bytes(hash.to_ne_bytes())
    }

    fn mix_road_clip_signature(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x100000001b3);
    }

    fn append_cdt_terrain_mesh(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_input: TerrainCdtInput,
        render_step_m: f32,
        has_grounded_road_contributors: bool,
        requires_road_clip: bool,
        clip_error_label: Option<&'static str>,
        include_debug: bool,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let input_road_loops = cdt_input.road_loops.len();
        let input_source_samples = cdt_input.source_samples.len();
        let cdt_patch = cdt_input.patch;
        if cdt_input.road_loops.is_empty() {
            if let Some(error_label) = clip_error_label {
                Self::append_empty_cdt_failure(dict, error_label, include_debug);
            } else if has_grounded_road_contributors || requires_road_clip {
                Self::append_empty_cdt_failure(dict, "missing_road_clip_loops", include_debug);
            }
            if road_debug {
                debug_log!(
                    "road",
                    "terrain_cdt key=({},{}) include_debug={} status=empty road_loops={} source_samples={} total_ms={:.3}",
                    patch.patch_x,
                    patch.patch_z,
                    include_debug,
                    input_road_loops,
                    input_source_samples,
                    total_start
                        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or(0.0)
                );
            }
            return;
        }

        let cdt_start = road_debug.then(Instant::now);
        match build_road_touched_terrain_patch(cdt_input) {
            Ok(mesh) => {
                let cdt_ms = cdt_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let cdt_status = if mesh.stats.invalid_constraint_edges > 0 {
                    "conflicted"
                } else {
                    "ok"
                };
                let metadata_start = road_debug.then(Instant::now);
                dict.set("terrain_cdt_status", GString::from(cdt_status));
                if include_debug {
                    Self::append_cdt_diagnostic_metadata(dict, TERRAIN_CDT_BACKEND_SPADE_LABEL);
                }
                dict.set(
                    "terrain_cdt_input_vertices",
                    i64::try_from(mesh.stats.input_vertices).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_constraint_edges",
                    i64::try_from(mesh.stats.constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_road_constraint_edges",
                    i64::try_from(mesh.stats.road_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_accepted_faces",
                    i64::try_from(mesh.stats.accepted_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_rejected_road_faces",
                    i64::try_from(mesh.stats.rejected_road_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_preserved_road_constraint_edges",
                    i64::try_from(mesh.stats.preserved_road_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_invalid_constraints",
                    i64::try_from(mesh.stats.invalid_constraint_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_max_face_y_delta_m",
                    f64::from(mesh.stats.max_face_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_max_face_slope_ratio",
                    f64::from(mesh.stats.max_face_slope_ratio),
                );
                dict.set(
                    "terrain_cdt_road_seam_faces",
                    i64::try_from(mesh.stats.road_seam_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_road_seam_max_y_delta_m",
                    f64::from(mesh.stats.road_seam_max_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_road_seam_max_slope_ratio",
                    f64::from(mesh.stats.road_seam_max_slope_ratio),
                );
                dict.set(
                    "terrain_cdt_retaining_wall_faces",
                    i64::try_from(mesh.stats.retaining_wall_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_retaining_wall_max_y_delta_m",
                    f64::from(mesh.stats.retaining_wall_max_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_retaining_wall_max_slope_ratio",
                    f64::from(mesh.stats.retaining_wall_max_slope_ratio),
                );
                dict.set(
                    "terrain_cdt_accepted_seam_edges",
                    i64::try_from(mesh.stats.accepted_seam_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_merged_subbudget_seam_edges",
                    i64::try_from(mesh.stats.merged_subbudget_seam_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_omitted_near_seam_source_samples",
                    i64::try_from(mesh.stats.omitted_near_seam_source_samples).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_retaining_wall_required_seam_edges",
                    i64::try_from(mesh.stats.retaining_wall_required_seam_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_retaining_wall_required_seam_faces",
                    i64::try_from(mesh.stats.retaining_wall_required_seam_faces).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_blocking_degenerate_seam_edges",
                    i64::try_from(mesh.stats.blocking_degenerate_seam_edges).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_tie_in_widened_source_samples",
                    i64::try_from(mesh.stats.tie_in_widened_source_samples).unwrap_or(0),
                );
                dict.set(
                    "terrain_cdt_tie_in_widened_max_y_delta_m",
                    f64::from(mesh.stats.tie_in_widened_max_y_delta_m),
                );
                dict.set(
                    "terrain_cdt_tie_in_widened_max_slope_ratio",
                    f64::from(mesh.stats.tie_in_widened_max_slope_ratio),
                );
                let metadata_ms = metadata_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let debug_sidecars_start = road_debug.then(Instant::now);
                if include_debug {
                    Self::append_cdt_road_seam_face_samples(dict, &mesh);
                    Self::append_cdt_retaining_wall_face_samples(dict, &mesh);
                    Self::append_cdt_tie_in_widened_samples(dict, &mesh);
                    Self::append_cdt_seam_quality_samples(dict, &mesh);
                    Self::append_cdt_invalid_constraint_samples(dict, &mesh);
                }
                let debug_sidecars_ms = debug_sidecars_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                let mesh_export_start = road_debug.then(Instant::now);
                Self::append_cdt_mesh_buffers(
                    dict,
                    patch,
                    cdt_patch,
                    &mesh,
                    render_step_m,
                    include_debug,
                );
                let mesh_export_ms = mesh_export_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                if road_debug {
                    debug_log!(
                        "road",
                        "terrain_cdt key=({},{}) include_debug={} status={} input_vertices={} road_loops={} source_samples={} constraints={} accepted_faces={} cdt_ms={:.3} metadata_ms={:.3} debug_sidecars_ms={:.3} mesh_export_ms={:.3} total_ms={:.3}",
                        patch.patch_x,
                        patch.patch_z,
                        include_debug,
                        cdt_status,
                        mesh.stats.input_vertices,
                        input_road_loops,
                        input_source_samples,
                        mesh.stats.constraint_edges,
                        mesh.stats.accepted_faces,
                        cdt_ms,
                        metadata_ms,
                        debug_sidecars_ms,
                        mesh_export_ms,
                        total_start
                            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                            .unwrap_or(0.0)
                    );
                }
            }
            Err(err) => {
                let cdt_ms = cdt_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0);
                Self::append_empty_cdt_failure(
                    dict,
                    Self::terrain_cdt_error_label(&err),
                    include_debug,
                );
                if road_debug {
                    debug_log!(
                        "road",
                        "terrain_cdt key=({},{}) include_debug={} status=failed error={} road_loops={} source_samples={} cdt_ms={:.3} total_ms={:.3}",
                        patch.patch_x,
                        patch.patch_z,
                        include_debug,
                        Self::terrain_cdt_error_label(&err),
                        input_road_loops,
                        input_source_samples,
                        cdt_ms,
                        total_start
                            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                            .unwrap_or(0.0)
                    );
                }
            }
        }
    }

    fn append_empty_cdt_failure(
        dict: &mut VarDictionary,
        error_label: &'static str,
        include_debug: bool,
    ) {
        dict.set("terrain_cdt_status", GString::from("failed"));
        dict.set("terrain_cdt_error", GString::from(error_label));
        let backend_label = if error_label == "triangulation_failed" {
            TERRAIN_CDT_BACKEND_SPADE_LABEL
        } else {
            TERRAIN_CDT_BACKEND_NONE_LABEL
        };
        if include_debug {
            Self::append_cdt_diagnostic_metadata(dict, backend_label);
        }
        dict.set("terrain_cdt_input_vertices", 0i64);
        dict.set("terrain_cdt_constraint_edges", 0i64);
        dict.set("terrain_cdt_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_accepted_faces", 0i64);
        dict.set("terrain_cdt_rejected_road_faces", 0i64);
        dict.set("terrain_cdt_preserved_road_constraint_edges", 0i64);
        dict.set("terrain_cdt_invalid_constraints", 1i64);
        dict.set("terrain_cdt_emitted_faces", 0i64);
        dict.set("terrain_cdt_retaining_wall_emitted_faces", 0i64);
        dict.set("terrain_cdt_max_face_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_max_face_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_road_seam_faces", 0i64);
        dict.set("terrain_cdt_road_seam_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_road_seam_max_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_retaining_wall_faces", 0i64);
        dict.set("terrain_cdt_retaining_wall_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_retaining_wall_max_slope_ratio", 0.0f64);
        dict.set("terrain_cdt_accepted_seam_edges", 0i64);
        dict.set("terrain_cdt_merged_subbudget_seam_edges", 0i64);
        dict.set("terrain_cdt_omitted_near_seam_source_samples", 0i64);
        dict.set("terrain_cdt_retaining_wall_required_seam_edges", 0i64);
        dict.set("terrain_cdt_retaining_wall_required_seam_faces", 0i64);
        dict.set("terrain_cdt_blocking_degenerate_seam_edges", 0i64);
        dict.set("terrain_cdt_tie_in_widened_source_samples", 0i64);
        dict.set("terrain_cdt_tie_in_widened_max_y_delta_m", 0.0f64);
        dict.set("terrain_cdt_tie_in_widened_max_slope_ratio", 0.0f64);
        dict.set("terrain_mesh_vertices", PackedVector3Array::new());
        dict.set("terrain_mesh_normals", PackedVector3Array::new());
        dict.set("terrain_mesh_uvs", PackedVector2Array::new());
        dict.set("terrain_mesh_indices", PackedInt32Array::new());
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::new(),
        );
        dict.set("terrain_retaining_wall_mesh_uvs", PackedVector2Array::new());
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::new(),
        );
        if !include_debug {
            return;
        }
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_kinds",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_road_seam");
        dict.set(
            "terrain_cdt_retaining_wall_sample_centroids",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_bounds",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_vertices",
            PackedVector3Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_retaining_wall");
        dict.set(
            "terrain_cdt_tie_in_widened_sample_points",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_sample_metrics",
            PackedFloat32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_tie_in_widened");
        dict.set(
            "terrain_cdt_seam_quality_sample_edges",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_metrics",
            PackedFloat32Array::new(),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_kinds",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_seam_quality");
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::new(),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::new(),
        );
        Self::append_empty_cdt_sample_source_export(dict, "terrain_cdt_invalid_constraint");
        Self::append_empty_cdt_face_source_export(dict, "terrain_mesh");
        Self::append_empty_cdt_face_source_export(dict, "terrain_retaining_wall_mesh");
    }

    fn append_cdt_diagnostic_metadata(dict: &mut VarDictionary, backend_label: &str) {
        let backend_code = if backend_label == TERRAIN_CDT_BACKEND_SPADE_LABEL {
            TERRAIN_CDT_BACKEND_SPADE_CODE
        } else {
            TERRAIN_CDT_BACKEND_NONE_CODE
        };
        dict.set(
            "terrain_cdt_diagnostic_stage",
            GString::from(TERRAIN_CDT_DIAGNOSTIC_STAGE_LABEL),
        );
        dict.set(
            "terrain_cdt_diagnostic_stage_code",
            TERRAIN_CDT_DIAGNOSTIC_STAGE_CODE,
        );
        dict.set(
            "terrain_cdt_diagnostic_backend",
            GString::from(backend_label),
        );
        dict.set("terrain_cdt_diagnostic_backend_code", backend_code);
    }

    fn append_empty_cdt_face_source_export(dict: &mut VarDictionary, prefix: &str) {
        Self::append_cdt_face_source_export(dict, prefix, &TerrainCdtSourceExport::default());
    }

    fn append_cdt_face_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
        export: &TerrainCdtSourceExport,
    ) {
        let field_prefix = format!("{prefix}_face_source");
        let label_key = format!("{prefix}_face_sources");
        Self::append_cdt_source_export(dict, &field_prefix, &label_key, export);
    }

    fn append_empty_cdt_sample_source_export(dict: &mut VarDictionary, prefix: &str) {
        Self::append_cdt_sample_source_export(dict, prefix, &TerrainCdtSourceExport::default());
    }

    fn append_cdt_sample_source_export(
        dict: &mut VarDictionary,
        prefix: &str,
        export: &TerrainCdtSourceExport,
    ) {
        let field_prefix = format!("{prefix}_sample_source");
        let label_key = format!("{prefix}_sample_sources");
        Self::append_cdt_source_export(dict, &field_prefix, &label_key, export);
    }

    fn append_cdt_source_export(
        dict: &mut VarDictionary,
        field_prefix: &str,
        label_key: &str,
        export: &TerrainCdtSourceExport,
    ) {
        Self::set_cdt_source_i32(dict, field_prefix, "counts", &export.counts);
        dict.set(
            label_key,
            PackedStringArray::from_iter(
                export
                    .labels
                    .iter()
                    .map(|label| GString::from(label.as_str())),
            ),
        );
        Self::set_cdt_source_i32(dict, field_prefix, "kind_codes", &export.kind_codes);
        Self::set_cdt_source_i32(dict, field_prefix, "primary_ids", &export.primary_ids);
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "node_kind_codes",
            &export.node_kind_codes,
        );
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "edge_class_codes",
            &export.edge_class_codes,
        );
        Self::set_cdt_source_i32(dict, field_prefix, "owner_kinds", &export.owner_kinds);
        Self::set_cdt_source_i32(dict, field_prefix, "owner_indices", &export.owner_indices);
        Self::set_cdt_source_i32(
            dict,
            field_prefix,
            "support_policies",
            &export.support_policies,
        );
        Self::set_cdt_source_i32(dict, field_prefix, "roles", &export.roles);
        Self::set_cdt_source_i32(dict, field_prefix, "section_ranges", &export.section_ranges);
        Self::set_cdt_source_f32(dict, field_prefix, "s_ranges", &export.s_ranges);
    }

    fn set_cdt_source_i32(
        dict: &mut VarDictionary,
        field_prefix: &str,
        suffix: &str,
        values: &[i32],
    ) {
        let key = format!("{field_prefix}_{suffix}");
        dict.set(
            key.as_str(),
            PackedInt32Array::from_iter(values.iter().copied()),
        );
    }

    fn set_cdt_source_f32(
        dict: &mut VarDictionary,
        field_prefix: &str,
        suffix: &str,
        values: &[f32],
    ) {
        let key = format!("{field_prefix}_{suffix}");
        dict.set(
            key.as_str(),
            PackedFloat32Array::from_iter(values.iter().copied()),
        );
    }

    fn terrain_cdt_input(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> TerrainCdtInput {
        if road_loops.is_empty() {
            return TerrainCdtInput::new(
                Self::terrain_cdt_patch_for_bounds(
                    terrain,
                    patch.world_origin_x,
                    patch.world_origin_z,
                    patch.world_origin_x + patch.world_size_x,
                    patch.world_origin_z + patch.world_size_z,
                ),
                Vec::new(),
                Vec::new(),
            );
        }

        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let (min_x, min_z, max_x, max_z) =
            Self::terrain_cdt_local_sample_bounds(terrain, patch, road_loops, safe_render_step_m)
                .unwrap_or((
                    patch.world_origin_x,
                    patch.world_origin_z,
                    patch.world_origin_x + patch.world_size_x,
                    patch.world_origin_z + patch.world_size_z,
                ));
        let patch_model = Self::terrain_cdt_patch_for_bounds(terrain, min_x, min_z, max_x, max_z);
        let mut source_samples = Vec::new();
        let mut tie_in_guide_samples = Vec::new();
        let mut sample_keys = BTreeMap::new();
        let grid_step_m =
            Self::terrain_cdt_grid_sample_step_m(min_x, min_z, max_x, max_z, safe_render_step_m);
        Self::append_terrain_cdt_tie_in_guide_samples(
            terrain,
            road_loops,
            safe_render_step_m,
            &mut tie_in_guide_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_grid_samples(
            terrain,
            patch,
            min_x,
            min_z,
            max_x,
            max_z,
            grid_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_window_boundary_samples(
            terrain,
            min_x,
            min_z,
            max_x,
            max_z,
            safe_render_step_m,
            &mut source_samples,
            &mut sample_keys,
        );

        TerrainCdtInput::new(patch_model, road_loops.to_vec(), source_samples)
            .with_tie_in_guide_samples(tie_in_guide_samples)
    }

    fn terrain_cdt_window_build_inputs(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        previous: Option<&CachedRefinedTerrainPatch>,
    ) -> Vec<RefinedTerrainCdtWindowBuildInput> {
        if road_loops.is_empty() {
            return Vec::new();
        }
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let previous_windows = previous
            .map(|cached| {
                cached
                    .windows
                    .iter()
                    .map(|window| (window.key, window.clone()))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        let mut loops_by_group = BTreeMap::<u64, Vec<TerrainCdtRoadLoop>>::new();
        for road_loop in road_loops {
            loops_by_group
                .entry(road_loop.footprint_group_id)
                .or_default()
                .push(road_loop.clone());
        }

        let mut drafts = loops_by_group
            .into_values()
            .filter_map(|loops| {
                Self::terrain_cdt_local_sample_bounds(terrain, patch, &loops, safe_render_step_m)
                    .map(|bounds| TerrainCdtLoopWindowDraft { bounds, loops })
            })
            .collect::<Vec<_>>();
        Self::merge_terrain_cdt_window_drafts(terrain, patch, safe_render_step_m, &mut drafts);

        drafts
            .into_iter()
            .filter_map(|draft| {
                let cdt_input = Self::terrain_cdt_input_for_bounds(
                    terrain,
                    patch,
                    &draft.loops,
                    safe_render_step_m,
                    draft.bounds,
                );
                if cdt_input.road_loops.is_empty() {
                    return None;
                }
                let key = Self::terrain_cdt_window_key(&cdt_input);
                Some(RefinedTerrainCdtWindowBuildInput {
                    key,
                    cdt_input,
                    previous: previous_windows.get(&key).cloned(),
                })
            })
            .collect()
    }

    fn merge_terrain_cdt_window_drafts(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        render_step_m: f32,
        drafts: &mut Vec<TerrainCdtLoopWindowDraft>,
    ) {
        drafts.sort_by(|left, right| {
            left.bounds
                .0
                .total_cmp(&right.bounds.0)
                .then_with(|| left.bounds.1.total_cmp(&right.bounds.1))
                .then_with(|| left.bounds.2.total_cmp(&right.bounds.2))
                .then_with(|| left.bounds.3.total_cmp(&right.bounds.3))
        });
        let mut merged: Vec<TerrainCdtLoopWindowDraft> = Vec::new();
        'drafts: for mut draft in drafts.drain(..) {
            for existing in &mut merged {
                if Self::terrain_cdt_window_bounds_overlap(existing.bounds, draft.bounds) {
                    existing.loops.append(&mut draft.loops);
                    if let Some(bounds) = Self::terrain_cdt_local_sample_bounds(
                        terrain,
                        patch,
                        &existing.loops,
                        render_step_m,
                    ) {
                        existing.bounds = bounds;
                    }
                    continue 'drafts;
                }
            }
            merged.push(draft);
        }
        *drafts = merged;
    }

    fn terrain_cdt_window_bounds_overlap(
        left: (f32, f32, f32, f32),
        right: (f32, f32, f32, f32),
    ) -> bool {
        left.0 <= right.2 && left.2 >= right.0 && left.1 <= right.3 && left.3 >= right.1
    }

    fn terrain_cdt_input_for_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        bounds: (f32, f32, f32, f32),
    ) -> TerrainCdtInput {
        let (min_x, min_z, max_x, max_z) = bounds;
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let patch_model = Self::terrain_cdt_patch_for_bounds(terrain, min_x, min_z, max_x, max_z);
        let mut source_samples = Vec::new();
        let mut tie_in_guide_samples = Vec::new();
        let mut sample_keys = BTreeMap::new();
        let grid_step_m =
            Self::terrain_cdt_grid_sample_step_m(min_x, min_z, max_x, max_z, safe_render_step_m);
        Self::append_terrain_cdt_tie_in_guide_samples(
            terrain,
            road_loops,
            safe_render_step_m,
            &mut tie_in_guide_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_grid_samples(
            terrain,
            patch,
            min_x,
            min_z,
            max_x,
            max_z,
            grid_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        Self::append_terrain_cdt_window_boundary_samples(
            terrain,
            min_x,
            min_z,
            max_x,
            max_z,
            safe_render_step_m,
            &mut source_samples,
            &mut sample_keys,
        );
        TerrainCdtInput::new(patch_model, road_loops.to_vec(), source_samples)
            .with_tie_in_guide_samples(tie_in_guide_samples)
    }

    fn terrain_cdt_grid_sample_step_m(
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        render_step_m: f32,
    ) -> f32 {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let width_m = (max_x - min_x).max(0.0);
        let height_m = (max_z - min_z).max(0.0);
        let sample_x = (width_m / safe_step_m).ceil() + 1.0;
        let sample_z = (height_m / safe_step_m).ceil() + 1.0;
        let estimated_samples = sample_x * sample_z;
        if estimated_samples <= TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES {
            return safe_step_m;
        }

        let scale = (estimated_samples / TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES).sqrt();
        (safe_step_m * scale).max(safe_step_m)
    }

    fn terrain_cdt_window_key(input: &TerrainCdtInput) -> RefinedTerrainCdtWindowKey {
        RefinedTerrainCdtWindowKey {
            min_x_mm: Self::quantize_cdt_coord_mm(input.patch.min_x),
            min_z_mm: Self::quantize_cdt_coord_mm(input.patch.min_z),
            max_x_mm: Self::quantize_cdt_coord_mm(input.patch.max_x),
            max_z_mm: Self::quantize_cdt_coord_mm(input.patch.max_z),
            fingerprint: Self::terrain_cdt_input_fingerprint(input),
        }
    }

    fn quantize_cdt_coord_mm(value: f64) -> i64 {
        (value * 1000.0).round() as i64
    }

    fn terrain_cdt_input_fingerprint(input: &TerrainCdtInput) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325_u64;
        Self::hash_u64(&mut hash, input.road_loops.len() as u64);
        for road_loop in &input.road_loops {
            Self::hash_u64(&mut hash, road_loop.stable_piece_id);
            Self::hash_u64(&mut hash, road_loop.footprint_group_id);
            Self::hash_u64(&mut hash, u64::from(road_loop.local_loop_index));
            Self::hash_u64(&mut hash, u64::from(road_loop.is_hole));
            Self::hash_u64(&mut hash, road_loop.vertices.len() as u64);
            for vertex in &road_loop.vertices {
                Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(vertex.x));
                Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(vertex.z));
                Self::hash_i64(
                    &mut hash,
                    (f64::from(vertex.height_m) * 1000.0).round() as i64,
                );
            }
        }
        Self::hash_u64(&mut hash, input.tie_in_guide_samples.len() as u64);
        for sample in &input.tie_in_guide_samples {
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.vertex.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.vertex.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(sample.vertex.height_m) * 1000.0).round() as i64,
            );
        }
        Self::hash_u64(&mut hash, input.source_samples.len() as u64);
        for sample in &input.source_samples {
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.x));
            Self::hash_i64(&mut hash, Self::quantize_cdt_coord_mm(sample.z));
            Self::hash_i64(
                &mut hash,
                (f64::from(sample.height_m) * 1000.0).round() as i64,
            );
        }
        hash
    }

    fn hash_i64(hash: &mut u64, value: i64) {
        Self::hash_u64(hash, value as u64);
    }

    fn hash_u64(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }

    fn terrain_cdt_patch_for_bounds(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> TerrainCdtPatch {
        TerrainCdtPatch::new(
            f64::from(min_x),
            f64::from(min_z),
            f64::from(max_x),
            f64::from(max_z),
            [
                terrain.sample_visual_height_world(min_x, min_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(min_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, max_z) * config::HEIGHT_SCALE,
                terrain.sample_visual_height_world(max_x, min_z) * config::HEIGHT_SCALE,
            ],
        )
    }

    fn terrain_cdt_local_sample_bounds(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f32::INFINITY;
        let mut min_z = f32::INFINITY;
        let mut max_x = f32::NEG_INFINITY;
        let mut max_z = f32::NEG_INFINITY;
        for road_loop in road_loops {
            for vertex in &road_loop.vertices {
                let x = vertex.x as f32;
                let z = vertex.z as f32;
                min_x = min_x.min(x);
                min_z = min_z.min(z);
                max_x = max_x.max(x);
                max_z = max_z.max(z);
            }
        }
        if !min_x.is_finite() || !min_z.is_finite() || !max_x.is_finite() || !max_z.is_finite() {
            return None;
        }

        let margin_m = terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        min_x = (min_x - margin_m).clamp(patch_min_x, patch_max_x);
        min_z = (min_z - margin_m).clamp(patch_min_z, patch_max_z);
        max_x = (max_x + margin_m).clamp(patch_min_x, patch_max_x);
        max_z = (max_z + margin_m).clamp(patch_min_z, patch_max_z);
        if min_x > max_x || min_z > max_z {
            None
        } else {
            Some((min_x, min_z, max_x, max_z))
        }
    }

    fn append_terrain_cdt_grid_samples(
        terrain: &TerrainSystem,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let start_x_index = (((min_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .floor() as i64)
            .max(0);
        let start_z_index = (((min_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .floor() as i64)
            .max(0);
        let end_x_index = (((max_x.clamp(patch_min_x, patch_max_x) - patch_min_x) / safe_step_m)
            .ceil() as i64)
            .max(start_x_index);
        let end_z_index = (((max_z.clamp(patch_min_z, patch_max_z) - patch_min_z) / safe_step_m)
            .ceil() as i64)
            .max(start_z_index);

        for sample_z_index in start_z_index..=end_z_index {
            let world_z = (patch_min_z + sample_z_index as f32 * safe_step_m).min(patch_max_z);
            for sample_x_index in start_x_index..=end_x_index {
                let world_x = (patch_min_x + sample_x_index as f32 * safe_step_m).min(patch_max_x);
                Self::push_terrain_cdt_source_sample(
                    terrain,
                    world_x,
                    world_z,
                    source_samples,
                    sample_keys,
                );
            }
        }
    }

    fn append_terrain_cdt_window_boundary_samples(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = step_m.max(f32::EPSILON);
        let xs = Self::terrain_cdt_axis_samples(min_x, max_x, safe_step_m);
        let zs = Self::terrain_cdt_axis_samples(min_z, max_z, safe_step_m);
        for &x in &xs {
            Self::push_terrain_cdt_source_sample(terrain, x, min_z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, x, max_z, source_samples, sample_keys);
        }
        for &z in &zs {
            Self::push_terrain_cdt_source_sample(terrain, min_x, z, source_samples, sample_keys);
            Self::push_terrain_cdt_source_sample(terrain, max_x, z, source_samples, sample_keys);
        }
    }

    fn append_terrain_cdt_tie_in_guide_samples(
        terrain: &TerrainSystem,
        road_loops: &[TerrainCdtRoadLoop],
        render_step_m: f32,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let safe_step_m = render_step_m.max(f32::EPSILON);
        let max_distance_m = terrain_cdt_local_sample_margin_m(terrain, safe_step_m);
        for road_loop in road_loops {
            if road_loop.is_hole
                || road_loop.vertices.len() < 2
                || !Self::road_loop_uses_clean_grounded_tie_in(road_loop)
            {
                continue;
            }
            let signed_area = Self::terrain_cdt_road_loop_signed_area_xz(road_loop);
            if signed_area.abs() <= f64::EPSILON {
                continue;
            }
            let loop_is_ccw = signed_area > 0.0;
            for index in 0..road_loop.vertices.len() {
                let start = road_loop.vertices[index];
                let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
                let dx = end.x - start.x;
                let dz = end.z - start.z;
                let length_m = dx.hypot(dz);
                if length_m <= f64::EPSILON {
                    continue;
                }
                let outward_x = if loop_is_ccw { dz } else { -dz } / length_m;
                let outward_z = if loop_is_ccw { -dx } else { dx } / length_m;
                let sample_count = ((length_m as f32 / safe_step_m).ceil() as u32).max(1);
                for sample_index in 0..=sample_count {
                    let t = f64::from(sample_index) / f64::from(sample_count);
                    let seam_x = start.x + dx * t;
                    let seam_z = start.z + dz * t;
                    let seam_height_m = start.height_m + (end.height_m - start.height_m) * t as f32;
                    let mut previous_distance_m = 0.0_f32;
                    for multiplier in TERRAIN_CDT_TIE_IN_GUIDE_RING_MULTIPLIERS {
                        let distance_m = (safe_step_m * multiplier).min(max_distance_m);
                        if distance_m <= previous_distance_m + f32::EPSILON {
                            continue;
                        }
                        previous_distance_m = distance_m;
                        let world_x = seam_x + outward_x * f64::from(distance_m);
                        let world_z = seam_z + outward_z * f64::from(distance_m);
                        let terrain_height_m = terrain
                            .sample_visual_height_world(world_x as f32, world_z as f32)
                            * crate::config::HEIGHT_SCALE;
                        let guide_height_m = Self::terrain_cdt_grade_limited_tie_in_height(
                            seam_height_m,
                            terrain_height_m,
                            distance_m,
                        );
                        Self::push_terrain_cdt_tie_in_guide_sample(
                            world_x,
                            guide_height_m,
                            world_z,
                            tie_in_guide_samples,
                            sample_keys,
                        );
                    }
                }
            }
        }
    }

    fn road_loop_uses_clean_grounded_tie_in(road_loop: &TerrainCdtRoadLoop) -> bool {
        road_loop.source_edges.is_empty()
            || road_loop
                .source_edges
                .iter()
                .all(|edge| Self::boundary_source_uses_clean_grounded_tie_in(edge.source))
    }

    fn boundary_source_uses_clean_grounded_tie_in(source: TerrainCdtRoadBoundarySource) -> bool {
        match source {
            TerrainCdtRoadBoundarySource::SpanSupportBoundary {
                edge_class,
                support_policy,
                ..
            } => {
                edge_class == TerrainCdtEdgeClass::Standard
                    && support_policy == TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan
            }
            TerrainCdtRoadBoundarySource::NodeFootprintBoundary { .. }
            | TerrainCdtRoadBoundarySource::NodeSameMaterialBoundaryHandoff { .. }
            | TerrainCdtRoadBoundarySource::SyntheticTestBoundary { .. } => true,
        }
    }

    fn terrain_cdt_road_loop_signed_area_xz(road_loop: &TerrainCdtRoadLoop) -> f64 {
        let mut area = 0.0;
        for index in 0..road_loop.vertices.len() {
            let start = road_loop.vertices[index];
            let end = road_loop.vertices[(index + 1) % road_loop.vertices.len()];
            area += start.x * end.z - end.x * start.z;
        }
        area * 0.5
    }

    fn terrain_cdt_grade_limited_tie_in_height(
        seam_height_m: f32,
        terrain_height_m: f32,
        distance_m: f32,
    ) -> f32 {
        let max_delta_m = distance_m.max(0.0) * MAX_TERRAIN_TIE_IN_SLOPE_RATIO;
        let delta_m = terrain_height_m - seam_height_m;
        if delta_m.abs() <= max_delta_m {
            terrain_height_m
        } else {
            seam_height_m + delta_m.signum() * max_delta_m
        }
    }

    fn push_terrain_cdt_tie_in_guide_sample(
        world_x: f64,
        height_m: f32,
        world_z: f64,
        tie_in_guide_samples: &mut Vec<TerrainCdtTieInGuideSample>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        if !world_x.is_finite() || !world_z.is_finite() || !height_m.is_finite() {
            return;
        }
        let key = (
            (world_x * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
            (world_z * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
        );
        if sample_keys.insert(key, ()).is_some() {
            return;
        }
        tie_in_guide_samples.push(TerrainCdtTieInGuideSample {
            vertex: TerrainCdtVertex::new(world_x, height_m, world_z),
        });
    }

    fn terrain_cdt_axis_samples(min: f32, max: f32, step_m: f32) -> Vec<f32> {
        let safe_step_m = step_m.max(f32::EPSILON);
        let mut samples = vec![min];
        let mut next = min + safe_step_m;
        while next < max - 0.001 {
            samples.push(next);
            next += safe_step_m;
        }
        if samples
            .last()
            .is_none_or(|last| (*last - max).abs() > 0.001)
        {
            samples.push(max);
        }
        samples
    }

    fn push_terrain_cdt_source_sample(
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
        source_samples: &mut Vec<TerrainCdtVertex>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let key = (
            (f64::from(world_x) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
            (f64::from(world_z) * TERRAIN_CDT_SAMPLE_KEY_SCALE).round() as i64,
        );
        if sample_keys.insert(key, ()).is_some() {
            return;
        }
        source_samples.push(TerrainCdtVertex::new(
            f64::from(world_x),
            terrain.sample_visual_height_world(world_x, world_z) * config::HEIGHT_SCALE,
            f64::from(world_z),
        ));
    }

    fn terrain_patch_sample_height_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        sample_x: usize,
        sample_z: usize,
    ) -> f32 {
        if patch.texture_width == 0 || patch.height_data.is_empty() {
            return 0.0;
        }
        let texture_x = patch
            .inner_offset_x
            .saturating_add(sample_x.min(patch.sample_width.saturating_sub(1)));
        let texture_z = patch
            .inner_offset_z
            .saturating_add(sample_z.min(patch.sample_height.saturating_sub(1)));
        let index = texture_z
            .saturating_mul(patch.texture_width)
            .saturating_add(texture_x)
            .min(patch.height_data.len().saturating_sub(1));
        patch.height_data[index] * config::HEIGHT_SCALE
    }

    fn terrain_patch_height_at_world_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        world_x: f32,
        world_z: f32,
    ) -> f32 {
        if patch.sample_width == 0 || patch.sample_height == 0 {
            return 0.0;
        }
        let local_x = ((world_x - patch.world_origin_x) / patch.world_size_x.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_width.saturating_sub(1) as f32;
        let local_z = ((world_z - patch.world_origin_z) / patch.world_size_z.max(0.001))
            .clamp(0.0, 1.0)
            * patch.sample_height.saturating_sub(1) as f32;

        let x0 = local_x.floor() as usize;
        let z0 = local_z.floor() as usize;
        let x1 = (x0 + 1).min(patch.sample_width.saturating_sub(1));
        let z1 = (z0 + 1).min(patch.sample_height.saturating_sub(1));
        let tx = local_x.fract();
        let tz = local_z.fract();

        let h00 = Self::terrain_patch_sample_height_m(patch, x0, z0);
        let h10 = Self::terrain_patch_sample_height_m(patch, x1, z0);
        let h01 = Self::terrain_patch_sample_height_m(patch, x0, z1);
        let h11 = Self::terrain_patch_sample_height_m(patch, x1, z1);
        let h0 = h00 * (1.0 - tx) + h10 * tx;
        let h1 = h01 * (1.0 - tx) + h11 * tx;
        h0 * (1.0 - tz) + h1 * tz
    }

    fn append_cdt_mesh_buffers(
        dict: &mut VarDictionary,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
        boundary_step_m: f32,
        include_debug: bool,
    ) {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let terrain_buffer_start = road_debug.then(Instant::now);
        let terrain_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.triangles,
            &mesh.terrain_triangle_sources,
            include_debug,
        );
        let mut terrain_buffers = terrain_buffers;
        Self::append_regular_terrain_mesh_outside_cdt_patch(
            &mut terrain_buffers,
            patch,
            cdt_patch,
            boundary_step_m,
        );
        let terrain_buffer_ms = terrain_buffer_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let retaining_buffer_start = road_debug.then(Instant::now);
        let retaining_buffers = Self::terrain_cdt_triangle_buffers(
            patch,
            &mesh.vertices,
            &mesh.retaining_wall_triangles,
            &mesh.retaining_wall_triangle_sources,
            include_debug,
        );
        let retaining_buffer_ms = retaining_buffer_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let terrain_vertices = terrain_buffers.vertices.len();
        let terrain_indices = terrain_buffers.indices.len();
        let retaining_vertices = retaining_buffers.vertices.len();
        let retaining_indices = retaining_buffers.indices.len();
        let terrain_emitted_faces = terrain_buffers.emitted_faces;
        let retaining_emitted_faces = retaining_buffers.emitted_faces;

        let dict_start = road_debug.then(Instant::now);
        dict.set(
            "terrain_cdt_emitted_faces",
            i64::try_from(terrain_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_cdt_retaining_wall_emitted_faces",
            i64::try_from(retaining_emitted_faces).unwrap_or(0),
        );
        dict.set(
            "terrain_mesh_vertices",
            PackedVector3Array::from_iter(terrain_buffers.vertices),
        );
        dict.set(
            "terrain_mesh_normals",
            PackedVector3Array::from_iter(terrain_buffers.normals),
        );
        dict.set(
            "terrain_mesh_uvs",
            PackedVector2Array::from_iter(terrain_buffers.uvs),
        );
        dict.set(
            "terrain_mesh_indices",
            PackedInt32Array::from_iter(terrain_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_mesh",
                &terrain_buffers.face_sources,
            );
        }
        dict.set(
            "terrain_retaining_wall_mesh_vertices",
            PackedVector3Array::from_iter(retaining_buffers.vertices),
        );
        dict.set(
            "terrain_retaining_wall_mesh_normals",
            PackedVector3Array::from_iter(retaining_buffers.normals),
        );
        dict.set(
            "terrain_retaining_wall_mesh_uvs",
            PackedVector2Array::from_iter(retaining_buffers.uvs),
        );
        dict.set(
            "terrain_retaining_wall_mesh_indices",
            PackedInt32Array::from_iter(retaining_buffers.indices),
        );
        if include_debug {
            Self::append_cdt_face_source_export(
                dict,
                "terrain_retaining_wall_mesh",
                &retaining_buffers.face_sources,
            );
        }
        let dict_ms = dict_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "terrain_cdt_mesh_buffers key=({},{}) include_debug={} terrain_vertices={} terrain_indices={} terrain_faces={} retaining_vertices={} retaining_indices={} retaining_faces={} terrain_buffer_ms={:.3} retaining_buffer_ms={:.3} dict_ms={:.3} total_ms={:.3}",
                patch.patch_x,
                patch.patch_z,
                include_debug,
                terrain_vertices,
                terrain_indices,
                terrain_emitted_faces,
                retaining_vertices,
                retaining_indices,
                retaining_emitted_faces,
                terrain_buffer_ms,
                retaining_buffer_ms,
                dict_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    fn terrain_cdt_triangle_buffers(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        vertices_source: &[TerrainCdtVertex],
        triangles: &[[usize; 3]],
        triangle_sources: &[Vec<TerrainCdtRoadBoundarySource>],
        include_debug: bool,
    ) -> TerrainCdtTriangleBufferExport {
        debug_assert_eq!(
            triangles.len(),
            triangle_sources.len(),
            "CDT triangle source sidecars must match the emitted triangle bucket"
        );
        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        let mut vertices = Vec::new();
        let mut normals = Vec::new();
        let mut uvs = Vec::new();
        let mut indices = Vec::with_capacity(triangles.len() * 3);
        let mut vertex_remap = vec![usize::MAX; vertices_source.len()];
        let mut source_export = TerrainCdtSourceExport::with_sample_capacity(triangles.len());
        let mut emitted_faces = 0usize;

        for (triangle_index, triangle) in triangles.iter().enumerate() {
            let mut source_indices = *triangle;
            let mut points = [
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[0]]),
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[1]]),
                Self::terrain_cdt_vertex_to_vector3(vertices_source[triangle[2]]),
            ];
            let mut raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            if raw_normal.length_squared() <= 0.000_001 {
                continue;
            }
            if raw_normal.y < 0.0 {
                source_indices.swap(1, 2);
                points.swap(1, 2);
                raw_normal = (points[1] - points[0]).cross(points[2] - points[0]);
            }
            let normal = raw_normal.normalized();
            emitted_faces += 1;
            if include_debug {
                let triangle_face_sources = triangle_sources
                    .get(triangle_index)
                    .map_or(&[][..], Vec::as_slice);
                source_export.push_sources(triangle_face_sources);
            }
            for source_index in source_indices {
                let mut export_index = vertex_remap[source_index];
                if export_index == usize::MAX {
                    let point = Self::terrain_cdt_vertex_to_vector3(vertices_source[source_index]);
                    export_index = vertices.len();
                    vertex_remap[source_index] = export_index;
                    vertices.push(Vector3::new(
                        point.x - center_x,
                        point.y,
                        point.z - center_z,
                    ));
                    normals.push(Vector3::new(0.0, 0.0, 0.0));
                    uvs.push(Vector2::new(
                        ((point.x - patch.world_origin_x) / patch.world_size_x.max(0.001))
                            .clamp(0.0, 1.0),
                        ((point.z - patch.world_origin_z) / patch.world_size_z.max(0.001))
                            .clamp(0.0, 1.0),
                    ));
                }
                normals[export_index] = normals[export_index] + normal;
                indices.push(i32::try_from(export_index).unwrap_or(i32::MAX));
            }
        }

        for normal in &mut normals {
            if normal.length_squared() <= 0.000_001 {
                *normal = Vector3::new(0.0, 1.0, 0.0);
            } else {
                *normal = normal.normalized();
            }
        }

        TerrainCdtTriangleBufferExport {
            vertices,
            normals,
            uvs,
            indices,
            face_sources: source_export,
            emitted_faces,
        }
    }

    fn append_triangle_buffer_export(
        target: &mut TerrainCdtTriangleBufferExport,
        source: TerrainCdtTriangleBufferExport,
    ) {
        let vertex_offset = i32::try_from(target.vertices.len()).unwrap_or(i32::MAX);
        target.vertices.extend(source.vertices);
        target.normals.extend(source.normals);
        target.uvs.extend(source.uvs);
        target.indices.extend(
            source
                .indices
                .into_iter()
                .map(|index| index.saturating_add(vertex_offset)),
        );
        target
            .face_sources
            .counts
            .extend(source.face_sources.counts);
        target
            .face_sources
            .labels
            .extend(source.face_sources.labels);
        target
            .face_sources
            .kind_codes
            .extend(source.face_sources.kind_codes);
        target
            .face_sources
            .primary_ids
            .extend(source.face_sources.primary_ids);
        target
            .face_sources
            .node_kind_codes
            .extend(source.face_sources.node_kind_codes);
        target
            .face_sources
            .edge_class_codes
            .extend(source.face_sources.edge_class_codes);
        target
            .face_sources
            .owner_kinds
            .extend(source.face_sources.owner_kinds);
        target
            .face_sources
            .owner_indices
            .extend(source.face_sources.owner_indices);
        target
            .face_sources
            .support_policies
            .extend(source.face_sources.support_policies);
        target.face_sources.roles.extend(source.face_sources.roles);
        target
            .face_sources
            .section_ranges
            .extend(source.face_sources.section_ranges);
        target
            .face_sources
            .s_ranges
            .extend(source.face_sources.s_ranges);
        target.emitted_faces += source.emitted_faces;
    }

    fn append_regular_terrain_mesh_outside_cdt_patch(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        boundary_step_m: f32,
    ) {
        let windows = Self::terrain_cdt_window_bounds(patch, cdt_patch, boundary_step_m)
            .into_iter()
            .collect::<Vec<_>>();
        Self::append_regular_terrain_mesh_outside_cdt_windows(export, patch, &windows);
    }

    fn append_regular_terrain_mesh_outside_cdt_windows(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        windows: &[TerrainCdtWindowBounds],
    ) {
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        if windows.is_empty() {
            Self::append_regular_terrain_grid_region(
                export,
                patch,
                patch_min_x,
                patch_min_z,
                patch_max_x,
                patch_max_z,
                Self::regular_terrain_mesh_step_m(patch),
            );
            return;
        }
        let step_m = Self::regular_terrain_mesh_step_m(patch);

        let mut x_lines = vec![patch_min_x, patch_max_x];
        let mut z_lines = vec![patch_min_z, patch_max_z];
        for window in windows {
            x_lines.extend([window.min_x, window.max_x]);
            z_lines.extend([window.min_z, window.max_z]);
        }
        Self::sort_dedup_axis_lines(&mut x_lines);
        Self::sort_dedup_axis_lines(&mut z_lines);
        for x_pair in x_lines.windows(2) {
            let min_x = x_pair[0];
            let max_x = x_pair[1];
            if max_x <= min_x + 0.001 {
                continue;
            }
            for z_pair in z_lines.windows(2) {
                let min_z = z_pair[0];
                let max_z = z_pair[1];
                if max_z <= min_z + 0.001 {
                    continue;
                }
                let mid_x = (min_x + max_x) * 0.5;
                let mid_z = (min_z + max_z) * 0.5;
                if windows.iter().any(|window| {
                    mid_x >= window.min_x
                        && mid_x <= window.max_x
                        && mid_z >= window.min_z
                        && mid_z <= window.max_z
                }) {
                    continue;
                }
                let mut xs =
                    Self::regular_terrain_axis_samples_aligned(min_x, max_x, step_m, patch_min_x);
                let mut zs =
                    Self::regular_terrain_axis_samples_aligned(min_z, max_z, step_m, patch_min_z);
                Self::refine_regular_terrain_axes_for_cdt_window_sides(
                    &mut xs, &mut zs, min_x, min_z, max_x, max_z, windows,
                );
                Self::append_regular_terrain_grid_region_with_axes(export, patch, &xs, &zs);
            }
        }
    }

    fn terrain_cdt_window_bounds(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        cdt_patch: TerrainCdtPatch,
        boundary_step_m: f32,
    ) -> Option<TerrainCdtWindowBounds> {
        let patch_min_x = patch.world_origin_x;
        let patch_min_z = patch.world_origin_z;
        let patch_max_x = patch.world_origin_x + patch.world_size_x;
        let patch_max_z = patch.world_origin_z + patch.world_size_z;
        let min_x = (cdt_patch.min_x as f32).clamp(patch_min_x, patch_max_x);
        let min_z = (cdt_patch.min_z as f32).clamp(patch_min_z, patch_max_z);
        let max_x = (cdt_patch.max_x as f32).clamp(patch_min_x, patch_max_x);
        let max_z = (cdt_patch.max_z as f32).clamp(patch_min_z, patch_max_z);
        if max_x <= min_x + 0.001 || max_z <= min_z + 0.001 {
            return None;
        }
        Some(TerrainCdtWindowBounds {
            min_x,
            min_z,
            max_x,
            max_z,
            boundary_step_m: boundary_step_m.max(f32::EPSILON),
        })
    }

    fn refine_regular_terrain_axes_for_cdt_window_sides(
        xs: &mut Vec<f32>,
        zs: &mut Vec<f32>,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        windows: &[TerrainCdtWindowBounds],
    ) {
        for window in windows {
            let z_overlap_min = min_z.max(window.min_z);
            let z_overlap_max = max_z.min(window.max_z);
            if z_overlap_max > z_overlap_min + 0.001
                && (Self::axis_lines_touch(max_x, window.min_x)
                    || Self::axis_lines_touch(min_x, window.max_x))
            {
                Self::extend_axis_samples(zs, z_overlap_min, z_overlap_max, window.boundary_step_m);
            }

            let x_overlap_min = min_x.max(window.min_x);
            let x_overlap_max = max_x.min(window.max_x);
            if x_overlap_max > x_overlap_min + 0.001
                && (Self::axis_lines_touch(max_z, window.min_z)
                    || Self::axis_lines_touch(min_z, window.max_z))
            {
                Self::extend_axis_samples(xs, x_overlap_min, x_overlap_max, window.boundary_step_m);
            }
        }
    }

    fn axis_lines_touch(left: f32, right: f32) -> bool {
        (left - right).abs() <= 0.001
    }

    fn extend_axis_samples(samples: &mut Vec<f32>, min: f32, max: f32, step_m: f32) {
        samples.extend(Self::terrain_cdt_axis_samples(min, max, step_m));
        Self::sort_dedup_axis_lines(samples);
    }

    fn sort_dedup_axis_lines(values: &mut Vec<f32>) {
        values.sort_by(|left, right| left.total_cmp(right));
        values.dedup_by(|left, right| (*left - *right).abs() <= 0.001);
    }

    fn regular_terrain_mesh_step_m(
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> f32 {
        let sample_step_x = patch.world_size_x / patch.sample_width.saturating_sub(1).max(1) as f32;
        let sample_step_z =
            patch.world_size_z / patch.sample_height.saturating_sub(1).max(1) as f32;
        sample_step_x
            .max(sample_step_z)
            .max(TERRAIN_CDT_FAR_SAMPLE_MIN_STEP_M)
    }

    fn regular_terrain_axis_samples_aligned(
        min: f32,
        max: f32,
        step_m: f32,
        anchor: f32,
    ) -> Vec<f32> {
        let safe_step_m = step_m.max(f32::EPSILON);
        let mut samples = vec![min];
        let first = ((min - anchor) / safe_step_m).ceil() as i64;
        let last = ((max - anchor) / safe_step_m).floor() as i64;
        for index in first..=last {
            let sample = anchor + index as f32 * safe_step_m;
            if sample > min + 0.001 && sample < max - 0.001 {
                samples.push(sample);
            }
        }
        if samples
            .last()
            .is_none_or(|last| (*last - max).abs() > 0.001)
        {
            samples.push(max);
        }
        Self::sort_dedup_axis_lines(&mut samples);
        samples
    }

    fn append_regular_terrain_grid_region(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        step_m: f32,
    ) {
        if max_x <= min_x + 0.001 || max_z <= min_z + 0.001 {
            return;
        }

        let xs =
            Self::regular_terrain_axis_samples_aligned(min_x, max_x, step_m, patch.world_origin_x);
        let zs =
            Self::regular_terrain_axis_samples_aligned(min_z, max_z, step_m, patch.world_origin_z);
        Self::append_regular_terrain_grid_region_with_axes(export, patch, &xs, &zs);
    }

    fn append_regular_terrain_grid_region_with_axes(
        export: &mut TerrainCdtTriangleBufferExport,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
        xs: &[f32],
        zs: &[f32],
    ) {
        if xs.len() < 2 || zs.len() < 2 {
            return;
        }

        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        let base_index = export.vertices.len();
        for &z in zs {
            for &x in xs {
                export.vertices.push(Vector3::new(
                    x - center_x,
                    Self::terrain_patch_height_at_world_m(patch, x, z),
                    z - center_z,
                ));
                export.normals.push(Vector3::ZERO);
                export.uvs.push(Vector2::new(
                    ((x - patch.world_origin_x) / patch.world_size_x.max(0.001)).clamp(0.0, 1.0),
                    ((z - patch.world_origin_z) / patch.world_size_z.max(0.001)).clamp(0.0, 1.0),
                ));
            }
        }

        let width = xs.len();
        for z_index in 0..zs.len() - 1 {
            for x_index in 0..xs.len() - 1 {
                let i00 = base_index + z_index * width + x_index;
                let i10 = i00 + 1;
                let i01 = i00 + width;
                let i11 = i01 + 1;
                Self::append_regular_terrain_triangle(export, [i00, i11, i10]);
                Self::append_regular_terrain_triangle(export, [i00, i01, i11]);
            }
        }

        for normal in &mut export.normals[base_index..] {
            if normal.length_squared() <= 0.000_001 {
                *normal = Vector3::UP;
            } else {
                *normal = normal.normalized();
            }
        }
    }

    fn append_regular_terrain_triangle(
        export: &mut TerrainCdtTriangleBufferExport,
        triangle: [usize; 3],
    ) {
        let points = [
            export.vertices[triangle[0]],
            export.vertices[triangle[1]],
            export.vertices[triangle[2]],
        ];
        let normal = (points[1] - points[0]).cross(points[2] - points[0]);
        if normal.length_squared() <= 0.000_001 {
            return;
        }
        let normal = normal.normalized();
        for index in triangle {
            export.normals[index] = export.normals[index] + normal;
            export
                .indices
                .push(i32::try_from(index).unwrap_or(i32::MAX));
        }
        export.emitted_faces += 1;
        export.face_sources.push_sources(&[]);
    }

    fn append_cdt_road_seam_face_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut centroids = Vec::with_capacity(mesh.road_seam_face_samples.len());
        let mut bounds = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.road_seam_face_samples.len() * 2);
        let mut vertices = Vec::with_capacity(mesh.road_seam_face_samples.len() * 3);
        let mut kinds = Vec::with_capacity(mesh.road_seam_face_samples.len());
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.road_seam_face_samples.len());
        for sample in &mesh.road_seam_face_samples {
            centroids.push(Self::terrain_cdt_vertex_to_vector3(sample.centroid));
            bounds.push(Vector3::new(
                sample.min_x as f32,
                sample.min_y_m,
                sample.min_z as f32,
            ));
            bounds.push(Vector3::new(
                sample.max_x as f32,
                sample.max_y_m,
                sample.max_z as f32,
            ));
            metrics.push(sample.max_y_delta_m);
            metrics.push(sample.max_slope_ratio);
            kinds.push(sample.kind.debug_code());
            source_export.push_sources(&sample.sources);
            vertices.extend(
                sample
                    .vertices
                    .into_iter()
                    .map(Self::terrain_cdt_vertex_to_vector3),
            );
        }
        dict.set(
            "terrain_cdt_road_seam_sample_centroids",
            PackedVector3Array::from_iter(centroids),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_bounds",
            PackedVector3Array::from_iter(bounds),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_vertices",
            PackedVector3Array::from_iter(vertices),
        );
        dict.set(
            "terrain_cdt_road_seam_sample_kinds",
            PackedInt32Array::from_iter(kinds),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_road_seam", &source_export);
    }

    fn append_cdt_retaining_wall_face_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut centroids = Vec::with_capacity(mesh.retaining_wall_face_samples.len());
        let mut bounds = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 2);
        let mut vertices = Vec::with_capacity(mesh.retaining_wall_face_samples.len() * 3);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.retaining_wall_face_samples.len());
        for sample in &mesh.retaining_wall_face_samples {
            centroids.push(Self::terrain_cdt_vertex_to_vector3(sample.centroid));
            bounds.push(Vector3::new(
                sample.min_x as f32,
                sample.min_y_m,
                sample.min_z as f32,
            ));
            bounds.push(Vector3::new(
                sample.max_x as f32,
                sample.max_y_m,
                sample.max_z as f32,
            ));
            metrics.push(sample.max_y_delta_m);
            metrics.push(sample.max_slope_ratio);
            source_export.push_sources(&sample.sources);
            vertices.extend(
                sample
                    .vertices
                    .into_iter()
                    .map(Self::terrain_cdt_vertex_to_vector3),
            );
        }
        dict.set(
            "terrain_cdt_retaining_wall_sample_centroids",
            PackedVector3Array::from_iter(centroids),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_bounds",
            PackedVector3Array::from_iter(bounds),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_retaining_wall_sample_vertices",
            PackedVector3Array::from_iter(vertices),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_retaining_wall", &source_export);
    }

    fn append_cdt_tie_in_widened_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut points = Vec::with_capacity(mesh.tie_in_widened_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.tie_in_widened_samples.len() * 4);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.tie_in_widened_samples.len());
        for sample in &mesh.tie_in_widened_samples {
            points.push(Self::terrain_cdt_vertex_to_vector3(sample.source_sample));
            points.push(Self::terrain_cdt_vertex_to_vector3(sample.seam_point));
            source_export.push_sources(&[sample.seam_source]);
            metrics.push(sample.distance_m);
            metrics.push(sample.required_distance_m);
            metrics.push(sample.height_delta_m);
            metrics.push(sample.slope_ratio);
        }
        dict.set(
            "terrain_cdt_tie_in_widened_sample_points",
            PackedVector3Array::from_iter(points),
        );
        dict.set(
            "terrain_cdt_tie_in_widened_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_tie_in_widened", &source_export);
    }

    fn append_cdt_invalid_constraint_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut edges = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 2);
        let mut metadata = Vec::with_capacity(mesh.invalid_constraint_samples.len() * 4);
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.invalid_constraint_samples.len());
        for sample in &mesh.invalid_constraint_samples {
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.start));
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.end));
            metadata.push(if sample.road_owned { 1 } else { 0 });
            metadata.push(i32::try_from(sample.stable_piece_id).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_loop_index).unwrap_or(i32::MAX));
            metadata.push(i32::try_from(sample.local_edge_index).unwrap_or(i32::MAX));
            if let Some(source) = sample.source {
                source_export.push_sources(&[source]);
            } else {
                source_export.push_sources(&[]);
            }
        }
        dict.set(
            "terrain_cdt_invalid_constraint_sample_edges",
            PackedVector3Array::from_iter(edges),
        );
        dict.set(
            "terrain_cdt_invalid_constraint_sample_metadata",
            PackedInt32Array::from_iter(metadata),
        );
        Self::append_cdt_sample_source_export(
            dict,
            "terrain_cdt_invalid_constraint",
            &source_export,
        );
    }

    fn append_cdt_seam_quality_samples(
        dict: &mut VarDictionary,
        mesh: &crate::simulation::terrain::cdt::TerrainCdtMesh,
    ) {
        let mut edges = Vec::with_capacity(mesh.seam_quality_samples.len() * 2);
        let mut metrics = Vec::with_capacity(mesh.seam_quality_samples.len() * 2);
        let mut kinds = Vec::with_capacity(mesh.seam_quality_samples.len());
        let mut source_export =
            TerrainCdtSourceExport::with_sample_capacity(mesh.seam_quality_samples.len());
        for sample in &mesh.seam_quality_samples {
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.start));
            edges.push(Self::terrain_cdt_vertex_to_vector3(sample.end));
            metrics.push(sample.length_m);
            metrics.push(sample.height_delta_m);
            kinds.push(sample.kind.debug_code());
            source_export.push_sources(&[sample.source]);
        }
        dict.set(
            "terrain_cdt_seam_quality_sample_edges",
            PackedVector3Array::from_iter(edges),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_metrics",
            PackedFloat32Array::from_iter(metrics),
        );
        dict.set(
            "terrain_cdt_seam_quality_sample_kinds",
            PackedInt32Array::from_iter(kinds),
        );
        Self::append_cdt_sample_source_export(dict, "terrain_cdt_seam_quality", &source_export);
    }

    fn terrain_cdt_vertex_to_vector3(vertex: TerrainCdtVertex) -> Vector3 {
        Vector3::new(vertex.x as f32, vertex.height_m, vertex.z as f32)
    }

    fn terrain_cdt_error_label(err: &TerrainCdtError) -> &'static str {
        match err {
            TerrainCdtError::InvalidPatch => "invalid_patch",
            TerrainCdtError::MissingRoadBoundarySource => "missing_road_boundary_source",
            TerrainCdtError::TriangulationFailed => "triangulation_failed",
        }
    }

    fn water_patch_dict(patch: &crate::simulation::water::WaterPatchSnapshot) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::try_from(patch.patch_x).unwrap_or(0));
        dict.set("patch_z", i64::try_from(patch.patch_z).unwrap_or(0));
        dict.set(
            "sample_width",
            i64::try_from(patch.sample_width).unwrap_or(0),
        );
        dict.set(
            "sample_height",
            i64::try_from(patch.sample_height).unwrap_or(0),
        );
        dict.set(
            "texture_width",
            i64::try_from(patch.texture_width).unwrap_or(0),
        );
        dict.set(
            "texture_height",
            i64::try_from(patch.texture_height).unwrap_or(0),
        );
        dict.set(
            "inner_offset_x",
            i64::try_from(patch.inner_offset_x).unwrap_or(0),
        );
        dict.set(
            "inner_offset_z",
            i64::try_from(patch.inner_offset_z).unwrap_or(0),
        );
        dict.set("world_origin_x", f64::from(patch.world_origin_x));
        dict.set("world_origin_z", f64::from(patch.world_origin_z));
        dict.set("world_size_x", f64::from(patch.world_size_x));
        dict.set("world_size_z", f64::from(patch.world_size_z));
        dict.set(
            "depth_nonzero_count",
            i64::try_from(patch.depth_nonzero_count).unwrap_or(0),
        );
        dict.set(
            "velocity_nonzero_count",
            i64::try_from(patch.velocity_nonzero_count).unwrap_or(0),
        );
        dict.set(
            "depth_data",
            PackedFloat32Array::from_iter(patch.depth_data.iter().copied()),
        );
        dict.set(
            "velocity_data",
            PackedFloat32Array::from_iter(patch.velocity_data.iter().copied()),
        );
        dict
    }

    fn water_patch_layer_debug_dict(
        stats: &crate::simulation::water::WaterPatchLayerStats,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "total_samples",
            i64::try_from(stats.total_samples).unwrap_or(0),
        );
        dict.set(
            "baseline_nonzero",
            i64::try_from(stats.baseline_nonzero).unwrap_or(0),
        );
        dict.set("baseline_max", f64::from(stats.baseline_max));
        dict.set("baseline_sum", f64::from(stats.baseline_sum));
        dict.set(
            "dynamic_nonzero",
            i64::try_from(stats.dynamic_nonzero).unwrap_or(0),
        );
        dict.set("dynamic_max", f64::from(stats.dynamic_max));
        dict.set("dynamic_sum", f64::from(stats.dynamic_sum));
        dict.set(
            "combined_nonzero",
            i64::try_from(stats.combined_nonzero).unwrap_or(0),
        );
        dict.set("combined_max", f64::from(stats.combined_max));
        dict.set("combined_sum", f64::from(stats.combined_sum));
        dict.set(
            "velocity_nonzero",
            i64::try_from(stats.velocity_nonzero).unwrap_or(0),
        );
        dict.set("velocity_max", f64::from(stats.velocity_max));
        dict.set("velocity_sum", f64::from(stats.velocity_sum));
        dict.set(
            "source_count_in_patch",
            i64::try_from(stats.source_count_in_patch).unwrap_or(0),
        );
        dict.set("source_rate_sum", f64::from(stats.source_rate_sum));
        dict.set("source_rate_abs_sum", f64::from(stats.source_rate_abs_sum));
        dict.set(
            "source_count_total",
            i64::try_from(stats.source_count_total).unwrap_or(0),
        );
        dict
    }

    fn authored_water_patch_fill_debug_dict(
        fill: &crate::nodes::sim::core::AuthoredWaterPatchFillDebug,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set(
            "kind",
            GString::from(match fill.kind {
                WorldWaterFillKind::Lake => "lake",
                WorldWaterFillKind::OpenWater => "open_water",
            }),
        );
        dict.set("fill_index", i64::from(fill.fill_index));
        dict.set("preview", fill.preview);
        dict.set("world_x", f64::from(fill.world_x));
        dict.set("world_z", f64::from(fill.world_z));
        dict.set("surface_elevation_m", f64::from(fill.surface_elevation_m));
        dict.set(
            "filled_cells",
            i64::try_from(fill.filled_cells).unwrap_or(0),
        );
        dict.set("touches_world_edge", fill.touches_world_edge);
        dict.set(
            "patch_nonzero_samples",
            i64::try_from(fill.patch_nonzero_samples).unwrap_or(0),
        );
        dict.set("patch_max_depth_m", f64::from(fill.patch_max_depth_m));
        dict.set("patch_sum_depth_m", f64::from(fill.patch_sum_depth_m));
        dict
    }
}

#[godot_api]
impl SimulationNode {
    // ── Environment ──

    /// Returns the pollution image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_pollution_image_data(&self) -> PackedByteArray {
        self.lock_core().get_pollution_image_data_internal()
    }

    /// Returns the noise image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_noise_image_data(&self) -> PackedByteArray {
        self.lock_core().get_noise_image_data_internal()
    }

    /// Returns the desirability image data as a PackedByteArray (RGBA8).
    #[func]
    pub fn get_desirability_image_data(&self) -> PackedByteArray {
        self.lock_core().get_desirability_image_data_internal()
    }

    // ── System ──

    /// Undoes the last action.
    #[func]
    pub fn undo_action(&mut self) -> bool {
        let changed = self.lock_core().undo_action_internal();
        if changed {
            self.refresh_snapshot_from_core();
        }
        changed
    }

    // ── Terrain & Water ──

    /// Sculpts the terrain heightmap.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Begins one batched world-editor terrain stroke.
    #[func]
    pub fn begin_terrain_stroke(&mut self) {
        self.lock_core().start_terrain_stroke_internal();
    }

    /// Finalizes one batched world-editor terrain stroke.
    #[func]
    pub fn end_terrain_stroke(&mut self) -> bool {
        let (ended, terrain_dirty, water_dirty, network_dirty) = {
            let mut core = self.lock_core();
            let ended = core.end_terrain_stroke_internal();
            (
                ended,
                core.terrain_dirty,
                core.water_dirty,
                core.network_dirty,
            )
        };
        if ended {
            let mut snapshot = self.snapshot.write().unwrap();
            snapshot.terrain_dirty = terrain_dirty;
            snapshot.water_dirty = water_dirty;
            snapshot.network_dirty = network_dirty;
        }
        ended
    }

    /// Applies one batched terrain sculpt step during an active editor stroke.
    #[func]
    pub fn sculpt_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Moves terrain toward a clicked rendered heightmap level.
    #[func]
    pub fn level_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.lock_core()
            .level_terrain_internal(pos, radius, target_height_m, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-level step during an active editor stroke.
    #[func]
    pub fn level_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.lock_core()
            .level_terrain_stroke_step_internal(pos, radius, target_height_m, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Smooths terrain toward the local neighborhood average.
    #[func]
    pub fn smooth_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-smooth step during an active editor stroke.
    #[func]
    pub fn smooth_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Moves terrain toward a slope defined by two clicked rendered anchor points.
    #[func]
    pub fn slope_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.lock_core().slope_terrain_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-slope step during an active editor stroke.
    #[func]
    pub fn slope_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.lock_core().slope_terrain_stroke_step_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Adds a volume of water at a specific grid position.
    #[func]
    pub fn add_water(&mut self, pos: Vector2, amount: f32) {
        self.lock_core().add_water_internal(pos, amount);
    }

    /// Adds a continuous water source at a specific grid position.
    #[func]
    pub fn add_water_source(&mut self, pos: Vector2, rate_add: f32) {
        self.lock_core().add_water_source_internal(pos, rate_add);
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool {
        self.snapshot.read().unwrap().terrain_dirty
    }

    /// Returns whether the water mesh needs rebuilding.
    #[func]
    pub fn is_water_dirty(&self) -> bool {
        self.snapshot.read().unwrap().water_dirty
    }

    /// Clears the terrain dirty flag.
    #[func]
    pub fn clear_terrain_dirty(&mut self) {
        let (preview_context, road_query_snapshot) = {
            let mut core = self.lock_core();
            core.terrain_dirty = false;
            core.heightmap.clear_dirty_render_patches();
            core.refined_terrain_patch_cache.clear();
            (
                RoadPreviewWorkerContext::from_core(&core),
                RoadToolQuerySnapshot::from_core(&core),
            )
        };
        *self.road_preview_context.write().unwrap() = preview_context;
        *self.road_tool_query_snapshot.write().unwrap() = road_query_snapshot;
        self.snapshot.write().unwrap().terrain_dirty = false;
    }

    /// Clears the water dirty flag.
    #[func]
    pub fn clear_water_dirty(&mut self) {
        self.lock_core().water_dirty = false;
        self.lock_core().watermap.clear_dirty_render_patches();
        self.snapshot.write().unwrap().water_dirty = false;
    }

    /// Returns true if the road/rail network was mutated and the visual mesh needs a rebuild.
    ///
    /// `NetworkRenderer._process` polls this each frame. The flag stays `true` until
    /// `clear_network_dirty()` is called by GDScript after the refresh is complete,
    /// matching the same explicit-clear pattern used by `terrain_dirty` and `water_dirty`.
    #[func]
    pub fn is_network_dirty(&self) -> bool {
        self.snapshot.read().unwrap().network_dirty
    }

    /// Returns true when the background sim thread currently owns the core mutex.
    #[func]
    pub fn is_sim_core_busy(&self) -> bool {
        match self.core.try_lock() {
            Ok(_) => false,
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(std::sync::TryLockError::Poisoned(_)) => false,
        }
    }

    /// Clears the network-dirty flag after `NetworkRenderer` has rebuilt the road/rail mesh.
    #[func]
    pub fn clear_network_dirty(&mut self) {
        self.lock_core().network_dirty = false;
        self.snapshot.write().unwrap().network_dirty = false;
    }

    /// Returns render-patch layout metadata shared by terrain and water renderers.
    #[func]
    pub fn get_terrain_patch_layout(&self) -> VarDictionary {
        let core = self.lock_core();
        let mut dict = VarDictionary::new();
        dict.set(
            "patch_cols",
            i64::try_from(core.heightmap.render_patch_cols()).unwrap_or(0),
        );
        dict.set(
            "patch_rows",
            i64::try_from(core.heightmap.render_patch_rows()).unwrap_or(0),
        );
        dict.set(
            "patch_interval_cells",
            i64::try_from(core.heightmap.render_patch_interval_cells()).unwrap_or(0),
        );
        dict.set("terrain_cell_m", f64::from(core.config.terrain_cell_m));
        dict.set("chunk_span_m", f64::from(core.heightmap.chunk_span_m()));
        dict
    }

    /// Returns the currently dirty terrain render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_terrain_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut patches: Vec<(usize, usize)> = core
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        patches.sort_unstable();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in patches {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns one visual-terrain render patch, including its one-sample border ring.
    #[func]
    pub fn get_terrain_patch(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::terrain_patch_dict(&patch)
    }

    /// Returns one visible-terrain render patch resampled at a finer render step.
    #[func]
    pub fn get_refined_terrain_patch(
        &self,
        patch_x: i32,
        patch_z: i32,
        render_step_m: f32,
    ) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let cache_key = Self::refined_patch_cache_key(patch_x, patch_z, render_step_m);
        if let Some(cached) = core.refined_terrain_patch_cache.get(&cache_key) {
            let road_debug = crate::debug::category_enabled("road");
            let total_start = road_debug.then(Instant::now);
            let dict = Self::cached_refined_terrain_patch_dict(cached, false);
            if road_debug {
                debug_log!(
                    "road",
                    "refined_patch_cache_hit key=({},{}) render_step_mm={} windows={} reused_windows={} input_road_loops={} source_samples={} cdt_ms={:.3} total_ms={:.3}",
                    patch_x,
                    patch_z,
                    cache_key.render_step_mm,
                    cached.windows.len(),
                    cached.reused_windows,
                    cached.input_road_loops,
                    cached.input_source_samples,
                    cached.cdt_ms,
                    total_start
                        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or(0.0)
                );
            }
            return dict;
        }
        if crate::debug::category_enabled("road") {
            debug_log!(
                "road",
                "refined_patch_cache_miss key=({},{}) render_step_mm={}",
                patch_x,
                patch_z,
                cache_key.render_step_mm
            );
        }
        Self::refined_terrain_patch_dict(&core, patch_x, patch_z, render_step_m, false)
    }

    /// Returns a refined terrain patch with CDT provenance sidecars for diagnostics.
    #[func]
    pub fn get_refined_terrain_patch_debug(
        &self,
        patch_x: i32,
        patch_z: i32,
        render_step_m: f32,
    ) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        Self::refined_terrain_patch_dict(&core, patch_x, patch_z, render_step_m, true)
    }

    /// Returns the terrain-border perimeter loop as world-space top positions.
    #[func]
    pub fn get_terrain_border_loop(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.lock_core().heightmap.border_loop_positions())
    }

    /// Returns the currently dirty water render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_water_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut patches: Vec<(usize, usize)> = core
            .watermap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        patches.sort_unstable();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in patches {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns one visible-water render patch, including its one-sample border ring.
    #[func]
    pub fn get_water_patch(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(patch) = core.watermap.visible_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        let mut dict = Self::water_patch_dict(&patch);
        Self::append_road_clip_loops_for_bounds(
            &mut dict,
            &core,
            patch.world_origin_x,
            patch.world_origin_z,
            patch.world_origin_x + patch.world_size_x,
            patch.world_origin_z + patch.world_size_z,
        );
        dict
    }

    /// Returns road-clip metadata for a cached water patch without exporting water textures.
    #[func]
    pub fn get_water_patch_road_clip(
        &self,
        patch_x: i32,
        patch_z: i32,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> VarDictionary {
        let mut dict = VarDictionary::new();
        dict.set("patch_x", i64::from(patch_x));
        dict.set("patch_z", i64::from(patch_z));
        let core = self.lock_core();
        Self::append_road_clip_loops_for_bounds(&mut dict, &core, min_x, min_z, max_x, max_z);
        dict
    }

    /// Returns debug-only baseline/dynamic/combined water stats for one render patch.
    #[func]
    pub fn get_water_patch_debug(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(stats) = core.watermap.visible_patch_layer_stats(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::water_patch_layer_debug_dict(&stats)
    }

    /// Returns debug-only authored baseline-water fill contributors for one render patch.
    #[func]
    pub fn get_water_patch_authored_fill_debug(&self, patch_x: i32, patch_z: i32) -> VarArray {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarArray::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarArray::new();
        };
        let core = self.lock_core();
        let contributors = core.authored_water_patch_fill_debug_internal(patch_x, patch_z);
        let mut array = VarArray::new();
        for contributor in contributors {
            let dict = Self::authored_water_patch_fill_debug_dict(&contributor);
            array.push(&dict.to_variant());
        }
        array
    }

    /// Returns the visible water depth along the world-edge perimeter loop.
    #[func]
    pub fn get_water_border_depths(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(self.lock_core().watermap.border_loop_depths())
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        self.get_heightmap_size_internal()
    }

    /// Returns the terrain world extent in metres.
    #[func]
    pub fn get_terrain_world_size(&self) -> Vector2 {
        self.snapshot.read().unwrap().terrain_world_size
    }

    // ── Zoning ──

    /// Returns the validated runtime zoning-profile registry for Godot tools and UI.
    #[func]
    pub fn get_zone_profiles(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for profile in core.zoning.profiles.profiles() {
            let mut dict = VarDictionary::new();
            dict.set("id", GString::from(profile.id.as_str()));
            dict.set("runtime_id", i64::from(profile.runtime_id));
            dict.set("display_name", GString::from(profile.display_name.as_str()));
            dict.set("ui_order", i64::from(profile.ui_order));
            dict.set("zone_type", GString::from(profile.zone_type.as_str()));
            dict.set("density", GString::from(profile.density.as_str()));
            dict.set(
                "ui_color",
                GString::from(
                    format!(
                        "#{:02X}{:02X}{:02X}",
                        profile.ui_color_rgb[0], profile.ui_color_rgb[1], profile.ui_color_rgb[2]
                    )
                    .as_str(),
                ),
            );
            dict.set("ui_icon", GString::from(profile.ui_icon.as_str()));
            dict.set(
                "ui_description",
                GString::from(profile.ui_description.as_str()),
            );
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Creates or rezones a road-aligned zoning parcel at one world-space point.
    #[func]
    pub fn apply_zoning_parcel_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return false;
        };
        let result = core.zoning.place_or_rezone_parcel_at(
            world_x,
            world_z,
            runtime_id,
            frontage_m,
            depth_m,
            &core.region_graph,
        );
        match result {
            Ok(_) => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            Err(_) => false,
        }
    }

    /// Returns preview geometry for a road-aligned zoning parcel.
    #[func]
    pub fn get_zoning_parcel_preview(
        &self,
        world_x: f32,
        world_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarDictionary::new();
        };
        if let Some(geometry) = core.zoning.parcel_geometry_at(world_x, world_z) {
            return zoning_parcel_geometry_dict(&core, &geometry, runtime_id, false, 0);
        }
        let Ok(geometry) = core.zoning.preview_parcel_at(
            world_x,
            world_z,
            frontage_m,
            depth_m,
            &core.region_graph,
        ) else {
            return VarDictionary::new();
        };
        zoning_parcel_geometry_dict(&core, &geometry, runtime_id, false, 0)
    }

    /// Returns true when one world-space point is inside an authored zoning parcel.
    #[func]
    pub fn has_zoning_parcel_at(&self, world_x: f32, world_z: f32) -> bool {
        self.lock_core().zoning.has_parcel_at(world_x, world_z)
    }

    /// Returns the parcel profile id at a world-space point, or `-1` when no parcel is present.
    #[func]
    pub fn get_zoning_parcel_profile_runtime_id_at(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core()
            .zoning
            .parcel_profile_runtime_id_at(world_x, world_z)
            .map(i32::from)
            .unwrap_or(-1)
    }

    /// Returns preview geometry for an all-or-nothing road-side parcel drag run.
    #[func]
    pub fn get_zoning_parcel_drag_preview(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> VarArray {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarArray::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarArray::new();
        };
        let Ok(geometries) = core.zoning.preview_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        ) else {
            return VarArray::new();
        };
        zoning_parcel_geometries_array(&core, &geometries, runtime_id)
    }

    /// Returns packed preview geometry for an all-or-nothing road-side parcel drag run.
    #[func]
    pub fn get_zoning_parcel_drag_preview_packed(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return VarDictionary::new();
        };
        let Ok(geometries) = core.zoning.preview_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        ) else {
            return VarDictionary::new();
        };
        zoning_parcel_geometries_packed_dict(&core, &geometries, runtime_id)
    }

    /// Returns packed preview geometry for existing parcels touched by a zoning paint stroke.
    #[func]
    pub fn get_zoning_parcel_rezone_drag_preview_packed(
        &self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
    ) -> VarDictionary {
        let core = self.lock_core();
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return VarDictionary::new();
        };
        if core
            .zoning
            .profiles
            .profile_by_runtime_id(runtime_id)
            .is_none()
            && runtime_id != 0
        {
            return VarDictionary::new();
        }
        let geometries = core
            .zoning
            .preview_rezone_stroke(start_x, start_z, end_x, end_z);
        if geometries.is_empty() {
            return VarDictionary::new();
        }
        zoning_parcel_geometries_packed_dict(&core, &geometries, runtime_id)
    }

    /// Creates an all-or-nothing road-side parcel drag run.
    #[func]
    pub fn apply_zoning_parcel_drag(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
        frontage_cells: i32,
        depth_cells: i32,
        gap_m: f32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let Some((frontage_m, depth_m)) =
            zoning_parcel_cell_dimensions(&core.config, frontage_cells, depth_cells)
        else {
            return false;
        };
        let result = core.zoning.place_parcel_run_at(
            start_x,
            start_z,
            end_x,
            end_z,
            runtime_id,
            frontage_m,
            depth_m,
            gap_m,
            &core.region_graph,
        );
        match result {
            Ok(ids) if !ids.is_empty() => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            _ => false,
        }
    }

    /// Rezones every existing parcel touched by a world-space zoning paint stroke.
    #[func]
    pub fn apply_zoning_parcel_rezone_drag(
        &mut self,
        start_x: f32,
        start_z: f32,
        end_x: f32,
        end_z: f32,
        target_profile_runtime_id: i32,
    ) -> bool {
        let Ok(runtime_id) = u16::try_from(target_profile_runtime_id) else {
            return false;
        };
        let mut core = self.lock_core();
        let core = &mut *core;
        let result = core
            .zoning
            .rezone_stroke(start_x, start_z, end_x, end_z, runtime_id);
        match result {
            Ok(ids) if !ids.is_empty() => {
                core.allocator.dirty = true;
                core.allocator.dirty_index = true;
                true
            }
            _ => false,
        }
    }

    /// Returns committed zoning parcels for the Godot overlay mesh.
    #[func]
    pub fn get_zoning_parcels_overlay(&self) -> VarArray {
        let core = self.lock_core();
        let mut arr = VarArray::new();
        for parcel in core.zoning.parcels() {
            let geometry = crate::simulation::zoning::ParcelGeometry {
                edge_idx: parcel.edge_idx(),
                side: parcel.side(),
                frontage_center_t: parcel.frontage_center_t(),
                frontage_m: parcel.frontage_m(),
                depth_m: parcel.depth_m(),
                front_center: parcel.front_center(),
                center: parcel.center(),
                tangent: parcel.tangent(),
                normal: parcel.normal(),
                corners: parcel.corners(),
                aabb_min: parcel.aabb_min(),
                aabb_max: parcel.aabb_max(),
            };
            let dict = zoning_parcel_geometry_dict(
                &core,
                &geometry,
                parcel.zone_profile_runtime_id(),
                parcel.occupied_building().is_some(),
                parcel.id().raw(),
            );
            arr.push(&dict.to_variant());
        }
        arr
    }

    /// Returns the ID of the edge hovered by the mouse.
    #[func]
    pub fn get_hovered_edge(&self, world_x: f32, world_z: f32) -> i32 {
        self.lock_core().get_hovered_edge_internal(world_x, world_z)
    }

    /// Returns the raycast depth against the road network.
    #[func]
    pub fn get_max_polygon_depth(
        &self,
        origin_x: f32,
        origin_z: f32,
        dir_x: f32,
        dir_z: f32,
        max_search: f32,
    ) -> f32 {
        self.lock_core()
            .get_max_polygon_depth_internal(origin_x, origin_z, dir_x, dir_z, max_search)
    }

    // ── Simulation ──

    /// Sets the simulation speed multiplier.
    #[func]
    pub fn set_simulation_speed(&mut self, speed: f32) {
        // Use channel so we don't block waiting for the tick lock.
        let _ = self.cmd_tx.send(SimCommand::SetSpeed(speed.max(0.0)));
    }

    /// Updates the camera world-space AABB used to cull agent transform uploads.
    ///
    /// Call once per frame from GDScript with the camera's visible world rect,
    /// padded by ~200 m to avoid pop-in at the viewport edge. Agents outside the
    /// rect are excluded from the next `RenderSnapshot` transform buffers, reducing
    /// GPU upload cost from O(A_total) to O(A_visible).
    #[func]
    pub fn set_camera_aabb(&mut self, x_min: f32, x_max: f32, z_min: f32, z_max: f32) {
        let _ = self
            .cmd_tx
            .send(SimCommand::SetCameraAabb(x_min, x_max, z_min, z_max));
    }

    /// Maximum far-plane distance used when building the camera frustum AABB for agent culling.
    #[func]
    pub fn get_agent_cull_far_m() -> f32 {
        crate::config::AGENT_CULL_FAR_M
    }

    /// Padding added to each side of the camera frustum AABB to prevent pop-in.
    #[func]
    pub fn get_agent_cull_padding_m() -> f32 {
        crate::config::AGENT_CULL_PADDING_M
    }

    /// Target render FPS cap. Applied to `Engine.max_fps` at startup.
    #[func]
    pub fn get_target_fps() -> u32 {
        crate::config::TARGET_FPS
    }

    /// Returns the current simulation day count.
    #[func]
    pub fn get_current_day(&self) -> u32 {
        self.snapshot.read().unwrap().current_day
    }

    /// Returns the current operational minute since midnight.
    #[func]
    pub fn get_current_minute_of_day(&self) -> u16 {
        self.snapshot.read().unwrap().current_minute_of_day
    }

    /// Returns normalized residential, commercial, and industrial growth
    /// pressures for the gameplay HUD as `Vector3(x=residential, y=commercial, z=industrial)`.
    ///
    /// Each component is clamped to `-1.0..1.0`, corresponding to `-100%..100%`
    /// on the UI meter. This is a UI-facing read path only and is not used in
    /// the simulation tick.
    #[func]
    pub fn get_demand_pressures(&self) -> Vector3 {
        let core = self.lock_core();
        Vector3::new(
            core.demand.net_residential_pressure().clamp(-1.0, 1.0),
            core.demand.net_commercial_pressure().clamp(-1.0, 1.0),
            core.demand.net_industrial_pressure().clamp(-1.0, 1.0),
        )
    }

    // ── Agents ──

    /// Returns a Dictionary of packed transforms for visible non-car agents, keyed by pedestrian_type.
    #[func]
    pub fn get_agent_transforms(&self) -> VarDictionary {
        use super::sim::bridge::agents::get_agent_transforms;
        get_agent_transforms(&self.snapshot.read().unwrap())
    }

    /// Returns a Dictionary of packed transforms for visible car agents, keyed by vehicle type.
    #[func]
    pub fn get_car_transforms(&self) -> VarDictionary {
        use super::sim::bridge::agents::get_car_transforms;
        get_car_transforms(&self.snapshot.read().unwrap())
    }

    /// Returns render IDs for visible car agents, keyed to match `get_car_transforms`.
    #[func]
    pub fn get_car_render_ids(&self) -> VarDictionary {
        use super::sim::bridge::agents::get_car_render_ids;
        get_car_render_ids(&self.snapshot.read().unwrap())
    }

    /// Returns debug path geometry for active agents.
    #[func]
    pub fn get_agent_paths_debug(&self) -> VarDictionary {
        self.lock_core().get_agent_paths_debug_internal()
    }

    // ── Buildings ──

    /// Returns `true` when the node was launched with `--asset-editor`.
    ///
    /// GDScript uses this to confirm it is running the editor shell rather than the normal
    /// city simulation, and to skip tick-driven systems that are not active in sandbox mode.
    #[func]
    pub fn is_asset_editor_mode(&self) -> bool {
        self.asset_editor_mode
    }

    /// Returns `true` when the node was launched with `--economy-editor`.
    #[func]
    pub fn is_economy_editor_mode(&self) -> bool {
        self.economy_editor_mode
    }

    /// Returns `true` when the node was launched with `--world-editor`.
    #[func]
    pub fn is_world_editor_mode(&self) -> bool {
        self.world_editor_mode
    }

    /// Loads the canonical authored economy folder and returns a JSON envelope
    /// containing profiles, controllers, scenarios, and validation messages.
    #[func]
    pub fn load_economy_project(&self, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::load_project_json;
        match load_project_json(std::path::Path::new(&dir_path.to_string())) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Validates the authored economy JSON payload, writes the canonical TOML
    /// files, and rebuilds the derived `economy.index.bin` cache.
    #[func]
    pub fn export_economy_project(&self, project_json: GString, dir_path: GString) -> GString {
        use crate::simulation::economy::definitions::export_project_json;
        match export_project_json(
            &project_json.to_string(),
            std::path::Path::new(&dir_path.to_string()),
        ) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Runs the small authored-economy sandbox for the selected scenario and
    /// returns daily series data plus summary bottleneck metrics as JSON.
    #[func]
    pub fn run_economy_sandbox(&self, project_json: GString, scenario_id: GString) -> GString {
        use crate::simulation::economy::definitions::run_sandbox_json;
        match run_sandbox_json(&project_json.to_string(), &scenario_id.to_string()) {
            Ok(json) => GString::from(json.as_str()),
            Err(err) => {
                let payload = serde_json::json!({
                    "ok": false,
                    "error": err,
                    "validation": [],
                });
                GString::from(payload.to_string().as_str())
            }
        }
    }

    /// Scans a native filesystem directory for content packs and registers all valid assets.
    #[func]
    pub fn load_asset_packs(&mut self, dir_path: GString, enabled_pack_ids: GString) -> GString {
        use super::sim::bridge::assets::load_asset_packs;
        load_asset_packs(&mut self.lock_core(), dir_path, enabled_pack_ids)
    }

    /// Returns all qualified asset IDs (`"pack_id:asset_id"`) currently in the registry.
    ///
    /// Godot uses this to enumerate which meshes to load for building rendering.
    #[func]
    pub fn get_registered_asset_ids(&self) -> PackedStringArray {
        let core = self.lock_core();
        let mut ids: Vec<GString> = core
            .allocator
            .registry
            .qualified_ids()
            .map(GString::from)
            .collect();

        let has_broken = core.allocator.buildings.iter().any(|b| b.broken);
        if has_broken {
            ids.push(GString::from("broken:error"));
        }

        PackedStringArray::from_iter(ids)
    }

    /// Returns the native filesystem path to the LOD0 mesh file for a registered asset.
    #[func]
    pub fn get_lod0_native_path(&self, qualified_id: GString) -> GString {
        use super::sim::bridge::assets::get_lod0_native_path;
        get_lod0_native_path(&self.lock_core(), qualified_id)
    }

    /// Returns the packed 12-float transforms for all placed buildings with the given asset ID.
    #[func]
    pub fn get_building_transforms_for_asset(&self, asset_id: GString) -> PackedFloat32Array {
        self.lock_core()
            .get_building_transforms_for_asset_internal(&asset_id.to_string())
    }

    /// Returns the packed 12-float transforms for all deserted buildings with the given asset ID.
    #[func]
    pub fn get_deserted_building_transforms_for_asset(
        &self,
        asset_id: GString,
    ) -> PackedFloat32Array {
        self.lock_core()
            .get_deserted_building_transforms_for_asset_internal(&asset_id.to_string())
    }

    /// Returns the packed transforms for building plots/foundations of a specific zone type.
    #[func]
    pub fn get_building_plot_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core()
            .get_building_plot_transforms_internal(zone_type_int)
    }

    /// Returns the packed construction-site slab transforms for a specific zone type.
    #[func]
    pub fn get_construction_site_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core()
            .get_construction_site_transforms_internal(zone_type_int)
    }

    /// Returns the packed construction-foundation transforms for a specific zone type.
    #[func]
    pub fn get_construction_foundation_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core()
            .get_construction_foundation_transforms_internal(zone_type_int)
    }

    /// Returns the packed procedural scaffold bar transforms for a specific zone type.
    #[func]
    pub fn get_construction_scaffold_transforms(&self, zone_type_int: u8) -> PackedFloat32Array {
        self.lock_core()
            .get_construction_scaffold_transforms_internal(zone_type_int)
    }

    /// Returns a Dictionary of live stats for the building whose centre is closest to
    /// (`world_x`, `world_z`) within a 30 m pick radius.
    ///
    /// Returns an empty Dictionary when no building is within range.
    /// Keys: `asset_id`, `zone_type`, `level`, `occupancy`, `worker_count`,
    /// `worker_capacity`, compact business summary fields, `budget_distress`,
    /// `economy_broken`, `broken`, `pending_redevelopment`, `rezone_grace_days`,
    /// `economy_profile`, `center_x`, `center_z`, residential household aggregates,
    /// and `inventory` (Array of `{name, amount}` Dictionaries).
    #[func]
    pub fn get_building_info_at(&self, world_x: f32, world_z: f32) -> VarDictionary {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;
        use crate::simulation::economy::households::{
            REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
            REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
            REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER,
        };
        use crate::simulation::economy::households::{
            building_inventory_fill_ratio, building_operation_factors,
        };
        use crate::simulation::zoning::ZoneType;

        let core = self.lock_core();

        // Linear scan — only called on explicit user clicks, never on the hot path.
        let pick_radius_sq = 30.0_f32 * 30.0;
        let mut best_idx = usize::MAX;
        let mut best_dist_sq = pick_radius_sq;
        for (i, b) in core.allocator.buildings.iter().enumerate() {
            let dx = b.center_x - world_x;
            let dz = b.center_y - world_z; // center_y is world-Z in the building struct
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_idx = i;
            }
        }
        if best_idx == usize::MAX {
            return VarDictionary::new();
        }

        let b = &core.allocator.buildings[best_idx];
        let catalog = load_runtime_economy_catalog().ok();

        let zone_type_str = match b.zone_type {
            ZoneType::None => "utility",
            ZoneType::Residential => "residential",
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            ZoneType::Office => "office",
            ZoneType::Mixed => "mixed",
        };

        let profile_id = catalog
            .as_ref()
            .and_then(|c| c.profile_by_runtime_id(b.economy_profile_runtime_id))
            .map(|p| p.id.clone())
            .unwrap_or_default();

        let worker_capacity = catalog
            .as_ref()
            .map(|catalog| {
                core.allocator
                    .worker_capacity_with_catalog(best_idx, catalog.as_ref())
            })
            .unwrap_or_else(|| core.allocator.worker_capacity(best_idx));

        // Inventory: only non-zero resource slots.
        let mut inv_arr = VarArray::new();
        if let Some(cat) = &catalog {
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount > 0.001 {
                    let runtime_id = (slot + 1) as u16;
                    let name = cat
                        .resource_id_for_runtime_id(runtime_id)
                        .unwrap_or("unknown");
                    let mut entry = VarDictionary::new();
                    entry.set("name", GString::from(name));
                    entry.set("amount", amount as f64);
                    inv_arr.push(&entry.to_variant());
                }
            }
        }

        let mut dict = VarDictionary::new();
        dict.set("asset_id", GString::from(b.asset_id.as_str()));
        dict.set("zone_type", GString::from(zone_type_str));
        dict.set("level", b.level as i32);
        dict.set("under_construction", b.is_under_construction());
        dict.set(
            "construction_remaining_hours",
            b.construction_remaining_hours as i32,
        );
        dict.set("construction_progress", b.construction_progress() as f64);
        dict.set("occupancy", b.occupancy as i32);
        dict.set("center_x", b.center_x as f64);
        dict.set("center_z", b.center_y as f64);

        let mut total_agents = 0i32;
        let mut child_agents = 0i32;
        let mut adult_agents = 0i32;
        let mut elder_agents = 0i32;
        let mut household_count = 0i32;
        let mut household_budget_total = 0.0f32;
        let mut household_stock_total = 0.0f32;
        let mut household_stock_days_total = 0.0f32;
        let mut household_stock_days_min = f32::INFINITY;
        let mut household_replenishment_active = 0i32;
        let mut first_replenishment_state = None;
        let mut mixed_replenishment_state = false;
        if b.zone_type == ZoneType::Residential {
            for h in &core.households.households {
                if h.home_building_id == best_idx {
                    household_count += 1;
                    total_agents += h.member_count as i32;
                    child_agents += h.child_count as i32;
                    adult_agents += h.adult_count as i32;
                    elder_agents += h.elder_count as i32;
                    household_budget_total += h.budget;
                    household_stock_total += h.stock;
                    household_stock_days_total += h.stock_days;
                    household_stock_days_min = household_stock_days_min.min(h.stock_days);
                    if h.replenishment_state != REPLENISHMENT_STABLE {
                        household_replenishment_active += 1;
                    }
                    match first_replenishment_state {
                        Some(state) if state != h.replenishment_state => {
                            mixed_replenishment_state = true;
                        }
                        None => {
                            first_replenishment_state = Some(h.replenishment_state);
                        }
                        _ => {}
                    }
                }
            }
        }
        dict.set("agent_count", total_agents);
        dict.set("child_count", child_agents);
        dict.set("adult_count", adult_agents);
        dict.set("elder_count", elder_agents);
        if b.zone_type == ZoneType::Residential {
            let household_divisor = household_count.max(1) as f32;
            let replenishment_state = if household_count == 0 {
                "-"
            } else if mixed_replenishment_state {
                "Mixed"
            } else {
                match first_replenishment_state.unwrap_or(REPLENISHMENT_STABLE) {
                    REPLENISHMENT_STABLE => "Stable",
                    REPLENISHMENT_NEEDS => "Needs restock",
                    REPLENISHMENT_WAITING_FOR_SHOPPER => "Waiting for shopper",
                    REPLENISHMENT_SHOPPING_TO_STORE => "Shopping to store",
                    REPLENISHMENT_SHOPPING_RETURNING => "Shopping returning",
                    REPLENISHMENT_FULFILLED => "Fulfilled",
                    REPLENISHMENT_COOLDOWN => "Cooldown",
                    REPLENISHMENT_FAILED_TERMINAL => "Unresolved shortage",
                    _ => "Unknown",
                }
            };
            dict.set("household_count", household_count);
            dict.set("household_budget_total", household_budget_total as f64);
            dict.set(
                "household_budget_avg",
                (household_budget_total / household_divisor) as f64,
            );
            dict.set("household_stock_total", household_stock_total as f64);
            dict.set(
                "household_stock_days_avg",
                (household_stock_days_total / household_divisor) as f64,
            );
            dict.set(
                "household_stock_days_min",
                if household_stock_days_min.is_finite() {
                    household_stock_days_min as f64
                } else {
                    0.0
                },
            );
            dict.set(
                "household_replenishment_active",
                household_replenishment_active,
            );
            dict.set(
                "household_replenishment_state",
                GString::from(replenishment_state),
            );
        }
        dict.set("worker_count", b.worker_count as i32);
        dict.set("worker_capacity", worker_capacity as i32);
        dict.set("operating_budget", b.operating_budget as f64);
        dict.set("revenue", b.revenue as f64);
        if b.zone_type != ZoneType::Residential {
            if let Some(cat) = &catalog
                && let Some(profile) = cat.profile_by_runtime_id(b.economy_profile_runtime_id)
            {
                let factors = building_operation_factors(cat.as_ref(), b, profile);
                let inventory_fill = building_inventory_fill_ratio(cat.as_ref(), b, profile);
                let profit_today = b.operating_budget - b.profit_tax_budget_baseline;
                let business_status = if b.broken {
                    "Asset broken"
                } else if b.economy_broken {
                    "Economy broken"
                } else if b.is_deserted {
                    "Deserted"
                } else if b.is_under_construction() {
                    "Under construction"
                } else if b.budget_distress || b.operating_budget < 0.0 {
                    "Distressed"
                } else if factors.active_worker_capacity > 0 && factors.effective_workers == 0 {
                    "No workers"
                } else if factors.input_factor < 0.5 {
                    "Needs inputs"
                } else if factors.output_headroom_factor < 0.5 {
                    "Storage full"
                } else if factors.active_worker_capacity < profile.worker_capacity {
                    "Demand-limited"
                } else if factors.throughput_factor >= 0.8 {
                    "Running"
                } else if factors.throughput_factor > 0.0 {
                    "Limited"
                } else {
                    "Idle"
                };
                dict.set("business_summary", true);
                dict.set("business_status", GString::from(business_status));
                dict.set("business_profit_today", profit_today as f64);
                dict.set("business_profit_yesterday", b.last_day_profit as f64);
                dict.set(
                    "business_active_worker_capacity",
                    factors.active_worker_capacity as i32,
                );
                dict.set(
                    "business_production_ratio",
                    factors.throughput_factor as f64,
                );
                dict.set(
                    "business_inventory_fill_ratio",
                    inventory_fill.unwrap_or(0.0) as f64,
                );
                dict.set("business_has_inventory_fill", inventory_fill.is_some());
            }
        }
        dict.set("budget_distress", b.budget_distress);
        dict.set("economy_broken", b.economy_broken);
        dict.set("broken", b.broken);
        dict.set("is_deserted", b.is_deserted);
        dict.set("pending_redevelopment", b.pending_redevelopment);
        dict.set("rezone_grace_days", b.rezone_grace_days_remaining as i32);
        dict.set("economy_profile", GString::from(profile_id.as_str()));
        dict.set("inventory", inv_arr.to_variant());
        dict
    }

    /// Validates the JSON export params, writes `pack.toml` (if absent) and
    /// `assets/<asset_id>/asset.toml` under `output_dir`, and returns an error
    /// string or `""` on success.
    ///
    /// `output_dir` must be an absolute native path (resolve `user://mods/<pack_id>/`
    /// with `ProjectSettings.globalize_path` before passing it in).
    #[func]
    pub fn validate_and_export_asset(&self, params_json: GString, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::validate_and_export_asset_internal;
        let result =
            validate_and_export_asset_internal(&params_json.to_string(), &output_dir.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object describing the manifest for an already-registered asset,
    /// or `""` if the qualified ID is not in the registry.
    ///
    /// GDScript uses this to repopulate the importer form when re-editing an existing asset.
    #[func]
    pub fn get_asset_manifest_json(&self, qualified_id: GString) -> GString {
        use crate::nodes::sim::asset_export::get_asset_manifest_json_internal;
        let core = self.lock_core();
        let result =
            get_asset_manifest_json_internal(&core.allocator.registry, &qualified_id.to_string());
        GString::from(result.as_str())
    }

    /// Returns a JSON object with pack metadata (`pack_id`, `display_name`, `author`,
    /// `version`, `license`) read from `<output_dir>/pack.toml`, or `""` if not found.
    ///
    /// `output_dir` must be the absolute native path to the pack directory
    /// (i.e. `ProjectSettings.globalize_path("user://mods/<pack_id>/")` ).
    #[func]
    pub fn get_pack_manifest_json(&self, output_dir: GString) -> GString {
        use crate::nodes::sim::asset_export::get_pack_manifest_json_internal;
        GString::from(get_pack_manifest_json_internal(&output_dir.to_string()).as_str())
    }

    // ── Network ──

    /// Returns the closest boundary point on a road edge to the given position.
    #[func]
    pub fn get_closest_point_on_edge(&self, edge_idx: i32, point_x: f32, point_y: f32) -> Vector2 {
        self.lock_core()
            .get_closest_point_on_edge_internal(edge_idx, point_x, point_y)
    }

    /// Returns the physical segment geometry for a road edge.
    #[func]
    pub fn get_edge_geometry(&self, edge_idx: i32) -> PackedVector2Array {
        self.lock_core().get_edge_geometry_internal(edge_idx)
    }

    /// Returns the 3D geometry for a road edge.
    #[func]
    pub fn get_edge_geometry_3d(&self, edge_idx: i32) -> PackedVector3Array {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return PackedVector3Array::new();
        }
        let edge = core.region_graph.edge(edge_idx as usize);
        PackedVector3Array::from_iter(edge.physical_geometry.iter().cloned())
    }

    /// Returns the width of a specific road edge.
    #[func]
    pub fn get_edge_width(&self, edge_idx: i32) -> f32 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 6.0;
        }
        core.region_graph.edge(edge_idx as usize).width
    }

    /// Returns a curved frontage between two points on an edge.
    #[func]
    pub fn get_curved_frontage(
        &self,
        edge_idx: i32,
        start_p: Vector2,
        end_p: Vector2,
    ) -> PackedVector2Array {
        self.lock_core()
            .get_curved_frontage_internal(edge_idx, start_p, end_p)
    }

    /// Adds a new road segment to the network.
    #[func]
    pub fn add_road(&mut self, points: PackedVector3Array, fwd_lanes: i32, bkw_lanes: i32) {
        // Send to the background thread so the Godot main thread is never blocked
        // by the expensive lane-rebuild and zoning-obstruction passes (~500 ms).
        // The road appears on the next sim tick (~16 ms later) — imperceptible delay.
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let send_start = road_debug.then(Instant::now);
        let send_ok = self
            .cmd_tx
            .send(crate::nodes::sim::core::SimCommand::AddRoad {
                points,
                fwd_lanes,
                bkw_lanes,
            })
            .is_ok();
        let send_ms = send_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "add_road_bridge points={} fwd_lanes={} bkw_lanes={} clone_ms={:.3} send_ms={:.3} send_ok={} total_ms={:.3}",
                point_count,
                fwd_lanes,
                bkw_lanes,
                clone_ms,
                send_ms,
                send_ok,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }

    /// Returns the node ID of the nearest graph node near the border, or -1.
    #[func]
    pub fn check_border_candidate(&self, pos: Vector3) -> i64 {
        self.lock_core().check_border_candidate_internal(pos)
    }

    /// Marks the node at `node_id` as an external border connection.
    #[func]
    pub fn set_border_connection(&mut self, node_id: i32) {
        self.lock_core().set_border_connection_internal(node_id);
    }

    /// Returns the world-space positions of all active border nodes as a flat float array.
    #[func]
    pub fn get_border_nodes(&self) -> PackedFloat32Array {
        self.lock_core().get_border_nodes_internal()
    }

    /// Returns the classification of an edge as an integer (0=Standard, 1=Bridge, 2=Tunnel).
    /// Returns 0 if the edge index is invalid.
    #[func]
    pub fn get_edge_class(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 0;
        }
        match core.region_graph.edge(edge_idx as usize).class {
            crate::simulation::network::types::EdgeClass::Bridge => 1,
            crate::simulation::network::types::EdgeClass::Tunnel => 2,
            _ => 0,
        }
    }

    /// Sets the classification of an edge (Standard, Bridge, Tunnel).
    #[func]
    pub fn set_edge_class(&mut self, edge_idx: i32, class_int: u8) {
        self.lock_core()
            .set_edge_class_internal(edge_idx, class_int);
    }

    /// Sets or clears the no-building-spawn flag on an edge. When true the building
    /// allocator skips this edge. Player-toggleable; also auto-set for speed ≥ 80 km/h.
    #[func]
    pub fn set_no_building_spawn(&mut self, edge_idx: i32, enabled: bool) {
        self.lock_core()
            .set_no_building_spawn_internal(edge_idx, enabled);
    }

    /// Sets the vehicle frontage-access policy on an edge.
    ///
    /// `0 = SameSideOnly`, `1 = BothSides`. Invalid values are ignored.
    #[func]
    pub fn set_vehicle_frontage_access(&mut self, edge_idx: i32, access_int: u8) {
        self.lock_core()
            .set_vehicle_frontage_access_internal(edge_idx, access_int);
    }

    /// Returns true if the given edge has the no-building-spawn flag set.
    #[func]
    pub fn get_no_building_spawn(&self, edge_idx: i32) -> bool {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return false;
        }
        core.region_graph.edge(edge_idx as usize).no_building_spawn
    }

    /// Returns the vehicle frontage-access policy on an edge.
    ///
    /// Returns `1` (`BothSides`) if the edge index is invalid.
    #[func]
    pub fn get_vehicle_frontage_access(&self, edge_idx: i32) -> u8 {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return 1;
        }
        match core
            .region_graph
            .edge(edge_idx as usize)
            .vehicle_frontage_access
        {
            crate::simulation::network::types::VehicleFrontageAccess::SameSideOnly => 0,
            crate::simulation::network::types::VehicleFrontageAccess::BothSides => 1,
        }
    }

    /// Returns the start and end node indices of an edge as `Vector2i(start, end)`.
    /// Returns `(-1, -1)` if the edge index is invalid.
    #[func]
    pub fn get_edge_nodes(&self, edge_idx: i32) -> Vector2i {
        let core = self.lock_core();
        if edge_idx < 0 || edge_idx as usize >= core.region_graph.edge_count() {
            return Vector2i::new(-1, -1);
        }
        let e = core.region_graph.edge(edge_idx as usize);
        Vector2i::new(e.start_node as i32, e.end_node as i32)
    }

    /// Returns the indices of all non-deleted edges with `no_building_spawn = true`.
    /// Used by the zone-tool overlay to draw the hatched no-build indicator.
    #[func]
    pub fn get_no_building_spawn_edge_indices(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut out = PackedInt32Array::new();
        for (i, e) in core.region_graph.edges().iter().enumerate() {
            if !e.deleted && e.no_building_spawn {
                out.push(i as i32);
            }
        }
        out
    }

    /// Returns dictionary of road/intersection mesh data.
    #[func]
    pub fn get_road_mesh_data(&self) -> VarDictionary {
        let mut core = self.lock_core();
        core.get_road_mesh_data_internal()
    }

    /// Requests temporary preview-surface compilation for the road tool.
    ///
    /// The result is published asynchronously through [`get_preview_road_surface_result`], so the
    /// commit path can wait for a valid preview without blocking on the simulation mutex.
    #[func]
    pub fn request_preview_road_surface(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> i64 {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let request_id = self
            .road_preview_request_counter
            .fetch_add(1, Ordering::Relaxed)
            + 1;
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let send_start = road_debug.then(Instant::now);
        let send_ok = self
            .road_preview_tx
            .send(RoadPreviewRequest {
                request_id,
                points,
                fwd_lanes,
                bkw_lanes,
            })
            .is_ok();
        let send_ms = send_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "preview_surface_request request_id={} points={} fwd_lanes={} bkw_lanes={} clone_ms={:.3} send_ms={:.3} send_ok={} total_ms={:.3}",
                request_id,
                point_count,
                fwd_lanes,
                bkw_lanes,
                clone_ms,
                send_ms,
                send_ok,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        i64::try_from(request_id).unwrap_or(i64::MAX)
    }

    /// Compiles a current-frame road-tool preview from immutable Rust preview state.
    ///
    /// This uses the same mesh-only road-surface compiler as the background preview worker, but
    /// returns immediately so the live cursor preview and the idle preview share one geometry path.
    #[func]
    pub fn get_preview_road_surface_immediate(
        &self,
        points: PackedVector3Array,
        fwd_lanes: i32,
        bkw_lanes: i32,
    ) -> Variant {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = points.len();
        let clone_start = road_debug.then(Instant::now);
        let points = points.to_vec();
        let clone_ms = clone_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let compile_start = road_debug.then(Instant::now);
        let preview = {
            let context = self.road_preview_context.read().unwrap();
            compile_road_preview_from_context(
                &context,
                RoadPreviewRequest {
                    request_id: 0,
                    points,
                    fwd_lanes,
                    bkw_lanes,
                },
            )
        };
        let compile_ms = compile_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        if road_debug {
            debug_log!(
                "road",
                "preview_surface_immediate points={} prepared_points={} surface_vertices={} valid={} clone_ms={:.3} compile_ms={:.3} total_ms={:.3}",
                point_count,
                preview.prepared_points.len(),
                preview.surface_vertices.len(),
                preview.is_valid,
                clone_ms,
                compile_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
        Self::road_preview_snapshot_to_variant(&preview)
    }

    /// Returns the completed road-tool preview for `request_id`, or `null` while pending/stale.
    #[func]
    pub fn get_preview_road_surface_result(&self, request_id: i64) -> Variant {
        let Ok(request_id) = u64::try_from(request_id) else {
            return Variant::nil();
        };
        let preview_result = self.road_preview_result.read().unwrap();
        let Some(preview) = preview_result.as_ref() else {
            return Variant::nil();
        };
        if preview.request_id != request_id {
            return Variant::nil();
        }

        Self::road_preview_snapshot_to_variant(preview)
    }

    fn road_preview_snapshot_to_variant(preview: &RoadPreviewSnapshot) -> Variant {
        let mut dict = VarDictionary::new();
        dict.set(
            "request_id",
            i64::try_from(preview.request_id).unwrap_or(i64::MAX),
        );
        dict.set(
            "prepared_points",
            PackedVector3Array::from_iter(preview.prepared_points.iter().copied()),
        );
        dict.set(
            "surface_vertices",
            PackedVector3Array::from_iter(preview.surface_vertices.iter().copied()),
        );
        dict.set("is_valid", preview.is_valid);
        dict.to_variant()
    }

    /// Returns compiled road-surface debug line data for editor visualization.
    ///
    /// Uses `try_lock` because this is only a debug/editor helper and should never stall the
    /// Godot main thread while the simulation mutex is busy.
    #[func]
    pub fn get_road_surface_debug_data(&self) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_debug_data_internal().to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns terrain render patches that must keep full mesh resolution where compiled road ownership is visible.
    #[func]
    pub fn get_road_locked_terrain_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        core.get_road_locked_terrain_patches_internal()
    }

    /// Returns the closest network point (node/edge) within range.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain` — called every
    /// frame while the road tool is active; must not stall on the sim mutex.
    /// Returns `null` when contended; GDScript handles null from this call already.
    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.get_closest_network_point_internal(world_pos, max_dist) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Resolves the road-tool cursor position in one non-blocking Rust query.
    ///
    /// This combines visible-surface picking, angle snapping, network snapping, optional
    /// ghost-guide snapping, map-border snapping, and self-snapping so the Godot editor loop
    /// does not perform several bridge calls for every mouse frame.
    #[func]
    pub fn get_road_tool_cursor_pos(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        altitude_offset_m: f32,
        active: bool,
        current_state: i32,
        start_pos: Vector3,
        control_pos: Vector3,
        shift_pressed: bool,
        start_tangent_angle: f32,
        ghost_enabled: bool,
        border_snap_dist_m: f32,
    ) -> Variant {
        let query = self.road_tool_query_snapshot.read().unwrap();
        let (world_w, world_h) = query.terrain.world_size();
        let half_w = world_w * 0.5;
        let half_h = world_h * 0.5;
        let border_snap_dist = border_snap_dist_m.min(half_w.min(half_h));

        let mut pos = match query.road_surface.raycast_visible_surface(
            &query.region_graph,
            &query.terrain,
            ray_origin,
            ray_dir,
        ) {
            Some(hit) => hit,
            None => {
                if ray_dir.y >= -0.001 {
                    return Variant::nil();
                }
                let t_plane = -ray_origin.y / ray_dir.y;
                let mut hit = ray_origin + ray_dir * t_plane;
                hit.x = hit.x.clamp(-half_w, half_w);
                hit.z = hit.z.clamp(-half_h, half_h);
                hit = Self::road_tool_snap_to_border(hit, half_w, half_h);
                hit.y = query
                    .road_surface
                    .sample_visible_surface_height(
                        &query.region_graph,
                        &query.terrain,
                        hit.x,
                        hit.z,
                    )
                    .unwrap_or_else(|| {
                        query.terrain.sample_visual_height_world(hit.x, hit.z)
                            * crate::config::HEIGHT_SCALE
                    });
                return hit.to_variant();
            }
        };

        pos.y += altitude_offset_m;

        if active && shift_pressed {
            let ref_pos = if current_state == 1 {
                start_pos
            } else {
                control_pos
            };
            let dir = pos - ref_pos;
            let length = dir.length();
            if length > 0.1 {
                let snap_rad = std::f32::consts::PI / 12.0;
                let relative =
                    ((dir.z.atan2(dir.x) - start_tangent_angle) / snap_rad).round() * snap_rad;
                let angle = start_tangent_angle + relative;
                let snapped_length = ((length / 10.0).round() * 10.0).max(10.0);
                pos = ref_pos + Vector3::new(angle.cos(), 0.0, angle.sin()) * snapped_length;
            }
        }

        if let Some(mut snapped_pos) = crate::simulation::network::interaction::get_closest_point_xz(
            &query.region_graph,
            pos,
            5.0,
        ) {
            snapped_pos.y = query
                .road_surface
                .sample_visible_surface_height(
                    &query.region_graph,
                    &query.terrain,
                    snapped_pos.x,
                    snapped_pos.z,
                )
                .unwrap_or_else(|| {
                    query
                        .terrain
                        .sample_visual_height_world(snapped_pos.x, snapped_pos.z)
                        * crate::config::HEIGHT_SCALE
                })
                + altitude_offset_m;
            return snapped_pos.to_variant();
        }

        if ghost_enabled && !shift_pressed {
            use super::sim::bridge::network::get_road_ghost_snap_from_parts;
            if let Some(ghost_snap) = get_road_ghost_snap_from_parts(
                &query.region_graph,
                &query.road_surface,
                &query.terrain,
                &query.ghost_snap_index,
                pos,
                10.0,
                altitude_offset_m,
            ) {
                return ghost_snap.to_variant();
            }
        }

        if Self::road_tool_is_near_border(pos, half_w, half_h, border_snap_dist) {
            pos = Self::road_tool_snap_to_border(pos, half_w, half_h);
            pos.y = query
                .road_surface
                .sample_visible_surface_height(&query.region_graph, &query.terrain, pos.x, pos.z)
                .unwrap_or_else(|| {
                    query.terrain.sample_visual_height_world(pos.x, pos.z)
                        * crate::config::HEIGHT_SCALE
                });
            return pos.to_variant();
        }

        if active {
            if pos.distance_to(start_pos) < 2.5 {
                return start_pos.to_variant();
            }
            if current_state == 2 && pos.distance_to(control_pos) < 2.5 {
                return control_pos.to_variant();
            }
        }

        pos.to_variant()
    }

    /// Returns the ID of the closest network node.
    #[func]
    pub fn get_closest_node(&self, world_pos: Vector3, max_dist: f32) -> i32 {
        self.lock_core()
            .get_closest_node_internal(world_pos, max_dist)
    }

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn set_node_cul_de_sac(&mut self, _node_id: i32, _enabled: bool, _radius: f32) {}

    /// Placeholder for cul-de-sac tools.
    #[func]
    pub fn has_cul_de_sac(&self, _node_id: i32) -> bool {
        false
    }

    /// Returns the number of road connections for a node.
    #[func]
    pub fn get_node_connection_count(&self, node_id: i32) -> i32 {
        self.lock_core().get_node_connection_count_internal(node_id)
    }

    /// Repositions a network node.
    #[func]
    pub fn move_network_node(&mut self, node_id: i32, pos: Vector3) {
        self.lock_core().move_network_node_internal(node_id, pos);
        self.refresh_snapshot_from_core();
    }

    /// Returns all junction node positions, read from the pre-computed snapshot.
    ///
    /// Reading from the snapshot (RwLock) avoids acquiring the SimCore mutex, which
    /// would stall the Godot main thread while `add_road_internal` holds the lock.
    #[func]
    pub fn get_network_nodes(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.snapshot.read().unwrap().node_positions.iter().copied())
    }

    /// Returns ghost guide data for the road-tool overlay.
    #[func]
    pub fn get_road_ghost_guides(&self) -> PackedFloat32Array {
        use super::sim::bridge::network::get_road_ghost_guides;
        match self.try_lock_core() {
            Some(core) => get_road_ghost_guides(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns fully-resolved ghost guide line vertices and colors.
    #[func]
    pub fn get_road_ghost_line_data(&self) -> VarDictionary {
        use super::sim::bridge::network::get_road_ghost_line_data;
        match self.try_lock_core() {
            Some(mut core) => get_road_ghost_line_data(&mut core),
            None => VarDictionary::new(),
        }
    }

    /// Returns the closest ghost-guide snap point within range, or null.
    #[func]
    pub fn get_road_ghost_snap(
        &self,
        world_pos: Vector3,
        max_dist_m: f32,
        altitude_offset_m: f32,
    ) -> Variant {
        use super::sim::bridge::network::get_road_ghost_snap_from_parts;
        let query = self.road_tool_query_snapshot.read().unwrap();
        match get_road_ghost_snap_from_parts(
            &query.region_graph,
            &query.road_surface,
            &query.terrain,
            &query.ghost_snap_index,
            world_pos,
            max_dist_m,
            altitude_offset_m,
        ) {
            Some(point) => point.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns the full physical geometry of every non-deleted road edge.
    #[func]
    pub fn get_road_edge_polylines(&self) -> PackedFloat32Array {
        use super::sim::bridge::network::get_road_edge_polylines;
        match self.try_lock_core() {
            Some(core) => get_road_edge_polylines(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns the tangent direction of the road whose endpoint is nearest to `pos`
    /// within `max_dist` metres.
    ///
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    ///
    /// Walks the physical geometry of every non-deleted edge and finds the closest point
    /// on the polyline (not just endpoints), returning the segment tangent at that point.
    /// This ensures mid-road snaps give the correct perpendicular direction.
    ///
    /// The returned `Vector2` is `(world_x, world_z)` of the unit tangent (either
    /// direction along the edge — callers only need the axis, not the sign).
    /// Returns `Vector2(0, 1)` (world +Z) if no road is within range.
    ///
    /// Non-blocking: returns `Vector2(0, 1)` if the SimCore mutex is contended.
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    #[func]
    pub fn get_road_tangent_at(&self, pos: Vector3, max_dist: f32) -> Vector2 {
        use super::sim::bridge::network::get_road_tangent_at;
        match self.try_lock_core() {
            Some(core) => get_road_tangent_at(&core, pos, max_dist),
            None => Vector2::new(0.0, 1.0),
        }
    }

    /// Configures a lane connection rule at a junction.
    #[func]
    pub fn set_lane_connection(
        &mut self,
        node_id: u32,
        from_edge: i32,
        from_lane: i32,
        to_edge: i32,
        to_lane: i32,
    ) {
        self.lock_core()
            .set_lane_connection_internal(node_id, from_edge, from_lane, to_edge, to_lane);
    }

    /// Clears all lane rules at a junction node.
    #[func]
    pub fn clear_lane_connections(&mut self, node_id: u32) {
        self.lock_core().clear_lane_connections_internal(node_id);
    }

    /// Toggles a user override for a crosswalk at a specific road mouth.
    #[func]
    pub fn set_crosswalk_override(&mut self, node_id: u32, edge_id: i32, enabled: bool) {
        self.lock_core()
            .set_crosswalk_override_internal(node_id, edge_id, enabled);
    }

    /// Returns true if a crosswalk exists natively or by user override.
    #[func]
    pub fn has_crosswalk(&self, node_id: u32, edge_id: i32) -> bool {
        self.lock_core().has_crosswalk_internal(node_id, edge_id)
    }

    /// Returns the world-space position of a node.
    #[func]
    pub fn get_node_pos(&self, node_id: u32) -> Vector3 {
        self.lock_core().get_node_pos_internal(node_id)
    }

    /// Returns information about all lanes entering/leaving a junction.
    #[func]
    pub fn get_node_lanes(&self, node_id: u32) -> VarArray {
        self.lock_core().get_node_lanes_internal(node_id)
    }

    /// Returns an array of current lane turn restrictions at a node.
    #[func]
    pub fn get_lane_connections_array(&self, node_id: u32) -> VarArray {
        self.lock_core()
            .get_lane_connections_array_internal(node_id)
    }

    /// Clears lane rules for a specific source lane.
    #[func]
    pub fn clear_lane_source(&mut self, node_id: u32, from_edge: i32, from_lane: i32) {
        self.lock_core()
            .clear_lane_source_internal(node_id, from_edge, from_lane);
    }

    /// Returns the average network direction at a given point.
    #[func]
    pub fn get_network_direction_at_point(&self, pos: Vector3) -> Vector3 {
        self.lock_core()
            .get_network_direction_at_point_internal(pos)
    }

    /// Returns terrain height at a position.
    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        self.lock_core().get_height_at_internal(pos)
    }

    /// Returns the visible world-surface height at a position.
    ///
    /// This reads the already compiled roadbed when a road surface owns the queried XZ location
    /// and otherwise falls back to the current visual terrain.
    #[func]
    pub fn get_world_surface_height(&self, pos: Vector2) -> f32 {
        self.lock_core().get_world_surface_height_internal(pos)
    }

    /// Raycasts against the terrain heightmap.
    ///
    /// Uses `try_lock` so this never stalls the Godot main thread if the sim thread
    /// is currently holding the mutex (e.g. during `add_road_internal`). Returns
    /// `null` when contended; GDScript already handles null from this call gracefully.
    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.intersect_terrain_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Raycasts against the visible world surface.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain`. The combined surface prefers
    /// compiled roadbed ownership and otherwise falls back to the visible terrain surface.
    #[func]
    pub fn intersect_world_surface(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.intersect_world_surface_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Resets the runtime to a new blank authored world with the given terrain settings.
    #[func]
    pub fn create_blank_world(
        &mut self,
        width_m: f32,
        height_m: f32,
        terrain_cell_m: f32,
        terrain_chunk_m: f32,
        base_elevation_m: f32,
    ) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.create_blank_world_internal(
                width_m,
                height_m,
                terrain_cell_m,
                terrain_chunk_m,
                base_elevation_m,
            )
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Create blank world failed: {}", err);
                false
            }
        }
    }

    /// Saves the current simulation into a single SQLite snapshot file.
    #[func]
    pub fn save_game(&self, path: GString) -> bool {
        match self.lock_core().save_game_internal(&path.to_string()) {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save failed: {}", err);
                false
            }
        }
    }

    /// Saves the current blank-world authoring state as a reusable world-definition asset.
    #[func]
    pub fn save_world_definition(&self, path: GString, name: GString) -> bool {
        match self
            .lock_core()
            .save_world_definition_internal(&path.to_string(), &name.to_string())
        {
            Ok(()) => true,
            Err(err) => {
                godot_error!("Save world definition failed: {}", err);
                false
            }
        }
    }

    /// Loads a SQLite save snapshot and replaces the live simulation state.
    #[func]
    pub fn load_game(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_game_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load failed: {}", err);
                false
            }
        }
    }

    /// Loads a reusable world-definition asset and replaces the live runtime with a blank city.
    #[func]
    pub fn load_world_definition(&mut self, path: GString) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.load_world_definition_internal(&path.to_string())
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Load world definition failed: {}", err);
                false
            }
        }
    }

    /// Adds or strengthens one authored world-water source at the clicked terrain cell.
    #[func]
    pub fn add_world_water_source(&mut self, pos: Vector2, rate_m_per_tick: f32) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.add_world_water_source_internal(pos, rate_m_per_tick)
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Add world water source failed: {}", err);
                false
            }
        }
    }

    /// Adds or strengthens one authored world-water sink at the clicked terrain cell.
    #[func]
    pub fn add_world_water_sink(&mut self, pos: Vector2, rate_m_per_tick: f32) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.add_world_water_sink_internal(pos, rate_m_per_tick)
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Add world water sink failed: {}", err);
                false
            }
        }
    }

    /// Removes the nearest authored world-water source within the given radius.
    #[func]
    pub fn remove_world_water_source_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_water_source_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Removes the nearest authored world-water sink within the given radius.
    #[func]
    pub fn remove_world_water_sink_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_water_sink_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Starts one transient authored lake-fill preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_lake_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_lake_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => {
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Begin world lake fill preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Starts one transient authored open-water preview at the clicked terrain cell.
    #[func]
    pub fn begin_world_open_water_fill_preview(
        &mut self,
        pos: Vector2,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.begin_world_open_water_fill_preview_internal(pos, surface_elevation_m)
        };
        self.refresh_snapshot_from_core();
        match result {
            Ok(preview) => Self::world_lake_fill_preview_dict(
                Some(preview),
                true,
                "open water preview updated",
            ),
            Err(err) => {
                godot_error!("Begin world open water preview failed: {}", err);
                Self::world_lake_fill_preview_dict(None, false, &err)
            }
        }
    }

    /// Updates the active transient lake-fill preview surface elevation.
    #[func]
    pub fn update_world_lake_fill_preview(&mut self, surface_elevation_m: f32) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_lake_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(Some(preview), true, "lake fill preview updated")
            }
            Err(err) => {
                godot_error!("Update world lake fill preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Updates the active transient open-water preview surface elevation.
    #[func]
    pub fn update_world_open_water_fill_preview(
        &mut self,
        surface_elevation_m: f32,
    ) -> VarDictionary {
        let result = {
            let mut core = self.lock_core();
            core.update_world_open_water_fill_preview_internal(surface_elevation_m)
        };
        match result {
            Ok(preview) => {
                self.refresh_snapshot_from_core();
                Self::world_lake_fill_preview_dict(
                    Some(preview),
                    true,
                    "open water preview updated",
                )
            }
            Err(err) => {
                godot_error!("Update world open water preview failed: {}", err);
                let active_preview = self.lock_core().world_water_fill_preview_internal();
                Self::world_lake_fill_preview_dict(active_preview, false, &err)
            }
        }
    }

    /// Returns the current transient lake-fill preview state.
    #[func]
    pub fn get_world_lake_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "surface fill preview active"
        } else {
            "no surface fill preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns the current transient open-water preview state.
    #[func]
    pub fn get_world_open_water_fill_preview(&self) -> VarDictionary {
        let preview = self.lock_core().world_water_fill_preview_internal();
        let message = if preview.is_some() {
            "open water preview active"
        } else {
            "no open water preview is active"
        };
        Self::world_lake_fill_preview_dict(preview, true, message)
    }

    /// Returns committed authored-world water markers for world-editor overlays.
    #[func]
    pub fn get_world_water_authoring_markers(&self) -> VarArray {
        let core = self.lock_core();
        let mut markers = VarArray::new();

        for point in &core.world_water_boundary_points {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(point.world_x, point.world_z)
                * config::HEIGHT_SCALE;
            let kind = match point.kind {
                crate::simulation::world_definition::AuthoredWaterBoundaryKind::Source => "source",
                crate::simulation::world_definition::AuthoredWaterBoundaryKind::Sink => "sink",
            };
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    kind,
                    point.world_x,
                    point.world_z,
                    terrain_height_m,
                    None,
                    Some(point.rate_m_per_tick),
                )
                .to_variant(),
            );
        }

        for lake in &core.world_lake_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(lake.world_x, lake.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "lake_fill",
                    lake.world_x,
                    lake.world_z,
                    terrain_height_m,
                    Some(lake.surface_elevation_m),
                    None,
                )
                .to_variant(),
            );
        }

        for open_water in &core.world_open_water_fills {
            let terrain_height_m = core
                .heightmap
                .sample_height_world(open_water.world_x, open_water.world_z)
                * config::HEIGHT_SCALE;
            markers.push(
                &Self::world_water_authoring_marker_dict(
                    "open_water_fill",
                    open_water.world_x,
                    open_water.world_z,
                    terrain_height_m,
                    Some(open_water.surface_elevation_m),
                    None,
                )
                .to_variant(),
            );
        }

        markers
    }

    /// Commits the active transient lake-fill preview into authored world state.
    #[func]
    pub fn commit_world_lake_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_lake_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world lake fill preview failed: {}", err);
                false
            }
        }
    }

    /// Commits the active transient open-water preview into authored world state.
    #[func]
    pub fn commit_world_open_water_fill_preview(&mut self) -> bool {
        let result = {
            let mut core = self.lock_core();
            core.commit_world_open_water_fill_preview_internal()
        };
        match result {
            Ok(()) => {
                self.refresh_snapshot_from_core();
                true
            }
            Err(err) => {
                godot_error!("Commit world open water preview failed: {}", err);
                false
            }
        }
    }

    /// Cancels the active transient lake-fill preview.
    #[func]
    pub fn cancel_world_lake_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Cancels the active transient open-water preview.
    #[func]
    pub fn cancel_world_open_water_fill_preview(&mut self) -> bool {
        let cancelled = {
            let mut core = self.lock_core();
            core.cancel_world_water_fill_preview_internal()
        };
        if cancelled {
            self.refresh_snapshot_from_core();
        }
        cancelled
    }

    /// Removes the nearest authored lake fill within the given radius.
    #[func]
    pub fn remove_world_lake_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_lake_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Removes the nearest authored open-water fill within the given radius.
    #[func]
    pub fn remove_world_open_water_fill_near(&mut self, pos: Vector2, radius_m: f32) -> bool {
        let removed = {
            let mut core = self.lock_core();
            core.remove_world_open_water_fill_near_internal(pos, radius_m)
        };
        if removed {
            self.refresh_snapshot_from_core();
        }
        removed
    }

    /// Returns the current city treasury balance in currency units. May be negative.
    #[func]
    pub fn get_treasury_balance(&self) -> f64 {
        self.snapshot.read().unwrap().treasury_balance
    }

    /// Returns the total number of live agents from the latest render snapshot.
    #[func]
    pub fn get_agent_count(&self) -> i32 {
        self.snapshot.read().unwrap().agent_count
    }

    /// Returns global lane width.
    #[func]
    pub fn get_lane_width(&self) -> f32 {
        config::LANE_WIDTH
    }

    /// High-level city setup for performance testing.
    #[func]
    pub fn setup_benchmark_city(&mut self, grid_size: i32, agent_count: i32) {
        self.lock_core()
            .setup_benchmark_city_internal(grid_size, agent_count);
    }

    /// Returns performance stats (ms, FPS, agents).
    #[func]
    pub fn get_perf_stats(&self) -> VarDictionary {
        self.get_perf_stats_internal()
    }
}

fn zoning_parcel_geometry_dict(
    core: &SimCore,
    geometry: &crate::simulation::zoning::ParcelGeometry,
    runtime_id: u16,
    occupied: bool,
    parcel_id: u64,
) -> VarDictionary {
    let mut corners = PackedVector3Array::new();
    for corner in zoning_parcel_surface_corners(core, geometry) {
        corners.push(corner);
    }

    let color = zoning_parcel_color(core, runtime_id, occupied);

    let mut dict = VarDictionary::new();
    dict.set("id", i64::try_from(parcel_id).unwrap_or(i64::MAX));
    dict.set("profile_runtime_id", i64::from(runtime_id));
    dict.set("occupied", occupied);
    dict.set("corners", corners);
    dict.set("color", color);
    dict
}

fn zoning_parcel_color(core: &SimCore, runtime_id: u16, occupied: bool) -> Color {
    let mut color = if runtime_id == 0 {
        Color::from_rgba(0.78, 0.82, 0.78, 0.30)
    } else if let Some(profile) = core.zoning.profiles.profile_by_runtime_id(runtime_id) {
        Color::from_rgba(
            profile.ui_color_rgb[0] as f32 / 255.0,
            profile.ui_color_rgb[1] as f32 / 255.0,
            profile.ui_color_rgb[2] as f32 / 255.0,
            0.34,
        )
    } else {
        Color::from_rgba(0.78, 0.82, 0.78, 0.30)
    };
    if occupied {
        color = Color::from_rgba(color.r * 0.55, color.g * 0.55, color.b * 0.55, 0.28);
    }
    color
}

fn zoning_parcel_geometries_array(
    core: &SimCore,
    geometries: &[crate::simulation::zoning::ParcelGeometry],
    runtime_id: u16,
) -> VarArray {
    let mut arr = VarArray::new();
    for geometry in geometries {
        let dict = zoning_parcel_geometry_dict(core, geometry, runtime_id, false, 0);
        arr.push(&dict.to_variant());
    }
    arr
}

fn zoning_parcel_geometries_packed_dict(
    core: &SimCore,
    geometries: &[crate::simulation::zoning::ParcelGeometry],
    runtime_id: u16,
) -> VarDictionary {
    let mut corners = PackedVector3Array::new();
    for geometry in geometries {
        for corner in zoning_parcel_surface_corners(core, geometry) {
            corners.push(corner);
        }
    }

    let mut dict = VarDictionary::new();
    dict.set(
        "parcel_count",
        i64::try_from(geometries.len()).unwrap_or(i64::MAX),
    );
    dict.set("corners", corners);
    dict.set("color", zoning_parcel_color(core, runtime_id, false));
    dict
}

fn zoning_parcel_cell_dimensions(
    config: &WorldConfig,
    frontage_cells: i32,
    depth_cells: i32,
) -> Option<(f32, f32)> {
    if frontage_cells <= 0
        || depth_cells <= 0
        || !config.zone_cell_m.is_finite()
        || config.zone_cell_m <= 0.0
    {
        return None;
    }
    let frontage_m = frontage_cells as f32 * config.zone_cell_m;
    let depth_m = depth_cells as f32 * config.zone_cell_m;
    if frontage_m.is_finite() && depth_m.is_finite() {
        Some((frontage_m, depth_m))
    } else {
        None
    }
}

fn zoning_parcel_surface_corners(
    core: &SimCore,
    geometry: &crate::simulation::zoning::ParcelGeometry,
) -> [Vector3; 4] {
    geometry.corners.map(|corner| {
        let surface_y = core.get_world_surface_height_internal(Vector2::new(corner.x, corner.y));
        Vector3::new(corner.x, surface_y, corner.y)
    })
}

#[godot_api]
impl INode3D for SimulationNode {
    fn init(base: Base<Node3D>) -> Self {
        debug_log!("init", "Simulation Engine Initialized");

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let mut generate_benchmark = false;
        let mut run_benchmark = false;
        let mut asset_editor_mode = false;
        let mut economy_editor_mode = false;
        let mut world_editor_mode = false;
        for arg in args.as_slice() {
            match arg.to_string().as_str() {
                "--huge-map" | "--benchmark" => {
                    run_benchmark = true;
                }
                "--generate-benchmark" => {
                    generate_benchmark = true;
                }
                "--asset-editor" => {
                    asset_editor_mode = true;
                    // Always enable debug logging in the asset editor so creators
                    // can follow export/validation output in the terminal.
                    crate::debug::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                "--economy-editor" => {
                    economy_editor_mode = true;
                    crate::debug::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                "--world-editor" => {
                    world_editor_mode = true;
                }
                _ => {}
            }
        }

        let config = if asset_editor_mode || economy_editor_mode || world_editor_mode {
            WorldConfig::editor_sandbox()
        } else {
            WorldConfig::gameplay_default()
        };

        let benchmark_mode = run_benchmark || generate_benchmark;

        let core = SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(
                load_runtime_economy_tuning()
                    .map(|t| t.startup_treasury_balance)
                    .unwrap_or(100_000.0),
            ),
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_water_boundary_points: Vec::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            water_runtime_realtime_when_paused: world_editor_mode,
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            benchmark_mode,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_surface_debug_edges: Vec::new(),
            refined_terrain_patch_cache: HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            cached_road_mesh_data: None,
            camera_aabb: (0.0, 0.0, 0.0, 0.0), // 0.0 == 0.0 → cull disabled by default
        };

        let road_preview_context =
            Arc::new(RwLock::new(RoadPreviewWorkerContext::from_core(&core)));
        let road_tool_query_snapshot =
            Arc::new(RwLock::new(RoadToolQuerySnapshot::from_core(&core)));
        let road_preview_result = Arc::new(RwLock::new(None));
        let (road_preview_tx, road_preview_rx) = std::sync::mpsc::channel();
        let _road_preview_thread = {
            let context = Arc::clone(&road_preview_context);
            let result = Arc::clone(&road_preview_result);
            std::thread::spawn(move || {
                run_road_preview_worker(context, result, road_preview_rx);
            })
        };

        let core_arc = Arc::new(Mutex::new(core));
        let snapshot = Arc::new(RwLock::new(RenderSnapshot::default()));
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();

        if generate_benchmark {
            godot_print!("BENCHMARK GENERATION MODE — will build city, save, and exit");
        } else if run_benchmark {
            godot_print!("BENCHMARK RUN MODE — will load benchmark.sav and simulate");
        }

        Self {
            core: core_arc,
            snapshot,
            sim_thread: None,
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            road_preview_tx,
            road_preview_context,
            road_preview_result,
            road_tool_query_snapshot,
            road_preview_request_counter: AtomicU64::new(0),
            benchmark_mode,
            asset_editor_mode,
            economy_editor_mode,
            world_editor_mode,
            benchmark_tick_count: 0,
            last_logged_day: 0,
            time_passed: 0.0,
            base,
        }
    }

    fn ready(&mut self) {
        godot::classes::Engine::singleton().set_max_fps(crate::config::TARGET_FPS as i32);

        let args = godot::classes::Os::singleton().get_cmdline_user_args();
        let generate = args
            .as_slice()
            .iter()
            .any(|a| a.to_string() == "--generate-benchmark");
        let run = args
            .as_slice()
            .iter()
            .any(|a| matches!(a.to_string().as_str(), "--benchmark" | "--huge-map"));

        if generate {
            self.generate_benchmark_map();
            return; // generate_benchmark_map() calls quit() — never reaches thread spawn
        } else if run {
            self.run_benchmark_from_save();
        }

        // Asset editor mode: sandbox only — no simulation thread.
        if self.asset_editor_mode {
            godot_print!(
                "[asset-editor] sandbox ready — {:.0} m world, no simulation thread",
                WorldConfig::EDITOR_SANDBOX_WIDTH_M
            );
            debug_log!("asset-editor", "sandbox ready");
            return;
        }
        if self.economy_editor_mode {
            godot_print!(
                "[economy-editor] shell ready — authoritative economy data only, no simulation thread"
            );
            debug_log!("economy-editor", "shell ready");
            return;
        }
        if self.world_editor_mode {
            godot_print!(
                "[world-editor] shell ready — blank-world authoring runtime, simulation thread available"
            );
            debug_log!("world-editor", "shell ready");
        }

        // Start the background simulation thread.
        self.start_sim_thread();
    }

    fn process(&mut self, delta: f64) {
        self.time_passed += delta;

        if self.benchmark_mode {
            self.benchmark_tick_count += 1;

            // Periodic console log every 600 frames.
            if self.benchmark_tick_count % 600 == 0 {
                let snap = self.snapshot.read().unwrap();
                godot_print!(
                    "[bench] frame={} agents={} agent_tick_us={} sim_tick_ms={:.2} pathfinds={} RSS={}MB",
                    self.benchmark_tick_count,
                    snap.agent_count,
                    snap.last_agent_tick_us,
                    snap.last_tick_ms,
                    snap.pathfind_count,
                    crate::nodes::sim::benchmark::rss_mb()
                );
            }

            // CSV log once per in-game day.
            {
                let day = self.snapshot.read().unwrap().current_day;
                if day > self.last_logged_day {
                    self.last_logged_day = day;
                    self.log_benchmark_to_csv();
                }
            }

            if self.benchmark_tick_count >= 3000 {
                godot_print!("[bench] DONE — 3000 frames complete. See benchmark_results.csv.");
                self.base_mut().get_tree().unwrap().quit();
            }
        }
    }
}

impl SimCore {
    pub(crate) fn collect_refined_terrain_patch_build_inputs(
        &mut self,
        render_step_m: f32,
    ) -> Vec<RefinedTerrainPatchBuildInput> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let dirty_patches: Vec<(usize, usize)> = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        if dirty_patches.is_empty() {
            return Vec::new();
        }

        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let road_locked_margin_m =
            terrain_cdt_local_sample_margin_m(&self.heightmap, safe_render_step_m);
        let road_locked_start = road_debug.then(Instant::now);
        self.transit_network
            .road_surface
            .compile_dirty(&self.region_graph, &self.heightmap);
        let road_locked_key_vec = self
            .transit_network
            .road_surface
            .terrain_render_patch_keys_with_visible_road_margin(
                &self.region_graph,
                &self.heightmap,
                road_locked_margin_m,
            );
        let road_locked_keys: HashSet<(usize, usize)> =
            road_locked_key_vec.iter().copied().collect();
        self.road_locked_terrain_patch_keys = road_locked_key_vec;
        let road_locked_ms = road_locked_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let mut inputs = Vec::new();
        for (patch_x, patch_z) in dirty_patches.iter().copied() {
            if !road_locked_keys.contains(&(patch_x, patch_z)) {
                continue;
            }
            let Some(base_patch) = self.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
                continue;
            };
            let road_clip_query = SimulationNode::road_clip_loop_query_for_bounds(
                self,
                base_patch.world_origin_x - road_locked_margin_m,
                base_patch.world_origin_z - road_locked_margin_m,
                base_patch.world_origin_x + base_patch.world_size_x + road_locked_margin_m,
                base_patch.world_origin_z + base_patch.world_size_z + road_locked_margin_m,
            );
            let key = SimulationNode::refined_patch_cache_key(patch_x, patch_z, safe_render_step_m);
            let previous = self.refined_terrain_patch_cache.get(&key);
            let windows = SimulationNode::terrain_cdt_window_build_inputs(
                &self.heightmap,
                &base_patch,
                &road_clip_query.cdt_road_loops,
                safe_render_step_m,
                previous,
            );
            inputs.push(RefinedTerrainPatchBuildInput {
                key,
                patch: base_patch,
                windows,
                road_clip_source_count: road_clip_query.source_count,
                clip_error_label: road_clip_query.clip_error_label,
            });
        }

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_precompute_inputs dirty_patches={} road_locked_patches={} inputs={} road_locked_margin_m={:.3} road_locked_ms={:.3} total_ms={:.3}",
                dirty_patches.len(),
                road_locked_keys.len(),
                inputs.len(),
                road_locked_margin_m,
                road_locked_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        inputs
    }

    pub(crate) fn build_refined_terrain_patch_cache_entries(
        inputs: Vec<RefinedTerrainPatchBuildInput>,
    ) -> Vec<CachedRefinedTerrainPatch> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let input_count = inputs.len();
        let mut entries: Vec<CachedRefinedTerrainPatch> = inputs
            .into_par_iter()
            .map(|input| {
                let mut cdt_ms = 0.0;
                let mut reused_windows = 0usize;
                let mut windows = input
                    .windows
                    .into_par_iter()
                    .map(|window| {
                        if let Some(mut previous) = window.previous {
                            previous.reused = true;
                            return previous;
                        }
                        let input_road_loops = window.cdt_input.road_loops.len();
                        let input_source_samples = window.cdt_input.source_samples.len();
                        let cdt_patch = window.cdt_input.patch;
                        let cdt_start = Instant::now();
                        let mesh_result = build_road_touched_terrain_patch(window.cdt_input);
                        let cdt_ms = cdt_start.elapsed().as_secs_f64() * 1000.0;
                        CachedRefinedTerrainCdtWindow {
                            key: window.key,
                            input_road_loops,
                            input_source_samples,
                            cdt_patch,
                            mesh_result,
                            cdt_ms,
                            reused: false,
                        }
                    })
                    .collect::<Vec<_>>();
                windows.sort_by_key(|window| window.key);
                for window in &windows {
                    if window.reused {
                        reused_windows += 1;
                    } else {
                        cdt_ms += window.cdt_ms;
                    }
                }
                let input_road_loops = windows
                    .iter()
                    .map(|window| window.input_road_loops)
                    .sum::<usize>();
                let input_source_samples = windows
                    .iter()
                    .map(|window| window.input_source_samples)
                    .sum::<usize>();
                CachedRefinedTerrainPatch {
                    key: input.key,
                    patch: input.patch,
                    input_road_loops,
                    input_source_samples,
                    windows,
                    road_clip_source_count: input.road_clip_source_count,
                    clip_error_label: input.clip_error_label,
                    cdt_ms,
                    reused_windows,
                }
            })
            .collect();
        entries.sort_by_key(|entry| (entry.key.patch_x, entry.key.patch_z));

        if road_debug {
            for entry in &entries {
                let status = if entry.windows.is_empty() {
                    "empty"
                } else if entry.windows.iter().any(|window| {
                    window
                        .mesh_result
                        .as_ref()
                        .is_ok_and(|mesh| mesh.stats.invalid_constraint_edges > 0)
                }) {
                    "conflicted"
                } else if let Some(err) = entry
                    .windows
                    .iter()
                    .find_map(|window| window.mesh_result.as_ref().err())
                {
                    SimulationNode::terrain_cdt_error_label(err)
                } else {
                    "ok"
                };
                debug_log!(
                    "road",
                    "refined_patch_precompute key=({},{}) render_step_mm={} status={} windows={} reused_windows={} road_loops={} source_samples={} cdt_ms={:.3}",
                    entry.key.patch_x,
                    entry.key.patch_z,
                    entry.key.render_step_mm,
                    status,
                    entry.windows.len(),
                    entry.reused_windows,
                    entry.input_road_loops,
                    entry.input_source_samples,
                    entry.cdt_ms
                );
            }
            debug_log!(
                "road",
                "refined_patch_precompute_total inputs={} built={} total_ms={:.3}",
                input_count,
                entries.len(),
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        entries
    }

    pub(crate) fn insert_refined_terrain_patch_cache_entries(
        &mut self,
        entries: Vec<CachedRefinedTerrainPatch>,
    ) {
        for entry in entries {
            self.refined_terrain_patch_cache.insert(entry.key, entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::terrain::TerrainPatchSnapshot;
    use crate::simulation::terrain::cdt::{
        TerrainCdtEarthworkSupportPolicy, TerrainCdtEdgeClass, TerrainCdtNodePieceKind,
        TerrainCdtRoadBandKind, TerrainCdtRoadLoopSourceEdge, TerrainCdtSpanRegionRole,
    };

    #[test]
    fn zoning_parcel_surface_corners_use_visible_world_surface_height() {
        let raw_height = 3.25;
        let core = test_core_with_flat_terrain(raw_height);
        let geometry = crate::simulation::zoning::ParcelGeometry {
            edge_idx: 0,
            side: 1,
            frontage_center_t: 0.5,
            frontage_m: 20.0,
            depth_m: 30.0,
            front_center: Vector2::ZERO,
            center: Vector2::ZERO,
            tangent: Vector2::new(1.0, 0.0),
            normal: Vector2::new(0.0, -1.0),
            corners: [
                Vector2::new(-5.0, -5.0),
                Vector2::new(5.0, -5.0),
                Vector2::new(5.0, 5.0),
                Vector2::new(-5.0, 5.0),
            ],
            aabb_min: Vector2::new(-5.0, -5.0),
            aabb_max: Vector2::new(5.0, 5.0),
        };

        let corners = zoning_parcel_surface_corners(&core, &geometry);
        let expected_y = raw_height * config::HEIGHT_SCALE;

        assert!(corners.iter().all(|corner| {
            (corner.y - expected_y).abs() <= 1e-4 && corner.x.abs() == 5.0 && corner.z.abs() == 5.0
        }));
    }

    #[test]
    fn zoning_parcel_cell_dimensions_use_world_zone_cell_size() {
        let mut config = WorldConfig::default();
        config.zone_cell_m = 10.0;

        assert_eq!(
            zoning_parcel_cell_dimensions(&config, 2, 3),
            Some((20.0, 30.0))
        );
        assert_eq!(zoning_parcel_cell_dimensions(&config, 0, 3), None);
    }

    #[test]
    fn terrain_cdt_structured_face_sources_preserve_span_fields() {
        let source = span_source();
        let export = SimulationNode::terrain_cdt_triangle_buffers(
            &test_patch(),
            &[
                TerrainCdtVertex::new(0.0, 0.0, 0.0),
                TerrainCdtVertex::new(1.0, 0.0, 0.0),
                TerrainCdtVertex::new(0.0, 0.0, 1.0),
            ],
            &[[0, 1, 2]],
            &[vec![source]],
            true,
        );

        assert_eq!(export.emitted_faces, 1);
        assert_eq!(export.face_sources.counts, vec![1]);
        assert_eq!(export.face_sources.labels.len(), 1);
        assert_eq!(export.face_sources.kind_codes, vec![0]);
        assert_eq!(export.face_sources.primary_ids, vec![123]);
        assert_eq!(export.face_sources.node_kind_codes, vec![-1]);
        assert_eq!(export.face_sources.edge_class_codes, vec![1]);
        assert_eq!(export.face_sources.owner_kinds, vec![2]);
        assert_eq!(export.face_sources.owner_indices, vec![7]);
        assert_eq!(export.face_sources.support_policies, vec![1]);
        assert_eq!(export.face_sources.roles, vec![2]);
        assert_eq!(export.face_sources.section_ranges, vec![2, 5]);
        assert_eq!(export.face_sources.s_ranges, vec![10.5, 14.0]);
    }

    #[test]
    fn terrain_cdt_structured_face_sources_preserve_node_fields() {
        let source = node_source();
        let export = SimulationNode::terrain_cdt_triangle_buffers(
            &test_patch(),
            &[
                TerrainCdtVertex::new(0.0, 0.0, 0.0),
                TerrainCdtVertex::new(1.0, 0.0, 0.0),
                TerrainCdtVertex::new(0.0, 0.0, 1.0),
            ],
            &[[0, 1, 2]],
            &[vec![source]],
            true,
        );

        assert_eq!(export.emitted_faces, 1);
        assert_eq!(export.face_sources.counts, vec![1]);
        assert_eq!(export.face_sources.labels.len(), 1);
        assert_eq!(export.face_sources.kind_codes, vec![1]);
        assert_eq!(export.face_sources.primary_ids, vec![77]);
        assert_eq!(export.face_sources.node_kind_codes, vec![2]);
        assert_eq!(export.face_sources.edge_class_codes, vec![-1]);
        assert_eq!(export.face_sources.owner_kinds, vec![1]);
        assert_eq!(export.face_sources.owner_indices, vec![3]);
        assert_eq!(export.face_sources.support_policies, vec![-1]);
        assert_eq!(export.face_sources.roles, vec![-1]);
        assert_eq!(export.face_sources.section_ranges, vec![-1, -1]);
        assert_eq!(export.face_sources.s_ranges, vec![-1.0, -1.0]);
    }

    #[test]
    fn terrain_cdt_face_source_counts_skip_degenerate_triangles() {
        let span = span_source();
        let node = node_source();
        let export = SimulationNode::terrain_cdt_triangle_buffers(
            &test_patch(),
            &[
                TerrainCdtVertex::new(0.0, 0.0, 0.0),
                TerrainCdtVertex::new(1.0, 0.0, 0.0),
                TerrainCdtVertex::new(2.0, 0.0, 0.0),
                TerrainCdtVertex::new(0.0, 0.0, 1.0),
            ],
            &[[0, 1, 2], [0, 1, 3]],
            &[vec![span], vec![span, node]],
            true,
        );

        assert_eq!(export.emitted_faces, 1);
        assert_eq!(export.face_sources.counts, vec![2]);
        assert_eq!(export.face_sources.labels.len(), 2);
        assert_eq!(export.face_sources.kind_codes, vec![0, 1]);
        assert_eq!(export.face_sources.primary_ids, vec![123, 77]);
        assert_eq!(export.face_sources.section_ranges, vec![2, 5, -1, -1]);
        assert_eq!(export.face_sources.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
    }

    #[test]
    fn terrain_cdt_road_seam_sample_sources_export_structured_rows() {
        let span = span_source();
        let node = node_source();
        let sources = [span, node];
        let export = source_export_for_samples(&[&sources]);

        assert_eq!(export.counts, vec![2]);
        assert_eq!(export.labels.len(), 2);
        assert_eq!(export.kind_codes, vec![0, 1]);
        assert_eq!(export.primary_ids, vec![123, 77]);
        assert_eq!(export.node_kind_codes, vec![-1, 2]);
        assert_eq!(export.edge_class_codes, vec![1, -1]);
        assert_eq!(export.owner_kinds, vec![2, 1]);
        assert_eq!(export.owner_indices, vec![7, 3]);
        assert_eq!(export.support_policies, vec![1, -1]);
        assert_eq!(export.roles, vec![2, -1]);
        assert_eq!(export.section_ranges, vec![2, 5, -1, -1]);
        assert_eq!(export.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
    }

    #[test]
    fn terrain_cdt_retaining_wall_sample_sources_export_structured_rows() {
        let span = [span_source()];
        let node = [node_source()];
        let export = source_export_for_samples(&[&span, &node]);

        assert_eq!(export.counts, vec![1, 1]);
        assert_eq!(export.labels.len(), 2);
        assert_eq!(export.kind_codes, vec![0, 1]);
        assert_eq!(export.primary_ids, vec![123, 77]);
        assert_eq!(export.owner_kinds, vec![2, 1]);
        assert_eq!(export.owner_indices, vec![7, 3]);
        assert_eq!(export.section_ranges, vec![2, 5, -1, -1]);
        assert_eq!(export.s_ranges, vec![10.5, 14.0, -1.0, -1.0]);
    }

    #[test]
    fn terrain_cdt_tie_in_widened_sample_sources_export_one_source_per_sample() {
        let first = [span_source()];
        let second = [span_source()];
        let export = source_export_for_samples(&[&first, &second]);

        assert_eq!(export.counts, vec![1, 1]);
        assert_eq!(export.labels.len(), 2);
        assert_eq!(export.kind_codes, vec![0, 0]);
        assert_eq!(export.primary_ids, vec![123, 123]);
        assert_eq!(export.section_ranges, vec![2, 5, 2, 5]);
        assert_eq!(export.s_ranges, vec![10.5, 14.0, 10.5, 14.0]);
    }

    #[test]
    fn terrain_cdt_invalid_constraint_sample_sources_keep_absence_visible() {
        let present = [node_source()];
        let export = source_export_for_samples(&[&[], &present]);

        assert_eq!(export.counts, vec![0, 1]);
        assert_eq!(export.labels.len(), 1);
        assert_eq!(export.kind_codes, vec![1]);
        assert_eq!(export.primary_ids, vec![77]);
        assert_eq!(export.owner_kinds, vec![1]);
        assert_eq!(export.owner_indices, vec![3]);
        assert_eq!(export.section_ranges, vec![-1, -1]);
        assert_eq!(export.s_ranges, vec![-1.0, -1.0]);
    }

    #[test]
    fn road_clip_query_metadata_keeps_clip_failure_visible_without_loops() {
        let query = RoadClipLoopQuery {
            cdt_road_loops: Vec::new(),
            source_count: 1,
            clip_error_label: Some("terrain_clip_missing_output_boundary_owner"),
        };

        let (status, error, source_count) = SimulationNode::road_clip_status_values(&query);

        assert_eq!(status, "failed");
        assert_eq!(error, "terrain_clip_missing_output_boundary_owner");
        assert_eq!(source_count, 1);
        assert!(
            query.cdt_road_loops.is_empty(),
            "the failure status must survive even when there are no loops to upload"
        );
    }

    #[test]
    fn road_clip_query_metadata_marks_absent_road_clip_as_ok() {
        let query = RoadClipLoopQuery {
            cdt_road_loops: Vec::new(),
            source_count: 0,
            clip_error_label: None,
        };

        let (status, error, source_count) = SimulationNode::road_clip_status_values(&query);

        assert_eq!(status, "ok");
        assert_eq!(error, "none");
        assert_eq!(source_count, 0);
    }

    #[test]
    fn terrain_cdt_local_window_input_samples_arbitrary_boundary() {
        let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
        let patch = TerrainPatchSnapshot {
            patch_x: 0,
            patch_z: 0,
            sample_width: 5,
            sample_height: 5,
            texture_width: 5,
            texture_height: 5,
            inner_offset_x: 0,
            inner_offset_z: 0,
            world_origin_x: 0.0,
            world_origin_z: 0.0,
            world_size_x: 40.0,
            world_size_z: 40.0,
            height_data: vec![0.0; 25],
        };

        let input = SimulationNode::terrain_cdt_input_for_bounds(
            &terrain,
            &patch,
            &[],
            5.0,
            (3.0, 4.0, 23.0, 29.0),
        );

        assert!(
            input
                .source_samples
                .iter()
                .any(|sample| sample.x == 3.0 && sample.z == 9.0),
            "local CDT windows must seed non-corner vertices along arbitrary vertical boundaries"
        );
        assert!(
            input
                .source_samples
                .iter()
                .any(|sample| sample.x == 18.0 && sample.z == 29.0),
            "local CDT windows must seed non-corner vertices along arbitrary horizontal boundaries"
        );
    }

    #[test]
    fn terrain_cdt_input_adds_grade_limited_guides_for_grounded_standard_roads() {
        let terrain = TerrainSystem::with_chunking(8, 8, 10.0, 4, 0.0);
        let patch = TerrainPatchSnapshot {
            patch_x: 0,
            patch_z: 0,
            sample_width: 5,
            sample_height: 5,
            texture_width: 5,
            texture_height: 5,
            inner_offset_x: 0,
            inner_offset_z: 0,
            world_origin_x: 0.0,
            world_origin_z: 0.0,
            world_size_x: 40.0,
            world_size_z: 40.0,
            height_data: vec![0.0; 25],
        };
        let source = standard_span_source();
        let road = vec![
            TerrainCdtVertex::new(10.0, 3.0, 10.0),
            TerrainCdtVertex::new(30.0, 3.0, 10.0),
            TerrainCdtVertex::new(30.0, 3.0, 20.0),
            TerrainCdtVertex::new(10.0, 3.0, 20.0),
        ];
        let source_edges = road
            .iter()
            .copied()
            .enumerate()
            .map(|(index, start)| TerrainCdtRoadLoopSourceEdge {
                start,
                end: road[(index + 1) % road.len()],
                source,
            })
            .collect();
        let road_loop = TerrainCdtRoadLoop::new_with_source_edges(123, 0, road, source_edges);

        let input = SimulationNode::terrain_cdt_input_for_bounds(
            &terrain,
            &patch,
            &[road_loop],
            2.0,
            (0.0, 0.0, 40.0, 40.0),
        );

        assert!(
            input.tie_in_guide_samples.iter().any(|sample| {
                (sample.vertex.x - 10.0).abs() <= 0.001
                    && (sample.vertex.z - 8.0).abs() <= 0.001
                    && (sample.vertex.height_m - 2.0).abs() <= 0.001
            }),
            "grounded Standard road tie-ins should add explicit guide vertices at the slope budget"
        );

        let mesh = build_road_touched_terrain_patch(input)
            .expect("grade-limited grounded road tie-in should triangulate");
        assert_eq!(mesh.stats.retaining_wall_faces, 0);
        assert!(mesh.retaining_wall_triangles.is_empty());
    }

    #[test]
    fn terrain_cdt_grid_sampling_is_bounded_for_large_local_windows() {
        let small_step = SimulationNode::terrain_cdt_grid_sample_step_m(0.0, 0.0, 32.0, 32.0, 1.0);
        assert_eq!(small_step, 1.0);

        let large_step =
            SimulationNode::terrain_cdt_grid_sample_step_m(0.0, 0.0, 512.0, 512.0, 1.0);
        assert!(
            large_step > 1.0,
            "large CDT windows must not keep one source sample per metre across the whole area"
        );
    }

    #[test]
    fn regular_terrain_filler_refines_cdt_window_sides() {
        let patch = TerrainPatchSnapshot {
            patch_x: 0,
            patch_z: 0,
            sample_width: 5,
            sample_height: 5,
            texture_width: 5,
            texture_height: 5,
            inner_offset_x: 0,
            inner_offset_z: 0,
            world_origin_x: 0.0,
            world_origin_z: 0.0,
            world_size_x: 40.0,
            world_size_z: 40.0,
            height_data: vec![0.0; 25],
        };
        let cdt_patch = TerrainCdtPatch::new(10.0, 10.0, 30.0, 30.0, [0.0; 4]);
        let window = SimulationNode::terrain_cdt_window_bounds(&patch, cdt_patch, 5.0).unwrap();
        let mut export = TerrainCdtTriangleBufferExport::empty();

        SimulationNode::append_regular_terrain_mesh_outside_cdt_windows(
            &mut export,
            &patch,
            &[window],
        );

        assert!(
            export_has_world_xz(&export, &patch, 10.0, 15.0),
            "regular filler must share non-corner vertical CDT-window boundary samples"
        );
        assert!(
            export_has_world_xz(&export, &patch, 15.0, 10.0),
            "regular filler must share non-corner horizontal CDT-window boundary samples"
        );
    }

    fn export_has_world_xz(
        export: &TerrainCdtTriangleBufferExport,
        patch: &TerrainPatchSnapshot,
        world_x: f32,
        world_z: f32,
    ) -> bool {
        let center_x = patch.world_origin_x + patch.world_size_x * 0.5;
        let center_z = patch.world_origin_z + patch.world_size_z * 0.5;
        export.vertices.iter().any(|vertex| {
            (vertex.x - (world_x - center_x)).abs() <= 0.001
                && (vertex.z - (world_z - center_z)).abs() <= 0.001
        })
    }

    fn test_patch() -> TerrainPatchSnapshot {
        TerrainPatchSnapshot {
            patch_x: 0,
            patch_z: 0,
            sample_width: 2,
            sample_height: 2,
            texture_width: 2,
            texture_height: 2,
            inner_offset_x: 0,
            inner_offset_z: 0,
            world_origin_x: 0.0,
            world_origin_z: 0.0,
            world_size_x: 10.0,
            world_size_z: 10.0,
            height_data: vec![0.0; 4],
        }
    }

    fn test_core_with_flat_terrain(raw_height: f32) -> SimCore {
        let config = WorldConfig::default();
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::with_chunking(8, 8, 10.0, 4, raw_height),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new_with_surface_chunk_span(config.terrain_chunk_m),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(0.0),
            debug_household_admissions_since_daily: 0,
            undo_stack: std::collections::VecDeque::new(),
            world_water_boundary_points: Vec::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: std::collections::HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            water_runtime_realtime_when_paused: false,
            terrain_dirty: false,
            water_dirty: false,
            network_dirty: false,
            benchmark_mode: false,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_surface_debug_edges: Vec::new(),
            refined_terrain_patch_cache: std::collections::HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            cached_road_mesh_data: None,
            camera_aabb: (0.0, 0.0, 0.0, 0.0),
        }
    }

    fn source_export_for_samples(
        samples: &[&[TerrainCdtRoadBoundarySource]],
    ) -> TerrainCdtSourceExport {
        let mut export = TerrainCdtSourceExport::with_sample_capacity(samples.len());
        for sources in samples {
            export.push_sources(sources);
        }
        export
    }

    fn span_source() -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            edge_idx: 123,
            edge_class: TerrainCdtEdgeClass::Bridge,
            support_policy: TerrainCdtEarthworkSupportPolicy::BridgeEndpointAbutments,
            source_band_index: 7,
            band_kind: TerrainCdtRoadBandKind::Sidewalk,
            role: TerrainCdtSpanRegionRole::NonRoad,
            start_section_index: 2,
            end_section_index: 5,
            start_s_m: 10.5,
            end_s_m: 14.0,
        }
    }

    fn standard_span_source() -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::SpanSupportBoundary {
            edge_idx: 123,
            edge_class: TerrainCdtEdgeClass::Standard,
            support_policy: TerrainCdtEarthworkSupportPolicy::StandardFullGroundedSpan,
            source_band_index: 7,
            band_kind: TerrainCdtRoadBandKind::Sidewalk,
            role: TerrainCdtSpanRegionRole::NonRoad,
            start_section_index: 2,
            end_section_index: 5,
            start_s_m: 10.5,
            end_s_m: 14.0,
        }
    }

    fn node_source() -> TerrainCdtRoadBoundarySource {
        TerrainCdtRoadBoundarySource::NodeFootprintBoundary {
            node_id: 77,
            node_kind: TerrainCdtNodePieceKind::JunctionN,
            owner_kind: TerrainCdtRoadBandKind::CurbOrShoulder,
            owner_index: 3,
            boundary_source: None,
        }
    }
}
