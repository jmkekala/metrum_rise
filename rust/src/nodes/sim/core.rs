//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads only from the snapshot for rendering
//! and locks the `Arc<Mutex<SimCore>>` briefly for mutations (road edits, etc.).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::debug_log;
use crate::nodes::sim::render::lane_pose::sample_lane_pose;
use crate::nodes::sim::road_tool::RoadGhostSnapIndex;
use godot::prelude::{Vector3, godot_error};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, AgentSystem, MODE_CAR, TRANSIT_IN_BUILDING,
    age_group_can_work, transit_is_visible,
};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::surface::RoadSurfaceSystem;
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch,
};
use crate::simulation::terrain::{TerrainPatchSnapshot, TerrainSystem};
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{
    AuthoredLakeFill, AuthoredOpenWaterFill, AuthoredWaterBoundaryPoint,
};
use crate::simulation::zoning::{ZoneType, ZoningSystem};

fn access_phase_target(core: &SimCore, agent_idx: usize, egress: bool) -> Option<Vector3> {
    let building_id = if egress {
        core.agents.current_building[agent_idx]
    } else {
        core.agents.target_building[agent_idx]
    };
    let entrance = core.allocator.entrances.get(building_id)?;
    if egress {
        if core.agents.transit_mode[agent_idx] == MODE_CAR {
            let lane_id = core.agents.planned_attach_lane_id[agent_idx] as usize;
            let lane_d = core.agents.planned_attach_lane_d[agent_idx];
            let lane = core.transit_network.lane_system.lanes.get(lane_id)?;
            let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
            Some(Vector3::new(lane_pos.x, 0.0, lane_pos.y))
        } else {
            Some(Vector3::new(entrance.curb_pos.x, 0.0, entrance.curb_pos.y))
        }
    } else {
        Some(Vector3::new(entrance.door_pos.x, 0.0, entrance.door_pos.y))
    }
}

/// Currency cost per meter of new road laid, deducted from the city treasury at placement.
pub(crate) const ROAD_BUILD_COST_PER_METER: f64 = 100.0;
/// Currency upkeep per meter of road per day, settled from the city treasury each day.
pub(crate) const ROAD_UPKEEP_PER_METER_PER_DAY: f64 = 0.1;
/// Fine render step used for terrain patches whose topology is clipped by visible road surfaces.
pub(crate) const ROAD_LOCKED_TERRAIN_RENDER_STEP_M: f32 = 2.0;
/// Minimum terrain-CDT sample margin around road loops inside one render patch.
const TERRAIN_CDT_LOCAL_MIN_SAMPLE_MARGIN_M: f32 = 8.0;
/// Extra terrain-CDT sample margin expressed in road-locked render steps.
const TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_RENDER_STEPS: f32 = 4.0;
/// Extra terrain-CDT sample margin expressed in authored terrain cells.
const TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS: f32 = 2.0;
/// First continuous runtime water pass tick interval in simulated seconds.
const CONTINUOUS_WATER_TICK_DT: f32 = 0.2;
/// First continuous runtime water pass tick interval in real-time seconds.
const CONTINUOUS_WATER_TICK_INTERVAL_S: f64 = CONTINUOUS_WATER_TICK_DT as f64;

/// Returns the deterministic seam margin used by local terrain-CDT windows.
pub(crate) fn terrain_cdt_local_sample_margin_m(
    terrain: &TerrainSystem,
    render_step_m: f32,
) -> f32 {
    TERRAIN_CDT_LOCAL_MIN_SAMPLE_MARGIN_M
        .max(render_step_m.max(f32::EPSILON) * TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_RENDER_STEPS)
        .max(terrain.cell_size_m() * TERRAIN_CDT_LOCAL_SAMPLE_MARGIN_TERRAIN_CELLS)
}

/// City-level fiscal ledger, separate from household budgets and building budgets.
///
/// The balance may go negative: deficits are an explicit fiscal state rather than
/// a blocked operation. Future debt/credit systems may add consequences later.
pub struct CityTreasury {
    /// Current balance in currency units. May be negative.
    pub balance: f64,
    /// Running total of all infrastructure build costs since game start.
    pub lifetime_build_cost: f64,
    /// Running total of all collected tax revenue since game start.
    pub lifetime_tax_revenue: f64,
    /// Road upkeep deducted in the most recent daily settlement.
    pub last_daily_upkeep: f64,
    /// Income tax collected in the most recently finalized fiscal day.
    pub last_daily_income_tax: f64,
    /// Household VAT collected in the most recently finalized fiscal day.
    pub last_daily_household_vat: f64,
    /// Business purchase tax collected in the most recently finalized fiscal day.
    pub last_daily_business_purchase_tax: f64,
    /// Business profit tax collected in the most recently finalized fiscal day.
    pub last_daily_business_profit_tax: f64,
    /// Property tax collected in the most recently finalized fiscal day.
    pub last_daily_property_tax: f64,
    /// Income tax collected since the last daily fiscal finalization.
    pub pending_income_tax: f64,
    /// Household VAT collected since the last daily fiscal finalization.
    pub pending_household_vat: f64,
    /// Business purchase tax collected since the last daily fiscal finalization.
    pub pending_business_purchase_tax: f64,
    /// Business profit tax collected since the last daily fiscal finalization.
    pub pending_business_profit_tax: f64,
    /// Property tax collected since the last daily fiscal finalization.
    pub pending_property_tax: f64,
}

impl CityTreasury {
    /// Initialises the treasury with the given startup balance.
    pub(crate) fn new(startup_balance: f64) -> Self {
        Self {
            balance: startup_balance,
            lifetime_build_cost: 0.0,
            lifetime_tax_revenue: 0.0,
            last_daily_upkeep: 0.0,
            last_daily_income_tax: 0.0,
            last_daily_household_vat: 0.0,
            last_daily_business_purchase_tax: 0.0,
            last_daily_business_profit_tax: 0.0,
            last_daily_property_tax: 0.0,
            pending_income_tax: 0.0,
            pending_household_vat: 0.0,
            pending_business_purchase_tax: 0.0,
            pending_business_profit_tax: 0.0,
            pending_property_tax: 0.0,
        }
    }

    /// Deducts an infrastructure build cost from the treasury. Balance may go negative.
    pub(crate) fn deduct_build_cost(&mut self, amount: f64) {
        self.balance -= amount;
        self.lifetime_build_cost += amount;
    }

    /// Settles one day's infrastructure upkeep cost. Balance may go negative.
    pub(crate) fn settle_daily_upkeep(&mut self, amount: f64) {
        self.balance -= amount;
        self.last_daily_upkeep = amount;
    }

    /// Records wage income tax withheld from household income.
    pub(crate) fn collect_income_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Income);
    }

    /// Records VAT collected from household shopping purchases.
    pub(crate) fn collect_household_vat(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::HouseholdVat);
    }

    /// Records tax collected from business input purchases.
    pub(crate) fn collect_business_purchase_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessPurchase);
    }

    /// Records tax collected from positive daily business profit.
    pub(crate) fn collect_business_profit_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessProfit);
    }

    /// Records one-time property tax from new private construction.
    pub(crate) fn collect_property_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Property);
    }

    /// Rolls the current pending fiscal window into daily reporting buckets.
    pub(crate) fn finalize_daily_tax_window(&mut self) {
        self.last_daily_income_tax = self.pending_income_tax;
        self.last_daily_household_vat = self.pending_household_vat;
        self.last_daily_business_purchase_tax = self.pending_business_purchase_tax;
        self.last_daily_business_profit_tax = self.pending_business_profit_tax;
        self.last_daily_property_tax = self.pending_property_tax;
        self.pending_income_tax = 0.0;
        self.pending_household_vat = 0.0;
        self.pending_business_purchase_tax = 0.0;
        self.pending_business_profit_tax = 0.0;
        self.pending_property_tax = 0.0;
    }

    fn record_tax(&mut self, amount: f64, bucket: TaxBucket) {
        if amount <= 0.0 {
            return;
        }
        self.balance += amount;
        self.lifetime_tax_revenue += amount;
        match bucket {
            TaxBucket::Income => self.pending_income_tax += amount,
            TaxBucket::HouseholdVat => self.pending_household_vat += amount,
            TaxBucket::BusinessPurchase => self.pending_business_purchase_tax += amount,
            TaxBucket::BusinessProfit => self.pending_business_profit_tax += amount,
            TaxBucket::Property => self.pending_property_tax += amount,
        }
    }
}

#[derive(Clone, Copy)]
enum TaxBucket {
    Income,
    HouseholdVat,
    BusinessPurchase,
    BusinessProfit,
    Property,
}

/// Validation state for one transient world-editor lake-fill preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLakeFillPreviewStatus {
    /// The preview covers a closed basin and can be committed.
    Ready,
    /// The chosen surface is at or below the seed terrain height.
    SurfaceBelowSeedTerrain,
    /// The chosen surface spills out of the basin and reaches the world edge.
    EscapesWorldEdge,
    /// The chosen open-water surface does not connect to the world edge.
    DoesNotReachWorldEdge,
}

/// Preview feature kind for world-editor surface fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldWaterFillKind {
    /// Closed inland basin fill.
    Lake,
    /// Edge-connected open-water fill.
    OpenWater,
}

/// Debug summary for one authored water fill that contributes to a render patch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AuthoredWaterPatchFillDebug {
    /// Authored fill kind that produced the patch contribution.
    pub(crate) kind: WorldWaterFillKind,
    /// Committed fill index in its authored list, or `-1` for a transient preview.
    pub(crate) fill_index: i32,
    /// Whether this contribution came from the active transient preview.
    pub(crate) preview: bool,
    /// Snapped seed X coordinate in world metres.
    pub(crate) world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub(crate) world_z: f32,
    /// Authored flat water surface elevation in metres.
    pub(crate) surface_elevation_m: f32,
    /// Number of cells in the complete fill body.
    pub(crate) filled_cells: usize,
    /// Whether the complete fill body touches the world edge.
    pub(crate) touches_world_edge: bool,
    /// Number of non-zero water samples contributed inside the requested patch.
    pub(crate) patch_nonzero_samples: usize,
    /// Maximum contributed water depth inside the requested patch.
    pub(crate) patch_max_depth_m: f32,
    /// Sum of contributed water depths inside the requested patch.
    pub(crate) patch_sum_depth_m: f32,
}

/// Transient lake-fill preview state owned by the world editor runtime.
///
/// This state is never serialized into `WorldDefinition`. It exists only so the
/// editor can show live water feedback while the author adjusts the target
/// surface elevation before confirming the lake fill.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLakeFillPreview {
    /// Preview feature kind.
    pub kind: WorldWaterFillKind,
    /// Snapped seed X coordinate in world metres.
    pub seed_world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub seed_world_z: f32,
    /// Seed terrain height in rendered world metres.
    pub seed_height_m: f32,
    /// Preview surface elevation in rendered world metres.
    pub surface_elevation_m: f32,
    /// Preview validation outcome.
    pub status: WorldLakeFillPreviewStatus,
    /// Number of filled terrain cells in the preview flood.
    pub filled_cells: usize,
}

impl WorldLakeFillPreview {
    /// Returns `true` when the preview is valid and may be committed.
    pub(crate) fn is_valid(self) -> bool {
        self.status == WorldLakeFillPreviewStatus::Ready
    }
}

/// Full water runtime snapshot for undo history.
pub(crate) struct WaterRuntimeSnapshot {
    /// Flat authored or loaded baseline water depth above terrain.
    pub baseline_depth: Vec<f32>,
    /// Transient dynamic water depth above the support surface.
    pub dynamic_depth: Vec<f32>,
    /// Dynamic water velocity magnitude.
    pub velocity: Vec<f32>,
    /// Dynamic directional flux values.
    pub flux: Vec<[f32; 4]>,
    /// Dynamic water boundary points.
    pub sources: Vec<(usize, usize, f32)>,
}

/// A snapshot of simulation state for undo history.
pub(crate) struct SimulationSnapshot {
    /// Terrain heightmap data.
    pub(crate) terrain: Option<Vec<f32>>,
    /// Water runtime state.
    pub(crate) water: Option<WaterRuntimeSnapshot>,
    /// Road network graph state.
    pub(crate) trans_graph: Option<crate::simulation::network::graph::RegionGraph>,
    /// Zoning system state.
    pub(crate) zoning: Option<ZoningSystem>,
}

/// Cache key for one production refined terrain patch mesh.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RefinedTerrainPatchCacheKey {
    /// Terrain render-patch X index.
    pub(crate) patch_x: usize,
    /// Terrain render-patch Z index.
    pub(crate) patch_z: usize,
    /// Refined render step quantized to millimetres.
    pub(crate) render_step_mm: u32,
}

/// Complete input needed to build a refined road-clipped terrain patch off the Godot frame.
pub(crate) struct RefinedTerrainPatchBuildInput {
    /// Cache key for the produced patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Local CDT windows assembled from source terrain samples and road footprint loops.
    pub(crate) windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
}

/// Cache key for one local CDT window inside a refined render patch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RefinedTerrainCdtWindowKey {
    /// Window minimum X in quantized millimetres.
    pub(crate) min_x_mm: i64,
    /// Window minimum Z in quantized millimetres.
    pub(crate) min_z_mm: i64,
    /// Window maximum X in quantized millimetres.
    pub(crate) max_x_mm: i64,
    /// Window maximum Z in quantized millimetres.
    pub(crate) max_z_mm: i64,
    /// Stable fingerprint of road loops and terrain samples in this window.
    pub(crate) fingerprint: u64,
}

/// Build input for one local CDT window inside a refined render patch.
pub(crate) struct RefinedTerrainCdtWindowBuildInput {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// CDT input for this local window.
    pub(crate) cdt_input: TerrainCdtInput,
    /// Previous compiled window when the fingerprint did not change.
    pub(crate) previous: Option<CachedRefinedTerrainCdtWindow>,
}

/// Cached local CDT window built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainCdtWindow {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT window used inside the base render patch.
    pub(crate) cdt_patch: TerrainCdtPatch,
    /// CDT result for this window.
    pub(crate) mesh_result: Result<TerrainCdtMesh, TerrainCdtError>,
    /// Time spent in CDT construction for this window.
    pub(crate) cdt_ms: f64,
    /// True when this window was reused from the previous patch cache.
    pub(crate) reused: bool,
}

/// Cached production refined terrain patch built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainPatch {
    /// Cache key for this patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT windows composed into this render patch.
    pub(crate) windows: Vec<CachedRefinedTerrainCdtWindow>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
    /// Time spent in CDT construction for this patch's rebuilt windows.
    pub(crate) cdt_ms: f64,
    /// Number of windows reused from the previous cache entry.
    pub(crate) reused_windows: usize,
}

/// All simulation state — owned exclusively by the background sim thread when running.
///
/// The main thread accesses this via `Arc<Mutex<SimCore>>`. The lock is held for at
/// most one tick duration (~7 ms at 100 k agents) per mutation.
pub struct SimCore {
    /// Simulation clock and day counter.
    pub time: TimeSystem,
    /// Terrain heightmap.
    pub heightmap: TerrainSystem,
    /// Shallow-water simulation.
    pub watermap: WaterSystem,
    /// Road topology graph.
    pub region_graph: crate::simulation::network::graph::RegionGraph,
    /// Lane system, CCH pathfinder, and road mutation helpers.
    pub transit_network: TransitNetwork,
    /// Road-aligned parcel zoning.
    pub zoning: ZoningSystem,
    /// Pollution diffusion grid.
    pub pollution: PollutionSystem,
    /// Traffic noise grid.
    pub noise: NoiseSystem,
    /// Composite desirability grid.
    pub desirability: DesirabilitySystem,
    /// Global R/C/I demand counters.
    pub demand: DemandSystem,
    /// Building placement and vacancy index.
    pub allocator: BuildingAllocator,
    /// Agent FSM in Structure-of-Arrays layout.
    pub agents: AgentSystem,
    /// Explicit household runtime records and first-pass daily economy logic.
    pub households: HouseholdSystem,
    /// Active building-level freight reservations and delayed deliveries.
    pub logistics: ShipmentSystem,
    /// World configuration (extent, chunk metadata, cell sizes).
    pub config: WorldConfig,
    /// City-level fiscal ledger tracking infrastructure build cost and daily upkeep.
    pub treasury: CityTreasury,
    /// Runtime-only economy debug counter reset after each daily diagnostic line.
    pub(crate) debug_household_admissions_since_daily: u32,
    /// Undo history stack — kept in SimCore so all mutations are co-located.
    pub(crate) undo_stack: VecDeque<SimulationSnapshot>,
    /// Authored-world inflow / outflow points when editing or playing from a `WorldDefinition`.
    pub(crate) world_water_boundary_points: Vec<AuthoredWaterBoundaryPoint>,
    /// Authored-world lake fill records when editing or playing from a `WorldDefinition`.
    pub(crate) world_lake_fills: Vec<AuthoredLakeFill>,
    /// Authored-world edge-connected open-water fills when editing or playing from a `WorldDefinition`.
    pub(crate) world_open_water_fills: Vec<AuthoredOpenWaterFill>,
    /// Transient world-editor lake-fill preview. Never saved into `WorldDefinition`.
    pub(crate) world_lake_fill_preview: Option<WorldLakeFillPreview>,
    /// Cached authored-water fill debug summaries keyed by water render patch.
    pub(crate) authored_water_patch_fill_debug_cache:
        HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
    /// True while the world editor is accumulating one terrain brush stroke.
    pub(crate) terrain_stroke_active: bool,
    /// True once the active terrain brush stroke has applied at least one terrain mutation.
    pub(crate) terrain_stroke_has_changes: bool,
    /// Allows continuous water to advance in real time while the operational clock is paused.
    pub(crate) water_runtime_realtime_when_paused: bool,
    /// Set by terrain mutations; cleared by the Godot render layer.
    pub terrain_dirty: bool,
    /// Set by water mutations; cleared by the Godot render layer.
    pub water_dirty: bool,
    /// Set by any network mutation (road, rail); cleared by `clear_network_dirty()` after
    /// `NetworkRenderer` finishes rebuilding the visual mesh. Stays `true` until GDScript
    /// explicitly clears it — same pattern as `terrain_dirty` and `water_dirty`.
    pub network_dirty: bool,
    /// True when running in benchmark mode (skips undo stack on road placement).
    pub benchmark_mode: bool,
    /// Duration of the last daily economy tick in milliseconds.
    pub last_tick_duration: f64,
    /// Duration of the last agent movement tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Per-phase timing breakdown from the last road placement, for profiling.
    pub last_road_timing: String,
    /// Edge ids touched by the most recent committed network edit and queued for one focused
    /// road-surface debug dump after the next terrain/mesh rebuild.
    pub(crate) last_surface_debug_edges: Vec<usize>,
    /// Production refined terrain patches precomputed by the sim thread for Godot upload.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, CachedRefinedTerrainPatch>,
    /// Sorted terrain render patches that must use road-locked refined terrain meshes.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
    /// Latest full road mesh generated by the sim thread after a network edit.
    pub(crate) cached_road_mesh_data: Option<NetworkMeshData>,
    /// World-space AABB for frustum culling: (x_min, x_max, z_min, z_max).
    /// Agents outside this rect are excluded from `RenderSnapshot` transforms.
    /// Updated each frame via `SimCommand::SetCameraAabb`. Defaults to "show all".
    pub camera_aabb: (f32, f32, f32, f32),
}

#[derive(Clone, Copy, Debug, Default)]
struct DailyCityFlowDiagnostics {
    active_households: u32,
    housed_households: u32,
    unhoused_households: u32,
    zero_budget_households: u32,
    stock_empty_households: u32,
    stock_low_households: u32,
    total_household_slots: u32,
    vacant_household_slots: u32,
    resident_agents: u32,
    child_agents: u32,
    adult_agents: u32,
    elder_agents: u32,
    pending_household_carriers: u32,
    employed_agents: u32,
    unemployed_agents: u32,
    commercial_job_capacity: u32,
    commercial_filled_jobs: u32,
    industrial_job_capacity: u32,
    industrial_filled_jobs: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewSnapshot {
    pub(crate) request_id: u64,
    pub(crate) prepared_points: Vec<godot::prelude::Vector3>,
    pub(crate) surface_vertices: Vec<godot::prelude::Vector3>,
    pub(crate) is_valid: bool,
}

#[derive(Clone)]
pub(crate) struct RoadPreviewWorkerContext {
    terrain: TerrainSystem,
    region_graph: RegionGraph,
    road_surface: RoadSurfaceSystem,
    surface_chunk_span_m: f32,
}

impl RoadPreviewWorkerContext {
    pub(crate) fn from_core(core: &SimCore) -> Self {
        Self {
            terrain: core.heightmap.clone(),
            region_graph: core.region_graph.clone(),
            road_surface: core.transit_network.road_surface.clone(),
            surface_chunk_span_m: core.transit_network.road_surface.chunk_span_m(),
        }
    }
}

pub(crate) struct RoadPreviewRequest {
    pub(crate) request_id: u64,
    pub(crate) points: Vec<godot::prelude::Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
}

#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    pub(crate) terrain: TerrainSystem,
    pub(crate) region_graph: RegionGraph,
    pub(crate) road_surface: RoadSurfaceSystem,
    pub(crate) ghost_snap_index: RoadGhostSnapIndex,
}

impl RoadToolQuerySnapshot {
    pub(crate) fn from_core(core: &SimCore) -> Self {
        Self {
            terrain: core.heightmap.clone(),
            region_graph: core.region_graph.clone(),
            road_surface: core.transit_network.road_surface.clone(),
            ghost_snap_index: RoadGhostSnapIndex::from_graph(&core.region_graph),
        }
    }
}

/// Pre-computed rendering data written by the sim thread and read by the render thread.
///
/// Contains only pure Rust types so the struct is `Send + Sync` without unsafe.
/// The Godot main thread converts these `Vec<f32>` buffers to `PackedFloat32Array`
/// when the `#[func]` render getters are called.
pub struct RenderSnapshot {
    /// Per `pedestrian_type` → flat 12-float `Transform3D` buffer.
    pub pedestrian_transforms: HashMap<u8, Vec<f32>>,
    /// Per `(vehicle_type * 10 + color_variant)` → flat 12-float `Transform3D` buffer.
    pub car_transforms: HashMap<u8, Vec<f32>>,
    /// Per car transform bucket → render IDs matching `car_transforms` instance order.
    pub car_render_ids: HashMap<u8, Vec<i64>>,
    /// Mirrors `SimCore::terrain_dirty` at snapshot time.
    pub terrain_dirty: bool,
    /// Mirrors `SimCore::water_dirty` at snapshot time.
    pub water_dirty: bool,
    /// Mirrors `SimCore::network_dirty` at snapshot time; cleared the same frame.
    pub network_dirty: bool,
    /// Current simulation day.
    pub current_day: u32,
    /// Current minute since operational midnight.
    pub current_minute_of_day: u16,
    /// Duration of the last daily tick in milliseconds.
    pub last_tick_ms: f64,
    /// Duration of the last agent tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Number of CCH pathfinding calls since the last daily tick reset.
    pub pathfind_count: u32,
    /// Total number of live agents.
    pub agent_count: i32,
    /// Current city treasury balance in currency units.
    pub treasury_balance: f64,
    /// Heightmap width in cells (for CSV logging on the main thread).
    pub heightmap_width: usize,
    /// Heightmap height in cells (for CSV logging on the main thread).
    pub heightmap_height: usize,
    /// Terrain world extent in metres, cached so Godot tools do not lock `SimCore` per frame.
    pub terrain_world_size: godot::prelude::Vector2,
    /// World-space positions of all canonical (non-virtual) network nodes.
    /// Pre-computed here so `get_network_nodes()` reads the snapshot (RwLock)
    /// instead of locking SimCore — avoids main-thread stalls during road placement.
    pub node_positions: Vec<godot::prelude::Vector3>,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            pedestrian_transforms: HashMap::new(),
            car_transforms: HashMap::new(),
            car_render_ids: HashMap::new(),
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            current_day: 1,
            current_minute_of_day: 0,
            last_tick_ms: 0.0,
            last_agent_tick_us: 0,
            pathfind_count: 0,
            agent_count: 0,
            treasury_balance: 0.0,
            heightmap_width: 0,
            terrain_world_size: godot::prelude::Vector2::ZERO,
            node_positions: Vec::new(),
            heightmap_height: 0,
        }
    }
}

/// Commands sent from the Godot main thread to the sim background thread.
pub enum SimCommand {
    /// Update the simulation speed multiplier.
    SetSpeed(f32),
    /// Update the camera world-space AABB used for agent frustum culling.
    /// Values: (x_min, x_max, z_min, z_max) in world units, padded by ~200 m.
    SetCameraAabb(f32, f32, f32, f32),
    /// Place a new road segment.  Executed in the sim thread so the main thread
    /// never blocks on the expensive lane-rebuild and zoning-obstruction passes.
    AddRoad {
        /// World-space polyline points.
        points: Vec<godot::prelude::Vector3>,
        /// Forward lane count.
        fwd_lanes: i32,
        /// Backward lane count.
        bkw_lanes: i32,
    },
    /// Ask the background thread to exit cleanly.
    Quit,
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
            let context = context.read().unwrap();
            compile_road_preview_from_context(&context, request)
        };
        let prepared_count = preview.prepared_points.len();
        let surface_vertex_count = preview.surface_vertices.len();
        let is_valid = preview.is_valid;
        *result.write().unwrap() = Some(preview);
        if road_debug {
            debug_log!(
                "road",
                "preview_surface_worker points={} prepared_points={} surface_vertices={} valid={} total_ms={:.3}",
                point_count,
                prepared_count,
                surface_vertex_count,
                is_valid,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }
}

pub(crate) fn compile_road_preview_from_context(
    context: &RoadPreviewWorkerContext,
    request: RoadPreviewRequest,
) -> RoadPreviewSnapshot {
    let preview_surface = RoadSurfaceSystem::new(context.surface_chunk_span_m);
    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &request.points,
        request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        &context.terrain,
        &context.region_graph,
        &context.road_surface,
    );

    RoadPreviewSnapshot {
        request_id: request.request_id,
        prepared_points: preview.prepared_points,
        surface_vertices: preview.surface_vertices,
        is_valid: preview.is_valid,
    }
}

impl SimCore {
    fn tick_continuous_water_runtime_internal(&mut self, dt: f32) {
        if !self.watermap.has_sources() {
            return;
        }
        let terrain_world = self.heightmap.clone_source_dense_world_heights();
        self.watermap.tick(&terrain_world, dt);
        self.water_dirty = true;
    }

    fn print_sim_console_summary(&self, day_index: u32, minute_of_day: u16) {
        let mut at_home = 0usize;
        let mut at_work = 0usize;
        let mut shopping = 0usize;
        let mut travelling = 0usize;
        let mut other = 0usize;

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != TRANSIT_IN_BUILDING {
                travelling += 1;
                continue;
            }

            match self.agents.activity[i] {
                0 => at_home += 1,
                1 => at_work += 1,
                2 => shopping += 1,
                _ => other += 1,
            }
        }

        let household_count = self
            .households
            .households
            .iter()
            .filter(|household| household.member_count > 0)
            .count();
        let hours = minute_of_day / 60;
        let minutes = minute_of_day % 60;

        println!(
            "[SIM_DEBUG] Day {} {:02}:{:02} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} buildings={} households={} agents={} states=(home={}, work={}, shopping={}, travelling={}, other={}) actions=spawn({}/{}/{}) upgrade({}/{}/{}) downgrade({}/{}/{}) despawn({}/{}/{})",
            day_index,
            hours,
            minutes,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.allocator.buildings.len(),
            household_count,
            self.agents.len(),
            at_home,
            at_work,
            shopping,
            travelling,
            other,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    fn daily_city_flow_diagnostics(&self) -> DailyCityFlowDiagnostics {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        let mut diagnostics = DailyCityFlowDiagnostics::default();
        let catalog = load_runtime_economy_catalog().ok();

        for (building_idx, building) in self.allocator.buildings.iter().enumerate() {
            if matches!(building.zone_type, ZoneType::Residential) {
                let household_capacity = self.allocator.household_capacity(building_idx);
                diagnostics.total_household_slots = diagnostics
                    .total_household_slots
                    .saturating_add(household_capacity);
                diagnostics.vacant_household_slots =
                    diagnostics.vacant_household_slots.saturating_add(
                        household_capacity
                            .saturating_sub(building.occupancy.min(household_capacity)),
                    );
            }

            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(building_idx, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(building_idx));
            match building.zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_job_capacity = diagnostics
                        .commercial_job_capacity
                        .saturating_add(worker_capacity);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_job_capacity = diagnostics
                        .industrial_job_capacity
                        .saturating_add(worker_capacity);
                }
                _ => {}
            }
        }

        for household in &self.households.households {
            if household.member_count == 0 {
                continue;
            }
            diagnostics.active_households = diagnostics.active_households.saturating_add(1);
            let live_home = self
                .allocator
                .buildings
                .get(household.home_building_id)
                .is_some_and(|building| {
                    !building.broken
                        && !building.economy_broken
                        && !building.is_deserted
                        && building.is_operational()
                });
            if live_home {
                diagnostics.housed_households = diagnostics.housed_households.saturating_add(1);
            } else {
                diagnostics.unhoused_households = diagnostics.unhoused_households.saturating_add(1);
            }
            if household.budget <= f32::EPSILON {
                diagnostics.zero_budget_households =
                    diagnostics.zero_budget_households.saturating_add(1);
            }
            if household.stock_days <= f32::EPSILON {
                diagnostics.stock_empty_households =
                    diagnostics.stock_empty_households.saturating_add(1);
            }
            if household.stock_days <= 1.0 {
                diagnostics.stock_low_households =
                    diagnostics.stock_low_households.saturating_add(1);
            }
        }

        for agent_idx in 0..self.agents.len() {
            if self.agents.pending_household_size[agent_idx] > 0 {
                diagnostics.pending_household_carriers =
                    diagnostics.pending_household_carriers.saturating_add(1);
                continue;
            }
            let household_id = self.agents.household_id[agent_idx];
            if household_id == usize::MAX || household_id >= self.households.households.len() {
                continue;
            }
            diagnostics.resident_agents = diagnostics.resident_agents.saturating_add(1);
            match self.agents.age_group[agent_idx] {
                AGE_CHILD => {
                    diagnostics.child_agents = diagnostics.child_agents.saturating_add(1);
                }
                AGE_ADULT => {
                    diagnostics.adult_agents = diagnostics.adult_agents.saturating_add(1);
                }
                AGE_ELDER => {
                    diagnostics.elder_agents = diagnostics.elder_agents.saturating_add(1);
                }
                _ => {}
            }

            if !age_group_can_work(self.agents.age_group[agent_idx]) {
                continue;
            }

            let work_building = self.agents.work_building[agent_idx];
            if work_building >= self.allocator.buildings.len() {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }
            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(work_building, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(work_building));
            if worker_capacity == 0 {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }

            diagnostics.employed_agents = diagnostics.employed_agents.saturating_add(1);
            match self.allocator.buildings[work_building].zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_filled_jobs =
                        diagnostics.commercial_filled_jobs.saturating_add(1);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_filled_jobs =
                        diagnostics.industrial_filled_jobs.saturating_add(1);
                }
                _ => {}
            }
        }

        diagnostics
    }

    fn log_daily_city_flow_diagnostics(&self, day_index: u32, removed_households: u32) {
        if !crate::debug::category_enabled("economy") {
            return;
        }

        let diagnostics = self.daily_city_flow_diagnostics();
        let total_job_capacity = diagnostics
            .commercial_job_capacity
            .saturating_add(diagnostics.industrial_job_capacity);
        let filled_jobs = diagnostics
            .commercial_filled_jobs
            .saturating_add(diagnostics.industrial_filled_jobs);
        let open_jobs = total_job_capacity.saturating_sub(filled_jobs);
        let commercial_open_jobs = diagnostics
            .commercial_job_capacity
            .saturating_sub(diagnostics.commercial_filled_jobs);
        let industrial_open_jobs = diagnostics
            .industrial_job_capacity
            .saturating_sub(diagnostics.industrial_filled_jobs);
        let occupied_household_slots = diagnostics
            .total_household_slots
            .saturating_sub(diagnostics.vacant_household_slots);
        let net_households =
            self.debug_household_admissions_since_daily as i32 - removed_households as i32;

        debug_log!(
            "economy",
            "city flow diagnostics: day={} net_households={:+} admitted_since_daily={} \
             removed_today={} households={} housed={} unhoused={} zero_budget={} \
             stock_empty={} stock_low={} resident_agents={} pending_carriers={} \
             children={} adults={} elders={} employed={} unemployed={} jobs={}/{} open_jobs={} \
             commercial_jobs={}/{} commercial_open={} industrial_jobs={}/{} industrial_open={} \
             homes={}/{} vacant_homes={} treasury={:.0} taxes=(income={:.1} household_vat={:.1} \
             business_purchase={:.1} business_profit={:.1} property={:.1} lifetime={:.1})",
            day_index,
            net_households,
            self.debug_household_admissions_since_daily,
            removed_households,
            diagnostics.active_households,
            diagnostics.housed_households,
            diagnostics.unhoused_households,
            diagnostics.zero_budget_households,
            diagnostics.stock_empty_households,
            diagnostics.stock_low_households,
            diagnostics.resident_agents,
            diagnostics.pending_household_carriers,
            diagnostics.child_agents,
            diagnostics.adult_agents,
            diagnostics.elder_agents,
            diagnostics.employed_agents,
            diagnostics.unemployed_agents,
            filled_jobs,
            total_job_capacity,
            open_jobs,
            diagnostics.commercial_filled_jobs,
            diagnostics.commercial_job_capacity,
            commercial_open_jobs,
            diagnostics.industrial_filled_jobs,
            diagnostics.industrial_job_capacity,
            industrial_open_jobs,
            occupied_household_slots,
            diagnostics.total_household_slots,
            diagnostics.vacant_household_slots,
            self.treasury.balance,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
        );
    }

    fn print_daily_building_economy(&mut self, day_index: u32) {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        if !crate::debug::category_enabled("economy") {
            self.households.reset_daily_ledgers();
            return;
        }
        let Ok(catalog) = load_runtime_economy_catalog() else {
            self.households.reset_daily_ledgers();
            return;
        };

        for (idx, b) in self.allocator.buildings.iter().enumerate() {
            if b.zone_type == ZoneType::Residential {
                continue;
            }
            let zone_tag = match b.zone_type {
                ZoneType::Residential => "RES",
                ZoneType::Commercial => "COM",
                ZoneType::Industrial => "IND",
                _ => "OTHER",
            };
            let worker_cap = self
                .allocator
                .worker_capacity_with_catalog(idx, catalog.as_ref());
            let _resident_cap = self.allocator.household_capacity(idx);
            let profile_id = catalog
                .profile_by_runtime_id(b.economy_profile_runtime_id)
                .map(|p| p.id.as_str())
                .unwrap_or("none");

            // Build inventory snapshot string for all non-zero resources.
            let mut inv_parts = Vec::new();
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount <= 0.0 {
                    continue;
                }
                let rid = (slot + 1) as u16;
                let name = catalog.resource_id_for_runtime_id(rid).unwrap_or("?");
                // capacity from output port if available
                let cap =
                    if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                        p.outputs
                            .iter()
                            .find(|o| o.resource_runtime_id == rid)
                            .map(|o| p.output_buffer_capacity_units_for(o))
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                if cap > 0.0 {
                    inv_parts.push(format!("{}={:.1}/{:.1}", name, amount, cap));
                } else {
                    inv_parts.push(format!("{}={:.1}", name, amount));
                }
            }
            let inv_str = if inv_parts.is_empty() {
                "none".to_owned()
            } else {
                inv_parts.join(" ")
            };

            // Daily I/O from profile (per-day throughput at full capacity).
            let mut io_parts = Vec::new();
            if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                for port in &p.inputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("-{:.1}{}/day", port.units_per_day, name));
                }
                for port in &p.outputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("+{:.1}{}/day", port.units_per_day, name));
                }
            }
            let io_str = if io_parts.is_empty() {
                "none".to_owned()
            } else {
                io_parts.join(" ")
            };

            println!(
                "[ECON] Day {:>4} idx={} {} asset={} profile={} workers={}/{} budget={:.1} revenue={:.1} distress={} broken={} io=[{}] inventory=[{}]",
                day_index,
                idx,
                zone_tag,
                b.asset_id,
                profile_id,
                b.worker_count,
                worker_cap,
                b.operating_budget,
                b.revenue,
                if b.budget_distress { "Y" } else { "N" },
                if b.broken || b.economy_broken {
                    "Y"
                } else {
                    "N"
                },
                io_str,
                inv_str,
            );
        }

        let mut households_at_budget_floor = 0u32;
        let mut households_below_1d_stock = 0u32;
        let mut households_below_2d_stock = 0u32;
        let mut households_below_3d_stock = 0u32;
        let mut total_wages_paid = 0.0f32;
        let mut total_household_shopping_spend = 0.0f32;
        let mut total_benefits_paid = 0.0f32;
        let mut total_utility_stock_cost = 0.0f32;

        for (idx, h) in self.households.households.iter().enumerate() {
            if h.member_count == 0 {
                continue;
            }
            let ledger = self
                .households
                .daily_ledgers()
                .get(idx)
                .copied()
                .unwrap_or_default();
            if h.budget <= f32::EPSILON {
                households_at_budget_floor += 1;
            }
            if h.stock_days < 1.0 {
                households_below_1d_stock += 1;
            }
            if h.stock_days < 2.0 {
                households_below_2d_stock += 1;
            }
            if h.stock_days < 3.0 {
                households_below_3d_stock += 1;
            }
            total_wages_paid += ledger.wage_income;
            total_household_shopping_spend += ledger.shopping_spend;
            total_benefits_paid += ledger.unemployment_benefit_income;
            total_utility_stock_cost += ledger.utility_stock_consumption_cost;
            let home_asset = self
                .allocator
                .buildings
                .get(h.home_building_id)
                .map(|b| b.asset_id.as_str())
                .unwrap_or("none");

            let state_str = match h.replenishment_state {
                0 => "STABLE",
                1 => "NEEDS",
                2 => "WAITING_SHOPPER",
                3 => "SHOPPING_TO_STORE",
                4 => "SHOPPING_RETURNING",
                5 => "FULFILLED",
                6 => "COOLDOWN",
                7 => "FAILED_TERMINAL",
                _ => "UNKNOWN",
            };

            let ub_str = if h.unemployment_days_elapsed > 0 {
                format!(" ub={}d", h.unemployment_days_elapsed)
            } else {
                String::new()
            };
            println!(
                "[ECON] Day {:>4} HH:{:<2} home_idx={:<2} asset={} residents={} children={} adults={} elders={} budget={:<5.1} stock={:<4.2}days state={}{} ledger=(before={:.1} wage={:.1} benefit={:.1} shopping={:.1} utility_stock={:.1} after={:.1} unemployed_adults={} shopper_trips={}/{})",
                day_index,
                idx,
                h.home_building_id,
                home_asset,
                h.member_count,
                h.child_count,
                h.adult_count,
                h.elder_count,
                h.budget,
                h.stock_days,
                state_str,
                ub_str,
                ledger.budget_before,
                ledger.wage_income,
                ledger.unemployment_benefit_income,
                ledger.shopping_spend,
                ledger.utility_stock_consumption_cost,
                ledger.budget_after,
                ledger.unemployed_adults,
                ledger.shopper_trips_completed,
                ledger.shopper_trips_failed,
            );
        }
        println!(
            "[ECON] Day {:>4} household ledger summary: budget_floor={} stock_below_1d={} stock_below_2d={} stock_below_3d={} wages_paid={:.1} shopping_spend={:.1} benefits_paid={:.1} utility_stock_cost={:.1}",
            day_index,
            households_at_budget_floor,
            households_below_1d_stock,
            households_below_2d_stock,
            households_below_3d_stock,
            total_wages_paid,
            total_household_shopping_spend,
            total_benefits_paid,
            total_utility_stock_cost,
        );
        println!(
            "[ECON] Day {:>4} fiscal summary: income_tax={:.1} household_vat={:.1} business_purchase_tax={:.1} business_profit_tax={:.1} property_tax={:.1} tax_total={:.1} lifetime_tax={:.1} road_upkeep={:.1} treasury={:.1}",
            day_index,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.last_daily_income_tax
                + self.treasury.last_daily_household_vat
                + self.treasury.last_daily_business_purchase_tax
                + self.treasury.last_daily_business_profit_tax
                + self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
            self.treasury.last_daily_upkeep,
            self.treasury.balance,
        );
        self.households.reset_daily_ledgers();
    }

    /// Executes one coarse operational-hour economy step before the daily settlement boundary.
    pub fn simulate_operational_hour_internal(&mut self, day_index: u32, minute_of_day: u16) {
        let absolute_hour = day_index
            .saturating_sub(1)
            .saturating_mul(24)
            .saturating_add(u32::from(minute_of_day / 60));
        self.allocator.advance_construction_hour();
        let fiscal_revenue = self.households.operational_hour_tick(
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
            &self.transit_network,
            &self.region_graph,
            absolute_hour,
            minute_of_day,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        if minute_of_day != 0 {
            self.execute_hourly_demand_pass(day_index, minute_of_day);
        }
    }

    /// Executes one full economy / daily tick (called once per in-game day).
    pub fn simulate_tick_internal(&mut self, day_index: u32) {
        let tick_start = Instant::now();

        debug_log!(
            "economy",
            "daily tick start: buildings={} households={} agents={}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
        );
        self.allocator.tick(
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        // Drain building dirty-zone flags → mark matching flow fields for rebuild.
        {
            use crate::simulation::buildings::allocator::BASELINE_PRIVATE_ZONES;
            for (zone_idx, zone) in BASELINE_PRIVATE_ZONES.iter().enumerate() {
                if self.allocator.dirty_zones[zone_idx] {
                    self.allocator.dirty_zones[zone_idx] = false;
                    self.transit_network.flow_fields.mark_zone_dirty(*zone);
                }
            }
        }

        self.pollution.tick(&self.allocator, &self.config);
        self.noise
            .tick(&self.allocator, &self.region_graph, &self.config);
        self.desirability
            .tick(&self.zoning, &self.pollution, &self.noise);
        let fiscal_revenue = self.households.daily_settlement_tick(
            &mut self.agents,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            &mut self.treasury.balance,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        // City treasury: settle daily road upkeep on the fiscal cadence.
        let road_length_m: f64 = self
            .region_graph
            .edges()
            .iter()
            .filter(|e| !e.deleted)
            .map(|e| e.physical_length as f64)
            .sum();
        self.treasury
            .settle_daily_upkeep(road_length_m * ROAD_UPKEEP_PER_METER_PER_DAY);
        self.demand.run_daily_pass(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
        );
        let removed_households = self.households.execute_demand_household_removal(
            self.demand.households_to_remove_today,
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
        );
        self.demand
            .record_household_removal_execution(removed_households);
        self.demand
            .log_daily_household_action_diagnostics(day_index);
        self.execute_hourly_demand_pass(day_index, 0);
        // Minute 0 is the deterministic closing boundary: operational-hour work,
        // daily settlement, and midnight demand all post before the daily tax
        // buckets roll into the report.
        self.treasury.finalize_daily_tax_window();
        self.log_daily_city_flow_diagnostics(day_index, removed_households);
        self.debug_household_admissions_since_daily = 0;
        // Reset OWA/local input accumulators after the daily and midnight demand snapshots have
        // been taken.
        self.allocator.reset_daily_input_accumulators();
        debug_log!(
            "economy",
            "daily tick end: buildings={} households={} agents={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{}) treasury={:.0}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
            self.treasury.balance,
        );
        self.agents.daily_update(&self.pollution, &self.config);
        self.agents
            .pathfind_count
            .store(0, std::sync::atomic::Ordering::Relaxed);

        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
    }

    fn execute_hourly_demand_pass(&mut self, day_index: u32, minute_of_day: u16) {
        self.demand.run_hourly_pass(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
        );
        let launched_households = self.allocator.execute_demand_household_admission(
            self.demand.households_to_admit_today,
            &mut self.agents,
            &self.transit_network,
            &self.region_graph,
        );
        self.debug_household_admissions_since_daily = self
            .debug_household_admissions_since_daily
            .saturating_add(launched_households);
        self.demand
            .record_household_admission_execution(launched_households);
        let building_action_execution = self.allocator.execute_demand_building_actions(
            &self.demand.building_actions,
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &self.region_graph,
            &self.transit_network.lane_system,
            &self.heightmap,
            self.demand.runtime_catalog(),
            self.demand.runtime_tuning(),
        );
        self.treasury
            .collect_property_tax(building_action_execution.property_tax_paid as f64);
        self.demand
            .log_hourly_household_action_diagnostics(day_index, minute_of_day);
        self.demand
            .log_hourly_building_action_diagnostics(day_index, minute_of_day);
        debug_log!(
            "economy",
            "hourly demand: day={} minute={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{})",
            day_index,
            minute_of_day,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    fn collect_fiscal_revenue(&mut self, revenue: FiscalRevenue) {
        self.treasury.collect_income_tax(revenue.income_tax as f64);
        self.treasury
            .collect_household_vat(revenue.household_vat as f64);
        self.treasury
            .collect_business_purchase_tax(revenue.business_purchase_tax as f64);
        self.treasury
            .collect_business_profit_tax(revenue.business_profit_tax as f64);
        self.treasury
            .collect_property_tax(revenue.property_tax as f64);
    }

    /// Called once per in-game day by the tick loop to emit per-building economy lines.
    pub fn print_daily_building_economy_for_day(&mut self, day_index: u32) {
        self.print_daily_building_economy(day_index);
    }

    /// Pre-computes all per-frame rendering data into a `RenderSnapshot`.
    ///
    /// Called from the background thread at the end of every movement tick.
    /// Uses only pure Rust types so the resulting snapshot is `Send`.
    pub fn build_snapshot(&self) -> RenderSnapshot {
        use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

        let mut pedestrian_transforms: HashMap<u8, Vec<f32>> = HashMap::new();
        let mut car_transforms: HashMap<u8, Vec<f32>> = HashMap::new();
        let mut car_render_ids: HashMap<u8, Vec<i64>> = HashMap::new();

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }

            let mut world_x = self.agents.pos_x[i];
            let mut world_z = self.agents.pos_y[i];
            let mut lane_pose = None;
            let lane_id = self.agents.current_lane_id[i];
            if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
                let lane = &self.transit_network.lane_system.lanes[lane_id];
                lane_pose = sample_lane_pose(lane, self.agents.lane_distance[i]);
                if let Some((pos, _)) = lane_pose {
                    world_x = pos.x;
                    world_z = pos.z;
                }
            }

            if cull
                && (world_x < aabb_x_min
                    || world_x > aabb_x_max
                    || world_z < aabb_z_min
                    || world_z > aabb_z_max)
            {
                continue;
            }
            let terrain_y = self.heightmap.sample_height_world(world_x, world_z) * 20.0;

            if self.agents.transit_mode[i] != MODE_CAR {
                // Pedestrian / walker — use variant MMI and oriented basis.
                let p_type = self.agents.pedestrian_type[i];
                let walk_cycle = self.agents.walk_phase[i];
                let buffer = pedestrian_transforms.entry(p_type).or_default();

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let world_y = terrain_y + 0.05; // small lift so feet clear terrain surface

                if let Some((_, tangent)) = lane_pose {
                    // GLTF export converts Blender -Y (character facing) to +Z, so the
                    // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
                    basis_z = tangent;
                    let right = Vector3::UP.cross(basis_z);
                    if right.length_squared() > 1e-6 {
                        basis_x = right.normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);

                // Add walk_phase in CUSTOM_DATA0.x (requires MultiMesh use_custom_data = true)
                buffer.push(walk_cycle);
                buffer.push(0.0);
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                // Car — oriented along lane geometry.
                let v_type = self.agents.vehicle_type[i];
                let render_id = self.agents.render_id[i];
                let variant_id = (render_id % 5) as u8;
                let model_key = (v_type * 10) + variant_id;
                let buffer = car_transforms.entry(model_key).or_default();
                car_render_ids
                    .entry(model_key)
                    .or_default()
                    .push(render_id.min(i64::MAX as u64) as i64);

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let mut world_y = terrain_y + 0.02;

                if let Some((pos, tangent)) = lane_pose {
                    world_y = pos.y + 0.02;
                    basis_z = -tangent;
                    let right = Vector3::UP.cross(basis_z);
                    if right.length_squared() > 1e-6 {
                        basis_x = right.normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);
            }
        }

        let node_positions: Vec<godot::prelude::Vector3> = self
            .region_graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(i, _)| self.region_graph.get_valid_node(*i as u32) == *i as u32)
            .map(|(_, n)| n.pos)
            .collect();

        let (terrain_world_w, terrain_world_h) = self.heightmap.world_size();

        RenderSnapshot {
            pedestrian_transforms,
            car_transforms,
            car_render_ids,
            terrain_dirty: self.terrain_dirty,
            water_dirty: self.water_dirty,
            network_dirty: self.network_dirty,
            node_positions,
            current_day: self.time.day_index,
            current_minute_of_day: self.time.minute_of_day,
            last_tick_ms: self.last_tick_duration,
            last_agent_tick_us: self.last_agent_tick_us,
            pathfind_count: self
                .agents
                .pathfind_count
                .load(std::sync::atomic::Ordering::Relaxed),
            agent_count: self.agents.len() as i32,
            treasury_balance: self.treasury.balance,
            heightmap_width: self.heightmap.width,
            heightmap_height: self.heightmap.height,
            terrain_world_size: godot::prelude::Vector2::new(terrain_world_w, terrain_world_h),
        }
    }
}

/// Background simulation thread loop.
///
/// Runs at ~60 Hz, decoupled from Godot's render frame. The `core` mutex is held
/// for the duration of each movement tick; main-thread `#[func]` calls block for
/// at most one tick duration while the lock is held (~7 ms at 100 k agents).
/// After the tick the snapshot is written while the lock is *not* held, so render
/// reads are completely non-blocking.
pub(crate) fn run_sim_thread(
    core: Arc<Mutex<SimCore>>,
    snapshot: Arc<RwLock<RenderSnapshot>>,
    road_preview_context: Arc<RwLock<RoadPreviewWorkerContext>>,
    road_query_snapshot: Arc<RwLock<RoadToolQuerySnapshot>>,
    cmd_rx: std::sync::mpsc::Receiver<SimCommand>,
) {
    const TARGET_DT: f64 = 1.0 / 60.0;
    let target = Duration::from_micros(16_667); // ~60 Hz
    let mut continuous_water_accumulator_s = 0.0_f64;

    loop {
        let frame_start = Instant::now();

        // Drain all pending commands — non-blocking.
        let mut should_quit = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(SimCommand::Quit) => {
                    should_quit = true;
                    break;
                }
                Ok(SimCommand::SetSpeed(s)) => {
                    core.lock().unwrap().time.speed_multiplier = s;
                }
                Ok(SimCommand::SetCameraAabb(x0, x1, z0, z1)) => {
                    core.lock().unwrap().camera_aabb = (x0, x1, z0, z1);
                }
                Ok(SimCommand::AddRoad {
                    points,
                    fwd_lanes,
                    bkw_lanes,
                }) => {
                    let road_total = Instant::now();
                    let cache_inputs = {
                        let mut c = core.lock().unwrap();
                        // Bulk-load defers per-edge rebuilds until finalization.
                        c.transit_network.bulk_load = true;
                        c.add_road_internal(points, fwd_lanes, bkw_lanes);
                        {
                            let c = &mut *c;
                            c.transit_network.bulk_load = false;

                            // Take dirty edges first so we can derive the affected nodes
                            // for the incremental clips pass.
                            let dirty = std::mem::take(&mut c.transit_network.bulk_dirty_edges);
                            let dirty_count = dirty.len();

                            // Collect nodes touched by the new/split edges.
                            let mut affected_nodes = std::collections::HashSet::new();
                            for &e_id in &dirty {
                                if e_id < c.region_graph.edge_count()
                                    && !c.region_graph.edge(e_id).deleted
                                {
                                    let e = c.region_graph.edge(e_id);
                                    affected_nodes
                                        .insert(c.region_graph.get_valid_node(e.start_node));
                                    affected_nodes
                                        .insert(c.region_graph.get_valid_node(e.end_node));
                                }
                            }
                            if crate::debug::category_enabled("road")
                                && std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
                                    .map(|value| !value.is_empty() && value != "0")
                                    .unwrap_or(false)
                            {
                                c.last_surface_debug_edges.extend(dirty.iter().copied());
                                c.last_surface_debug_edges.sort_unstable();
                                c.last_surface_debug_edges.dedup();
                            }

                            let t_clips = Instant::now();
                            c.region_graph
                                .rebuild_intersection_clips_for_nodes(&affected_nodes);
                            let dt_clips_us = t_clips.elapsed().as_micros();

                            let t_inv = Instant::now();
                            // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                            c.agents.invalidate_lane_ids_for_edges(
                                &dirty,
                                &c.transit_network.lane_system,
                            );
                            let dt_inv_us = t_inv.elapsed().as_micros();

                            let t_lanes = Instant::now();
                            c.transit_network
                                .lane_system
                                .rebuild_edges_incremental(&mut c.region_graph, &dirty);
                            let dt_lanes_us = t_lanes.elapsed().as_micros();
                            c.allocator.rebuild_entrance_cache(
                                &c.region_graph,
                                &c.transit_network.lane_system,
                            );

                            // Rebuild CCH and run the connectivity check. This is the only
                            // place the CCH is actually rebuilt for road placements — the
                            // sim-tick path is gated on speed > 0.0 and would miss paused edits.
                            c.transit_network.rebuild_cch_and_check(&c.region_graph);
                            c.transit_network.cch_dirty_chunks.clear();

                            // Zone flush is deferred to the next simulate_tick_internal call
                            // so it does not block road placement. zoning_dirty_edges accumulates.

                            let total_us = road_total.elapsed().as_micros();
                            let msg = format!(
                                "TOTAL={}µs  {}  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                                total_us,
                                c.last_road_timing,
                                dt_clips_us,
                                dt_lanes_us,
                                dirty_count,
                                dt_inv_us
                            );
                            debug_log!("road", "{}", msg);
                            c.last_road_timing = msg;
                        }
                        c.rebuild_network_surface_terrain_internal();
                        c.precompute_road_mesh_data();
                        c.collect_refined_terrain_patch_build_inputs(
                            ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
                        )
                    };
                    let cache_entries =
                        SimCore::build_refined_terrain_patch_cache_entries(cache_inputs);
                    let mut c = core.lock().unwrap();
                    c.insert_refined_terrain_patch_cache_entries(cache_entries);
                    c.network_dirty = true;
                    let preview_context = RoadPreviewWorkerContext::from_core(&c);
                    let query_snapshot = RoadToolQuerySnapshot::from_core(&c);
                    drop(c);
                    *road_preview_context.write().unwrap() = preview_context;
                    *road_query_snapshot.write().unwrap() = query_snapshot;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    should_quit = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        if should_quit {
            return;
        }

        // Tick and build snapshot inside one lock acquisition.
        let new_snapshot = {
            // Recover from a poisoned mutex (caused by a previous tick panic) rather
            // than propagating a PoisonError cascade to every main-thread call.
            let mut core = match core.lock() {
                Ok(g) => g,
                Err(e) => {
                    godot_error!("[sim] mutex was poisoned by a previous tick panic — recovering");
                    e.into_inner()
                }
            };
            let speed = core.time.speed_multiplier;

            if speed > 0.0 {
                // Rebuild CCH if dirty, then rebuild any dirty flow fields.
                let c = &mut *core;
                c.transit_network
                    .rebuild_pathing_if_dirty(&mut c.region_graph);
                {
                    let alloc = &c.allocator;
                    let graph = &c.region_graph;
                    c.transit_network
                        .flow_fields
                        .rebuild_dirty(graph, |zone, mode_flags| {
                            alloc.get_sources_for_zone(zone, graph, mode_flags)
                        });
                }

                let dt = (TARGET_DT * speed as f64) as f32;
                let t_agent = Instant::now();

                // Wrap the tick in catch_unwind so that a panic inside the agent loop
                // does NOT poison the mutex.  The MutexGuard stays alive in the outer
                // frame, so the lock is still held across the catch boundary.
                let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let c = &mut *core;
                    c.agents.tick(
                        &c.allocator,
                        &mut c.transit_network,
                        &mut c.region_graph,
                        dt,
                        c.time.day_index,
                        c.time.minute_of_day,
                    );
                }));
                if let Err(e) = tick_result {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] tick panicked — skipping frame: {}", msg);
                }

                core.last_agent_tick_us = t_agent.elapsed().as_micros() as u64;

                let time_advance = core.time.process_delta(TARGET_DT);
                if time_advance.has_elapsed_minutes() {
                    for (step_day_index, step_minute_of_day) in time_advance.iter_elapsed_minutes()
                    {
                        if step_minute_of_day % 60 == 0 {
                            let hourly_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    core.simulate_operational_hour_internal(
                                        step_day_index,
                                        step_minute_of_day,
                                    )
                                }));
                            if let Err(e) = hourly_result {
                                let msg = e
                                    .downcast_ref::<&str>()
                                    .copied()
                                    .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                                    .unwrap_or("(non-string payload)");
                                godot_error!(
                                    "[sim] operational hour tick panicked — skipping hour: {}",
                                    msg
                                );
                            }
                            if step_minute_of_day != 0 && crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                        }
                        if step_minute_of_day == 0 {
                            let daily_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    core.simulate_tick_internal(step_day_index)
                                }));
                            if let Err(e) = daily_result {
                                let msg = e
                                    .downcast_ref::<&str>()
                                    .copied()
                                    .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                                    .unwrap_or("(non-string payload)");
                                godot_error!("[sim] daily tick panicked — skipping day: {}", msg);
                            }
                            if crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                            core.print_daily_building_economy_for_day(step_day_index);
                        }
                    }
                }
            }

            let continuous_water_time_scale = if speed > 0.0 {
                f64::from(speed)
            } else if core.water_runtime_realtime_when_paused {
                1.0
            } else {
                0.0
            };
            if continuous_water_time_scale > 0.0 {
                continuous_water_accumulator_s += TARGET_DT * continuous_water_time_scale;
                while continuous_water_accumulator_s + f64::EPSILON
                    >= CONTINUOUS_WATER_TICK_INTERVAL_S
                {
                    core.tick_continuous_water_runtime_internal(CONTINUOUS_WATER_TICK_DT);
                    continuous_water_accumulator_s -= CONTINUOUS_WATER_TICK_INTERVAL_S;
                }
            }

            // build_snapshot only reads state; wrap anyway so a panic here does
            // not poison the mutex and kill the render thread.
            let snap_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.build_snapshot()));
            match snap_result {
                Ok(s) => s,
                Err(e) => {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] build_snapshot panicked — using default: {}", msg);
                    RenderSnapshot::default()
                }
            }
        };

        // Write snapshot — outside the sim lock so render reads are non-blocking.
        *snapshot.write().unwrap() = new_snapshot;

        // Sleep to maintain ~60 Hz.
        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
}
