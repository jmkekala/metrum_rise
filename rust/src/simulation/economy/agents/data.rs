//! Core data layout and lifecycle management for the agent simulation.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout provided by the `soa_derive` crate.
//! This enables cache-friendly bulk iteration and ensures all fields are kept in sync.

use super::{
    MODE_CAR, MODE_WALK, TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING,
};
use crate::config::DEFAULT_URBAN_ROAD_SPEED_MS;
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;

use rand::Rng;
use soa_derive::StructOfArray;
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Single agent data structure used for SoA generation.
#[derive(StructOfArray)]
#[soa_derive(Debug, Clone)]
pub struct Agent {
    /// Index into `BuildingAllocator::buildings` for the agent's home. `usize::MAX` = homeless (immigrating).
    pub home_building: usize,
    /// Index into `HouseholdSystem::households` for the agent's shared household record.
    pub household_id: usize,
    /// Pending resident count carried by a border-origin household arrival car; `0` for normal
    /// agents.
    pub pending_household_size: u16,
    /// Index into `BuildingAllocator::buildings` for the agent's workplace. `usize::MAX` = unemployed.
    pub work_building: usize,

    /// World-space X position (metres).
    pub pos_x: f32,
    /// World-space Z position (metres, Godot forward axis).
    pub pos_y: f32,
    /// Runtime-stable identifier used by render interpolation; not persisted in saves.
    pub render_id: u64,

    /// Current activity: `0` = Home, `1` = Work, `2` = other non-home stop.
    pub activity: u8,
    /// Current transit phase. One of the `TRANSIT_*` constants defined in this module.
    pub transit: u8,
    /// Agent wellbeing in `[0, 100]`.
    pub happiness: f32,
    /// Per-agent view of household money. Synced from the shared household budget.
    pub money: f32,
    /// Internal clock for journey duration calculation.
    pub journey_start_time: f32,
    /// Stable authored schedule seed used for repeatable departure offsets.
    pub schedule_seed: u32,
    /// Cached one-way home-to-work estimate in authored minutes.
    pub cached_commute_minutes: u16,
    /// Earliest operational time when the cached commute estimate may be refreshed.
    pub next_commute_refresh_time: f32,
    /// Operational day for the next cached work/home departure; `u32::MAX` means not cached.
    pub next_departure_day: u32,
    /// Minute-of-day for the next cached work/home departure.
    pub next_departure_minute: u16,
    /// Building the cached departure starts from; `usize::MAX` means not cached.
    pub next_departure_origin_building: usize,
    /// Building the cached departure targets; `usize::MAX` means no cached departure.
    pub next_departure_target_building: usize,
    /// Activity to apply when the cached departure starts.
    pub next_departure_activity: u8,
    /// Work building whose runtime work-profile lookup is cached.
    pub cached_schedule_work_building: usize,
    /// Index into `OperationalClockRuntimeTuning::work_profiles`; `u16::MAX` means no profile.
    pub cached_work_profile_index: u16,

    /// Building the agent is currently inside. `usize::MAX` = on the road.
    pub current_building: usize,
    /// Building the agent is travelling toward. `usize::MAX` = no active destination.
    pub target_building: usize,
    /// Economy-selected destination building for the next idle departure.
    pub planned_target_building: usize,
    /// Graph node the agent is currently at or most recently passed through.
    pub current_node: u32,
    /// Planned road-graph endpoint that the origin frontage leg routes toward.
    pub planned_attach_node: u32,
    /// Planned road-graph endpoint from which the destination frontage leg begins.
    pub planned_detach_node: u32,
    /// Exact lane ID to enter after the short egress leg. `u32::MAX` if there is no current plan.
    pub planned_attach_lane_id: u32,
    /// Exact lane ID used for the final frontage approach and network exit. `u32::MAX` if invalid.
    pub planned_detach_lane_id: u32,
    /// Exact lane distance where `ACCESS_EGRESS` transitions into the live network.
    pub planned_attach_lane_d: f32,
    /// Exact lane distance where the live network transitions into `ACCESS_INGRESS`.
    pub planned_detach_lane_d: f32,
    /// Compact authoritative trip-plan metadata. See the `ACCESS_*` constants in `mod.rs`.
    pub access_flags: u8,
    /// Earliest simulation time at which trip planning or replanning may be attempted again.
    pub next_replan_time: f32,

    /// Index into `RegionGraph::edges` for the edge the agent is currently traversing.
    pub current_edge: usize,
    /// Currently traversed lane ID in `LaneSystem`. `usize::MAX` if off-network.
    pub current_lane_id: usize,
    /// Distance (in metres) travelled along the `current_lane_id`.
    pub lane_distance: f32,
    /// Source lane for an active lane-change maneuver. `u32::MAX` means no active lane change.
    pub lane_change_from_lane_id: u32,
    /// Lane distance where the active lane-change S-curve started.
    pub lane_change_start_d: f32,
    /// Longitudinal distance over which the active lane-change S-curve completes.
    pub lane_change_length_m: f32,
    /// Time spent held below free-flow speed by traffic in the current lane.
    pub overtake_blocked_time_s: f32,
    /// Remaining cooldown before another discretionary overtaking/return lane change may start.
    pub overtake_cooldown_s: f32,
    /// Current movement speed in m/s. Updated each tick by the IDM model (cars) or held constant
    /// (pedestrians). Initialised to the edge speed limit on first lane entry.
    pub speed: f32,
    /// Current transit mode. One of the `MODE_*` constants.
    pub transit_mode: u8,
    /// Economy-selected next activity to execute on the next idle departure.
    pub planned_activity: u8,

    /// Sequence of node IDs forming the planned route.
    pub current_path: Vec<u32>,
    /// Index into the active route buffer: `current_path`.
    pub current_path_index: usize,

    /// `true` if the agent owns a car and drove to their current location.
    pub has_car: bool,
    /// Remaining days the agent is locked to their current job after a voluntary switch.
    /// Zero means the agent may freely seek a better position.
    pub job_lock_days: u8,
    /// Consecutive days the agent has gone without receiving wages from their employer.
    /// Resets to zero when wages are paid. Rises above `JOB_UNPAID_ABANDON_DAYS` to
    /// allow breaking the lock early at a failing employer.
    pub consecutive_unpaid_days: u8,
    /// Type of vehicle the agent uses (if driving). One of the `VEHICLE_*` constants.
    pub vehicle_type: u8,

    /// Variant for pedestrian models (0-3: Male A, Male B, Female A, Female B).
    pub pedestrian_type: u8,
    /// Current cycle of the walk animation [0, 1].
    pub walk_phase: f32,
}

/// Simulation-wide agent system.
///
/// Refactored to use `soa_derive` for robust parallel array management.
pub struct AgentSystem {
    /// The backing SoA storage.
    pub agents: AgentVec,
    /// Global simulation time for this system.
    pub sim_time: f32,
    /// Running count of pathfinding calls this session, used for benchmark logging.
    pub pathfind_count: AtomicU32,
    /// Retained per-lane traffic occupancy snapshot for IDM gap lookup and lane entry checks.
    /// Indexed by lane ID; each entry is `(lane_distance, agent_idx)` sorted ascending by dist.
    ///
    /// Contains physical current-lane occupancy plus source-lane ghost occupancy for active
    /// lane changes. Overlap correction is run before source-lane ghosts are inserted.
    pub lane_buckets: Vec<Vec<(f32, usize)>>,
    /// Dedup flag array — `lane_is_dirty[lid] == true` if lane `lid` has been pushed to this
    /// tick. Sized alongside `lane_buckets`. Cleared via `dirty_lanes` rather than a full scan.
    pub lane_is_dirty: Vec<bool>,
    /// Compact list of lane IDs that were touched this tick (no duplicates).
    /// Used to clear only the dirty buckets at the start of the next tick.
    pub dirty_lanes: Vec<usize>,
    /// Live lane-agent count represented by the retained occupancy snapshot.
    pub lane_bucket_live_agent_count: usize,
    /// Lane count represented by the retained occupancy snapshot.
    pub lane_bucket_snapshot_lane_count: usize,
    /// Agent count represented by the retained occupancy snapshot.
    pub lane_bucket_snapshot_agent_count: usize,
    /// True once `lane_buckets` matches the current authoritative lane state.
    pub lane_bucket_snapshot_valid: bool,
    /// Scratch list of agents whose active lane change also occupies their source lane.
    pub lane_change_ghost_agents: Vec<usize>,
    /// Scratch buffer: IDM double-buffer for next-tick speeds. Avoids read-write conflicts in
    /// the parallel IDM pass.
    pub new_speed: Vec<f32>,
    /// Scratch buffer: one-tick local-access handoff claims for car lane attach/detach.
    /// Prevents multiple cars from claiming the same exact frontage handoff on the same lane
    /// in a single tick when households or workplaces release or receive several agents at once.
    pub lane_attach_claimed: Vec<AtomicBool>,
    /// Scratch buffer: per-edge speed sum for congestion calculation, indexed by edge ID.
    pub edge_speed_sum: Vec<f32>,
    /// Scratch buffer: per-edge agent count for congestion calculation, indexed by edge ID.
    pub edge_agent_cnt: Vec<u32>,
    /// Dedup flag for edges touched by live traffic in the latest congestion pass.
    pub edge_is_dirty: Vec<bool>,
    /// Compact list of edge IDs touched by live traffic in the latest congestion pass.
    pub dirty_edges: Vec<usize>,
    /// Scratch list of previously dirty edges used to reset congestion when traffic leaves.
    pub stale_dirty_edges: Vec<usize>,
    /// Scratch buffer: per-lane speed sum for the low-frequency frontage delay cache.
    pub lane_speed_sum: Vec<f32>,
    /// Scratch buffer: per-lane vehicle count for the low-frequency frontage delay cache.
    pub lane_vehicle_cnt: Vec<u32>,
    /// Monotonic source for transient render IDs assigned to newly spawned agents.
    next_render_id: u64,
    /// Last allocator building-reference revision that was scrubbed against this agent store.
    pub(crate) last_building_ref_scrub_revision: u64,
}

impl Deref for AgentSystem {
    type Target = AgentVec;
    fn deref(&self) -> &Self::Target {
        &self.agents
    }
}

impl DerefMut for AgentSystem {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.agents
    }
}

impl AgentSystem {
    /// Creates a new, empty agent system.
    pub fn new() -> Self {
        Self {
            agents: AgentVec::new(),
            sim_time: 0.0,
            pathfind_count: AtomicU32::new(0),
            lane_buckets: Vec::new(),
            lane_is_dirty: Vec::new(),
            dirty_lanes: Vec::new(),
            lane_bucket_live_agent_count: 0,
            lane_bucket_snapshot_lane_count: 0,
            lane_bucket_snapshot_agent_count: 0,
            lane_bucket_snapshot_valid: false,
            lane_change_ghost_agents: Vec::new(),
            new_speed: Vec::new(),
            lane_attach_claimed: Vec::new(),
            edge_speed_sum: Vec::new(),
            edge_agent_cnt: Vec::new(),
            edge_is_dirty: Vec::new(),
            dirty_edges: Vec::new(),
            stale_dirty_edges: Vec::new(),
            lane_speed_sum: Vec::new(),
            lane_vehicle_cnt: Vec::new(),
            next_render_id: 0,
            last_building_ref_scrub_revision: u64::MAX,
        }
    }

    /// Allocates a runtime-stable render ID for one newly inserted agent.
    pub(crate) fn allocate_render_id(&mut self) -> u64 {
        let render_id = self.next_render_id;
        self.next_render_id = self.next_render_id.saturating_add(1);
        render_id
    }

    /// Invalidates the retained lane occupancy snapshot after external lane-state mutation.
    pub(crate) fn invalidate_lane_bucket_snapshot(&mut self) {
        self.lane_bucket_snapshot_valid = false;
    }

    /// Spawns one agent already housed inside a building.
    pub fn spawn_housed_agent(&mut self, home: usize, init_x: f32, init_y: f32) -> usize {
        let mut rng = rand::thread_rng();
        let schedule_seed = stable_schedule_seed(home, self.agents.len() as u32);
        let render_id = self.allocate_render_id();
        let agent = Agent {
            home_building: home,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: self.sim_time,
            schedule_seed,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: home,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: u32::MAX,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_WALK,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: rng.gen_range(0..4) as u8,
            pedestrian_type: rng.gen_range(0..4) as u8,
            walk_phase: rng.gen_range(0.0..1.0),
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        };

        self.agents.push(agent);
        self.invalidate_lane_bucket_snapshot();
        self.agents.len() - 1
    }

    /// Spawns a single border-origin arrival agent for explicit immigration-visualization paths.
    pub fn spawn_border_arrival_agent(
        &mut self,
        home: usize,
        _home_node: u32,
        _target_x: f32,
        _target_y: f32,
        highway_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let mut rng = rand::thread_rng();
        let schedule_seed = stable_schedule_seed(home, self.agents.len() as u32);
        let render_id = self.allocate_render_id();
        let agent = Agent {
            home_building: home,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: init_x,
            pos_y: init_y,
            render_id,
            activity: 0, // Heading Home
            transit: TRANSIT_IMMIGRATING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: self.sim_time,
            schedule_seed,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: home,
            planned_target_building: usize::MAX,
            current_node: highway_node,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: DEFAULT_URBAN_ROAD_SPEED_MS,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: Vec::new(),
            current_path_index: 0,
            has_car: true,
            vehicle_type: rng.gen_range(0..4) as u8,
            pedestrian_type: rng.gen_range(0..4) as u8,
            walk_phase: rng.gen_range(0.0..1.0),
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        };

        self.agents.push(agent);
        self.invalidate_lane_bucket_snapshot();
        self.agents.len() - 1
    }

    /// Spawns one visible border-origin car that represents a whole pending household.
    pub fn spawn_household_arrival_carrier(
        &mut self,
        home: usize,
        household_size: u16,
        border_node: u32,
        init_x: f32,
        init_y: f32,
    ) -> usize {
        let agent_idx = self.spawn_border_arrival_agent(
            home,
            u32::MAX,
            init_x,
            init_y,
            border_node,
            init_x,
            init_y,
        );
        self.agents.pending_household_size[agent_idx] = household_size.max(1);
        agent_idx
    }

    /// Efficiency helper to spawn a large number of agents at once for testing or benchmarks.
    pub fn spawn_random_agents(
        &mut self,
        count: usize,
        graph: &RegionGraph,
        allocator: &BuildingAllocator,
    ) {
        let mut rng = rand::thread_rng();
        let node_count = graph.node_count();
        let bldg_count = allocator.buildings.len();
        if node_count == 0 || bldg_count == 0 {
            return;
        }

        for _ in 0..count {
            let home_idx = rng.gen_range(0..bldg_count);
            let start_node = rng.gen_range(0..graph.node_count()) as u32;
            let start_pos = graph.node(start_node).pos;

            self.spawn_housed_agent(home_idx, start_pos.x, start_pos.z);
        }
    }

    /// Clears all agents from the system.
    pub fn clear(&mut self) {
        self.agents.clear();
        self.sim_time = 0.0;
        self.pathfind_count.store(0, Ordering::Relaxed);
        self.next_render_id = 0;
        self.invalidate_lane_bucket_snapshot();
    }

    /// Remaps the edge indices stored in all agents from [Old ID] to [New ID].
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for i in 0..self.agents.len() {
            if self.agents.current_edge[i] != usize::MAX {
                if let Some(&new_id) = mapping.get(&self.agents.current_edge[i]) {
                    self.agents.current_edge[i] = new_id;
                    self.agents.current_path[i].clear();
                } else {
                    self.agents.current_edge[i] = usize::MAX;
                    self.agents.current_path[i].clear();
                }
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Permanently removes an agent from the simulation using O(1) swap-remove.
    pub fn kill_agent(&mut self, index: usize, allocator: &mut BuildingAllocator) {
        if index >= self.agents.len() {
            return;
        }

        // Vacancy for residential is now household-based and managed by the HouseholdSystem.
        // Worker count for non-residential remains agent-based.

        let work = self.agents.work_building[index];
        if work != usize::MAX && work < allocator.buildings.len() {
            allocator.buildings[work].worker_count =
                allocator.buildings[work].worker_count.saturating_sub(1);
        }

        self.agents.swap_remove(index);
        self.invalidate_lane_bucket_snapshot();
    }

    /// Remaps household indices after a `swap_remove` in `HouseholdSystem`. O(A).
    pub fn remap_household_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            if let Some(&new_id) = mapping.get(&self.agents.household_id[i]) {
                self.agents.household_id[i] = new_id;
            }
        }
    }

    /// Remaps building indices after a `swap_remove` in `BuildingAllocator`. O(A).
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        if mapping.is_empty() {
            return;
        }
        for i in 0..self.agents.len() {
            if let Some(&new_id) = mapping.get(&self.agents.home_building[i]) {
                self.agents.home_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.work_building[i]) {
                self.agents.work_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.current_building[i]) {
                self.agents.current_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.target_building[i]) {
                self.agents.target_building[i] = new_id;
            }
            if let Some(&new_id) = mapping.get(&self.agents.planned_target_building[i]) {
                self.agents.planned_target_building[i] = new_id;
            }
        }
    }

    /// Forcefully removes all agents from a building that has been deleted.
    pub fn evict_building(&mut self, building_id: usize) {
        for i in 0..self.agents.len() {
            if self.agents.work_building[i] == building_id {
                self.agents.work_building[i] = usize::MAX; // Lose Job
            }
            if self.agents.home_building[i] == building_id {
                self.agents.home_building[i] = usize::MAX; // Become Homeless
                self.agents.household_id[i] = usize::MAX;
                self.agents.pending_household_size[i] = 0;
            }
            if self.agents.current_building[i] == building_id {
                // Building collapsed while they were inside!
                self.agents.current_building[i] = usize::MAX;
                self.agents.target_building[i] = usize::MAX;
                self.agents.transit[i] = TRANSIT_ACCESS_INGRESS; // Dump them physically onto the sidewalk/rubble
            } else if self.agents.target_building[i] == building_id {
                if self.agents.home_building[i] != usize::MAX {
                    // Target shop destroyed. Head back home!
                    self.agents.target_building[i] = self.agents.home_building[i];
                    self.agents.planned_target_building[i] = self.agents.home_building[i];
                    self.agents.activity[i] = 0;
                } else {
                    // Target destroyed, AND homeless! Become stranded on the street!
                    self.agents.target_building[i] = usize::MAX;
                    self.agents.pending_household_size[i] = 0;
                    self.agents.transit[i] = TRANSIT_ACCESS_INGRESS;
                }
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Finds a residential building with available vacancy.
    /// Uses the allocator's `vacancy_index` for O(1) random selection.
    pub fn find_available_home(&mut self, allocator: &mut BuildingAllocator) -> Option<usize> {
        let mut rng = rand::thread_rng();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };

        let total_vacant = allocator.vacancy_index[residential_slot].len();

        if total_vacant == 0 {
            return None;
        }

        // Pick random building index from the combined vacancy lists
        let pick = rng.gen_range(0..total_vacant);
        let mut building_idx = usize::MAX;

        let list = &allocator.vacancy_index[residential_slot];
        if pick < list.len() {
            building_idx = list[pick];
        }

        if building_idx != usize::MAX {
            allocator.claim_vacancy(building_idx);
            return Some(building_idx);
        }
        None
    }

    /// Update per-day agent state: home/work bonuses and pollution penalties.
    pub fn daily_update(
        &mut self,
        pollution: &PollutionSystem,
        config: &crate::simulation::core::config::WorldConfig,
    ) {
        let w = pollution.grid.width as f32;
        let h = pollution.grid.height as f32;

        for i in 0..self.agents.len() {
            // 1. Snapshot-based Activity Rewards
            if self.agents.transit[i] == TRANSIT_IN_BUILDING {
                if self.agents.activity[i] == 0 {
                    // Home
                    self.agents.happiness[i] += 1.0;
                }
            }

            // 2. Pollution Penalty
            let (gx_raw, gy_raw) = config.world_to_env_grid(
                self.agents.pos_x[i],
                self.agents.pos_y[i],
                w as usize,
                h as usize,
            );
            let gx = gx_raw.round() as i32;
            let gy = gy_raw.round() as i32;
            if gx >= 0 && gx < w as i32 && gy >= 0 && gy < h as i32 {
                if let Some(p) = pollution.grid.get(gx as usize, gy as usize) {
                    self.agents.happiness[i] -= p * 0.1;
                }
            }

            // 3. Final Clamping
            self.agents.happiness[i] = self.agents.happiness[i].clamp(0.0, 100.0);
            self.agents.money[i] = self.agents.money[i].max(0.0);
        }
    }

    /// Sets `current_lane_id = usize::MAX` for every agent whose active lane belongs to one
    /// of `affected_edges`, or whose connection lane leads directly into such a lane.
    ///
    /// Must be called **before** `LaneSystem::rebuild_edges_incremental` so the old lane IDs
    /// are still valid for lookup — old orphaned lanes retain their original `edge_id` even
    /// after `rebuild_edges_incremental` removes them from `edge_lanes`.
    pub fn invalidate_lane_ids_for_edges(
        &mut self,
        affected_edges: &std::collections::HashSet<usize>,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
    ) {
        // Collect lane IDs that belong to affected edges (from the current edge_lanes).
        let mut affected_lane_ids: std::collections::HashSet<usize> =
            std::collections::HashSet::new();
        for &edge_id in affected_edges {
            if let Some(lane_ids) = lane_system.edge_lanes.get(&edge_id) {
                affected_lane_ids.extend(lane_ids);
            }
        }

        for i in 0..self.agents.len() {
            let lid = self.agents.current_lane_id[i];
            if lid == usize::MAX || lid >= lane_system.lanes.len() {
                continue;
            }
            let lane = &lane_system.lanes[lid];
            let should_invalidate = if lane.edge_id != usize::MAX {
                // Road lane: invalidate if its edge is affected.
                affected_edges.contains(&lane.edge_id)
            } else {
                // Connection lane: invalidate if it leads into an affected road lane.
                lane.next_lanes
                    .first()
                    .map_or(false, |&next| affected_lane_ids.contains(&next))
            };
            if should_invalidate {
                self.agents.current_lane_id[i] = usize::MAX;
                self.agents.lane_change_from_lane_id[i] = u32::MAX;
                self.agents.lane_change_start_d[i] = 0.0;
                self.agents.lane_change_length_m[i] = 0.0;
                self.agents.overtake_blocked_time_s[i] = 0.0;
                self.agents.overtake_cooldown_s[i] = 0.0;
                // lane_distance is intentionally NOT zeroed here: the agent keeps its
                // visual position until the next tick re-attaches it to a new lane.
                // Zeroing it caused agents to visually teleport to edge position 0
                // immediately when a nearby road was modified.
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Re-calculates building occupancy and vacancy index from scratch.
    pub fn recalculate_occupancy(&mut self, allocator: &mut BuildingAllocator) {
        for b in &mut allocator.buildings {
            b.occupancy = 0;
        }
        for i in 0..self.agents.len() {
            let h = self.agents.home_building[i];
            if h != usize::MAX && h < allocator.buildings.len() {
                allocator.buildings[h].occupancy += 1;
            }
        }
        allocator.rebuild_zone_index();
    }
}

fn stable_schedule_seed(home_building: usize, spawn_index: u32) -> u32 {
    let mixed = (home_building as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(spawn_index).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    ((mixed >> 32) as u32) ^ (mixed as u32)
}

#[cfg(test)]
mod tests {
    use super::super::{TRANSIT_IN_BUILDING, TRANSIT_NETWORK};
    use super::*;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::lanes::LaneSystem;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    use std::collections::HashSet;

    fn make_simple_lane_system() -> (RegionGraph, LaneSystem) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(
            Vector3::new(0.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n1 = graph.add_node(
            Vector3::new(100.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );
        let n2 = graph.add_node(
            Vector3::new(200.0, 0.0, 0.0),
            crate::simulation::network::types::NodeType::Junction,
        );

        let _e0 = graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            ..Default::default()
        });
        let _e1 = graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
            ..Default::default()
        });
        graph.rebuild_adjacency_list();

        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes)
    }

    #[test]
    fn test_invalidate_clears_agents_on_affected_edge() {
        let (_graph, lane_system) = make_simple_lane_system();

        // Place two agents: one on edge 0, one on edge 1.
        let e0_lane = lane_system.edge_lanes[&0][0];
        let e1_lane = lane_system.edge_lanes[&1][0];

        let mut sys = AgentSystem::new();
        // Spawn minimal agents and manually set their lane IDs.
        let render_id_0 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id: render_id_0,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: 0,
            current_lane_id: e0_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });
        let render_id_1 = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 150.0,
            pos_y: 0.0,
            render_id: render_id_1,
            activity: 0,
            transit: TRANSIT_NETWORK,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 1,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: usize::MAX,
            target_building: usize::MAX,
            planned_target_building: usize::MAX,
            current_node: 1,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: 1,
            current_lane_id: e1_lane,
            lane_distance: 10.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 10.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: true,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        // Invalidate only edge 0.
        let mut affected = HashSet::new();
        affected.insert(0usize);
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);

        // Agent 0 (on edge 0) must be invalidated.
        assert_eq!(
            sys.agents.current_lane_id[0],
            usize::MAX,
            "Agent on affected edge should have lane_id cleared"
        );
        // lane_distance is intentionally preserved so the agent doesn't visually teleport
        // to position 0 before the next tick re-attaches it.
        assert_eq!(
            sys.agents.lane_distance[0], 10.0,
            "lane_distance should be preserved to avoid visual teleport on road edit"
        );

        // Agent 1 (on edge 1) must be untouched.
        assert_eq!(
            sys.agents.current_lane_id[1], e1_lane,
            "Agent on unaffected edge should keep its lane_id"
        );
    }

    #[test]
    fn test_invalidate_skips_already_invalid_agents() {
        let (_graph, lane_system) = make_simple_lane_system();

        let mut sys = AgentSystem::new();
        let render_id = sys.allocate_render_id();
        sys.agents.push(Agent {
            home_building: usize::MAX,
            household_id: usize::MAX,
            pending_household_size: 0,
            work_building: usize::MAX,
            pos_x: 0.0,
            pos_y: 0.0,
            render_id,
            activity: 0,
            transit: TRANSIT_IN_BUILDING,
            happiness: 50.0,
            money: 100.0,
            journey_start_time: 0.0,
            schedule_seed: 0,
            cached_commute_minutes: 0,
            next_commute_refresh_time: 0.0,
            next_departure_day: u32::MAX,
            next_departure_minute: 0,
            next_departure_origin_building: usize::MAX,
            next_departure_target_building: usize::MAX,
            next_departure_activity: 0,
            cached_schedule_work_building: usize::MAX,
            cached_work_profile_index: u16::MAX,
            current_building: 0,
            target_building: 0,
            planned_target_building: usize::MAX,
            current_node: 0,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: usize::MAX,
            lane_distance: 0.0,
            lane_change_from_lane_id: u32::MAX,
            lane_change_start_d: 0.0,
            lane_change_length_m: 0.0,
            overtake_blocked_time_s: 0.0,
            overtake_cooldown_s: 0.0,
            speed: 0.0,
            transit_mode: MODE_CAR,
            planned_activity: 0,
            current_path: vec![],
            current_path_index: 0,
            has_car: false,
            vehicle_type: 0,
            pedestrian_type: 0,
            walk_phase: 0.0,
            job_lock_days: 0,
            consecutive_unpaid_days: 0,
        });

        let mut affected = HashSet::new();
        affected.insert(0usize);
        // Should not panic on agents already at usize::MAX.
        sys.invalidate_lane_ids_for_edges(&affected, &lane_system);
        assert_eq!(sys.agents.current_lane_id[0], usize::MAX);
    }
}
