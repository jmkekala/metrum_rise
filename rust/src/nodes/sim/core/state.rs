// SPDX-License-Identifier: GPL-2.0-only

//! Authoritative simulation state and deterministic cadence transitions.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use super::budget::{
    CityServicePolicy, CityTreasury, DailyBudgetLedgerEntry, ROAD_UPKEEP_PER_METER_PER_DAY,
    RoadLengthRoll,
};
use super::frame::EconomyObserver;
use super::schedule::{SimEvent, SimSystem, event_slot, simulation_schedule};
use super::snapshot::SimulationSnapshot;
use super::terrain_payloads::{
    CachedRefinedTerrainPatch, ROAD_LOCKED_TERRAIN_RENDER_STEP_M, RefinedTerrainAssemblyLedger,
    RefinedTerrainPatchCacheKey,
};
use super::trips::TripEvent;
use super::water_preview::{AuthoredWaterPatchFillDebug, WorldLakeFillPreview};
use crate::debug_log;
use crate::simulation::agriculture::AgricultureSystem;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::handles::{Handle, HandleTable};
use crate::simulation::core::scheduler::{
    SLOTS_PER_HOUR, SLOTS_PER_MINUTE, Schedule, absolute_slot,
};
use crate::simulation::core::time::TimeSystem;
use crate::simulation::core::timing_wheel::{TimedEvent, TimingWheel};
use crate::simulation::core::weather::WeatherState;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::demand::{
    DemandBuildingActionPlan, DemandSpawnAction, DemandSystem, PlanningPreparation, PlanningSlot,
    SpawnPermit, StartRefusal,
};
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::hiring::{
    CENSUS_WINDOW_START, FrozenCensusParameters, PLANNING_PREPARATION_MASKS_SLOT,
    PLANNING_PREPARATION_REST_SLOT, PLANNING_PREPARATION_ROWS_SLOT, PLANNING_PREPARATION_SLOT,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::extraction::ResourceExtractionSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::sweep::EnvironmentSweep;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::road_edit::FinalizedRoadGeometry;
use crate::simulation::network::surface::{RoadSurfaceCompileReason, SurfaceChunkKey};
use crate::simulation::resources::ResourceDepositSystem;
use crate::simulation::terrain::{TerrainSystem, terrain_cdt_local_sample_margin_m};
use crate::simulation::vegetation::{VegetationConfig, VegetationGenerator};
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{AuthoredLakeFill, AuthoredOpenWaterFill};
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector3;

const MINUTES_PER_DAY_U64: u64 = 24 * 60;
const DEMAND_SPAWN_ACTIONS_PER_MINUTE: usize = 1;

/// Demand-planned private building spawn delayed to a later authored minute.
#[derive(Clone, Debug)]
pub(crate) struct PendingDemandSpawnAction {
    /// Stable id named by the spawn's construction-start event.
    pub(crate) id: u64,
    /// Absolute authored minute when this spawn may be released.
    pub(crate) due_minute: u64,
    /// Demand bucket that selected this spawn.
    pub(crate) zone_type: ZoneType,
    /// Final parcel and asset placement request.
    pub(crate) action: DemandSpawnAction,
    /// Operational day when demand originally planned the spawn.
    pub(crate) planned_day_index: u32,
    /// Minute of day when demand originally planned the spawn.
    pub(crate) planned_minute_of_day: u16,
    /// A commercial or industrial spawn's permit from the latest planning slot's construction
    /// epoch (`SIM-02-I`): it starts only with a granted one and waits on a voided one. Saved.
    pub(crate) permit: SpawnPermit,
    /// Whether the latest planning slot counted this home's applicants (its queued project was
    /// active, not a later duplicate of a parcel or on a parcel no longer available): a counted
    /// home that fails to place voids the granted business permits (`SIM-02-I`). Saved, so a
    /// loaded game decides as the planning slot did.
    pub(crate) counted: bool,
}

// Construction starts of `pending`, in queue order, at their due minute and at most one per
// minute after `current_minute`, each at its minute's first slot or, when that slot is
// queue-free, the next (`event_slot`).
fn reschedule_construction_starts(
    events: &mut TimingWheel<SimEvent>,
    pending: &VecDeque<PendingDemandSpawnAction>,
    current_minute: u64,
) {
    events.retain(|event| !matches!(event, SimEvent::ConstructionStart { .. }));
    let mut previous = current_minute;
    for spawn in pending {
        let minute = spawn.due_minute.max(previous + 1);
        events.schedule(
            event_slot(minute * SLOTS_PER_MINUTE),
            SimEvent::ConstructionStart { id: spawn.id },
        );
        previous = minute;
    }
}

// Completion events of every construction site in dense order, for a wheel restarted
// wholesale; replaces any completion events and makes the pending list redundant.
fn reschedule_construction_completions(
    events: &mut TimingWheel<SimEvent>,
    allocator: &BuildingAllocator,
    handles: &HandleTable<Building>,
) {
    events.retain(|event| !matches!(event, SimEvent::ConstructionComplete { .. }));
    for (building_idx, building) in allocator.buildings.iter().enumerate() {
        if building.is_under_construction() {
            events.schedule(
                building.construction_complete_slot,
                SimEvent::ConstructionComplete {
                    building: handles.handle(building_idx),
                },
            );
        }
    }
}

// Trip events of a wheel about to restart, in firing order.
fn pending_trips(events: &TimingWheel<SimEvent>) -> Vec<TimedEvent<SimEvent>> {
    let mut trips = events.pending();
    trips.retain(|timed| matches!(timed.event, SimEvent::Trip(_)));
    trips
}

// Puts trips back into a restarted wheel in their firing order; past slots fire next.
fn reschedule_trips(events: &mut TimingWheel<SimEvent>, trips: Vec<TimedEvent<SimEvent>>) {
    for timed in trips {
        events.schedule(timed.slot, timed.event);
    }
}

pub(super) fn absolute_operational_minute(day_index: u32, minute_of_day: u16) -> u64 {
    u64::from(day_index.saturating_sub(1)) * MINUTES_PER_DAY_U64 + u64::from(minute_of_day)
}

pub(super) fn demand_plan_has_non_spawn_actions(plan: &DemandBuildingActionPlan) -> bool {
    [&plan.residential, &plan.commercial, &plan.industrial]
        .iter()
        .any(|use_plan| {
            !use_plan.despawns.is_empty()
                || !use_plan.downgrades.is_empty()
                || !use_plan.upgrades.is_empty()
        })
}

// The part of the hour's plan for zone type `zone_type`, one of the private zone families.
fn zone_plan(
    plan: &mut DemandBuildingActionPlan,
    zone_type: ZoneType,
) -> &mut crate::simulation::economy::demand::DemandUseActionPlan {
    match zone_type {
        ZoneType::Commercial => &mut plan.commercial,
        ZoneType::Industrial => &mut plan.industrial,
        _ => &mut plan.residential,
    }
}

pub(super) fn demand_plan_without_spawns(
    plan: &DemandBuildingActionPlan,
) -> DemandBuildingActionPlan {
    let mut immediate_plan = plan.clone();
    immediate_plan.residential.spawns.clear();
    immediate_plan.commercial.spawns.clear();
    immediate_plan.industrial.spawns.clear();
    immediate_plan
}

/// All simulation state — owned exclusively by the background sim thread when running.
///
/// The main thread accesses this via `Arc<Mutex<SimCore>>`. Transactional edits and
/// missing/stale click-plan compilation may hold the lock longer than a simulation tick;
/// pointer previews use immutable worker snapshots and nonblocking local dependency capture.
pub struct SimCore {
    /// Simulation clock and day counter.
    pub time: TimeSystem,
    /// Weather scenario and cloud drift, advanced on the clock's speed-scaled seconds.
    pub weather: WeatherState,
    /// Counts world replacements (load, new world), so derived render data keyed by the
    /// water and terrain generations, which restart with each world, still changes.
    pub(crate) world_epoch: u64,
    /// Where morning mist can lie, derived from the water and terrain; see `refresh_mist_map`.
    pub(crate) mist_map: std::sync::Arc<crate::nodes::sim::render::mist::MistMap>,
    /// World epoch and water/terrain generations `mist_map` was built from.
    pub(crate) mist_map_key: [u64; 4],
    /// When `mist_map` was last rebuilt, to bound rebuilds during continuous edits.
    pub(crate) mist_map_built_at: Option<std::time::Instant>,
    /// Terrain heightmap.
    pub heightmap: TerrainSystem,
    /// Authored baseline water depths and derived rendering state.
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
    /// Demand building spawns scheduled across authored minutes to avoid hourly bursts.
    pub(crate) pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
    /// Next [`PendingDemandSpawnAction::id`]; saved.
    pub(crate) next_spawn_id: u64,
    /// System declarations in dispatch order (`SIM-02`); fixed, not saved.
    pub(crate) schedule: Schedule<SimSystem>,
    /// Timed events keyed by absolute slot; saved.
    pub(crate) events: TimingWheel<SimEvent>,
    /// Reused buffer for the events of the slot being dispatched.
    pub(crate) due_events: Vec<TimedEvent<SimEvent>>,
    /// Reused lists of the entities of one hourly phase (`SIM-02-F`).
    pub(crate) building_phase_scratch: Vec<Handle<Building>>,
    pub(crate) household_phase_scratch:
        Vec<Handle<crate::simulation::economy::households::Household>>,
    pub(crate) agent_phase_scratch: Vec<crate::simulation::economy::agents::AgentHandle>,
    /// Rolling sum of the road network's length for daily upkeep (`SIM-02-G`); saved.
    pub(crate) road_length: RoadLengthRoll,
    /// Rolling day-long sweep of pollution, noise and desirability (`SIM-02-H`); saved.
    pub(crate) environment_sweep: EnvironmentSweep,
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
    /// Vegetation generation parameters resolved against this world.
    pub vegetation: VegetationGenerator,
    /// Sparse authoritative player edits and render-patch revisions.
    pub vegetation_edits: crate::simulation::vegetation::edits::VegetationEdits,
    /// City-level fiscal ledger tracking infrastructure build cost and daily upkeep.
    pub treasury: CityTreasury,
    /// Player-controlled live service funding policies.
    pub(crate) service_policy: CityServicePolicy,
    /// Player-controlled live fiscal tax and transfer policies.
    pub(crate) fiscal_policy: CityFiscalPolicy,
    /// Completed daily budget ledger entries for overview windows and trend graphs.
    pub(crate) budget_history: VecDeque<DailyBudgetLedgerEntry>,
    /// Lifetime build cost observed when the most recent budget ledger entry was recorded.
    pub(crate) budget_last_lifetime_build_cost: f64,
    /// Runtime-only economy debug counter reset after each daily diagnostic line.
    pub(crate) debug_household_admissions_since_daily: u32,
    /// Undo history stack — kept in SimCore so all mutations are co-located.
    pub(crate) undo_stack: VecDeque<SimulationSnapshot>,
    /// Authored-world lake fill records when editing or playing from a `WorldDefinition`.
    pub(crate) world_lake_fills: Vec<AuthoredLakeFill>,
    /// Authored-world edge-connected open-water fills when editing or playing from a `WorldDefinition`.
    pub(crate) world_open_water_fills: Vec<AuthoredOpenWaterFill>,
    /// Authored natural-resource deposit layers when editing or playing from a `WorldDefinition`.
    pub(crate) resource_deposits: ResourceDepositSystem,
    /// Live resource-extractor polygons and depletion counters.
    pub(crate) resource_extraction: ResourceExtractionSystem,
    /// Live agricultural field polygons for renewable farm production.
    pub(crate) agriculture: AgricultureSystem,
    /// Transient world-editor lake-fill preview. Never saved into `WorldDefinition`.
    pub(crate) world_lake_fill_preview: Option<WorldLakeFillPreview>,
    /// Cached authored-water fill debug summaries keyed by water render patch.
    pub(crate) authored_water_patch_fill_debug_cache:
        HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
    /// True while the world editor is accumulating one terrain brush stroke.
    pub(crate) terrain_stroke_active: bool,
    /// True once the active terrain brush stroke has applied at least one terrain mutation.
    pub(crate) terrain_stroke_has_changes: bool,
    /// Set by terrain mutations until Godot acknowledges the exact dirty patch revisions.
    pub terrain_dirty: bool,
    /// Set by water mutations until Godot acknowledges the exact dirty patch revisions.
    pub water_dirty: bool,
    /// Set by any network mutation until `NetworkRenderer` acknowledges the published generation.
    pub network_dirty: bool,
    /// True when running in benchmark mode (skips undo stack on road placement).
    pub benchmark_mode: bool,
    /// Duration of the last daily economy tick in milliseconds.
    pub last_tick_duration: f64,
    /// Duration of the last agent movement tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Per-phase timing breakdown from the last road placement, for profiling.
    pub last_road_timing: String,
    /// Why the last refused road placement was refused; `commit_road` reports and clears it.
    pub(crate) last_road_rejection: Option<crate::nodes::sim::core::RoadRejection>,
    /// Fixed-size diagnostics; dictionary conversion happens only when explicitly requested.
    pub(crate) last_road_edit_metrics: crate::nodes::sim::benchmark::road_edit::RoadEditMetrics,
    /// Edge ids touched by the most recent committed network edit and queued for one focused
    /// road-surface debug dump after the next terrain/mesh rebuild.
    pub(crate) last_surface_debug_edges: Vec<usize>,
    /// Production refined terrain patches precomputed by the sim thread for Godot upload.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, Arc<CachedRefinedTerrainPatch>>,
    /// Sorted terrain render patches that must use road-locked refined terrain meshes.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
    /// Required grading margin for each road-locked terrain render patch.
    pub(crate) road_locked_terrain_patch_margins: BTreeMap<(usize, usize), f32>,
    /// Terrain patches owned by at least one indexed building site.
    pub(crate) building_site_owned_terrain_patch_keys: HashSet<(usize, usize)>,
    /// Site boxes whose render-patch ownership the building removal queue left for later slots
    /// (`SIM-02-K`), oldest first: render state only, refreshed one a light slot. Derived.
    pub(crate) deferred_site_terrain_boxes: std::collections::VecDeque<(f32, f32, f32, f32)>,
    /// Sorted terrain patches owned by roads, building sites, or both.
    pub(crate) engineered_terrain_patch_keys: Vec<(usize, usize)>,
    /// Required CDT query margin for every engineered-owned terrain patch.
    pub(crate) engineered_terrain_patch_margins: BTreeMap<(usize, usize), f32>,
    /// Monotonic source revision allocator for asynchronous terrain payloads.
    pub(crate) terrain_payload_generation_counter: u64,
    /// Latest source revision that invalidates every terrain render patch.
    pub(crate) terrain_payload_global_generation: u64,
    /// Latest building-site source revision for individually affected render patches.
    pub(crate) terrain_payload_patch_generations: HashMap<(usize, usize), u64>,
    /// Generation-stamped full/local refined-terrain assembly scopes by render patch.
    pub(crate) refined_terrain_assembly_ledgers:
        HashMap<(usize, usize), RefinedTerrainAssemblyLedger>,
    /// Current road render meshes keyed by deterministic road-surface chunk.
    pub(crate) cached_road_mesh_chunks: BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>,
    /// Upsert payloads accumulated for the unacknowledged road-render revision.
    ///
    /// Unlike the authoritative cache, this map contains only chunks the renderer must upload.
    /// Snapshot publication therefore copies metadata proportional to pending work, not city size.
    pub(crate) published_road_mesh_chunks: Arc<BTreeMap<SurfaceChunkKey, Arc<NetworkMeshData>>>,
    /// Changed or removed road chunks accumulated until the current generation is acknowledged.
    pub(crate) pending_road_mesh_chunks: Arc<BTreeSet<SurfaceChunkKey>>,
    /// True when the renderer must discard every chunk from the previous world.
    pub(crate) road_mesh_full_replace: bool,
    /// Road-tool surface generation represented by the cached road mesh.
    pub(crate) cached_road_mesh_generation: u64,
    /// Cached world-space positions of live canonical network nodes for render snapshots.
    pub(crate) cached_network_node_positions: Arc<Vec<Vector3>>,
    /// True when network topology changed and the cached node-position snapshot must rebuild.
    pub(crate) cached_network_node_positions_dirty: bool,
    /// Monotonic stamp for road-tool terrain/surface snapshots.
    pub(crate) road_tool_surface_generation: u64,
    /// World-space AABB for frustum culling: (x_min, x_max, z_min, z_max).
    /// Agents outside this rect are excluded from `RenderSnapshot` transforms.
    /// Updated each frame via `SimCommand::SetCameraAabb`. Defaults to "show all".
    pub camera_aabb: (f32, f32, f32, f32),
    /// Per-chunk posed-agent buffers the render snapshot reuses; render scratch, not state.
    pub(crate) snapshot_agent_scratch: super::snapshot::AgentPoseScratch,
    /// Route scratch of the slots' serial trip pricing and planning, installed as the frame
    /// thread's scratch while slots run, so it is prepared once (at load) rather than on each
    /// pool worker that first runs a slot (`SIM-02-K`); scratch, not state.
    pub(crate) slot_route_scratch: crate::simulation::economy::agents::tick::TripQueryScratch,
    /// Whether the zoning cell overlay is shown; stale warm cell chunks then regenerate in the
    /// same tick as the edit that invalidated them. UI state, kept across world resets.
    pub(crate) cell_overlay_visible: bool,
    /// Runtime mesh support geometry; rendering metadata, not saved simulation state.
    pub(crate) vehicle_ground_support:
        [crate::nodes::sim::render::vehicle_ground::VehicleGroundSupport; 5],
}

impl SimCore {
    /// Creates an empty, paused world of `config`'s extent with the authored startup treasury.
    ///
    /// Terrain and water start dirty so the first render snapshot uploads every patch.
    pub fn new(config: WorldConfig) -> Self {
        let time = TimeSystem::new();
        let next_slot = time.absolute_slot() + 1;
        let schedule = simulation_schedule();
        debug_log!("sim", "schedule: {}", schedule.describe());
        Self {
            time,
            weather: WeatherState::default(),
            world_epoch: 0,
            mist_map: Default::default(),
            mist_map_key: [u64::MAX; 4],
            mist_map_built_at: None,
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: crate::simulation::network::graph::RegionGraph::new(),
            transit_network: TransitNetwork::new_for_world(&config),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            pending_demand_spawns: VecDeque::new(),
            next_spawn_id: 0,
            schedule,
            events: TimingWheel::new(next_slot),
            due_events: Vec::new(),
            building_phase_scratch: Vec::new(),
            household_phase_scratch: Vec::new(),
            agent_phase_scratch: Vec::new(),
            road_length: RoadLengthRoll::default(),
            environment_sweep: EnvironmentSweep::default(),
            allocator: BuildingAllocator::new_at_slot(next_slot - 1),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            vegetation_edits: Default::default(),
            vegetation: VegetationGenerator::resolve(VegetationConfig::default(), &config),
            treasury: CityTreasury::default(),
            service_policy: Default::default(),
            fiscal_policy: Default::default(),
            budget_history: VecDeque::new(),
            budget_last_lifetime_build_cost: 0.0,
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            resource_deposits: ResourceDepositSystem::from_world_config(&config),
            resource_extraction: ResourceExtractionSystem::new(),
            agriculture: AgricultureSystem::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            benchmark_mode: false,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_road_rejection: None,
            last_road_edit_metrics: Default::default(),
            last_surface_debug_edges: Vec::new(),
            refined_terrain_patch_cache: HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            road_locked_terrain_patch_margins: BTreeMap::new(),
            building_site_owned_terrain_patch_keys: HashSet::new(),
            deferred_site_terrain_boxes: Default::default(),
            engineered_terrain_patch_keys: Vec::new(),
            engineered_terrain_patch_margins: BTreeMap::new(),
            terrain_payload_generation_counter: 1,
            terrain_payload_global_generation: 1,
            terrain_payload_patch_generations: HashMap::new(),
            refined_terrain_assembly_ledgers: HashMap::new(),
            cached_road_mesh_chunks: BTreeMap::new(),
            published_road_mesh_chunks: Arc::new(BTreeMap::new()),
            pending_road_mesh_chunks: Arc::new(BTreeSet::new()),
            road_mesh_full_replace: true,
            cached_road_mesh_generation: 0,
            cached_network_node_positions: Arc::new(Vec::new()),
            cached_network_node_positions_dirty: true,
            road_tool_surface_generation: 1,
            camera_aabb: (0.0, 0.0, 0.0, 0.0), // 0.0 == 0.0 → cull disabled by default
            snapshot_agent_scratch: Default::default(),
            slot_route_scratch: Default::default(),
            cell_overlay_visible: false,
            vehicle_ground_support: Default::default(),
        }
    }

    /// Invalidates immutable road-tool query snapshots without forcing terrain payload rebuilds.
    pub(crate) fn bump_road_tool_query_generation(&mut self) {
        self.road_tool_surface_generation =
            self.road_tool_surface_generation.wrapping_add(1).max(1);
    }

    fn next_terrain_payload_generation(&mut self) -> u64 {
        self.terrain_payload_generation_counter = self
            .terrain_payload_generation_counter
            .wrapping_add(1)
            .max(1);
        self.terrain_payload_generation_counter
    }

    pub(crate) fn bump_global_terrain_payload_generation(&mut self) {
        self.terrain_payload_global_generation = self.next_terrain_payload_generation();
    }

    pub(crate) fn bump_terrain_payload_patch_generations(&mut self, patch_keys: &[(usize, usize)]) {
        if patch_keys.is_empty() {
            return;
        }
        let generation = self.next_terrain_payload_generation();
        for &key in patch_keys {
            self.terrain_payload_patch_generations
                .insert(key, generation);
            self.refined_terrain_assembly_ledgers
                .entry(key)
                .or_default()
                .full_dirty_at = Some(generation);
        }
    }

    /// Advances patch revisions while retaining fixed query-chunk scope for local road edits.
    pub(crate) fn bump_local_road_terrain_payload_generations(
        &mut self,
        patch_keys: &[(usize, usize)],
        query_chunks: &[(i32, i32)],
    ) {
        if patch_keys.is_empty() {
            return;
        }
        if query_chunks.is_empty() {
            self.bump_terrain_payload_patch_generations(patch_keys);
            return;
        }
        let generation = self.next_terrain_payload_generation();
        for &key in patch_keys {
            self.terrain_payload_patch_generations
                .insert(key, generation);
            let ledger = self
                .refined_terrain_assembly_ledgers
                .entry(key)
                .or_default();
            for &chunk in query_chunks {
                ledger.road_query_chunk_dirty_at.insert(chunk, generation);
            }
        }
    }

    pub(crate) fn terrain_payload_generation_for_patch(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> u64 {
        self.terrain_payload_patch_generations
            .get(&(patch_x, patch_z))
            .copied()
            .unwrap_or(0)
            .max(self.terrain_payload_global_generation)
    }

    pub(crate) fn terrain_dirty_patch_states(&self) -> Vec<(usize, usize, u64)> {
        let mut states = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .map(|&(patch_x, patch_z)| {
                (
                    patch_x,
                    patch_z,
                    self.terrain_payload_generation_for_patch(patch_x, patch_z),
                )
            })
            .collect::<Vec<_>>();
        states.sort_unstable();
        states
    }

    /// Acknowledges one terrain upload only when it matches the current patch revision.
    pub(crate) fn acknowledge_terrain_render_patch(
        &mut self,
        patch_x: usize,
        patch_z: usize,
        generation: u64,
    ) -> bool {
        if self.terrain_payload_generation_for_patch(patch_x, patch_z) != generation {
            return false;
        }
        self.heightmap.clear_render_patch_dirty(patch_x, patch_z);
        let key = (patch_x, patch_z);
        if let Some(ledger) = self.refined_terrain_assembly_ledgers.get_mut(&key) {
            if ledger
                .full_dirty_at
                .is_some_and(|stamp| stamp <= generation)
            {
                ledger.full_dirty_at = None;
            }
            ledger
                .road_query_chunk_dirty_at
                .retain(|_, stamp| *stamp > generation);
            if ledger.full_dirty_at.is_none() && ledger.road_query_chunk_dirty_at.is_empty() {
                self.refined_terrain_assembly_ledgers.remove(&key);
            }
        }
        true
    }

    /// Acknowledges a terrain render batch and reports whether it fully cleared current dirtiness.
    pub(crate) fn acknowledge_terrain_render_patches(
        &mut self,
        acknowledged: &[(usize, usize, u64)],
    ) -> bool {
        let mut all_accepted = true;
        for &(patch_x, patch_z, generation) in acknowledged {
            all_accepted &= self.acknowledge_terrain_render_patch(patch_x, patch_z, generation);
        }
        self.terrain_dirty = !self.heightmap.dirty_render_patches().is_empty();
        all_accepted && !self.terrain_dirty
    }

    fn mark_network_visuals_dirty(&mut self) {
        self.bump_road_tool_query_generation();
        self.network_dirty = true;
        self.cached_network_node_positions_dirty = true;
    }

    /// Marks network visuals and every terrain payload dirty after a world-wide reset.
    pub(crate) fn mark_network_render_dirty(&mut self) {
        self.bump_global_terrain_payload_generation();
        self.cached_road_mesh_chunks.clear();
        self.published_road_mesh_chunks = Arc::new(BTreeMap::new());
        self.pending_road_mesh_chunks = Arc::new(BTreeSet::new());
        self.road_mesh_full_replace = true;
        self.cached_road_mesh_generation = 0;
        self.mark_network_visuals_dirty();
    }

    /// Marks network visuals dirty while terrain payload revisions remain patch-local.
    pub(crate) fn mark_local_network_render_dirty(&mut self) {
        self.mark_network_visuals_dirty();
    }

    /// Acknowledges one network upload only when no newer surface mutation exists.
    pub(crate) fn acknowledge_network_render_generation(&mut self, generation: u64) -> bool {
        if self.road_tool_surface_generation != generation {
            return false;
        }
        self.network_dirty = false;
        self.published_road_mesh_chunks = Arc::new(BTreeMap::new());
        self.pending_road_mesh_chunks = Arc::new(BTreeSet::new());
        self.road_mesh_full_replace = false;
        true
    }

    pub(super) fn finalize_bulk_road_geometry_for_dirty_edges(&mut self) -> FinalizedRoadGeometry {
        self.transit_network
            .finalize_road_geometry(&mut self.region_graph)
    }
}

impl SimCore {
    /// Slot `slot`'s chunk of the rolling freight supplier index (`SIM-02-I`) that the next
    /// hour's freight planning reads.
    pub(super) fn advance_freight_supplier_sweep(&mut self, slot: u64) {
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.logistics.advance_supplier_sweep(
            slot,
            &self.allocator,
            &self.transit_network,
            &self.region_graph,
        );
    }

    /// What is left of the operational hour at each hour's first slot, after the census
    /// publication and before the hour's residential planning slot (`SIM-02-K`): the demand
    /// snapshot front takes the entities created since its sweep began, the household hour
    /// recounts pending arrivals and registers appended members, the logistics clock advances
    /// and expired commute quotes leave. O(created entities + queued admissions + arrival
    /// carriers in flight + expired quotes), none of which grows with the city's size
    /// (`Cost::Dirty`); a debug build adds an O(agents) reference check.
    pub(super) fn run_operational_hour(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
        observer: &mut impl EconomyObserver,
    ) {
        let absolute_hour = day_index
            .saturating_sub(1)
            .saturating_mul(24)
            .saturating_add(u32::from(minute_of_day / 60));
        // The hour's economy consumers and the census published before the hour price the
        // frontage delays published when that census started (`ECON-13-D`, `SIM-02-I`).
        // Production, farms, mines and household consumption run at each entity's hourly
        // phase (`SIM-02-F`).
        // The demand snapshot front takes the households and buildings created since its sweep
        // began first, before exits and job search change them, as the sweep took the others
        // (`SIM-02-I`).
        self.demand.fold_created_snapshot_entities(
            &self.allocator,
            &self.households,
            &self.fiscal_policy,
            self.treasury.balance,
        );
        // Households whose daily phase chose to leave are removed by the `departures` queue
        // in the hour's removal window (`SIM-02-J`).
        self.households.operational_hour_tick(
            &mut self.agents,
            &self.allocator,
            &mut self.logistics,
            absolute_hour,
            minute_of_day,
        );
        // Capture receipts before demand can remove or replace their building.
        observer.hourly_economy(self);
        // The residential planning slot that follows lends the census published before this
        // hour, current for the live city (`SIM-02-I`); nothing between the publication and the
        // slot invalidates it. Timing runs skip the check.
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            let mut borders = Vec::new();
            crate::simulation::economy::logistics::collect_connected_border_nodes(
                &self.region_graph,
                &mut borders,
            );
            assert!(
                self.households.job_funding.has_current_input_supply(
                    self.households.census_key(&self.transit_network),
                    &borders,
                    crate::simulation::economy::logistics::freight_price_phase(
                        self.demand.runtime_tuning(),
                        minute_of_day,
                    ),
                ),
                "the census published at the planning slot is current"
            );
        }
    }

    /// Fits every lane to the compiled road surface, for setups that build roads and borders
    /// through internal calls without the render preparation that fits connector heights.
    /// Agents must not run on unfitted lanes: loading the save fits them, which would change
    /// lane lengths and therefore arrival times. One-time O(lane geometry vertices).
    pub(crate) fn fit_lanes_to_road_surface(&mut self) {
        self.transit_network
            .road_surface
            .compile_dirty(&self.region_graph, &self.heightmap);
        self.transit_network
            .lane_system
            .sync_heights_to_visible_surface(
                &self.region_graph,
                &self.heightmap,
                &self.transit_network.road_surface,
            );
    }

    /// Rebuilds derived queue promises after planning or loading persisted pending actions.
    pub(crate) fn refresh_demand_construction_queue(&mut self, day_index: u32, minute_of_day: u16) {
        let now = absolute_operational_minute(day_index, minute_of_day);
        self.demand.set_queued_construction_with_entrances(
            &self.allocator,
            &self.zoning,
            now,
            self.pending_demand_spawns.iter().map(|pending| {
                (
                    pending.id,
                    pending.due_minute,
                    pending.zone_type,
                    &pending.action,
                )
            }),
            |action| {
                self.allocator.demand_site_entrance(
                    action.parcel_id,
                    &action.asset_id,
                    &self.zoning,
                    &self.region_graph,
                    &self.transit_network.lane_system,
                )
            },
        );
    }

    // Opens the planning slot's construction epoch when a queued commercial or industrial
    // spawn needs a permit or an immediate business level change needs a proof, and writes
    // every queued spawn's permit (`SIM-02-I`). The epoch stays open for the level changes.
    // Returns whether business level changes may run: an epoch skipped on books not valid for
    // the live city proves none, and its spawns wait for the next slot.
    fn prove_construction_permits(
        &mut self,
        immediate: &crate::simulation::economy::demand::DemandBuildingActionPlan,
        day_index: u32,
        minute_of_day: u16,
    ) -> bool {
        let business_spawns = self
            .pending_demand_spawns
            .iter()
            .any(|pending| pending.zone_type != ZoneType::Residential);
        let [residential, commercial, industrial] = [
            &immediate.residential,
            &immediate.commercial,
            &immediate.industrial,
        ]
        .map(|plan| !plan.downgrades.is_empty() || !plan.upgrades.is_empty());
        let business_level_changes = commercial || industrial;
        // The level changes rebuild stale doors before their proofs; the epoch must not see
        // the doors change under its lent books.
        if (residential || business_level_changes) && self.allocator.entrances_dirty {
            self.allocator
                .rebuild_entrance_cache(&self.region_graph, &self.transit_network.lane_system);
        }
        let opened = (business_spawns || business_level_changes)
            && self.demand.open_construction_epoch(
                &self.allocator,
                &mut self.households,
                &self.zoning,
                &self.region_graph,
                &self.transit_network,
                &mut self.logistics,
                &self.fiscal_policy,
                self.treasury.balance,
                absolute_operational_minute(day_index, minute_of_day),
            );
        for pending in &mut self.pending_demand_spawns {
            if pending.zone_type == ZoneType::Residential {
                pending.counted = self.demand.counts_queued_home(pending.id, &pending.action);
                continue;
            }
            pending.permit = if opened {
                self.demand.construction_permit(
                    pending.id,
                    &pending.action,
                    &self.region_graph,
                    &self.transit_network.lane_system,
                )
            } else {
                SpawnPermit::Voided
            };
        }
        opened
    }

    fn enqueue_hourly_demand_spawns(&mut self, day_index: u32, minute_of_day: u16) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let first_due_minute = self
            .pending_demand_spawns
            .back()
            .map(|pending| pending.due_minute.saturating_add(1))
            .unwrap_or_else(|| now.saturating_add(1))
            .max(now.saturating_add(1));
        let mut queued = 0_usize;

        for (zone_type, spawns) in [
            (
                ZoneType::Residential,
                &self.demand.building_actions.residential.spawns,
            ),
            (
                ZoneType::Commercial,
                &self.demand.building_actions.commercial.spawns,
            ),
            (
                ZoneType::Industrial,
                &self.demand.building_actions.industrial.spawns,
            ),
        ] {
            for action in spawns {
                let due_minute = first_due_minute
                    .saturating_add((queued / DEMAND_SPAWN_ACTIONS_PER_MINUTE) as u64);
                let id = self.next_spawn_id;
                self.next_spawn_id += 1;
                self.events.schedule(
                    event_slot(due_minute * SLOTS_PER_MINUTE),
                    SimEvent::ConstructionStart { id },
                );
                self.pending_demand_spawns
                    .push_back(PendingDemandSpawnAction {
                        id,
                        due_minute,
                        zone_type,
                        action: action.clone(),
                        planned_day_index: day_index,
                        planned_minute_of_day: minute_of_day,
                        permit: SpawnPermit::Denied,
                        counted: false,
                    });
                queued += 1;
            }
        }

        if queued > 0 {
            let last_due_minute = first_due_minute
                .saturating_add(((queued - 1) / DEMAND_SPAWN_ACTIONS_PER_MINUTE) as u64);
            debug_log!(
                "economy",
                "queued demand spawns: day={} minute={} queued={} pending_total={} first_due={} last_due={}",
                day_index,
                minute_of_day,
                queued,
                self.pending_demand_spawns.len(),
                first_due_minute,
                last_due_minute,
            );
        }
        queued
    }

    /// Starts queued spawn `id` when it is next in the queue; its event names it, so a spawn
    /// dropped from the queue (loading repairs, undo) is skipped. Returns whether it started,
    /// placed or rejected; a spawn whose permit was voided waits for the next planning slot.
    pub(super) fn start_pending_demand_spawn(
        &mut self,
        id: u64,
        day_index: u32,
        minute_of_day: u16,
    ) -> bool {
        if self
            .pending_demand_spawns
            .front()
            .is_none_or(|pending| pending.id != id)
        {
            return false;
        }
        let Some(pending) = self.pending_demand_spawns.pop_front() else {
            return false;
        };
        self.execute_pending_demand_spawn(pending, day_index, minute_of_day)
    }

    // After a queued home the planning slot counted failed to place: its applicants backed
    // the queue's granted business permits, which are voided; those spawns wait for the next
    // slot's epoch (`SIM-02-I`). O(queue).
    fn void_granted_permits(&mut self) {
        for pending in &mut self.pending_demand_spawns {
            if matches!(pending.permit, SpawnPermit::Granted(_)) {
                pending.permit = SpawnPermit::Voided;
            }
        }
    }

    // Puts `pending`, whose permit was voided, back at the queue's front to start after the
    // next planning slot proves it again, and every later start after it, in queue order
    // (`SIM-02-I`). O(queue + scheduled events).
    fn wait_for_next_planning_slot(&mut self, mut pending: PendingDemandSpawnAction, now: u64) {
        let mut due = (now / 60 + 1) * 60 + 1;
        pending.due_minute = pending.due_minute.max(due);
        self.pending_demand_spawns.push_front(pending);
        for pending in &mut self.pending_demand_spawns {
            pending.due_minute = pending.due_minute.max(due);
            due = pending.due_minute + 1;
        }
        reschedule_construction_starts(&mut self.events, &self.pending_demand_spawns, now);
    }

    /// Replaces every construction-start event after the queue was replaced wholesale (loading,
    /// undo, a new world). Each spawn starts at its due minute, one per minute from the next
    /// minute on, as the per-minute queue check did before `SIM-02-C`.
    pub(crate) fn reschedule_construction_events(&mut self) {
        reschedule_construction_starts(
            &mut self.events,
            &self.pending_demand_spawns,
            absolute_operational_minute(self.time.day_index, self.time.minute_of_day),
        );
    }

    /// The timing wheel as the next frame will see it: a copy with the construction sites
    /// placed since the last slot scheduled, or re-aligned to the clock when the clock was set
    /// directly ([`Self::align_events_to_clock`]).
    pub(crate) fn aligned_events(&self) -> std::borrow::Cow<'_, TimingWheel<SimEvent>> {
        let next_slot = self.time.absolute_slot() + 1;
        if self.events.next_slot() == next_slot {
            if self.allocator.pending_construction.is_empty() {
                return std::borrow::Cow::Borrowed(&self.events);
            }
            let mut events = self.events.clone();
            for &(building, slot) in &self.allocator.pending_construction {
                events.schedule(slot, SimEvent::ConstructionComplete { building });
            }
            return std::borrow::Cow::Owned(events);
        }
        let mut events = self.events.clone();
        let trips = pending_trips(&events);
        events.reset(next_slot);
        reschedule_construction_starts(
            &mut events,
            &self.pending_demand_spawns,
            absolute_operational_minute(self.time.day_index, self.time.minute_of_day),
        );
        let mut handles = self.allocator.building_handles.clone();
        handles.register_appended(self.allocator.buildings.len());
        reschedule_construction_completions(&mut events, &self.allocator, &handles);
        reschedule_trips(&mut events, trips);
        std::borrow::Cow::Owned(events)
    }

    /// Re-aligns the timing wheel when the clock was set directly instead of advanced (tests,
    /// tools): the wheel restarts after the current slot, construction starts are rescheduled
    /// from the queue, completions from each site's deadline, and trips in progress keep their
    /// slots, those already past at the next one. A clock advanced by frames, a load or a new
    /// world is already aligned, so this costs one comparison per frame.
    pub(super) fn align_events_to_clock(&mut self) {
        let next_slot = self.time.absolute_slot() + 1;
        self.allocator.now_slot = next_slot - 1;
        if self.events.next_slot() != next_slot {
            let trips = pending_trips(&self.events);
            self.events.reset(next_slot);
            self.reschedule_construction_events();
            self.allocator.pending_construction.clear();
            self.allocator
                .building_handles
                .register_appended(self.allocator.buildings.len());
            reschedule_construction_completions(
                &mut self.events,
                &self.allocator,
                &self.allocator.building_handles,
            );
            // An overdue freight event fires at the next slot; its shipment records that slot.
            for timed in &trips {
                if let SimEvent::Trip(
                    TripEvent::FreightDispatch { shipment } | TripEvent::FreightCheck { shipment },
                ) = timed.event
                    && timed.slot < next_slot
                    && self.logistics.trip_event_due(shipment, timed.slot)
                {
                    self.logistics.set_trip_event_slot(shipment, next_slot);
                }
            }
            reschedule_trips(&mut self.events, trips);
        }
    }

    /// Runs every slot after the clock's through absolute slot `last`, in order, as frames
    /// would without moving agents, and leaves the clock at `last` (tests). The timing wheel is
    /// aligned to the clock first.
    #[cfg(test)]
    pub(crate) fn run_slots_through_for_test(&mut self, last: u64) {
        self.align_events_to_clock();
        let mut report = super::frame::FrameStepReport::default();
        for slot in self.time.absolute_slot() + 1..=last {
            self.step_slot(slot, &mut report, &mut ());
        }
        self.time.set_absolute_slot_for_test(last);
    }

    /// Test driver of the census publication for a city a test built by hand: drops any census
    /// in progress and the input copy, then publishes, at the first slot of the hour holding
    /// `minute_of_day`, the census of a copy taken now through the publication's own
    /// whole-census path (`census publish`). The frontage prices stay as the test left them.
    #[cfg(test)]
    pub(crate) fn publish_census_now_for_test(&mut self, day_index: u32, minute_of_day: u16) {
        let slot = absolute_slot(day_index, minute_of_day - minute_of_day % 60, 0);
        self.households.census_run.abandon();
        self.households.census_inputs.discard_front();
        let key = self.households.census_key(&self.transit_network);
        let fallback = self.census_parameters(slot);
        self.households.publish_census(
            slot,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            fallback,
            key,
        );
    }

    /// Test driver of a planning slot's permits without its pass: a census of the live city is
    /// published ([`Self::publish_census_now_for_test`]), the workforce counted and every
    /// queued spawn's permit proved, then the epoch closes, as the hour's actions slot does
    /// after planning (`SIM-02-I`).
    #[cfg(test)]
    pub(crate) fn prove_construction_permits_for_test(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
    ) {
        self.publish_census_now_for_test(day_index, minute_of_day);
        self.demand.count_workforce_for_test(
            &self.allocator,
            &self.households,
            &self.fiscal_policy,
        );
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        self.prove_construction_permits(&Default::default(), day_index, minute_of_day);
        self.demand.close_construction_epoch(
            &mut self.households,
            &self.allocator,
            &self.region_graph,
        );
    }

    /// Test driver of a planning slot's immediate actions without its pass: the permits as
    /// [`Self::prove_construction_permits_for_test`] proves them, then `plan`'s level changes
    /// and despawns in the same epoch, as the hour's first slot runs them (`SIM-02-I`).
    /// Returns each queued spawn's proof in the epoch, read before the level changes run.
    #[cfg(test)]
    pub(crate) fn run_planning_slot_actions_for_test(
        &mut self,
        plan: &DemandBuildingActionPlan,
        day_index: u32,
        minute_of_day: u16,
    ) -> Vec<Option<crate::simulation::economy::demand::ProjectProof>> {
        self.publish_census_now_for_test(day_index, minute_of_day);
        self.demand.count_workforce_for_test(
            &self.allocator,
            &self.households,
            &self.fiscal_policy,
        );
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        self.prove_construction_permits(plan, day_index, minute_of_day);
        let proofs = self
            .pending_demand_spawns
            .iter()
            .map(|pending| {
                self.demand
                    .project_proof_for_test(pending.action.parcel_id, &self.zoning)
            })
            .collect();
        self.demand.execute_building_actions(
            plan,
            &mut self.allocator,
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.treasury.balance,
            &self.region_graph,
            &self.transit_network,
            &self.heightmap,
            absolute_operational_minute(day_index, minute_of_day),
            &self.fiscal_policy,
        );
        // The level changes closed the epoch before their despawns.
        self.demand.close_construction_epoch(
            &mut self.households,
            &self.allocator,
            &self.region_graph,
        );
        proofs
    }

    /// Test driver: the permit a planning slot now would prove for `action` queued after the
    /// current queue, which keeps the permits that slot proves for it.
    #[cfg(test)]
    pub(crate) fn permit_for_test(
        &mut self,
        action: &DemandSpawnAction,
        zone: ZoneType,
        day_index: u32,
        minute_of_day: u16,
    ) -> Option<crate::simulation::economy::demand::ConstructionPermit> {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let due_minute = self
            .pending_demand_spawns
            .back()
            .map_or(now + 1, |pending| pending.due_minute + 1);
        self.pending_demand_spawns
            .push_back(PendingDemandSpawnAction {
                id: u64::MAX,
                due_minute,
                zone_type: zone,
                action: action.clone(),
                planned_day_index: day_index,
                planned_minute_of_day: minute_of_day,
                permit: Default::default(),
                counted: false,
            });
        self.prove_construction_permits_for_test(day_index, minute_of_day);
        let permit = self
            .pending_demand_spawns
            .pop_back()
            .and_then(|pending| pending.permit.granted());
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        permit
    }

    /// Test driver of the queue: starts at most one due spawn at this minute, as its start event
    /// would; returns how many started (a waiting spawn did not).
    #[cfg(test)]
    pub(super) fn execute_pending_demand_spawns_for_minute(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
    ) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        // As the minute's start event would see the clock.
        self.allocator.now_slot = event_slot(now * SLOTS_PER_MINUTE);
        let mut executed_this_minute = 0_usize;
        while executed_this_minute < DEMAND_SPAWN_ACTIONS_PER_MINUTE {
            let Some(pending) = self.pending_demand_spawns.front() else {
                return executed_this_minute;
            };
            if pending.due_minute > now {
                return executed_this_minute;
            }
            let Some(pending) = self.pending_demand_spawns.pop_front() else {
                return executed_this_minute;
            };
            if !self.execute_pending_demand_spawn(pending, day_index, minute_of_day) {
                return executed_this_minute;
            }
            executed_this_minute += 1;
        }
        executed_this_minute
    }

    // Starts `pending` at its minute: places it, or rejects it without a permit for its current
    // door, profile and level; returns `false` when its voided permit makes it wait instead.
    fn execute_pending_demand_spawn(
        &mut self,
        pending: PendingDemandSpawnAction,
        day_index: u32,
        minute_of_day: u16,
    ) -> bool {
        let total_start = Instant::now();
        match self.demand.check_construction_permit(
            pending.zone_type,
            &pending.action,
            pending.permit,
            &self.allocator,
            &self.zoning,
            &self.region_graph,
            &self.transit_network,
        ) {
            Ok(()) => {}
            Err(StartRefusal::Wait) => {
                debug_log!(
                    "economy",
                    "queued demand spawn waits for the next planning slot: parcel={} zone={:?}",
                    pending.action.parcel_id,
                    pending.zone_type,
                );
                self.wait_for_next_planning_slot(
                    pending,
                    absolute_operational_minute(day_index, minute_of_day),
                );
                return false;
            }
            Err(StartRefusal::Denied(reason)) => {
                self.demand.finish_queued_spawn(pending.id, &pending.action);
                debug_log!(
                    "economy",
                    "queued demand spawn rejected before owner funding: parcel={} zone={:?} reason={}",
                    pending.action.parcel_id,
                    pending.zone_type,
                    reason
                );
                return true;
            }
        }
        let compile_start = Instant::now();
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
        let execute_start = Instant::now();
        let execution = self.allocator.execute_single_demand_spawn_action(
            pending.zone_type,
            &pending.action,
            Some(pending.id),
            &mut self.zoning,
            &self.region_graph,
            &self.transit_network.lane_system,
            &self.transit_network.road_surface,
            &self.heightmap,
            self.demand.runtime_catalog(),
            self.demand.runtime_tuning(),
        );
        let execute_ms = execute_start.elapsed().as_secs_f64() * 1000.0;
        self.publish_pending_building_site_changes();
        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        let use_execution = match pending.zone_type {
            ZoneType::Residential => execution.residential,
            ZoneType::Commercial => execution.commercial,
            ZoneType::Industrial => execution.industrial,
            _ => return true,
        };
        self.demand.finish_queued_spawn(pending.id, &pending.action);
        if use_execution.spawn_executed == 0 && pending.counted {
            self.void_granted_permits();
        }
        debug_log!(
            "economy",
            "queued demand spawn execution: planned_day={} planned_minute={} executed_day={} executed_minute={} zone={:?} attempted={} placed={} failed={} pending_remaining={} compile_ms={:.3} execute_ms={:.3} total_ms={:.3}",
            pending.planned_day_index,
            pending.planned_minute_of_day,
            day_index,
            minute_of_day,
            pending.zone_type,
            use_execution.spawn_attempted,
            use_execution.spawn_executed,
            use_execution.spawn_rejections.total(),
            self.pending_demand_spawns.len(),
            compile_ms,
            execute_ms,
            total_ms,
        );
        true
    }

    /// Slot `slot`'s chunk of the rolling environment sweep (`SIM-02-H`): a share of the
    /// emissions, or of the diffused rows, of pollution, noise and desirability.
    pub(super) fn advance_environment_sweep(&mut self, slot: u64) {
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.environment_sweep.advance(
            slot,
            &mut self.pollution,
            &mut self.noise,
            &mut self.desirability,
            &self.allocator,
            &self.region_graph,
            &self.config,
        );
    }

    /// Slot `slot`'s chunk of the funding census's input copy (`SIM-02-I`). The fill's last
    /// slot freezes the treasury, policies and the minute of the hour the census feeds with it.
    pub(super) fn advance_census_inputs(&mut self, slot: u64) {
        let parameters = self.census_parameters(next_hour(slot));
        self.households.advance_census_inputs(
            slot,
            &mut self.allocator,
            &self.logistics,
            parameters,
        );
    }

    // The live census parameters for a census feeding the hour that starts at slot `feeds`.
    fn census_parameters(&self, feeds: u64) -> FrozenCensusParameters {
        let (_, minute_of_day) = super::schedule::slot_calendar(feeds);
        FrozenCensusParameters {
            treasury: self.treasury.balance,
            fiscal: self.fiscal_policy,
            electricity_funding: self.service_policy.electricity_funding,
            minute_of_day,
        }
    }

    /// Window slot `slot`'s share of the hourly funding census (`SIM-02-I`): the frontage prices
    /// are published where the hour's census starts, before its first step, so it and the next
    /// hour's consumers price the same delays. The hour's first half returns at once.
    pub(super) fn advance_census(&mut self, slot: u64) {
        if slot % SLOTS_PER_HOUR < CENSUS_WINDOW_START {
            return;
        }
        // A stale zone index waits for its own reader: its rebuild resets the vacancy order
        // admission reads, which the preparation must not move (`SIM-02-K`).
        // The preparation runs in parts, each in a slot of its own (`SIM-02-K`): half a minute
        // before minute 59 the book's pass rows and supplier rows, at minute 59's first slot
        // their layout, and from the second half's slot everything else, so no two heavy parts
        // share a 1× frame and the halves never share a 32× frame; a quarter minute later the
        // business finance masks a second half in its own slot left.
        let slot_of_hour = slot % SLOTS_PER_HOUR;
        if !self.allocator.zone_index_needs_rebuild() {
            let part = match slot_of_hour {
                PLANNING_PREPARATION_ROWS_SLOT => Some(PlanningPreparation::SupplierRows),
                PLANNING_PREPARATION_SLOT => Some(PlanningPreparation::Suppliers),
                rest if rest >= PLANNING_PREPARATION_REST_SLOT => {
                    Some(PlanningPreparation::Whole {
                        masks: rest != PLANNING_PREPARATION_REST_SLOT,
                    })
                }
                _ => None,
            };
            if let Some(part) = part {
                self.prepare_census_book_for_planning(slot, part);
            }
        }
        if slot_of_hour == PLANNING_PREPARATION_MASKS_SLOT
            && let Some(funding) = self.households.finished_census_book()
        {
            // Masks only prune unclaimable buyers; O(changed buyers' tranches and paths).
            funding.business.refresh_finance_masks();
        }
        if self
            .households
            .census_starts_at(slot, self.households.census_key(&self.transit_network))
        {
            self.transit_network.publish_frontage_prices();
        }
        let key = self.households.census_key(&self.transit_network);
        let fallback = self.census_parameters(next_hour(slot));
        self.households.advance_census(
            slot,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            fallback,
            key,
        );
    }

    /// Publishes the hour's census at its first slot `slot`, before the operational hour
    /// (`SIM-02-I`); a census the window did not finish for the live city runs whole first,
    /// after publishing the frontage prices as a census start does.
    pub(super) fn publish_census(&mut self, slot: u64) {
        if !self
            .households
            .census_ready(slot, self.households.census_key(&self.transit_network))
        {
            self.transit_network.publish_frontage_prices();
        }
        let key = self.households.census_key(&self.transit_network);
        // The census feeds this hour.
        let fallback = self.census_parameters(slot);
        self.households.publish_census(
            slot,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            fallback,
            key,
        );
    }

    /// Prepares at load what the first planning slot would prepare on the live books: the
    /// building site index, then the planning rows and storage (`SIM-02-K`). A census the load
    /// finished (one current at the save) is prepared now too, not in a window slot; census
    /// storage that holds no census takes a copy of the prepared live books, as an
    /// uninterrupted game's census storage holds the previous published census, so the first
    /// census after loading finishes on warm planning storage and site rows; the live books
    /// are then lent once more, last, so the first planning slot keeps their pass rows.
    pub(crate) fn warm_loaded_planning(&mut self) {
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        let key = self.households.census_key(&self.transit_network);
        self.demand.prepare_planning_books(
            &mut self.households.job_funding,
            key,
            &self.allocator,
            &self.region_graph,
            &self.zoning,
            &self.fiscal_policy,
            &self.transit_network,
            &mut self.logistics,
            self.time.minute_of_day,
            PlanningPreparation::Whole { masks: true },
        );
        self.prepare_census_book_for_planning(
            self.time.absolute_slot(),
            PlanningPreparation::Whole { masks: true },
        );
        // The copy is exact to its rows; preparing it again gives its storage the spare rows
        // the live books took (`utils/storage.rs`).
        if let Some(funding) = self.households.warm_census_storage() {
            self.demand.prepare_planning_books(
                funding,
                key,
                &self.allocator,
                &self.region_graph,
                &self.zoning,
                &self.fiscal_policy,
                &self.transit_network,
                &mut self.logistics,
                self.time.minute_of_day,
                PlanningPreparation::Whole { masks: true },
            );
            // Without a finished census the next planning slot lends the live books again: they
            // are lent last, so its pass keeps the rows built now (`SIM-02-K`).
            self.demand.prepare_planning_books(
                &mut self.households.job_funding,
                key,
                &self.allocator,
                &self.region_graph,
                &self.zoning,
                &self.fiscal_policy,
                &self.transit_network,
                &mut self.logistics,
                self.time.minute_of_day,
                PlanningPreparation::Whole { masks: true },
            );
        }
    }

    // A finished census whose book no planning pass has prepared since the census started is
    // prepared for the hour it serves, in parts from half a minute before minute 59
    // (`PLANNING_PREPARATION_ROWS_SLOT`) in the window while the zone index is current, and
    // whole at load, not by the planning slot that first lends it (`SIM-02-K`): the census
    // refilled a book whose site rows were laid out two hours before. A part short of the
    // whole leaves the book unprepared, so a later window slot prepares the rest on current
    // rows. O(1) otherwise.
    fn prepare_census_book_for_planning(&mut self, slot: u64, part: PlanningPreparation) {
        let key = self.households.census_key(&self.transit_network);
        let Some(funding) = self.households.unprepared_census_book() else {
            return;
        };
        let timing = crate::debug::Channel::EconomyPhases
            .enabled()
            .then(Instant::now);
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        let (_, minute_of_day) = super::schedule::slot_calendar(next_hour(slot));
        self.demand.prepare_planning_books(
            funding,
            key,
            &self.allocator,
            &self.region_graph,
            &self.zoning,
            &self.fiscal_policy,
            &self.transit_network,
            &mut self.logistics,
            minute_of_day,
            part,
        );
        if let Some(start) = timing {
            debug_log!(
                "economy_phases",
                "census_book_preparation_detail slot_of_hour={} ms={:.3}",
                slot % SLOTS_PER_HOUR,
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    /// Slot `slot`'s chunk of the hour-long demand snapshot sweep (`SIM-02-I`).
    pub(super) fn advance_demand_snapshot(&mut self, slot: u64) {
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.households
            .handles
            .register_appended(self.households.households.len());
        self.demand.advance_snapshot_sweep(
            slot,
            &self.allocator,
            &self.households,
            &self.fiscal_policy,
            self.treasury.balance,
        );
    }

    /// The calendar close at the start of a day, before any daily phase of the new day
    /// (`SIM-02-G`): every entity has settled once since the last close, and the city's day
    /// totals become the day's budget report. Road upkeep reads the rolling road length. O(1)
    /// apart from debug reports.
    pub(super) fn close_calendar_day(
        &mut self,
        day_index: u32,
        observer: &mut impl EconomyObserver,
    ) {
        let close_start = Instant::now();
        // Job locks count calendar days (`SIM-02-D`).
        self.agents.settlement_days += 1;
        self.households.close_calendar_day();
        self.allocator.close_city_input_day();
        let road_length_m = self.road_length.length_m(&self.region_graph);
        self.treasury
            .settle_daily_upkeep(road_length_m * ROAD_UPKEEP_PER_METER_PER_DAY);
        let removed_households = self.demand.households_to_remove_today;
        self.demand
            .log_daily_household_action_diagnostics(day_index);
        self.demand.close_exit_day();
        self.treasury.finalize_daily_tax_window();
        self.record_daily_budget_ledger(day_index);
        self.log_daily_city_flow_diagnostics(day_index, removed_households);
        observer.day_closed(self);
        self.debug_household_admissions_since_daily = 0;
        self.print_daily_building_economy(day_index);
        self.agents
            .pathfind_count
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.last_tick_duration = close_start.elapsed().as_secs_f64() * 1000.0;
    }

    // Planning slot `slot`'s setup at absolute slot `setup_at`, its minute's first slot
    // (`SIM-02-K`): the site index, road surface and construction queue prepared as its plan
    // prepares them, then the live workforce counted and the zone's lots and existing buildings
    // scanned for the plan `PLANNING_SETUP_LEAD` slots later.
    pub(super) fn execute_demand_planning_setup(
        &mut self,
        slot: PlanningSlot,
        setup_at: u64,
        day_index: u32,
        minute_of_day: u16,
    ) {
        let timing = crate::debug::Channel::EconomyPhases
            .enabled()
            .then(Instant::now);
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        self.demand.prepare_planning_setup(
            slot,
            setup_at,
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            &self.fiscal_policy,
            &self.transit_network,
            crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                road_surface: &self.transit_network.road_surface,
                terrain: &self.heightmap,
            },
            minute_of_day,
        );
        if let Some(start) = timing {
            debug_log!(
                "economy_phases",
                "demand_setup_detail day={} minute={} slot={:?} total_ms={:.3}",
                day_index,
                minute_of_day,
                slot,
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
    }

    // The hour's demand planning slot `slot` (`SIM-02-I`), on the cell lots the `cell lots`
    // queue has prepared so far (`SIM-02-H`): a zone slot plans its zone into the hour's plan;
    // the actions slot admits the hour's arrivals, queues the hour's spawns and executes the
    // rest of the plan. Returns whether buildings may have changed.
    pub(super) fn execute_demand_planning_slot(
        &mut self,
        slot: PlanningSlot,
        day_index: u32,
        minute_of_day: u16,
    ) -> bool {
        let timing = crate::debug::Channel::EconomyPhases
            .enabled()
            .then(Instant::now);
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        self.demand
            .set_admission_candidate_hour(day_index, minute_of_day);
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        let prepare_ms = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        let planned = self.demand.run_planning_slot(
            slot,
            &self.allocator,
            &mut self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            &self.fiscal_policy,
            &self.transit_network,
            &mut self.logistics,
            crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                road_surface: &self.transit_network.road_surface,
                terrain: &self.heightmap,
            },
            minute_of_day,
        );
        let plan_ms =
            timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0) - prepare_ms;
        let Some(zone_type) = slot.zone_type() else {
            self.execute_demand_actions_slot(planned, day_index, minute_of_day, timing, plan_ms);
            return true;
        };
        // A slot skipped on books not valid for the live city plans nothing for its zone this
        // hour; the earlier slots' plans stand. Slot 3 queues the hour's spawns.
        if let Some(start) = timing {
            debug_log!(
                "economy_phases",
                "demand_slot_detail day={} minute={} slot={:?} planned={} prepare_ms={:.3} plan_ms={:.3} total_ms={:.3}",
                day_index,
                minute_of_day,
                slot,
                planned,
                prepare_ms,
                plan_ms,
                start.elapsed().as_secs_f64() * 1000.0
            );
        }
        let plan = zone_plan(&mut self.demand.building_actions, zone_type);
        debug_log!(
            "economy",
            "demand planning slot: day={} minute={} zone={:?} planned={} spawns={} upgrades={} despawns={}",
            day_index,
            minute_of_day,
            zone_type,
            planned,
            plan.spawns.len(),
            plan.upgrades.len(),
            plan.despawns.len(),
        );
        false
    }

    // The hour's actions slot (`SIM-02-I`): the hour's admission between its plans and its
    // actions, as the single pass ran it, then the hour's spawns queued in the single pass's
    // order (homes, the optional homes the workforce claimed, shops, industry), the permits of
    // the whole queue and the hour's level changes in one construction epoch, and the despawns
    // last, after it closes. Admissions, level changes and despawns are queued for their
    // request queues (`SIM-02-J`). Without the slot's planning or epoch (books not valid for the
    // live city) no business level change is queued and the queue's business permits wait an
    // hour; residential level changes and despawns are still queued.
    fn execute_demand_actions_slot(
        &mut self,
        planned: bool,
        day_index: u32,
        minute_of_day: u16,
        timing: Option<Instant>,
        plan_ms: f64,
    ) {
        let actions_start = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        let hour_minute = minute_of_day - minute_of_day % 60;
        let actions_slot = absolute_slot(day_index, hour_minute, PlanningSlot::Actions as u64);
        // The hour's admissions are decided here and created over the hour by the
        // `admissions` queue, each carrier departing paced from its creation (`SIM-02-J`).
        let launched_households = self.decide_hour_admissions();
        self.debug_household_admissions_since_daily = self
            .debug_household_admissions_since_daily
            .saturating_add(launched_households);
        self.demand
            .record_household_admission_execution(launched_households);
        let admission_end = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        let queued_spawn_count = self.enqueue_hourly_demand_spawns(day_index, minute_of_day);
        let mut immediate_building_plan = demand_plan_without_spawns(&self.demand.building_actions);
        self.refresh_demand_construction_queue(day_index, minute_of_day);
        let proved =
            self.prove_construction_permits(&immediate_building_plan, day_index, minute_of_day);
        if !planned || !proved {
            // No business level change is proved; the next hour plans again.
            for plan in [
                &mut immediate_building_plan.commercial,
                &mut immediate_building_plan.industrial,
            ] {
                plan.upgrades.clear();
                plan.downgrades.clear();
            }
        }
        let queue_end = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        // The level changes are proved in the epoch and applied by the `level changes` queue
        // from the removal window's start; the clearances wait for the `building removals`
        // queue (`SIM-02-J`).
        if demand_plan_has_non_spawn_actions(&immediate_building_plan) {
            self.demand.queue_level_changes(
                &immediate_building_plan,
                actions_slot,
                &mut self.allocator,
                &self.zoning,
                &mut self.households,
                &mut self.logistics,
                self.treasury.balance,
                &self.region_graph,
                &self.transit_network,
                &self.heightmap,
                absolute_operational_minute(day_index, minute_of_day),
                &self.fiscal_policy,
            );
            self.queue_hour_clearances(&immediate_building_plan, actions_slot);
        }
        // The level changes closed the epoch; without any it closes here.
        self.demand.close_construction_epoch(
            &mut self.households,
            &self.allocator,
            &self.region_graph,
        );
        let actions_end = timing.map_or(0.0, |start| start.elapsed().as_secs_f64() * 1000.0);
        self.publish_pending_building_site_changes();
        if let Some(start) = timing {
            let total_ms = start.elapsed().as_secs_f64() * 1000.0;
            debug_log!(
                "economy_phases",
                "demand_phase_detail day={} minute={} plan_ms={:.3} execute_ms={:.3} total_ms={:.3} admission_ms={:.3} queue_ms={:.3} actions_ms={:.3} publish_ms={:.3}",
                day_index,
                minute_of_day,
                plan_ms,
                total_ms - actions_start,
                total_ms,
                admission_end - actions_start,
                queue_end - admission_end,
                actions_end - queue_end,
                total_ms - actions_end
            );
        }
        self.demand
            .log_hourly_household_action_diagnostics(day_index, minute_of_day);
        self.demand
            .log_hourly_building_action_diagnostics(day_index, minute_of_day);
        let plan = &self.demand.building_actions;
        debug_log!(
            "economy",
            "hourly demand actions: day={} minute={} admit={} planned_spawns=({}/{}/{}) queued_spawns={} pending_spawns={} upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{})",
            day_index,
            minute_of_day,
            self.demand.households_to_admit_today,
            plan.residential.spawns.len(),
            plan.commercial.spawns.len(),
            plan.industrial.spawns.len(),
            queued_spawn_count,
            self.pending_demand_spawns.len(),
            plan.residential.upgrades.len(),
            plan.commercial.upgrades.len(),
            plan.industrial.upgrades.len(),
            plan.residential.downgrades.len(),
            plan.commercial.downgrades.len(),
            plan.industrial.downgrades.len(),
            plan.residential.despawns.len(),
            plan.commercial.despawns.len(),
            plan.industrial.despawns.len(),
        );
        // The hour's plan is done; the next hour's first slot starts a new one.
        self.demand.clear_hour_plan();
    }

    /// Publishes allocator-owned site changes through one terrain invalidation path for every mode.
    pub(crate) fn publish_pending_building_site_changes(&mut self) {
        self.publish_pending_site_changes_but_terrain();
        let boxes = self.allocator.take_pending_site_dirty_bounds();
        if !boxes.is_empty() {
            self.mark_building_site_terrain_only_boxes(&boxes);
        }
        self.allocator.return_site_dirty_bounds_storage(boxes);
    }

    /// [`Self::publish_pending_building_site_changes`] for the building removal queue
    /// (`SIM-02-K`): the production sites, yards and cell lots now, as there, and each site
    /// box's terrain render-patch ownership left to [`Self::publish_deferred_site_terrain_box`]
    /// in later slots. The ownership is render state that no simulation system reads, and each
    /// refresh recomputes its patches from the live roads and sites, so a later refresh gives
    /// what the next frame's terrain needs. O(changes) now.
    pub(crate) fn publish_pending_building_site_changes_deferring_terrain(&mut self) {
        self.publish_pending_site_changes_but_terrain();
        let mut boxes = self.allocator.take_pending_site_dirty_bounds();
        self.deferred_site_terrain_boxes.extend(boxes.drain(..));
        self.allocator.return_site_dirty_bounds_storage(boxes);
    }

    /// Refreshes the terrain render-patch ownership of the oldest site box the building removal
    /// queue deferred (`SIM-02-K`), unless the zone index or the site clients are stale: their
    /// repair belongs to their own readers, since a zone-index rebuild resets the vacancy order
    /// admission reads. O(1) with none, else one box's patches.
    pub(crate) fn publish_deferred_site_terrain_box(&mut self) {
        if self.deferred_site_terrain_boxes.is_empty()
            || self.allocator.zone_index_needs_rebuild()
            || self.allocator.building_sites.len() != self.allocator.buildings.len()
        {
            return;
        }
        if let Some(bounds) = self.deferred_site_terrain_boxes.pop_front() {
            self.mark_building_site_terrain_only_boxes(&[bounds]);
        }
    }

    // The production sites, yards and cell-lot dirty marks of the pending site changes.
    fn publish_pending_site_changes_but_terrain(&mut self) {
        self.publish_pending_production_site_removals();
        crate::nodes::simulation_node::vegetation_api::publish_yards(self);
        while let Some(bounds) = self.allocator.pop_pending_cell_site_dirty_bounds() {
            self.zoning
                .mark_cell_lots_dirty(crate::simulation::zoning::cells::CellBounds {
                    min: glam::DVec2::new(f64::from(bounds.0), f64::from(bounds.1)),
                    max: glam::DVec2::new(f64::from(bounds.2), f64::from(bounds.3)),
                });
        }
    }

    /// Keeps saved production polygons aligned with allocator swap removals before further use.
    pub(crate) fn publish_pending_production_site_removals(&mut self) {
        for (removed, last) in self.allocator.pending_production_site_removals.drain(..) {
            // The allocator has already dropped clearance. Release cached cells over the
            // entire removed field before its saved polygon is removed or swap-remapped.
            if let Some(site) = self.agriculture.site_for_building(removed) {
                self.zoning.mark_cell_lots_dirty(
                    crate::simulation::zoning::cells::CellBounds::from_points(
                        site.polygon_world
                            .iter()
                            .map(|p| glam::DVec2::new(f64::from(p.x), f64::from(p.y))),
                    ),
                );
            }
            self.agriculture
                .remove_building_after_swap_remove(removed, last);
            self.resource_extraction
                .remove_building_after_swap_remove(removed, last);
        }
    }

    pub(crate) fn mark_building_site_terrain_dirty_bounds(&mut self, bounds: (f32, f32, f32, f32)) {
        self.zoning
            .mark_cell_lots_dirty(crate::simulation::zoning::cells::CellBounds {
                min: glam::DVec2::new(f64::from(bounds.0), f64::from(bounds.1)),
                max: glam::DVec2::new(f64::from(bounds.2), f64::from(bounds.3)),
            });
        self.mark_building_site_terrain_only_boxes(&[bounds]);
    }

    // Marks each site change's terrain and refreshes the ownership of the union of the render
    // patches the boxes touch, each once (`SIM-02-K`). A patch's ownership is a function of the
    // roads, sites and heightmap, so a patch no box touches keeps its state; one box merged over
    // changes far apart would refresh and re-upload every patch between them.
    // O(boxes × patches a box touches × log).
    fn mark_building_site_terrain_only_boxes(&mut self, boxes: &[(f32, f32, f32, f32)]) {
        let margin_m =
            terrain_cdt_local_sample_margin_m(&self.heightmap, ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        let mut dirty_patch_keys = Vec::new();
        for &bounds in boxes {
            self.allocator.mark_building_site_terrain_bounds_dirty(
                &mut self.heightmap,
                bounds,
                margin_m,
            );
            let (min_x, min_z, max_x, max_z) = bounds;
            self.heightmap.append_render_patch_keys_for_world_bounds(
                min_x - margin_m,
                min_z - margin_m,
                max_x + margin_m,
                max_z + margin_m,
                &mut dirty_patch_keys,
            );
        }
        dirty_patch_keys.sort_unstable();
        dirty_patch_keys.dedup();
        self.refresh_engineered_terrain_patch_ownership_for_keys(
            ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            &dirty_patch_keys,
        );
        self.bump_terrain_payload_patch_generations(&dirty_patch_keys);
        self.terrain_dirty = true;
    }
}

// The first slot of the hour after `slot`'s.
fn next_hour(slot: u64) -> u64 {
    (slot / SLOTS_PER_HOUR + 1) * SLOTS_PER_HOUR
}
