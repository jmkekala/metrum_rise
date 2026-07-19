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
//! | | `get_economy_overview` | `economy_overview.gd` |
//! | | `set_economy_service_funding` | `economy_overview.gd` |
//! | | `get_demand_pressures` | `main_ui.gd` |
//! | | `apply_money_and_max_demand_cheat` | `input_manager.gd` |
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
//! | | `acknowledge_terrain_patches` | `terrain.gd` |
//! | | `get_terrain_patch_layout` | `terrain.gd`, `water.gd` |
//! | | `get_dirty_terrain_patch_payload_states` | `terrain.gd` |
//! | | `request_terrain_patch_payloads` | `terrain.gd` |
//! | | `poll_ready_terrain_patch_payloads` | `terrain.gd` |
//! | | `get_terrain_border_loop` | `terrain.gd`, `water.gd` |
//! | | `get_height_at` | `road_tool.gd`, `building_tool.gd` |
//! | | `intersect_terrain` | `input_manager.gd` (mouse pick) |
//! | | `get_world_surface_height` | `road_tool.gd`, `move_tool.gd` |
//! | | `intersect_world_surface` | `road_tool.gd`, `select_tool.gd` |
//! | **Water** | `is_water_dirty` | `water.gd` |
//! | | `acknowledge_water_patches` | `water.gd` |
//! | | `get_dirty_water_patch_payload_states` | `water.gd` |
//! | | `request_water_patch_payloads` | `water.gd` |
//! | | `poll_ready_water_patch_payloads` | `water.gd` |
//! | | `request_water_patch_meshes` | `water.gd` |
//! | | `poll_ready_water_patch_meshes` | `water.gd` |
//! | | `clear_water_patch_mesh_cache` | `water.gd` |
//! | | `get_water_patch_debug` | `water.gd` |
//! | | `get_water_patch_authored_fill_debug` | `water.gd` |
//! | | `get_water_border_depths` | `water.gd` |
//! | **Network** | `add_road` | `road_tool.gd` |
//! | | `is_network_dirty` | `network_renderer.gd` |
//! | | `get_network_render_generation` | `network_renderer.gd` |
//! | | `acknowledge_network_render` | `network_renderer.gd` |
//! | | `get_road_mesh_data` | `network_renderer.gd` |
//! | | `validate_road_candidate` | `road_tool.gd` |
//! | | `request_preview_road_surface` | `road_tool.gd` |
//! | | `get_preview_road_surface_result` | `road_tool.gd` |
//! | | `get_road_surface_debug_data` | `network_tool.gd` |
//! | | `get_road_surface_probe_debug` | `network_tool.gd` |
//! | | `get_road_tool_cursor_pos` | `road_tool.gd` |
//! | | `get_closest_network_point` | `road_tool.gd`, `zoning_tool.gd` |
//! | | `check_border_candidate` | `road_tool.gd` |
//! | | `set_border_connection` | `road_tool.gd` |
//! | | `get_bulldoze_target_at` | `bulldoze_tool.gd` |
//! | | `bulldoze_at` | `bulldoze_tool.gd` |
//! | **Services** | `get_service_building_assets` | `main_ui.gd` |
//! | | `get_service_building_placement_preview` | `service_building_tool.gd` |
//! | | `place_service_building` | `service_building_tool.gd` |
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
//! | | `get_zoning_overlay_revision` | `zoning_overlay.gd` |
//! | | `get_zoning_overlay_occupancy_revision` | `zoning_overlay.gd` |
//! | | `get_zoning_parcels_overlay` | `zoning_overlay.gd` |
//! | | `try_get_zoning_parcels_overlay_packed` | `zoning_overlay.gd` |
//! | **Agents** | `get_agent_transforms` | `agent_renderer.gd` |
//! | | `get_car_transforms` | `agent_renderer.gd` |
//! | | `get_car_render_ids` | `agent_renderer.gd` |
//! | | `set_camera_aabb` | `agents.gd` (culling update) |

use godot::classes::{INode3D, Node3D};
use godot::prelude::*;

use crate::config;
use crate::nodes::sim::core::{
    CachedRefinedTerrainCdtWindow, CachedRefinedTerrainMeshBuffers, CachedRefinedTerrainPatch,
    CityTreasury, DailyBudgetLedgerEntry, ROAD_BUILD_COST_PER_METER,
    RefinedTerrainCdtWindowBuildInput, RefinedTerrainCdtWindowKey, RefinedTerrainPatchBuildInput,
    RefinedTerrainPatchCacheKey, RenderSnapshot, RoadPreviewRequest, RoadPreviewSnapshot,
    RoadPreviewWorkerContext, RoadToolQuerySnapshot, SERVICE_POLICY_ELECTRICITY, SimCommand,
    SimCore, road_tool_snapshots_from_core, run_road_preview_worker, run_sim_thread,
};
use crate::nodes::sim::core::{
    WorldLakeFillPreview, WorldLakeFillPreviewStatus, WorldWaterFillKind,
};
use crate::nodes::sim::render::water::{
    CachedWaterPatchMesh, WaterPatchMeshBuildInput, WaterPatchMeshCacheKey,
    water_patch_depth_signature,
};
use crate::simulation::buildings::allocator::{
    BuildingAllocator, BuildingSiteGradingRequest, BuildingSiteTerrainSnapshot,
    ExplicitServicePlacementRejection,
};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::surface::{
    RoadPreviewValidation, RoadSurfaceCompileReason, RoadSurfaceSystem,
};
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch,
    TerrainCdtRoadBoundarySource, TerrainCdtRoadLoop, TerrainCdtStats, TerrainCdtVertex,
    build_road_touched_terrain_patch,
};
use crate::simulation::terrain::{TerrainPatchSnapshot, TerrainSystem};
use crate::simulation::water::{WaterPatchSnapshot, WaterSystem};
use crate::simulation::zoning::ZoningSystem;

use crate::debug_log;
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

mod asset_api;
mod async_terrain;
mod economy_api;
mod network_api;
mod system_api;
mod terrain_api;
#[cfg(test)]
mod tests;
mod variant_export;
mod water_api;
mod world_api;
mod zoning_api;

use async_terrain::{
    TerrainPatchPayload, TerrainPatchPayloadAsyncState, TerrainPatchPayloadData,
    TerrainPatchPayloadRequest, TerrainPatchPayloadRequestState, WaterPatchMeshAsyncState,
    WaterPatchPayload, WaterPatchPayloadAsyncState, WaterPatchPayloadRequest,
    WaterPatchPayloadRequestState,
};
use variant_export::{
    TERRAIN_CDT_TILE_NEIGHBORS, TerrainCdtTileId, budget_ledger_entry_dict,
    zoning_geometries_without_explicit_sites, zoning_parcel_cell_dimensions, zoning_parcel_color,
    zoning_parcel_geometries_array, zoning_parcel_geometries_packed_dict,
    zoning_parcel_geometry_dict, zoning_parcel_surface_corners,
};

const TERRAIN_CDT_DIAGNOSTIC_STAGE_LABEL: &str = "cdt_triangulation";
const TERRAIN_CDT_DIAGNOSTIC_STAGE_CODE: i64 = 0;
const TERRAIN_CDT_BACKEND_NONE_LABEL: &str = "none";
const TERRAIN_CDT_BACKEND_NONE_CODE: i64 = -1;
const TERRAIN_CDT_BACKEND_SPADE_LABEL: &str = "spade";
const TERRAIN_CDT_BACKEND_SPADE_CODE: i64 = 0;
const TERRAIN_CDT_CONTRACT_REVISION: i64 = 4;
const TERRAIN_CDT_FAR_SAMPLE_MIN_STEP_M: f32 = 8.0;
const TERRAIN_CDT_MAX_LOCAL_GRID_SAMPLES: f32 = 8_192.0;
const TERRAIN_CDT_SAMPLE_KEY_SCALE: f64 = 1000.0;
const TERRAIN_CDT_PATHOLOGICAL_FACE_SLOPE_RATIO: f32 = 256.0;
const TERRAIN_CDT_PATHOLOGICAL_TRIANGLE_EDGE_M: f32 = 96.0;
const CHEAT_MONEY_GRANT_AMOUNT: f64 = 1_000_000.0;

#[derive(Clone, Copy)]
struct TerrainCdtSiteGradingContext<'a> {
    source: TerrainCdtSiteGradingSource<'a>,
    graph: &'a crate::simulation::network::graph::RegionGraph,
    road_surface: &'a RoadSurfaceSystem,
}

#[derive(Clone, Copy)]
enum TerrainCdtSiteGradingSource<'a> {
    Snapshot(&'a BuildingSiteTerrainSnapshot),
}

impl TerrainCdtSiteGradingContext<'_> {
    fn append_guides(
        self,
        terrain: &TerrainSystem,
        world_bounds: (f32, f32, f32, f32),
        render_step_m: f32,
        tie_in_guide_samples: &mut Vec<crate::simulation::terrain::cdt::TerrainCdtTieInGuideSample>,
        sample_keys: &mut BTreeMap<(i64, i64), ()>,
    ) {
        let request = || {
            BuildingSiteGradingRequest::new(
                terrain,
                self.graph,
                self.road_surface,
                world_bounds,
                render_step_m,
            )
        };
        match self.source {
            TerrainCdtSiteGradingSource::Snapshot(snapshot) => snapshot
                .append_terrain_cdt_site_grading_guides_for_world_bounds(
                    request(),
                    tie_in_guide_samples,
                    sample_keys,
                ),
        }
    }
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
    /// Water mesh jobs currently being prepared outside the Godot frame.
    pub(crate) water_patch_mesh_jobs: Arc<Mutex<WaterPatchMeshAsyncState>>,
    terrain_patch_payload_jobs: Arc<Mutex<TerrainPatchPayloadAsyncState>>,
    water_patch_payload_jobs: Arc<Mutex<WaterPatchPayloadAsyncState>>,
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
    road_source_count: usize,
    road_loop_count: usize,
    site_loop_count: usize,
    clip_error_label: Option<&'static str>,
}

impl SimulationNode {
    // ── Lifecycle ──

    /// Acquires the sim-core mutex.
    ///
    /// Poisoning is an authoritative simulation failure: callers must not expose
    /// potentially partial state after a failed phase.
    #[inline]
    fn lock_core(&self) -> std::sync::MutexGuard<'_, crate::nodes::sim::core::SimCore> {
        self.core
            .lock()
            .expect("simulation core lock poisoned by a failed authoritative phase")
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
            Err(std::sync::TryLockError::Poisoned(_)) => {
                panic!("simulation core lock poisoned by a failed authoritative phase")
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
        let (snapshot, road_tool_snapshots) = {
            let mut core = self.lock_core();
            let snapshot = core.build_snapshot();
            let road_tool_snapshots = road_tool_snapshots_from_core(&core);
            (snapshot, road_tool_snapshots)
        };
        *self.snapshot.write().unwrap() = snapshot;
        if let Some((preview_context, road_query_snapshot)) = road_tool_snapshots {
            *self.road_preview_context.write().unwrap() = preview_context;
            *self.road_tool_query_snapshot.write().unwrap() = road_query_snapshot;
        }
    }
}

#[godot_api]
impl SimulationNode {}

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

        let mut core = SimCore {
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
            pending_demand_spawns: VecDeque::new(),
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
            service_policy: Default::default(),
            budget_history: VecDeque::new(),
            budget_last_lifetime_build_cost: 0.0,
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
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
            road_locked_terrain_patch_margins: BTreeMap::new(),
            building_site_owned_terrain_patch_keys: HashSet::new(),
            engineered_terrain_patch_keys: Vec::new(),
            engineered_terrain_patch_margins: BTreeMap::new(),
            terrain_payload_generation_counter: 1,
            terrain_payload_global_generation: 1,
            terrain_payload_patch_generations: HashMap::new(),
            refined_terrain_assembly_ledgers: HashMap::new(),
            cached_road_mesh_data: None,
            cached_road_mesh_generation: 0,
            cached_network_node_positions: Arc::new(Vec::new()),
            cached_network_node_positions_dirty: true,
            road_tool_surface_generation: 1,
            camera_aabb: (0.0, 0.0, 0.0, 0.0), // 0.0 == 0.0 → cull disabled by default
        };

        core.precompute_road_mesh_data();
        let initial_snapshot = core.build_snapshot();
        let (road_preview_context, road_tool_query_snapshot) = road_tool_snapshots_from_core(&core)
            .expect("the initial road surface generation must be publishable");
        let road_preview_context = Arc::new(RwLock::new(road_preview_context));
        let road_tool_query_snapshot = Arc::new(RwLock::new(road_tool_query_snapshot));
        let road_preview_result = Arc::new(RwLock::new(None));
        let water_patch_mesh_jobs = Arc::new(Mutex::new(WaterPatchMeshAsyncState::new()));
        let terrain_patch_payload_jobs = Arc::new(Mutex::new(TerrainPatchPayloadAsyncState::new()));
        let water_patch_payload_jobs = Arc::new(Mutex::new(WaterPatchPayloadAsyncState::new()));
        let (road_preview_tx, road_preview_rx) = std::sync::mpsc::channel();
        let _road_preview_thread = {
            let context = Arc::clone(&road_preview_context);
            let result = Arc::clone(&road_preview_result);
            std::thread::spawn(move || {
                run_road_preview_worker(context, result, road_preview_rx);
            })
        };

        let core_arc = Arc::new(Mutex::new(core));
        let snapshot = Arc::new(RwLock::new(initial_snapshot));
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
            water_patch_mesh_jobs,
            terrain_patch_payload_jobs,
            water_patch_payload_jobs,
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
