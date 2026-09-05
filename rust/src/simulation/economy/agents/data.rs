// SPDX-License-Identifier: GPL-2.0-only

//! Core data layout for the agent simulation.
//!
//! # Memory layout
//!
//! [`AgentSystem`] uses a Structure-of-Arrays (SoA) layout provided by the `soa_derive` crate.
//! This enables cache-friendly bulk iteration and ensures all fields are kept in sync.

use soa_derive::StructOfArray;
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
    /// Resident lifecycle class. One of the `AGE_*` constants in `agents::mod`.
    pub age_group: u8,
    /// Pending resident count carried by a border-origin household arrival car; `0` for normal
    /// agents.
    pub pending_household_size: u16,
    /// Stable freight shipment id carried by this vehicle; `u64::MAX` means ordinary resident.
    pub freight_shipment_id: u64,
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
    /// Border node target for freight exports; `u32::MAX` when not carrying export freight.
    pub freight_target_border_node: u32,
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
    /// Consecutive failed live network/access replans. Transient watchdog state, not persisted.
    pub network_replan_failures: u8,

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
    /// Visual-only current cycle of the walk animation in `[0, 1]`.
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
    /// Scratch mask: agents marked `true` may touch lane-entry claims this tick and are run in
    /// stable index order after the non-claiming parallel movement pass.
    pub claim_serial_agents: Vec<bool>,
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
    pub(crate) next_render_id: u64,
    /// Last allocator building-reference revision that was scrubbed against this agent store.
    pub(crate) last_building_ref_scrub_revision: u64,
    /// Traffic-debug scratch: last observed visible pedestrian X position.
    pub(crate) traffic_debug_last_pos_x: Vec<f32>,
    /// Traffic-debug scratch: last observed visible pedestrian Z position.
    pub(crate) traffic_debug_last_pos_y: Vec<f32>,
    /// Traffic-debug scratch: accumulated no-progress time for visible pedestrians.
    pub(crate) traffic_debug_stationary_s: Vec<f32>,
    /// Traffic-debug scratch: next simulation time at which a stationary line may be emitted.
    pub(crate) traffic_debug_next_log_time: Vec<f32>,
}

impl Clone for AgentSystem {
    fn clone(&self) -> Self {
        Self {
            agents: self.agents.clone(),
            sim_time: self.sim_time,
            pathfind_count: AtomicU32::new(self.pathfind_count.load(Ordering::Relaxed)),
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
            claim_serial_agents: Vec::new(),
            edge_speed_sum: Vec::new(),
            edge_agent_cnt: Vec::new(),
            edge_is_dirty: Vec::new(),
            dirty_edges: Vec::new(),
            stale_dirty_edges: Vec::new(),
            lane_speed_sum: Vec::new(),
            lane_vehicle_cnt: Vec::new(),
            next_render_id: self.next_render_id,
            last_building_ref_scrub_revision: self.last_building_ref_scrub_revision,
            traffic_debug_last_pos_x: Vec::new(),
            traffic_debug_last_pos_y: Vec::new(),
            traffic_debug_stationary_s: Vec::new(),
            traffic_debug_next_log_time: Vec::new(),
        }
    }
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
            claim_serial_agents: Vec::new(),
            edge_speed_sum: Vec::new(),
            edge_agent_cnt: Vec::new(),
            edge_is_dirty: Vec::new(),
            dirty_edges: Vec::new(),
            stale_dirty_edges: Vec::new(),
            lane_speed_sum: Vec::new(),
            lane_vehicle_cnt: Vec::new(),
            next_render_id: 0,
            last_building_ref_scrub_revision: u64::MAX,
            traffic_debug_last_pos_x: Vec::new(),
            traffic_debug_last_pos_y: Vec::new(),
            traffic_debug_stationary_s: Vec::new(),
            traffic_debug_next_log_time: Vec::new(),
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
}
