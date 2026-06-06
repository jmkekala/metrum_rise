//! Main simulation loop for agents: transit state machine and movement.

mod access;
mod frontage;
mod lane_nav;
mod planning;
mod runtime;
mod schedule;
mod slices;
mod traffic;

use super::data::AgentSystem;
use super::{
    ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_EGRESS,
    TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    OperationalClockRuntimeTuning, RuntimeEconomyCatalog, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use godot::prelude::*;
use rand::Rng;
use rayon::prelude::*;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use access::{
    advance_along_local_access_path, local_access_path, local_access_point,
    local_access_side_label, local_access_target_segment, planned_attach_is_legal,
    planned_detach_is_legal,
};
use lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane, lane_origin_node,
    lane_terminal_node, planned_next_connector,
};
use planning::{plan_building_origin_trip, plan_immigration_trip, plan_network_replan};
use runtime::{PAR_THRESHOLD, dispatch_agents};
use schedule::maybe_schedule_work_trip;
use slices::{MovementSlices, RawSlice};

// ---------------------------------------------------------------------------
// IDM (Intelligent Driver Model) constants — defined in config.rs.
// ---------------------------------------------------------------------------

use crate::config::{
    AGENT_DRIVEWAY_SPEED_MS, AGENT_WALK_SPEED_MS, CAR_JUNCTION_SPEED_MS, CAR_LENGTH,
    DEFAULT_URBAN_ROAD_SPEED_MS, IDM_S_MIN,
};
use traffic::{
    ConnectorEntry, LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_COOLDOWN_S,
    OVERTAKE_DETACH_BUFFER_M, OVERTAKE_EDGE_BUFFER_M, OVERTAKE_MIN_GAP_GAIN_M,
    OVERTAKE_MIN_SPEED_GAIN_MS, OVERTAKE_RETURN_TARGET_GAP_M, OVERTAKE_STUCK_TIME_S,
    OVERTAKE_TARGET_AHEAD_GAP_M, braking_speed_for_distance, claim_connector_entry,
    connector_turn_speed, cruise_lane_return_target, idm_gap_bucket, idm_new_speed,
    junction_car_speed, junction_entry_speed, lane_attach_slot_clear, lane_change_gap_clear,
    lane_change_length_for_speed, lane_entry_slot_clear, limit_speed_change,
    live_lane_bucket_transit, overtake_follow_gap, overtaking_lane_target,
    planned_detach_distance_on_current_edge, planned_lane_change_target,
};

const BUILDING_REPLAN_DELAY_S: f32 = 30.0;
const NETWORK_REPLAN_DELAY_S: f32 = 5.0;

// ---------------------------------------------------------------------------
// Thread-local scratch buffers — pre-allocated per Rayon worker thread to
// avoid any heap allocation in the per-agent hot path.
// ---------------------------------------------------------------------------
thread_local! {
    static VALID_LANES: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

fn transit_mode_label(mode: u8) -> &'static str {
    if mode == MODE_CAR { "car" } else { "foot" }
}

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &mut TransitNetwork,
        graph: &mut RegionGraph,
        delta: f32,
        day_index: u32,
        minute_of_day: u16,
    ) {
        self.sim_time += delta;
        let n = self.agents.len();
        if n == 0 {
            self.update_frontage_delay_cache(transit_network, graph, delta);
            return;
        }

        let bldg_count = allocator.buildings.len();

        // -----------------------------------------------------------------------
        // 1. Safety Scrub — parallel, each agent is independent.
        // -----------------------------------------------------------------------
        let s_home = RawSlice::new(&mut self.agents.home_building);
        let s_work = RawSlice::new(&mut self.agents.work_building);
        let s_cur_b = RawSlice::new(&mut self.agents.current_building);
        let s_tgt_b = RawSlice::new(&mut self.agents.target_building);
        let s_plan_b = RawSlice::new(&mut self.agents.planned_target_building);
        let s_transit = RawSlice::new(&mut self.agents.transit);

        dispatch_agents(n, |i| unsafe {
            if *s_home.get(i) != usize::MAX && *s_home.get(i) >= bldg_count {
                *s_home.get_mut(i) = usize::MAX;
            }
            if *s_work.get(i) != usize::MAX && *s_work.get(i) >= bldg_count {
                *s_work.get_mut(i) = usize::MAX;
            }
            if *s_cur_b.get(i) != usize::MAX && *s_cur_b.get(i) >= bldg_count {
                *s_cur_b.get_mut(i) = usize::MAX;
                *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
            }
            let tgt = *s_tgt_b.get(i);
            if tgt != usize::MAX && tgt >= bldg_count {
                let home = *s_home.get(i);
                if home != usize::MAX {
                    *s_tgt_b.get_mut(i) = home;
                } else {
                    *s_tgt_b.get_mut(i) = usize::MAX;
                    *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                }
            }
            let planned = *s_plan_b.get(i);
            if planned != usize::MAX && planned >= bldg_count {
                *s_plan_b.get_mut(i) = usize::MAX;
            }
        });

        // -----------------------------------------------------------------------
        // 2. Lane bucket fill — sequential O(A).
        // -----------------------------------------------------------------------
        let lane_count = transit_network.lane_system.lanes.len();
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
        for &lid in &self.dirty_lanes {
            self.lane_buckets[lid].clear();
            self.lane_is_dirty[lid] = false;
        }
        self.dirty_lanes.clear();
        for i in 0..n {
            if live_lane_bucket_transit(self.agents.transit[i]) {
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    if !self.lane_is_dirty[lid] {
                        self.lane_is_dirty[lid] = true;
                        self.dirty_lanes.push(lid);
                    }
                    self.lane_buckets[lid].push((self.agents.lane_distance[i], i));
                }
                let source_lane_id = self.agents.lane_change_from_lane_id[i];
                let source_lid = source_lane_id as usize;
                if self.agents.transit[i] == TRANSIT_NETWORK
                    && source_lane_id != u32::MAX
                    && source_lid < lane_count
                    && source_lid != lid
                    && self.agents.lane_distance[i] + LANE_CHANGE_FINISH_EPS_M
                        < self.agents.lane_change_start_d[i] + self.agents.lane_change_length_m[i]
                {
                    if !self.lane_is_dirty[source_lid] {
                        self.lane_is_dirty[source_lid] = true;
                        self.dirty_lanes.push(source_lid);
                    }
                    self.lane_buckets[source_lid].push((self.agents.lane_distance[i], i));
                }
            }
        }
        // Parallel sort over dirty lanes. Each lid's Vec is disjoint → safe.
        // Safety: dirty_lanes has no duplicates, so each iteration accesses a
        // distinct element of lane_buckets — no data races.
        {
            let buckets_raw = RawSlice::new(&mut self.lane_buckets);
            if self.dirty_lanes.len() >= PAR_THRESHOLD {
                self.dirty_lanes.par_iter().for_each(|&lid| {
                    let bucket = unsafe { buckets_raw.get_mut(lid) };
                    bucket.sort_unstable_by(|a, b| {
                        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                });
            } else {
                for &lid in &self.dirty_lanes {
                    self.lane_buckets[lid].sort_unstable_by(|a, b| {
                        a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
            }
        }

        if self.lane_attach_claimed.len() < lane_count {
            self.lane_attach_claimed
                .resize_with(lane_count, || AtomicBool::new(false));
        }
        for claimed in &self.lane_attach_claimed {
            claimed.store(false, Ordering::Relaxed);
        }

        // -----------------------------------------------------------------------
        // 3. IDM speed update — parallel.
        // -----------------------------------------------------------------------
        self.new_speed.resize(n, 0.0_f32);
        {
            let s_transit_idm = RawSlice::new(&mut self.agents.transit);
            let s_tmode_idm = RawSlice::new(&mut self.agents.transit_mode);
            let s_lane_idm = RawSlice::new(&mut self.agents.current_lane_id);
            let s_lane_d_idm = RawSlice::new(&mut self.agents.lane_distance);
            let s_cur_e_idm = RawSlice::new(&mut self.agents.current_edge);
            let s_cur_n_idm = RawSlice::new(&mut self.agents.current_node);
            let s_path_idm = RawSlice::new(&mut self.agents.current_path);
            let s_path_idx_idm = RawSlice::new(&mut self.agents.current_path_index);
            let s_access_flags_idm = RawSlice::new(&mut self.agents.access_flags);
            let s_plan_detach_n_idm = RawSlice::new(&mut self.agents.planned_detach_node);
            let s_plan_detach_lane_idm = RawSlice::new(&mut self.agents.planned_detach_lane_id);
            let s_plan_detach_lane_d_idm = RawSlice::new(&mut self.agents.planned_detach_lane_d);
            let s_lane_change_from_idm = RawSlice::new(&mut self.agents.lane_change_from_lane_id);
            let s_overtake_blocked_idm = RawSlice::new(&mut self.agents.overtake_blocked_time_s);
            let s_overtake_cooldown_idm = RawSlice::new(&mut self.agents.overtake_cooldown_s);
            let s_speed_idm = RawSlice::new(&mut self.agents.speed);
            let new_spd_raw = RawSlice::new(&mut self.new_speed);
            let buckets: &Vec<Vec<(f32, usize)>> = &self.lane_buckets;

            dispatch_agents(n, |i| unsafe {
                let cur_spd = *s_speed_idm.get(i);
                let transit = *s_transit_idm.get(i);
                let tmode = *s_tmode_idm.get(i);
                let cooldown = (*s_overtake_cooldown_idm.get(i) - delta).max(0.0);
                *s_overtake_cooldown_idm.get_mut(i) = cooldown;

                if !live_lane_bucket_transit(transit) || tmode != MODE_CAR {
                    *s_overtake_blocked_idm.get_mut(i) = 0.0;
                    *new_spd_raw.get_mut(i) = cur_spd;
                    return;
                }

                let lid = *s_lane_idm.get(i);
                let my_d = *s_lane_d_idm.get(i);
                let eid = *s_cur_e_idm.get(i);

                let v_max = if transit == TRANSIT_INTERSECTION {
                    transit_network
                        .lane_system
                        .lanes
                        .get(lid)
                        .map(connector_turn_speed)
                        .unwrap_or(CAR_JUNCTION_SPEED_MS)
                } else if eid != usize::MAX && eid < graph.edge_count() {
                    graph.edge(eid).speed_limit
                } else {
                    DEFAULT_URBAN_ROAD_SPEED_MS
                };

                let gap = if lid < buckets.len() {
                    idm_gap_bucket(&buckets[lid], my_d)
                } else {
                    f32::MAX
                };
                let mut target_speed = idm_new_speed(cur_spd, v_max, gap, delta);

                if transit == TRANSIT_NETWORK {
                    if let Some(lane) = transit_network.lane_system.lanes.get(lid) {
                        if lane.edge_id != usize::MAX {
                            let planned_detach_lane_id = *s_plan_detach_lane_idm.get(i) as usize;
                            if let Some(connector_id) = planned_next_connector(
                                lid,
                                *s_cur_n_idm.get(i),
                                s_path_idm.get(i),
                                *s_path_idx_idm.get(i),
                                *s_access_flags_idm.get(i),
                                *s_plan_detach_n_idm.get(i),
                                planned_detach_lane_id,
                                transit_network,
                                graph,
                            ) {
                                let turn_target = transit_network
                                    .lane_system
                                    .lanes
                                    .get(connector_id)
                                    .map(|connector| {
                                        if lane_entry_slot_clear(connector_id, buckets) {
                                            connector_turn_speed(connector)
                                        } else {
                                            0.0
                                        }
                                    })
                                    .unwrap_or(CAR_JUNCTION_SPEED_MS);
                                let dist_to_end = (lane.length - my_d).max(0.0);
                                target_speed = target_speed
                                    .min(braking_speed_for_distance(turn_target, dist_to_end));
                            }
                        }
                    }

                    if *s_lane_change_from_idm.get(i) == u32::MAX
                        && (*s_access_flags_idm.get(i) & ACCESS_PLAN_VALID) != 0
                    {
                        let planned_detach_lane_id = *s_plan_detach_lane_idm.get(i) as usize;
                        let planned_detach_lane_d = *s_plan_detach_lane_d_idm.get(i);
                        if let Some(target_lane_id) = planned_lane_change_target(
                            lid,
                            planned_detach_lane_id,
                            my_d,
                            planned_detach_lane_d,
                            transit_network,
                        ) {
                            let target_gap_clear = buckets
                                .get(target_lane_id)
                                .map(|bucket| lane_change_gap_clear(bucket, my_d, cur_spd))
                                .unwrap_or(false);
                            if !target_gap_clear {
                                let stop_before_lane_change =
                                    (planned_detach_lane_d - my_d - LANE_CHANGE_MIN_LENGTH_M)
                                        .max(0.0);
                                target_speed = target_speed
                                    .min(braking_speed_for_distance(0.0, stop_before_lane_change));
                            }
                        }
                    }
                }

                let traffic_blocked = transit == TRANSIT_NETWORK
                    && *s_lane_change_from_idm.get(i) == u32::MAX
                    && cooldown <= 0.0
                    && gap < overtake_follow_gap(cur_spd)
                    && cur_spd + OVERTAKE_MIN_SPEED_GAIN_MS < v_max;
                if traffic_blocked {
                    *s_overtake_blocked_idm.get_mut(i) += delta;
                } else {
                    *s_overtake_blocked_idm.get_mut(i) = 0.0;
                }

                *new_spd_raw.get_mut(i) = limit_speed_change(cur_spd, target_speed, delta);
            });
        }
        for i in 0..n {
            self.agents.speed[i] = self.new_speed[i];
        }

        // -----------------------------------------------------------------------
        // 4. Main agent loop — parallel.
        // -----------------------------------------------------------------------
        let slices = MovementSlices {
            home: RawSlice::new(&mut self.agents.home_building),
            work: RawSlice::new(&mut self.agents.work_building),
            pos_x: RawSlice::new(&mut self.agents.pos_x),
            pos_y: RawSlice::new(&mut self.agents.pos_y),
            activity: RawSlice::new(&mut self.agents.activity),
            transit: RawSlice::new(&mut self.agents.transit),
            happiness: RawSlice::new(&mut self.agents.happiness),
            jstart: RawSlice::new(&mut self.agents.journey_start_time),
            schedule_seed: RawSlice::new(&mut self.agents.schedule_seed),
            cached_commute_minutes: RawSlice::new(&mut self.agents.cached_commute_minutes),
            next_commute_refresh_time: RawSlice::new(&mut self.agents.next_commute_refresh_time),
            cur_b: RawSlice::new(&mut self.agents.current_building),
            tgt_b: RawSlice::new(&mut self.agents.target_building),
            planned_tgt_b: RawSlice::new(&mut self.agents.planned_target_building),
            cur_n: RawSlice::new(&mut self.agents.current_node),
            planned_attach_n: RawSlice::new(&mut self.agents.planned_attach_node),
            planned_detach_n: RawSlice::new(&mut self.agents.planned_detach_node),
            planned_attach_lane: RawSlice::new(&mut self.agents.planned_attach_lane_id),
            planned_detach_lane: RawSlice::new(&mut self.agents.planned_detach_lane_id),
            planned_attach_lane_d: RawSlice::new(&mut self.agents.planned_attach_lane_d),
            planned_detach_lane_d: RawSlice::new(&mut self.agents.planned_detach_lane_d),
            access_flags: RawSlice::new(&mut self.agents.access_flags),
            next_replan_time: RawSlice::new(&mut self.agents.next_replan_time),
            cur_e: RawSlice::new(&mut self.agents.current_edge),
            lane_id: RawSlice::new(&mut self.agents.current_lane_id),
            lane_d: RawSlice::new(&mut self.agents.lane_distance),
            lane_change_from_lane: RawSlice::new(&mut self.agents.lane_change_from_lane_id),
            lane_change_start_d: RawSlice::new(&mut self.agents.lane_change_start_d),
            lane_change_length: RawSlice::new(&mut self.agents.lane_change_length_m),
            overtake_blocked_time: RawSlice::new(&mut self.agents.overtake_blocked_time_s),
            overtake_cooldown: RawSlice::new(&mut self.agents.overtake_cooldown_s),
            tmode: RawSlice::new(&mut self.agents.transit_mode),
            planned_activity: RawSlice::new(&mut self.agents.planned_activity),
            path: RawSlice::new(&mut self.agents.current_path),
            path_idx: RawSlice::new(&mut self.agents.current_path_index),
            has_car: RawSlice::new(&mut self.agents.has_car),
            speed: RawSlice::new(&mut self.agents.speed),
            walk_phase: RawSlice::new(&mut self.agents.walk_phase),
        };

        let lane_buckets = &self.lane_buckets;
        let lane_attach_claimed = &self.lane_attach_claimed;
        let sim_time = self.sim_time;
        let economy_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let economy_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        dispatch_agents(n, |i| unsafe {
            Self::process_agent_movement(
                i,
                delta,
                sim_time,
                day_index,
                minute_of_day,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
                lane_buckets,
                lane_attach_claimed,
                &economy_tuning.operational_clock,
                &economy_catalog,
                &slices,
            );
        });

        // -----------------------------------------------------------------------
        // 5. Post-movement overlap correction + congestion accumulation — sequential O(A).
        //    Merged into one pass to avoid a second O(A) scan in write_congestion.
        // -----------------------------------------------------------------------
        {
            for &lid in &self.dirty_lanes {
                self.lane_buckets[lid].clear();
                self.lane_is_dirty[lid] = false;
            }
            self.dirty_lanes.clear();

            let edge_count = graph.edge_count();
            self.edge_speed_sum.clear();
            self.edge_speed_sum.resize(edge_count, 0.0_f32);
            self.edge_agent_cnt.clear();
            self.edge_agent_cnt.resize(edge_count, 0_u32);

            for i in 0..n {
                if live_lane_bucket_transit(self.agents.transit[i]) {
                    let lid = self.agents.current_lane_id[i];
                    if lid != usize::MAX && lid < lane_count {
                        if !self.lane_is_dirty[lid] {
                            self.lane_is_dirty[lid] = true;
                            self.dirty_lanes.push(lid);
                        }
                        self.lane_buckets[lid].push((self.agents.lane_distance[i], i));
                    }
                    if self.agents.transit[i] == TRANSIT_NETWORK {
                        let eid = self.agents.current_edge[i];
                        if eid != usize::MAX && eid < edge_count {
                            self.edge_speed_sum[eid] += self.agents.speed[i];
                            self.edge_agent_cnt[eid] += 1;
                        }
                    }
                }
            }

            // Parallel sort + overlap correction.
            // Safety: dirty_lanes has no duplicates → each lid accesses a distinct Vec.
            let min_sep = CAR_LENGTH + IDM_S_MIN;
            {
                let buckets_raw = RawSlice::new(&mut self.lane_buckets);
                if self.dirty_lanes.len() >= PAR_THRESHOLD {
                    self.dirty_lanes.par_iter().for_each(|&lid| {
                        let bucket = unsafe { buckets_raw.get_mut(lid) };
                        bucket.sort_unstable_by(|a, b| {
                            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    });
                } else {
                    for &lid in &self.dirty_lanes {
                        self.lane_buckets[lid].sort_unstable_by(|a, b| {
                            a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                }
            }
            for &lid in &self.dirty_lanes {
                let bucket = &mut self.lane_buckets[lid];
                for j in (0..bucket.len().saturating_sub(1)).rev() {
                    let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
                    if bucket[j].0 > max_rear {
                        bucket[j].0 = max_rear;
                        self.agents.lane_distance[bucket[j].1] = max_rear;
                    }
                }
            }

            // 6. Commit congestion — O(E).
            for eid in 0..edge_count {
                if !graph.edge(eid).deleted && self.edge_agent_cnt[eid] > 0 {
                    let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                    let limit = graph.edge(eid).speed_limit.max(1.0);
                    graph.set_edge_congestion(eid, (1.0 - avg / limit).max(0.0));
                }
            }
        }

        self.update_frontage_delay_cache(transit_network, graph, delta);
    }

    /// Core agent movement logic (FSM and physics).
    /// Safety: Caller must ensure disjoint access to agent SoA via `MovementSlices`.
    #[inline(always)]
    pub(crate) unsafe fn process_agent_movement(
        i: usize,
        delta: f32,
        sim_time: f32,
        day_index: u32,
        minute_of_day: u16,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
        lane_buckets: &Vec<Vec<(f32, usize)>>,
        lane_attach_claimed: &Vec<AtomicBool>,
        operational_clock: &OperationalClockRuntimeTuning,
        economy_catalog: &RuntimeEconomyCatalog,
        slices: &MovementSlices,
    ) {
        let mut rng = rand::thread_rng();

        // Safety: index i is unique to this thread via par_iter.
        unsafe {
            let s_cur_n = &slices.cur_n;
            let s_tmode = &slices.tmode;
            let s_speed = &slices.speed;
            let s_walk_phase = &slices.walk_phase;
            let s_transit = &slices.transit;
            let s_activity = &slices.activity;
            let s_work = &slices.work;
            let s_home = &slices.home;
            let s_cur_b = &slices.cur_b;
            let s_tgt_b = &slices.tgt_b;
            let s_plan_b = &slices.planned_tgt_b;
            let s_has_car = &slices.has_car;
            let s_jstart = &slices.jstart;
            let s_schedule_seed = &slices.schedule_seed;
            let s_cached_commute_minutes = &slices.cached_commute_minutes;
            let s_next_commute_refresh_time = &slices.next_commute_refresh_time;
            let s_path = &slices.path;
            let s_path_idx = &slices.path_idx;
            let s_plan_attach_n = &slices.planned_attach_n;
            let s_plan_detach_n = &slices.planned_detach_n;
            let s_plan_attach_lane = &slices.planned_attach_lane;
            let s_plan_detach_lane = &slices.planned_detach_lane;
            let s_plan_attach_lane_d = &slices.planned_attach_lane_d;
            let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
            let s_access_flags = &slices.access_flags;
            let s_next_replan_time = &slices.next_replan_time;
            let s_lane_id = &slices.lane_id;
            let s_lane_d = &slices.lane_d;
            let s_lane_change_from_lane = &slices.lane_change_from_lane;
            let s_lane_change_start_d = &slices.lane_change_start_d;
            let s_lane_change_length = &slices.lane_change_length;
            let s_overtake_blocked_time = &slices.overtake_blocked_time;
            let s_overtake_cooldown = &slices.overtake_cooldown;
            let s_pos_x = &slices.pos_x;
            let s_pos_y = &slices.pos_y;
            let s_cur_e = &slices.cur_e;
            let s_happiness = &slices.happiness;
            let s_plan_act = &slices.planned_activity;

            *s_cur_n.get_mut(i) = graph.get_valid_node(*s_cur_n.get(i));
            if *s_transit.get(i) != TRANSIT_NETWORK && *s_lane_change_from_lane.get(i) != u32::MAX {
                *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                *s_lane_change_start_d.get_mut(i) = 0.0;
                *s_lane_change_length.get_mut(i) = 0.0;
                *s_overtake_blocked_time.get_mut(i) = 0.0;
            }

            // Update walk animation phase if not in a vehicle.
            if *s_tmode.get(i) != MODE_CAR {
                let spd = *s_speed.get(i);
                let phase = *s_walk_phase.get(i);
                // Cycle: about 1 time per meter traveled.
                *s_walk_phase.get_mut(i) = (phase + (spd.abs() * 0.8 * delta)) % 1.0;
            }

            match *s_transit.get(i) {
                TRANSIT_IN_BUILDING => {
                    let curr_bldg = *s_cur_b.get(i);
                    if *s_plan_b.get(i) == usize::MAX
                        && curr_bldg != usize::MAX
                        && curr_bldg < allocator.buildings.len()
                    {
                        if let Some((target_building, activity)) = maybe_schedule_work_trip(
                            curr_bldg,
                            *s_home.get(i),
                            *s_work.get(i),
                            *s_has_car.get(i),
                            *s_schedule_seed.get(i),
                            s_cached_commute_minutes.get_mut(i),
                            s_next_commute_refresh_time.get_mut(i),
                            sim_time,
                            day_index,
                            minute_of_day,
                            allocator,
                            transit_network,
                            graph,
                            pathfind_count,
                            operational_clock,
                            economy_catalog,
                        ) {
                            *s_plan_b.get_mut(i) = target_building;
                            *s_plan_act.get_mut(i) = activity;
                        }
                    }

                    let next_bldg = *s_plan_b.get(i);
                    let next_act = *s_plan_act.get(i);
                    if next_bldg == usize::MAX
                        || next_bldg >= allocator.buildings.len()
                        || curr_bldg == usize::MAX
                        || curr_bldg >= allocator.buildings.len()
                    {
                        // No actionable next trip.
                    } else if sim_time < *s_next_replan_time.get(i) {
                        // Cooldown gate blocks replanning this tick.
                    } else if let Some(plan) = plan_building_origin_trip(
                        curr_bldg,
                        next_bldg,
                        next_act,
                        *s_has_car.get(i),
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                    ) {
                        let origin_entrance = &allocator.entrances[curr_bldg];
                        *s_tgt_b.get_mut(i) = plan.target_building;
                        *s_activity.get_mut(i) = plan.activity;
                        *s_jstart.get_mut(i) = sim_time;
                        *s_tmode.get_mut(i) = plan.mode;
                        *s_plan_attach_n.get_mut(i) = plan.planned_attach_node;
                        *s_plan_detach_n.get_mut(i) = plan.planned_detach_node;
                        *s_plan_attach_lane.get_mut(i) = plan.planned_attach_lane_id as u32;
                        *s_plan_detach_lane.get_mut(i) = plan.planned_detach_lane_id as u32;
                        *s_plan_attach_lane_d.get_mut(i) = plan.planned_attach_lane_d;
                        *s_plan_detach_lane_d.get_mut(i) = plan.planned_detach_lane_d;
                        *s_access_flags.get_mut(i) = plan.access_flags;
                        *s_next_replan_time.get_mut(i) = 0.0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_pos_x.get_mut(i) = origin_entrance.door_pos.x;
                        *s_pos_y.get_mut(i) = origin_entrance.door_pos.y;
                        *s_transit.get_mut(i) = TRANSIT_ACCESS_EGRESS;
                        *s_path.get_mut(i) = plan.current_path;
                        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                        *s_plan_b.get_mut(i) = usize::MAX;
                        *s_plan_act.get_mut(i) = 0;
                        if crate::debug::is_traffic_enabled() {
                            let target_entrance = allocator.entrances.get(plan.target_building);
                            let attach_side = local_access_side_label(
                                plan.mode,
                                origin_entrance,
                                plan.planned_attach_lane_id,
                            );
                            let detach_side = target_entrance
                                .map(|entrance| {
                                    local_access_side_label(
                                        plan.mode,
                                        entrance,
                                        plan.planned_detach_lane_id,
                                    )
                                })
                                .unwrap_or("unknown-target");
                            traffic_log!(
                                "[ACCESS_PLAN] agent={} mode={} origin_bldg={} target_bldg={} attach_lane={}({}) attach_d={:.2} attach_node={} detach_lane={}({}) detach_d={:.2} detach_node={} flags=0x{:02x} node_path={:?}",
                                i,
                                transit_mode_label(plan.mode),
                                curr_bldg,
                                plan.target_building,
                                plan.planned_attach_lane_id,
                                attach_side,
                                plan.planned_attach_lane_d,
                                plan.planned_attach_node,
                                plan.planned_detach_lane_id,
                                detach_side,
                                plan.planned_detach_lane_d,
                                plan.planned_detach_node,
                                plan.access_flags,
                                s_path.get(i),
                            );
                        }
                    } else {
                        *s_access_flags.get_mut(i) = 0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_next_replan_time.get_mut(i) = sim_time + BUILDING_REPLAN_DELAY_S;
                    }
                }

                TRANSIT_ACCESS_EGRESS => {
                    let b_id = *s_cur_b.get(i);
                    if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        *s_access_flags.get_mut(i) = 0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        return;
                    }
                    let b = &allocator.buildings[b_id];
                    if b.edge_idx >= graph.edge_count() || graph.edge(b.edge_idx).deleted {
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        *s_access_flags.get_mut(i) = 0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        *s_next_replan_time.get_mut(i) = sim_time + BUILDING_REPLAN_DELAY_S;
                        return;
                    }
                    let plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
                        && b_id < allocator.entrances.len();
                    if !plan_valid {
                        let origin_door = allocator
                            .entrances
                            .get(b_id)
                            .map(|entrance| entrance.door_pos)
                            .unwrap_or(Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i)));
                        *s_pos_x.get_mut(i) = origin_door.x;
                        *s_pos_y.get_mut(i) = origin_door.y;
                        *s_cur_b.get_mut(i) = b_id;
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_access_flags.get_mut(i) = 0;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        *s_next_replan_time.get_mut(i) = sim_time + BUILDING_REPLAN_DELAY_S;
                        return;
                    }
                    let entrance = &allocator.entrances[b_id];
                    let attach_lane_id = *s_plan_attach_lane.get(i) as usize;
                    let attach_lane_d = *s_plan_attach_lane_d.get(i);
                    let legal_attach = planned_attach_is_legal(
                        *s_tmode.get(i),
                        entrance,
                        attach_lane_id,
                        attach_lane_d,
                        *s_plan_attach_n.get(i),
                        transit_network,
                        graph,
                    );
                    let exact_path = if legal_attach {
                        local_access_path(
                            *s_tmode.get(i),
                            entrance,
                            attach_lane_id,
                            attach_lane_d,
                            transit_network,
                            graph,
                            false,
                        )
                    } else {
                        None
                    };

                    if let (true, Some(path), Some(origin_node)) = (
                        legal_attach,
                        exact_path,
                        lane_origin_node(attach_lane_id, transit_network, graph),
                    ) {
                        let current = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
                        let step = if *s_tmode.get(i) == MODE_CAR {
                            AGENT_DRIVEWAY_SPEED_MS
                        } else {
                            AGENT_WALK_SPEED_MS
                        } * delta;
                        let (next_pos, reached_handoff) =
                            advance_along_local_access_path(current, &path, step);
                        *s_pos_x.get_mut(i) = next_pos.x;
                        *s_pos_y.get_mut(i) = next_pos.y;
                        if crate::debug::is_traffic_enabled() {
                            let seg_before = local_access_target_segment(current, &path);
                            let seg_after = local_access_target_segment(next_pos, &path);
                            traffic_log!(
                                "[ACCESS_EGRESS_STEP] agent={} bldg={} mode={} lane={}({}) lane_d={:.2} seg_before={:?} seg_after={:?} reached_handoff={} current=({:.2},{:.2}) next=({:.2},{:.2}) p0=({:.2},{:.2}) p1=({:.2},{:.2}) p2=({:.2},{:.2}) p3=({:.2},{:.2}) count={}",
                                i,
                                b_id,
                                transit_mode_label(*s_tmode.get(i)),
                                attach_lane_id,
                                local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                                attach_lane_d,
                                seg_before,
                                seg_after,
                                reached_handoff,
                                current.x,
                                current.y,
                                next_pos.x,
                                next_pos.y,
                                path.points[0].x,
                                path.points[0].y,
                                path.points[1].x,
                                path.points[1].y,
                                path.points[2].x,
                                path.points[2].y,
                                path.points[3].x,
                                path.points[3].y,
                                path.count,
                            );
                        }
                        if reached_handoff {
                            let attach_allowed = if *s_tmode.get(i) == MODE_CAR {
                                lane_buckets
                                    .get(attach_lane_id)
                                    .map(|bucket| lane_attach_slot_clear(bucket, attach_lane_d))
                                    .unwrap_or(false)
                                    && lane_attach_claimed
                                        .get(attach_lane_id)
                                        .map(|claimed| !claimed.swap(true, Ordering::AcqRel))
                                        .unwrap_or(false)
                            } else {
                                true
                            };
                            if !attach_allowed {
                                if crate::debug::is_traffic_enabled() {
                                    traffic_log!(
                                        "[ACCESS_EGRESS_WAIT] agent={} bldg={} lane={}({}) lane_d={:.2} pos=({:.2},{:.2}) reason=attach-slot-busy",
                                        i,
                                        b_id,
                                        attach_lane_id,
                                        local_access_side_label(
                                            *s_tmode.get(i),
                                            entrance,
                                            attach_lane_id
                                        ),
                                        attach_lane_d,
                                        path.points[path.count - 1].x,
                                        path.points[path.count - 1].y,
                                    );
                                }
                                *s_pos_x.get_mut(i) = path.points[path.count - 1].x;
                                *s_pos_y.get_mut(i) = path.points[path.count - 1].y;
                                *s_speed.get_mut(i) = 0.0;
                                return;
                            }
                            let parent_edge =
                                transit_network.lane_system.lanes[attach_lane_id].edge_id;
                            *s_pos_x.get_mut(i) = path.points[path.count - 1].x;
                            *s_pos_y.get_mut(i) = path.points[path.count - 1].y;
                            *s_cur_b.get_mut(i) = usize::MAX;
                            *s_cur_n.get_mut(i) = origin_node;
                            *s_cur_e.get_mut(i) = parent_edge;
                            *s_lane_id.get_mut(i) = attach_lane_id;
                            *s_lane_d.get_mut(i) = attach_lane_d;
                            *s_speed.get_mut(i) = if *s_tmode.get(i) == MODE_CAR {
                                graph
                                    .edge(parent_edge)
                                    .speed_limit
                                    .min(AGENT_DRIVEWAY_SPEED_MS)
                            } else {
                                0.0
                            };
                            if crate::debug::is_traffic_enabled() {
                                traffic_log!(
                                    "[ACCESS_EGRESS_ATTACH] agent={} bldg={} lane={}({}) lane_d={:.2} origin_node={} edge={} pos=({:.2},{:.2})",
                                    i,
                                    b_id,
                                    attach_lane_id,
                                    local_access_side_label(
                                        *s_tmode.get(i),
                                        entrance,
                                        attach_lane_id
                                    ),
                                    attach_lane_d,
                                    origin_node,
                                    parent_edge,
                                    path.points[path.count - 1].x,
                                    path.points[path.count - 1].y,
                                );
                            }
                            *s_transit.get_mut(i) = TRANSIT_NETWORK;
                        }
                    } else {
                        if crate::debug::is_traffic_enabled() {
                            traffic_log!(
                                "[ACCESS_EGRESS_ABORT] agent={} bldg={} mode={} lane={} lane_d={:.2} legal_attach={} exact_path={} reason=invalid-egress-plan flags=0x{:02x}",
                                i,
                                b_id,
                                transit_mode_label(*s_tmode.get(i)),
                                attach_lane_id,
                                attach_lane_d,
                                legal_attach,
                                exact_path.is_some(),
                                *s_access_flags.get(i),
                            );
                        }
                        let origin_door = entrance.door_pos;
                        *s_pos_x.get_mut(i) = origin_door.x;
                        *s_pos_y.get_mut(i) = origin_door.y;
                        *s_cur_b.get_mut(i) = b_id;
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_access_flags.get_mut(i) = 0;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        *s_next_replan_time.get_mut(i) = sim_time + BUILDING_REPLAN_DELAY_S;
                    }
                }

                TRANSIT_NETWORK | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
                    if *s_transit.get(i) == TRANSIT_IMMIGRATING
                        && (*s_access_flags.get(i) & ACCESS_PLAN_VALID) == 0
                    {
                        if sim_time >= *s_next_replan_time.get(i) {
                            let border_node = *s_cur_n.get(i);
                            let home_bldg = *s_tgt_b.get(i);
                            if let Some(plan) = plan_immigration_trip(
                                border_node,
                                home_bldg,
                                allocator,
                                transit_network,
                                graph,
                                pathfind_count,
                            ) {
                                *s_tmode.get_mut(i) = MODE_CAR;
                                *s_tgt_b.get_mut(i) = plan.target_building;
                                *s_plan_attach_n.get_mut(i) = plan.planned_attach_node;
                                *s_plan_detach_n.get_mut(i) = plan.planned_detach_node;
                                *s_plan_attach_lane.get_mut(i) = u32::MAX;
                                *s_plan_detach_lane.get_mut(i) = plan.planned_detach_lane_id as u32;
                                *s_plan_attach_lane_d.get_mut(i) = 0.0;
                                *s_plan_detach_lane_d.get_mut(i) = plan.planned_detach_lane_d;
                                *s_access_flags.get_mut(i) = plan.access_flags;
                                *s_next_replan_time.get_mut(i) = 0.0;
                                *s_cur_n.get_mut(i) = border_node;
                                *s_cur_e.get_mut(i) = usize::MAX;
                                *s_lane_id.get_mut(i) = usize::MAX;
                                *s_lane_d.get_mut(i) = 0.0;
                                *s_speed.get_mut(i) = 0.0;
                                *s_path.get_mut(i) = plan.current_path;
                                *s_path_idx.get_mut(i) =
                                    if s_path.get(i).len() >= 2 { 1 } else { 0 };
                                *s_transit.get_mut(i) = TRANSIT_NETWORK;
                            } else {
                                *s_speed.get_mut(i) = 0.0;
                                *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                                return;
                            }
                        } else {
                            *s_speed.get_mut(i) = 0.0;
                            return;
                        }
                    }

                    let target_building = *s_tgt_b.get(i);
                    let requires_exact_access_plan = target_building != usize::MAX;

                    if *s_transit.get(i) != TRANSIT_IMMIGRATING
                        && (*s_access_flags.get(i) & ACCESS_PLAN_VALID) == 0
                        && requires_exact_access_plan
                    {
                        let current_lane_id = *s_lane_id.get(i);
                        let lane_valid = current_lane_id != usize::MAX
                            && current_lane_id < transit_network.lane_system.lanes.len();
                        let replan_start_node = if lane_valid {
                            lane_terminal_node(current_lane_id, transit_network, graph)
                        } else if *s_cur_n.get(i) != u32::MAX {
                            Some(*s_cur_n.get(i))
                        } else {
                            None
                        };
                        let incoming_edge = if lane_valid {
                            transit_network.lane_system.lanes[current_lane_id].edge_id
                        } else {
                            *s_cur_e.get(i)
                        };
                        if sim_time < *s_next_replan_time.get(i) {
                            *s_speed.get_mut(i) = 0.0;
                            return;
                        }
                        let Some(start_node) = replan_start_node else {
                            s_path.get_mut(i).clear();
                            *s_path_idx.get_mut(i) = 0;
                            *s_speed.get_mut(i) = 0.0;
                            *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                            return;
                        };
                        if let Some(replan) = plan_network_replan(
                            start_node,
                            incoming_edge,
                            target_building,
                            *s_tmode.get(i),
                            0,
                            allocator,
                            transit_network,
                            graph,
                            pathfind_count,
                        ) {
                            *s_path.get_mut(i) = replan.current_path;
                            *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                            *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                            *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
                            *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                            *s_access_flags.get_mut(i) = replan.access_flags;
                            *s_next_replan_time.get_mut(i) = 0.0;
                        } else {
                            s_path.get_mut(i).clear();
                            *s_path_idx.get_mut(i) = 0;
                            *s_speed.get_mut(i) = 0.0;
                            *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                            return;
                        }
                    }

                    if (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0 {
                        let target_building = *s_tgt_b.get(i);
                        let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                        let detach_still_legal = target_building < allocator.entrances.len()
                            && planned_detach_lane_id != usize::MAX
                            && planned_detach_is_legal(
                                *s_tmode.get(i),
                                &allocator.entrances[target_building],
                                planned_detach_lane_id,
                                *s_plan_detach_lane_d.get(i),
                                *s_plan_detach_n.get(i),
                                transit_network,
                                graph,
                            );
                        if !detach_still_legal {
                            let current_lane_id = *s_lane_id.get(i);
                            let lane_valid = current_lane_id != usize::MAX
                                && current_lane_id < transit_network.lane_system.lanes.len();
                            let replan_start_node = if lane_valid {
                                lane_terminal_node(current_lane_id, transit_network, graph)
                            } else if *s_cur_n.get(i) != u32::MAX {
                                Some(*s_cur_n.get(i))
                            } else {
                                None
                            };
                            let incoming_edge = if lane_valid {
                                transit_network.lane_system.lanes[current_lane_id].edge_id
                            } else {
                                *s_cur_e.get(i)
                            };
                            if sim_time >= *s_next_replan_time.get(i) {
                                if let Some(start_node) = replan_start_node {
                                    if let Some(replan) = plan_network_replan(
                                        start_node,
                                        incoming_edge,
                                        target_building,
                                        *s_tmode.get(i),
                                        *s_access_flags.get(i),
                                        allocator,
                                        transit_network,
                                        graph,
                                        pathfind_count,
                                    ) {
                                        *s_path.get_mut(i) = replan.current_path;
                                        *s_path_idx.get_mut(i) =
                                            if s_path.get(i).len() >= 2 { 1 } else { 0 };
                                        *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                                        *s_plan_detach_lane.get_mut(i) =
                                            replan.planned_detach_lane_id as u32;
                                        *s_plan_detach_lane_d.get_mut(i) =
                                            replan.planned_detach_lane_d;
                                        *s_access_flags.get_mut(i) = replan.access_flags;
                                        *s_next_replan_time.get_mut(i) = 0.0;
                                    } else {
                                        s_path.get_mut(i).clear();
                                        *s_path_idx.get_mut(i) = 0;
                                        *s_speed.get_mut(i) = 0.0;
                                        *s_next_replan_time.get_mut(i) =
                                            sim_time + NETWORK_REPLAN_DELAY_S;
                                        return;
                                    }
                                } else {
                                    s_path.get_mut(i).clear();
                                    *s_path_idx.get_mut(i) = 0;
                                    *s_speed.get_mut(i) = 0.0;
                                    *s_next_replan_time.get_mut(i) =
                                        sim_time + NETWORK_REPLAN_DELAY_S;
                                    return;
                                }
                            } else {
                                *s_speed.get_mut(i) = 0.0;
                                return;
                            }
                        }
                    }

                    let speed = if *s_tmode.get(i) == MODE_CAR {
                        if *s_transit.get(i) == TRANSIT_INTERSECTION {
                            // Turn movement uses a junction-specific cap, separate from road design speed.
                            let lane_id = *s_lane_id.get(i);
                            let turn_speed = transit_network
                                .lane_system
                                .lanes
                                .get(lane_id)
                                .map(connector_turn_speed)
                                .unwrap_or(CAR_JUNCTION_SPEED_MS);
                            let turn_speed = junction_car_speed(*s_speed.get(i)).min(turn_speed);
                            *s_speed.get_mut(i) = turn_speed;
                            turn_speed
                        } else {
                            *s_speed.get(i)
                        }
                    } else {
                        4.0 // pedestrians use a fixed speed; IDM is car-only
                    };
                    let mut remaining_dist = speed * delta;
                    let mut allow_zero_speed_network_bootstrap =
                        remaining_dist <= 0.0 && *s_lane_id.get(i) == usize::MAX;

                    while remaining_dist > 0.0 || allow_zero_speed_network_bootstrap {
                        allow_zero_speed_network_bootstrap = false;
                        // 1. Init path if missing for exact planned trips only.
                        //
                        // Do not rebuild a node path while already attached to a live lane.
                        // Phase 5 exact plans may intentionally run a frontage-only approach
                        // with an empty node path while the agent is already on the detach lane.
                        if s_path.get(i).is_empty() && *s_lane_id.get(i) == usize::MAX {
                            let access_plan_valid =
                                (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
                            let zero_hop_node_path =
                                (*s_access_flags.get(i) & ACCESS_ZERO_HOP_NODE_PATH) != 0;
                            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                            if access_plan_valid
                                && zero_hop_node_path
                                && planned_detach_lane_id != usize::MAX
                                && *s_cur_n.get(i) == *s_plan_detach_n.get(i)
                                && *s_lane_id.get(i) == usize::MAX
                            {
                                if let Some(detach_origin) =
                                    lane_origin_node(planned_detach_lane_id, transit_network, graph)
                                {
                                    if detach_origin == *s_plan_detach_n.get(i) {
                                        let parent_edge = transit_network.lane_system.lanes
                                            [planned_detach_lane_id]
                                            .edge_id;
                                        *s_cur_e.get_mut(i) = parent_edge;
                                        *s_lane_id.get_mut(i) = planned_detach_lane_id;
                                        *s_lane_d.get_mut(i) = 0.0;
                                        if *s_speed.get(i) == 0.0 {
                                            *s_speed.get_mut(i) =
                                                graph.edge(parent_edge).speed_limit;
                                        }
                                        continue;
                                    }
                                }
                            }

                            if access_plan_valid {
                                if sim_time < *s_next_replan_time.get(i) {
                                    *s_speed.get_mut(i) = 0.0;
                                    break;
                                }
                                let cur_n = *s_cur_n.get(i);
                                if cur_n == u32::MAX {
                                    *s_speed.get_mut(i) = 0.0;
                                    *s_next_replan_time.get_mut(i) =
                                        sim_time + NETWORK_REPLAN_DELAY_S;
                                    break;
                                }
                                if let Some(replan) = plan_network_replan(
                                    cur_n,
                                    *s_cur_e.get(i),
                                    *s_tgt_b.get(i),
                                    *s_tmode.get(i),
                                    *s_access_flags.get(i),
                                    allocator,
                                    transit_network,
                                    graph,
                                    pathfind_count,
                                ) {
                                    *s_path.get_mut(i) = replan.current_path;
                                    *s_path_idx.get_mut(i) =
                                        if s_path.get(i).len() >= 2 { 1 } else { 0 };
                                    *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                                    *s_plan_detach_lane.get_mut(i) =
                                        replan.planned_detach_lane_id as u32;
                                    *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                                    *s_access_flags.get_mut(i) = replan.access_flags;
                                    *s_next_replan_time.get_mut(i) = 0.0;
                                } else {
                                    s_path.get_mut(i).clear();
                                    *s_path_idx.get_mut(i) = 0;
                                    *s_speed.get_mut(i) = 0.0;
                                    *s_next_replan_time.get_mut(i) =
                                        sim_time + NETWORK_REPLAN_DELAY_S;
                                    break;
                                }
                                if s_path.get(i).is_empty() {
                                    continue;
                                }
                            }

                            *s_speed.get_mut(i) = 0.0;
                            *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                            break;
                        }

                        // 2. Init lane if entering network
                        if *s_lane_id.get(i) == usize::MAX {
                            let path = s_path.get(i);
                            let idx = *s_path_idx.get(i);
                            if idx < path.len() {
                                let next_node = path[idx];
                                if let Some(best_e) =
                                    graph.get_edge_between_nodes(*s_cur_n.get(i), next_node)
                                {
                                    let edge = graph.edge(best_e);
                                    let is_fwd = edge.start_node == *s_cur_n.get(i);
                                    if let Some(edge_lanes) =
                                        transit_network.lane_system.edge_lanes.get(&best_e)
                                    {
                                        VALID_LANES.with(|v| {
                                            let mut valid_lanes = v.borrow_mut();
                                            valid_lanes.clear();
                                            for &l_id in edge_lanes {
                                                let lane = &transit_network.lane_system.lanes[l_id];
                                                if lane.is_fwd == is_fwd {
                                                    if *s_tmode.get(i) == MODE_WALK {
                                                        if lane.lane_type == crate::simulation::network::lanes::LaneType::Foot {
                                                            let b_idx = *s_cur_b.get(i);
                                                            if b_idx != usize::MAX && b_idx < allocator.buildings.len() {
                                                                let b_side = allocator.buildings[b_idx].side;
                                                                let lane_side = if lane.lane_idx > 0 { 1 } else { -1 };
                                                                if lane_side == b_side {
                                                                    valid_lanes.push(l_id);
                                                                }
                                                            } else {
                                                                valid_lanes.push(l_id);
                                                            }
                                                        }
                                                    } else if lane.lane_type == crate::simulation::network::lanes::LaneType::Vehicle {
                                                        valid_lanes.push(l_id);
                                                    }
                                                }
                                            }
                                            if !valid_lanes.is_empty() {
                                                let chosen = valid_lanes[rng.gen_range(0..valid_lanes.len())];
                                                *s_lane_id.get_mut(i) = chosen;
                                                *s_lane_d.get_mut(i) = 0.0;
                                                *s_cur_e.get_mut(i) = best_e;
                                                *s_transit.get_mut(i) = TRANSIT_NETWORK;
                                                *s_cur_b.get_mut(i) = usize::MAX;
                                                // Seed speed from edge limit on first lane entry.
                                                if *s_speed.get(i) == 0.0 {
                                                    *s_speed.get_mut(i) = graph.edge(best_e).speed_limit;
                                                }
                                            } else {
                                                s_path.get_mut(i).clear();
                                            }
                                        });
                                        if s_path.get(i).is_empty() {
                                            break;
                                        }
                                    } else {
                                        s_path.get_mut(i).clear();
                                        break;
                                    }
                                } else {
                                    s_path.get_mut(i).clear();
                                    break;
                                }
                            } else {
                                break;
                            }
                        }

                        // 3. Movement along lane
                        let mut lane_id = *s_lane_id.get(i);
                        if lane_id >= transit_network.lane_system.lanes.len() {
                            *s_lane_id.get_mut(i) = usize::MAX;
                            *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                            *s_lane_change_start_d.get_mut(i) = 0.0;
                            *s_lane_change_length.get_mut(i) = 0.0;
                            s_path.get_mut(i).clear();
                            break;
                        }

                        if *s_lane_change_from_lane.get(i) != u32::MAX {
                            let finish_d =
                                *s_lane_change_start_d.get(i) + *s_lane_change_length.get(i);
                            if *s_transit.get(i) != TRANSIT_NETWORK
                                || *s_lane_d.get(i) + LANE_CHANGE_FINISH_EPS_M >= finish_d
                            {
                                *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                                *s_lane_change_start_d.get_mut(i) = 0.0;
                                *s_lane_change_length.get_mut(i) = 0.0;
                            }
                        }

                        if *s_tmode.get(i) == MODE_CAR
                            && *s_transit.get(i) == TRANSIT_NETWORK
                            && *s_lane_change_from_lane.get(i) == u32::MAX
                            && (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
                        {
                            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                            if let Some(target_lane_id) = planned_lane_change_target(
                                lane_id,
                                planned_detach_lane_id,
                                *s_lane_d.get(i),
                                *s_plan_detach_lane_d.get(i),
                                transit_network,
                            ) {
                                let source_lane = &transit_network.lane_system.lanes[lane_id];
                                let target_lane =
                                    &transit_network.lane_system.lanes[target_lane_id];
                                let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                                let available_m = (source_lane.length - *s_lane_d.get(i))
                                    .min(*s_plan_detach_lane_d.get(i) - *s_lane_d.get(i));
                                let maneuver_length = lane_change_length_for_speed(*s_speed.get(i))
                                    .min(
                                        (available_m - LANE_CHANGE_FINISH_EPS_M)
                                            .max(LANE_CHANGE_MIN_LENGTH_M),
                                    );
                                let gap_clear = lane_buckets
                                    .get(target_lane_id)
                                    .map(|bucket| {
                                        lane_change_gap_clear(bucket, lane_d, *s_speed.get(i))
                                    })
                                    .unwrap_or(false);
                                if available_m > LANE_CHANGE_MIN_LENGTH_M && gap_clear {
                                    *s_lane_change_from_lane.get_mut(i) = lane_id as u32;
                                    *s_lane_change_start_d.get_mut(i) = *s_lane_d.get(i);
                                    *s_lane_change_length.get_mut(i) = maneuver_length;
                                    *s_lane_id.get_mut(i) = target_lane_id;
                                    *s_lane_d.get_mut(i) = lane_d;
                                    *s_cur_e.get_mut(i) = target_lane.edge_id;
                                    lane_id = target_lane_id;
                                    traffic_log!(
                                        "[LANE_CHANGE_START] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} detach_lane={} detach_d={:.2}",
                                        i,
                                        target_lane.edge_id,
                                        *s_lane_change_from_lane.get(i),
                                        target_lane_id,
                                        *s_lane_change_start_d.get(i),
                                        *s_lane_change_length.get(i),
                                        *s_speed.get(i),
                                        planned_detach_lane_id,
                                        *s_plan_detach_lane_d.get(i),
                                    );
                                } else if !gap_clear {
                                    traffic_log!(
                                        "[LANE_CHANGE_WAIT] agent={} lane={} target_lane={} lane_d={:.2} speed={:.2} reason=target-gap",
                                        i,
                                        lane_id,
                                        target_lane_id,
                                        *s_lane_d.get(i),
                                        *s_speed.get(i),
                                    );
                                }
                            }
                        }

                        if *s_tmode.get(i) == MODE_CAR
                            && *s_transit.get(i) == TRANSIT_NETWORK
                            && *s_lane_change_from_lane.get(i) == u32::MAX
                            && *s_overtake_cooldown.get(i) <= 0.0
                        {
                            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                            let planned_target_pending =
                                (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
                                    && planned_lane_change_target(
                                        lane_id,
                                        planned_detach_lane_id,
                                        *s_lane_d.get(i),
                                        *s_plan_detach_lane_d.get(i),
                                        transit_network,
                                    )
                                    .is_some();
                            if !planned_target_pending {
                                let source_lane = &transit_network.lane_system.lanes[lane_id];
                                let maneuver_length = lane_change_length_for_speed(*s_speed.get(i));
                                let dist_to_edge_end = source_lane.length - *s_lane_d.get(i);
                                let dist_to_detach = planned_detach_distance_on_current_edge(
                                    lane_id,
                                    planned_detach_lane_id,
                                    *s_lane_d.get(i),
                                    *s_plan_detach_lane_d.get(i),
                                    transit_network,
                                );
                                let enough_road_left =
                                    dist_to_edge_end > maneuver_length + OVERTAKE_EDGE_BUFFER_M;
                                let far_from_detach =
                                    dist_to_detach > maneuver_length + OVERTAKE_DETACH_BUFFER_M;

                                if source_lane.edge_id != usize::MAX
                                    && enough_road_left
                                    && far_from_detach
                                {
                                    let current_gap = lane_buckets
                                        .get(lane_id)
                                        .map(|bucket| idm_gap_bucket(bucket, *s_lane_d.get(i)))
                                        .unwrap_or(f32::MAX);
                                    let overtake_target = if *s_overtake_blocked_time.get(i)
                                        >= OVERTAKE_STUCK_TIME_S
                                    {
                                        overtaking_lane_target(lane_id, transit_network)
                                            .map(|target| (target, true))
                                    } else if *s_overtake_blocked_time.get(i) <= 0.0
                                        && current_gap > OVERTAKE_TARGET_AHEAD_GAP_M
                                    {
                                        cruise_lane_return_target(lane_id, transit_network)
                                            .map(|target| (target, false))
                                    } else {
                                        None
                                    };

                                    if let Some((target_lane_id, is_overtake)) = overtake_target {
                                        let target_lane =
                                            &transit_network.lane_system.lanes[target_lane_id];
                                        let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                                        let target_gap = lane_buckets
                                            .get(target_lane_id)
                                            .map(|bucket| idm_gap_bucket(bucket, lane_d))
                                            .unwrap_or(f32::MAX);
                                        let target_clear = lane_buckets
                                            .get(target_lane_id)
                                            .map(|bucket| {
                                                lane_change_gap_clear(
                                                    bucket,
                                                    lane_d,
                                                    *s_speed.get(i),
                                                )
                                            })
                                            .unwrap_or(false);
                                        let useful_overtake = target_gap
                                            > current_gap + OVERTAKE_MIN_GAP_GAIN_M
                                            && target_gap > OVERTAKE_TARGET_AHEAD_GAP_M;
                                        let safe_return = target_gap > OVERTAKE_RETURN_TARGET_GAP_M;

                                        if target_clear
                                            && ((is_overtake && useful_overtake)
                                                || (!is_overtake && safe_return))
                                        {
                                            *s_lane_change_from_lane.get_mut(i) = lane_id as u32;
                                            *s_lane_change_start_d.get_mut(i) = *s_lane_d.get(i);
                                            *s_lane_change_length.get_mut(i) = maneuver_length;
                                            *s_lane_id.get_mut(i) = target_lane_id;
                                            *s_lane_d.get_mut(i) = lane_d;
                                            *s_cur_e.get_mut(i) = target_lane.edge_id;
                                            *s_overtake_blocked_time.get_mut(i) = 0.0;
                                            *s_overtake_cooldown.get_mut(i) = OVERTAKE_COOLDOWN_S;
                                            lane_id = target_lane_id;
                                            if is_overtake {
                                                traffic_log!(
                                                    "[OVERTAKE_START] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} current_gap={:.2} target_gap={:.2}",
                                                    i,
                                                    target_lane.edge_id,
                                                    *s_lane_change_from_lane.get(i),
                                                    target_lane_id,
                                                    *s_lane_change_start_d.get(i),
                                                    *s_lane_change_length.get(i),
                                                    *s_speed.get(i),
                                                    current_gap,
                                                    target_gap,
                                                );
                                            } else {
                                                traffic_log!(
                                                    "[OVERTAKE_RETURN] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} target_gap={:.2}",
                                                    i,
                                                    target_lane.edge_id,
                                                    *s_lane_change_from_lane.get(i),
                                                    target_lane_id,
                                                    *s_lane_change_start_d.get(i),
                                                    *s_lane_change_length.get(i),
                                                    *s_speed.get(i),
                                                    target_gap,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        let lane = &transit_network.lane_system.lanes[lane_id];
                        let dist_to_end = lane.length - *s_lane_d.get(i);

                        if remaining_dist < dist_to_end {
                            *s_lane_d.get_mut(i) += remaining_dist;
                            remaining_dist = 0.0;

                            let access_plan_valid =
                                (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
                            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                            if access_plan_valid
                                && planned_detach_lane_id != usize::MAX
                                && lane_id == planned_detach_lane_id
                                && *s_lane_d.get(i) >= *s_plan_detach_lane_d.get(i)
                            {
                                let detach_d = *s_plan_detach_lane_d.get(i);
                                let detach_allowed = if *s_tmode.get(i) == MODE_CAR {
                                    lane_attach_claimed
                                        .get(planned_detach_lane_id)
                                        .map(|claimed| !claimed.swap(true, Ordering::AcqRel))
                                        .unwrap_or(false)
                                } else {
                                    true
                                };
                                if detach_allowed {
                                    let t_bldg_idx = *s_tgt_b.get(i);
                                    if t_bldg_idx < allocator.entrances.len() {
                                        if let Some(ingress_origin) = local_access_point(
                                            *s_tmode.get(i),
                                            &allocator.entrances[t_bldg_idx],
                                            planned_detach_lane_id,
                                            detach_d,
                                            transit_network,
                                        ) {
                                            *s_pos_x.get_mut(i) = ingress_origin.x;
                                            *s_pos_y.get_mut(i) = ingress_origin.y;
                                            s_path.get_mut(i).clear();
                                            *s_path_idx.get_mut(i) = 0;
                                            *s_cur_n.get_mut(i) = u32::MAX;
                                            *s_cur_e.get_mut(i) = usize::MAX;
                                            *s_lane_id.get_mut(i) = usize::MAX;
                                            *s_lane_d.get_mut(i) = 0.0;
                                            *s_speed.get_mut(i) = 0.0;
                                            *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                                            if crate::debug::is_traffic_enabled() {
                                                let entrance = &allocator.entrances[t_bldg_idx];
                                                traffic_log!(
                                                    "[ACCESS_INGRESS_DETACH] agent={} target_bldg={} lane={}({}) lane_d={:.2} ingress_origin=({:.2},{:.2}) detach_node={} path_count={} flags=0x{:02x}",
                                                    i,
                                                    t_bldg_idx,
                                                    planned_detach_lane_id,
                                                    local_access_side_label(
                                                        *s_tmode.get(i),
                                                        entrance,
                                                        planned_detach_lane_id,
                                                    ),
                                                    detach_d,
                                                    ingress_origin.x,
                                                    ingress_origin.y,
                                                    *s_plan_detach_n.get(i),
                                                    s_path.get(i).len(),
                                                    *s_access_flags.get(i),
                                                );
                                            }
                                            break;
                                        }
                                    }
                                } else {
                                    if crate::debug::is_traffic_enabled() {
                                        let target_entrance =
                                            allocator.entrances.get(*s_tgt_b.get(i));
                                        let side = target_entrance
                                            .map(|entrance| {
                                                local_access_side_label(
                                                    *s_tmode.get(i),
                                                    entrance,
                                                    planned_detach_lane_id,
                                                )
                                            })
                                            .unwrap_or("unknown-target");
                                        traffic_log!(
                                            "[ACCESS_INGRESS_WAIT] agent={} target_bldg={} lane={}({}) lane_d={:.2} reason=detach-slot-busy",
                                            i,
                                            *s_tgt_b.get(i),
                                            planned_detach_lane_id,
                                            side,
                                            detach_d,
                                        );
                                    }
                                    *s_lane_d.get_mut(i) = detach_d;
                                    *s_speed.get_mut(i) = 0.0;
                                    break;
                                }
                            }
                        } else {
                            // Reached end of lane
                            remaining_dist -= dist_to_end;

                            if lane.edge_id != usize::MAX {
                                *s_cur_n.get_mut(i) = if lane.is_fwd {
                                    graph.edge(lane.edge_id).end_node
                                } else {
                                    graph.edge(lane.edge_id).start_node
                                };

                                let path_len = s_path.get(i).len();
                                let access_plan_valid =
                                    (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
                                let path_idx_before_lane_end = (*s_path_idx.get(i)).min(path_len);
                                if path_idx_before_lane_end != *s_path_idx.get(i) {
                                    *s_path_idx.get_mut(i) = path_idx_before_lane_end;
                                }
                                let should_hold_frontage_idx = access_plan_valid
                                    && path_len >= 1
                                    && path_idx_before_lane_end == 1
                                    && *s_cur_n.get(i) == s_path.get(i)[0];
                                let should_hold_access_tail_idx =
                                    access_plan_valid && path_idx_before_lane_end >= path_len;
                                if !should_hold_frontage_idx && !should_hold_access_tail_idx {
                                    *s_path_idx.get_mut(i) += 1;
                                }
                                let path_idx = *s_path_idx.get(i);

                                if path_idx < path_len {
                                    let next_node = s_path.get(i)[path_idx];
                                    if let Some(best_e) =
                                        graph.get_edge_between_nodes(*s_cur_n.get(i), next_node)
                                    {
                                        let mut wait_for_gap = false;
                                        VALID_CONNS.with(|v| {
                                            let mut connector_candidates = v.borrow_mut();
                                            let any_routing_valid =
                                                collect_connector_lanes_to_edge(
                                                    lane_id,
                                                    best_e,
                                                    transit_network,
                                                    &mut connector_candidates,
                                                );
                                            match claim_connector_entry(
                                                &mut connector_candidates,
                                                any_routing_valid,
                                                &mut rng,
                                                lane_buckets,
                                                lane_attach_claimed,
                                            ) {
                                                ConnectorEntry::Enter(chosen_conn) => {
                                                    *s_lane_id.get_mut(i) = chosen_conn;
                                                    *s_lane_d.get_mut(i) = 0.0;
                                                    *s_transit.get_mut(i) =
                                                        TRANSIT_INTERSECTION;
                                                    *s_cur_e.get_mut(i) = usize::MAX;
                                                    if *s_tmode.get(i) == MODE_CAR {
                                                        let turn_speed = transit_network
                                                            .lane_system
                                                            .lanes
                                                            .get(chosen_conn)
                                                            .map(|conn_lane| {
                                                                junction_entry_speed(
                                                                    *s_speed.get(i),
                                                                    conn_lane,
                                                                )
                                                            })
                                                            .unwrap_or_else(|| {
                                                                junction_car_speed(*s_speed.get(i))
                                                            });
                                                        let time_left = if speed > 1.0e-5 {
                                                            remaining_dist / speed
                                                        } else {
                                                            0.0
                                                        };
                                                        remaining_dist = turn_speed * time_left;
                                                        *s_speed.get_mut(i) = turn_speed;
                                                    }
                                                    if crate::debug::is_traffic_enabled() {
                                                        let conn_lane = &transit_network
                                                            .lane_system
                                                            .lanes[chosen_conn];
                                                        let target_lane = conn_lane
                                                            .next_lanes
                                                            .first()
                                                            .copied()
                                                            .unwrap_or(usize::MAX);
                                                        let target_edge = transit_network
                                                            .lane_system
                                                            .lanes
                                                            .get(target_lane)
                                                            .map(|lane| lane.edge_id)
                                                            .unwrap_or(usize::MAX);
                                                        traffic_log!(
                                                            "[JUNCTION_ENTER] agent={} node={} from_lane={} from_edge={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{}",
                                                            i,
                                                            *s_cur_n.get(i),
                                                            lane_id,
                                                            lane.edge_id,
                                                            chosen_conn,
                                                            conn_lane.length,
                                                            target_lane,
                                                            target_edge,
                                                            *s_speed.get(i),
                                                            remaining_dist,
                                                            *s_path_idx.get(i),
                                                            path_len,
                                                        );
                                                    }
                                                }
                                                ConnectorEntry::ClaimedThisTick => {
                                                    *s_path_idx.get_mut(i) =
                                                        path_idx_before_lane_end;
                                                    *s_lane_d.get_mut(i) = lane.length;
                                                    wait_for_gap = true;
                                                    traffic_log!(
                                                        "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_edge={} path_idx={}/{} reason=connector-entry-claimed",
                                                        i,
                                                        *s_cur_n.get(i),
                                                        lane_id,
                                                        lane.edge_id,
                                                        best_e,
                                                        *s_path_idx.get(i),
                                                        path_len,
                                                    );
                                                }
                                                ConnectorEntry::Occupied => {
                                                    *s_path_idx.get_mut(i) =
                                                        path_idx_before_lane_end;
                                                    *s_lane_d.get_mut(i) = lane.length;
                                                    wait_for_gap = true;
                                                    traffic_log!(
                                                        "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_edge={} path_idx={}/{} reason=connector-occupied",
                                                        i,
                                                        *s_cur_n.get(i),
                                                        lane_id,
                                                        lane.edge_id,
                                                        best_e,
                                                        *s_path_idx.get(i),
                                                        path_len,
                                                    );
                                                }
                                                ConnectorEntry::MissingConnection => {
                                                    traffic_log!(
                                                        "[JUNCTION_MISSING_CONN] agent={} node={} from_lane={} from_edge={} to_edge={} path_idx={}/{} reason=no-connection-lane",
                                                        i,
                                                        *s_cur_n.get(i),
                                                        lane_id,
                                                        lane.edge_id,
                                                        best_e,
                                                        *s_path_idx.get(i),
                                                        path_len,
                                                    );
                                                    // No connection lane exists for this turn.
                                                    // Clear the path so the agent re-pathfinds on
                                                    // the next tick — the updated CCH will now route
                                                    // around the restricted junction.
                                                    s_path.get_mut(i).clear();
                                                    *s_lane_id.get_mut(i) = usize::MAX;
                                                }
                                            }
                                        });
                                        if wait_for_gap {
                                            break;
                                        }
                                        if s_path.get(i).is_empty() {
                                            break;
                                        }
                                    } else {
                                        traffic_log!(
                                            "[JUNCTION_MISSING_EDGE] agent={} node={} from_lane={} from_edge={} next_node={} path_idx={}/{} reason=no-road-edge",
                                            i,
                                            *s_cur_n.get(i),
                                            lane_id,
                                            lane.edge_id,
                                            next_node,
                                            *s_path_idx.get(i),
                                            path_len,
                                        );
                                        s_path.get_mut(i).clear();
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        break;
                                    }
                                } else {
                                    if access_plan_valid
                                        && *s_cur_n.get(i) == *s_plan_detach_n.get(i)
                                        && *s_plan_detach_lane.get(i) != u32::MAX
                                    {
                                        let detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                                        if let Some(detach_origin) =
                                            lane_origin_node(detach_lane_id, transit_network, graph)
                                        {
                                            if detach_origin == *s_plan_detach_n.get(i) {
                                                let mut entered_zero_hop_connector = false;
                                                let mut zero_hop_wait_for_gap = false;
                                                VALID_CONNS.with(|v| {
                                                    let mut connector_candidates = v.borrow_mut();
                                                    let any_routing_valid =
                                                        collect_connector_lanes_to_lane(
                                                            lane_id,
                                                            detach_lane_id,
                                                            transit_network,
                                                            &mut connector_candidates,
                                                        );
                                                    match claim_connector_entry(
                                                        &mut connector_candidates,
                                                        any_routing_valid,
                                                        &mut rng,
                                                        lane_buckets,
                                                        lane_attach_claimed,
                                                    ) {
                                                        ConnectorEntry::Enter(conn_lane_id) => {
                                                            *s_lane_id.get_mut(i) = conn_lane_id;
                                                            *s_lane_d.get_mut(i) = 0.0;
                                                            *s_transit.get_mut(i) =
                                                                TRANSIT_INTERSECTION;
                                                            *s_cur_e.get_mut(i) = usize::MAX;
                                                            if *s_tmode.get(i) == MODE_CAR {
                                                                let turn_speed = transit_network
                                                                    .lane_system
                                                                    .lanes
                                                                    .get(conn_lane_id)
                                                                    .map(|conn_lane| {
                                                                        junction_entry_speed(
                                                                            *s_speed.get(i),
                                                                            conn_lane,
                                                                        )
                                                                    })
                                                                    .unwrap_or_else(|| {
                                                                        junction_car_speed(
                                                                            *s_speed.get(i),
                                                                        )
                                                                    });
                                                                let time_left = if speed > 1.0e-5 {
                                                                    remaining_dist / speed
                                                                } else {
                                                                    0.0
                                                                };
                                                                remaining_dist =
                                                                    turn_speed * time_left;
                                                                *s_speed.get_mut(i) = turn_speed;
                                                            }
                                                            if crate::debug::is_traffic_enabled() {
                                                                let conn_lane = &transit_network
                                                                    .lane_system
                                                                    .lanes[conn_lane_id];
                                                                let target_edge = transit_network
                                                                    .lane_system
                                                                    .lanes
                                                                    .get(detach_lane_id)
                                                                    .map(|lane| lane.edge_id)
                                                                    .unwrap_or(usize::MAX);
                                                                traffic_log!(
                                                                    "[JUNCTION_ENTER] agent={} node={} from_lane={} from_edge={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{} reason=zero-hop-access",
                                                                    i,
                                                                    *s_cur_n.get(i),
                                                                    lane_id,
                                                                    lane.edge_id,
                                                                    conn_lane_id,
                                                                    conn_lane.length,
                                                                    detach_lane_id,
                                                                    target_edge,
                                                                    *s_speed.get(i),
                                                                    remaining_dist,
                                                                    *s_path_idx.get(i),
                                                                    path_len,
                                                                );
                                                            }
                                                            entered_zero_hop_connector = true;
                                                        }
                                                        ConnectorEntry::Occupied => {
                                                            *s_path_idx.get_mut(i) =
                                                                path_idx.min(path_len);
                                                            *s_lane_d.get_mut(i) = lane.length;
                                                            *s_speed.get_mut(i) = 0.0;
                                                            zero_hop_wait_for_gap = true;
                                                            traffic_log!(
                                                                "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-connector-occupied",
                                                                i,
                                                                *s_cur_n.get(i),
                                                                lane_id,
                                                                lane.edge_id,
                                                                detach_lane_id,
                                                                *s_path_idx.get(i),
                                                                path_len,
                                                            );
                                                        }
                                                        ConnectorEntry::ClaimedThisTick => {
                                                            *s_path_idx.get_mut(i) =
                                                                path_idx.min(path_len);
                                                            *s_lane_d.get_mut(i) = lane.length;
                                                            *s_speed.get_mut(i) = 0.0;
                                                            zero_hop_wait_for_gap = true;
                                                            traffic_log!(
                                                                "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-connector-entry-claimed",
                                                                i,
                                                                *s_cur_n.get(i),
                                                                lane_id,
                                                                lane.edge_id,
                                                                detach_lane_id,
                                                                *s_path_idx.get(i),
                                                                path_len,
                                                            );
                                                        }
                                                        ConnectorEntry::MissingConnection => {
                                                            traffic_log!(
                                                                "[JUNCTION_MISSING_CONN] agent={} node={} from_lane={} from_edge={} to_lane={} path_idx={}/{} reason=zero-hop-no-connection-lane",
                                                                i,
                                                                *s_cur_n.get(i),
                                                                lane_id,
                                                                lane.edge_id,
                                                                detach_lane_id,
                                                                *s_path_idx.get(i),
                                                                path_len,
                                                            );
                                                        }
                                                    }
                                                });
                                                if entered_zero_hop_connector {
                                                    continue;
                                                }
                                                if zero_hop_wait_for_gap {
                                                    break;
                                                }
                                            }
                                        }
                                    }
                                    s_path.get_mut(i).clear();
                                    *s_lane_id.get_mut(i) = usize::MAX;
                                    if access_plan_valid {
                                        if sim_time >= *s_next_replan_time.get(i) {
                                            if let Some(replan) = plan_network_replan(
                                                *s_cur_n.get(i),
                                                *s_cur_e.get(i),
                                                *s_tgt_b.get(i),
                                                *s_tmode.get(i),
                                                *s_access_flags.get(i),
                                                allocator,
                                                transit_network,
                                                graph,
                                                pathfind_count,
                                            ) {
                                                *s_path.get_mut(i) = replan.current_path;
                                                *s_path_idx.get_mut(i) =
                                                    if s_path.get(i).len() >= 2 { 1 } else { 0 };
                                                *s_plan_detach_n.get_mut(i) =
                                                    replan.planned_detach_node;
                                                *s_plan_detach_lane.get_mut(i) =
                                                    replan.planned_detach_lane_id as u32;
                                                *s_plan_detach_lane_d.get_mut(i) =
                                                    replan.planned_detach_lane_d;
                                                *s_access_flags.get_mut(i) = replan.access_flags;
                                                *s_next_replan_time.get_mut(i) = 0.0;
                                            } else {
                                                *s_path_idx.get_mut(i) = 0;
                                                *s_speed.get_mut(i) = 0.0;
                                                *s_next_replan_time.get_mut(i) =
                                                    sim_time + NETWORK_REPLAN_DELAY_S;
                                            }
                                        } else {
                                            *s_path_idx.get_mut(i) = 0;
                                            *s_speed.get_mut(i) = 0.0;
                                        }
                                        break;
                                    }
                                    break;
                                }
                            } else {
                                if !lane.next_lanes.is_empty() {
                                    let tgt_road_lane = lane.next_lanes[0];
                                    if tgt_road_lane < transit_network.lane_system.lanes.len() {
                                        let target_edge = transit_network.lane_system.lanes
                                            [tgt_road_lane]
                                            .edge_id;
                                        traffic_log!(
                                            "[JUNCTION_EXIT] agent={} node={} conn_lane={} conn_len={:.2} to_lane={} to_edge={} speed={:.2} remaining_dist={:.2} path_idx={}/{}",
                                            i,
                                            lane.node_id,
                                            lane_id,
                                            lane.length,
                                            tgt_road_lane,
                                            target_edge,
                                            *s_speed.get(i),
                                            remaining_dist,
                                            *s_path_idx.get(i),
                                            s_path.get(i).len(),
                                        );
                                        *s_lane_id.get_mut(i) = tgt_road_lane;
                                        *s_lane_d.get_mut(i) = 0.0;
                                        *s_transit.get_mut(i) = TRANSIT_NETWORK;
                                        *s_cur_e.get_mut(i) = target_edge;
                                    } else {
                                        traffic_log!(
                                            "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} next_lane={} reason=invalid-target-lane",
                                            i,
                                            lane.node_id,
                                            lane_id,
                                            lane.length,
                                            tgt_road_lane,
                                        );
                                        s_path.get_mut(i).clear();
                                        *s_lane_id.get_mut(i) = usize::MAX;
                                        break;
                                    }
                                } else {
                                    traffic_log!(
                                        "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} reason=no-next-lane",
                                        i,
                                        lane.node_id,
                                        lane_id,
                                        lane.length,
                                    );
                                    s_path.get_mut(i).clear();
                                    *s_lane_id.get_mut(i) = usize::MAX;
                                    break;
                                }
                            }
                        }
                    }

                    let current_lane = *s_lane_id.get(i);
                    if current_lane != usize::MAX
                        && current_lane < transit_network.lane_system.lanes.len()
                    {
                        let l = &transit_network.lane_system.lanes[current_lane];
                        let dist = *s_lane_d.get(i);
                        if dist <= 0.0 && !l.geometry.is_empty() {
                            *s_pos_x.get_mut(i) = l.geometry[0].x;
                            *s_pos_y.get_mut(i) = l.geometry[0].z;
                        } else if dist >= l.length && !l.geometry.is_empty() {
                            let end = l.geometry.last().unwrap();
                            *s_pos_x.get_mut(i) = end.x;
                            *s_pos_y.get_mut(i) = end.z;
                        } else if l.geometry.len() >= 2 && !l.cum_dist.is_empty() {
                            let seg = l.cum_dist.partition_point(|&d| d <= dist).saturating_sub(1);
                            let seg = seg.min(l.geometry.len() - 2);
                            let p0 = l.geometry[seg];
                            let p1 = l.geometry[seg + 1];
                            let seg_len = l.cum_dist[seg + 1] - l.cum_dist[seg];
                            let t = if seg_len > 1e-5 {
                                (dist - l.cum_dist[seg]) / seg_len
                            } else {
                                0.0
                            };
                            let mut out = p0.lerp(p1, t.clamp(0.0, 1.0));
                            if *s_tmode.get(i) == MODE_WALK && seg_len > 1e-5 {
                                let tangent = (p1 - p0) / seg_len;
                                let normal = Vector3::new(-tangent.z, 0.0, tangent.x);
                                let jitter =
                                    (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
                                out += normal * jitter;
                            }
                            *s_pos_x.get_mut(i) = out.x;
                            *s_pos_y.get_mut(i) = out.z;
                        }
                    }
                }

                TRANSIT_ACCESS_INGRESS => {
                    let b_id = *s_tgt_b.get(i);
                    if b_id == usize::MAX || b_id >= allocator.buildings.len() {
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        *s_access_flags.get_mut(i) = 0;
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        return;
                    }
                    let plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
                        && b_id < allocator.entrances.len();
                    if !plan_valid {
                        let ingress_target = allocator
                            .entrances
                            .get(b_id)
                            .map(|entrance| entrance.door_pos)
                            .unwrap_or(Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i)));
                        *s_pos_x.get_mut(i) = ingress_target.x;
                        *s_pos_y.get_mut(i) = ingress_target.y;
                        *s_cur_b.get_mut(i) = b_id;
                        *s_tgt_b.get_mut(i) = usize::MAX;
                        *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                        let home = *s_home.get(i);
                        let work = *s_work.get(i);
                        if b_id == home {
                            *s_activity.get_mut(i) = 0;
                        } else if b_id == work {
                            *s_activity.get_mut(i) = 1;
                        } else {
                            *s_activity.get_mut(i) = 2;
                        }
                        *s_plan_attach_n.get_mut(i) = u32::MAX;
                        *s_plan_detach_n.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_attach_lane_d.get_mut(i) = 0.0;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_access_flags.get_mut(i) = 0;
                        *s_next_replan_time.get_mut(i) = 0.0;
                        *s_cur_n.get_mut(i) = u32::MAX;
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;
                        let commute_time = sim_time - *s_jstart.get(i);
                        *s_happiness.get_mut(i) =
                            (*s_happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
                        return;
                    }
                    let entrance = &allocator.entrances[b_id];
                    let detach_lane_id = *s_plan_detach_lane.get(i) as usize;
                    let detach_lane_d = *s_plan_detach_lane_d.get(i);
                    let legal_detach = planned_detach_is_legal(
                        *s_tmode.get(i),
                        entrance,
                        detach_lane_id,
                        detach_lane_d,
                        *s_plan_detach_n.get(i),
                        transit_network,
                        graph,
                    );
                    let exact_path = if legal_detach {
                        local_access_path(
                            *s_tmode.get(i),
                            entrance,
                            detach_lane_id,
                            detach_lane_d,
                            transit_network,
                            graph,
                            true,
                        )
                    } else {
                        None
                    };

                    if let (true, Some(path)) = (legal_detach, exact_path) {
                        let current = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
                        let step = if *s_tmode.get(i) == MODE_CAR {
                            AGENT_DRIVEWAY_SPEED_MS
                        } else {
                            AGENT_WALK_SPEED_MS
                        } * delta;
                        let (next_pos, reached_door) =
                            advance_along_local_access_path(current, &path, step);
                        *s_pos_x.get_mut(i) = next_pos.x;
                        *s_pos_y.get_mut(i) = next_pos.y;
                        if crate::debug::is_traffic_enabled() {
                            let seg_before = local_access_target_segment(current, &path);
                            let seg_after = local_access_target_segment(next_pos, &path);
                            traffic_log!(
                                "[ACCESS_INGRESS_STEP] agent={} bldg={} mode={} lane={}({}) lane_d={:.2} seg_before={:?} seg_after={:?} reached_door={} current=({:.2},{:.2}) next=({:.2},{:.2}) p0=({:.2},{:.2}) p1=({:.2},{:.2}) p2=({:.2},{:.2}) p3=({:.2},{:.2}) count={}",
                                i,
                                b_id,
                                transit_mode_label(*s_tmode.get(i)),
                                detach_lane_id,
                                local_access_side_label(*s_tmode.get(i), entrance, detach_lane_id),
                                detach_lane_d,
                                seg_before,
                                seg_after,
                                reached_door,
                                current.x,
                                current.y,
                                next_pos.x,
                                next_pos.y,
                                path.points[0].x,
                                path.points[0].y,
                                path.points[1].x,
                                path.points[1].y,
                                path.points[2].x,
                                path.points[2].y,
                                path.points[3].x,
                                path.points[3].y,
                                path.count,
                            );
                        }
                        if reached_door {
                            let ingress_target = path.points[path.count - 1];
                            if crate::debug::is_traffic_enabled() {
                                traffic_log!(
                                    "[ACCESS_INGRESS_DONE] agent={} bldg={} pos=({:.2},{:.2}) activity_before={} flags=0x{:02x}",
                                    i,
                                    b_id,
                                    ingress_target.x,
                                    ingress_target.y,
                                    *s_activity.get(i),
                                    *s_access_flags.get(i),
                                );
                            }
                            *s_pos_x.get_mut(i) = ingress_target.x;
                            *s_pos_y.get_mut(i) = ingress_target.y;
                            *s_cur_b.get_mut(i) = b_id;
                            *s_tgt_b.get_mut(i) = usize::MAX;
                            *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                            let home = *s_home.get(i);
                            let work = *s_work.get(i);
                            if b_id == home {
                                *s_activity.get_mut(i) = 0;
                            } else if b_id == work {
                                *s_activity.get_mut(i) = 1;
                            } else {
                                *s_activity.get_mut(i) = 2;
                            }
                            *s_plan_attach_n.get_mut(i) = u32::MAX;
                            *s_plan_detach_n.get_mut(i) = u32::MAX;
                            *s_plan_attach_lane.get_mut(i) = u32::MAX;
                            *s_plan_detach_lane.get_mut(i) = u32::MAX;
                            *s_plan_attach_lane_d.get_mut(i) = 0.0;
                            *s_plan_detach_lane_d.get_mut(i) = 0.0;
                            *s_access_flags.get_mut(i) = 0;
                            *s_next_replan_time.get_mut(i) = 0.0;
                            *s_cur_n.get_mut(i) = u32::MAX;
                            *s_cur_e.get_mut(i) = usize::MAX;
                            *s_lane_id.get_mut(i) = usize::MAX;
                            *s_lane_d.get_mut(i) = 0.0;
                            *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                            *s_lane_change_start_d.get_mut(i) = 0.0;
                            *s_lane_change_length.get_mut(i) = 0.0;
                            *s_overtake_blocked_time.get_mut(i) = 0.0;
                            *s_overtake_cooldown.get_mut(i) = 0.0;
                            *s_speed.get_mut(i) = 0.0;
                            s_path.get_mut(i).clear();
                            *s_path_idx.get_mut(i) = 0;

                            let commute_time = sim_time - *s_jstart.get(i);
                            *s_happiness.get_mut(i) =
                                (*s_happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
                        }
                    } else if let Some(ingress_origin) = local_access_point(
                        *s_tmode.get(i),
                        entrance,
                        detach_lane_id,
                        detach_lane_d,
                        transit_network,
                    ) {
                        *s_pos_x.get_mut(i) = ingress_origin.x;
                        *s_pos_y.get_mut(i) = ingress_origin.y;
                        *s_cur_n.get_mut(i) = *s_plan_detach_n.get(i);
                        *s_cur_e.get_mut(i) = transit_network
                            .lane_system
                            .lanes
                            .get(detach_lane_id)
                            .map(|lane| lane.edge_id)
                            .unwrap_or(usize::MAX);
                        *s_lane_id.get_mut(i) = detach_lane_id;
                        *s_lane_d.get_mut(i) = detach_lane_d;
                        *s_speed.get_mut(i) = 0.0;
                        s_path.get_mut(i).clear();
                        if crate::debug::is_traffic_enabled() {
                            traffic_log!(
                                "[ACCESS_INGRESS_ABORT] agent={} bldg={} mode={} lane={} lane_d={:.2} legal_detach={} exact_path={} ingress_origin=({:.2},{:.2}) reason=invalid-ingress-plan flags=0x{:02x}",
                                i,
                                b_id,
                                transit_mode_label(*s_tmode.get(i)),
                                detach_lane_id,
                                detach_lane_d,
                                legal_detach,
                                exact_path.is_some(),
                                ingress_origin.x,
                                ingress_origin.y,
                                *s_access_flags.get(i),
                            );
                        }
                        *s_path_idx.get_mut(i) = 0;
                        *s_transit.get_mut(i) = TRANSIT_NETWORK;
                        if sim_time >= *s_next_replan_time.get(i) {
                            if let Some(replan) = plan_network_replan(
                                *s_plan_detach_n.get(i),
                                *s_cur_e.get(i),
                                b_id,
                                *s_tmode.get(i),
                                *s_access_flags.get(i),
                                allocator,
                                transit_network,
                                graph,
                                pathfind_count,
                            ) {
                                *s_path.get_mut(i) = replan.current_path;
                                *s_path_idx.get_mut(i) =
                                    if s_path.get(i).len() >= 2 { 1 } else { 0 };
                                *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                                *s_plan_detach_lane.get_mut(i) =
                                    replan.planned_detach_lane_id as u32;
                                *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                                *s_access_flags.get_mut(i) = replan.access_flags;
                                *s_next_replan_time.get_mut(i) = 0.0;
                            } else {
                                *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                            }
                        }
                    } else {
                        *s_cur_n.get_mut(i) = *s_plan_detach_n.get(i);
                        *s_cur_e.get_mut(i) = usize::MAX;
                        *s_lane_id.get_mut(i) = usize::MAX;
                        *s_lane_d.get_mut(i) = 0.0;
                        *s_speed.get_mut(i) = 0.0;
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;
                        *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                        *s_transit.get_mut(i) = TRANSIT_NETWORK;
                    }
                }
                _ => {
                    *s_transit.get_mut(i) = TRANSIT_IN_BUILDING;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::dispatch_agents;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Verifies that `dispatch_agents` visits every index in `0..n` exactly once,
    /// both below the PAR_THRESHOLD (sequential path) and above it (parallel path).
    #[test]
    fn test_dispatch_agents_visits_each_index_once() {
        for n in [10_usize, 499, 500, 501, 600] {
            let counts: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            dispatch_agents(n, |i| {
                counts[i].fetch_add(1, Ordering::Relaxed);
            });
            for (i, c) in counts.iter().enumerate() {
                assert_eq!(
                    c.load(Ordering::Relaxed),
                    1,
                    "n={n}: index {i} was visited {} time(s), expected 1",
                    c.load(Ordering::Relaxed)
                );
            }
        }
    }
}
