//! Main simulation loop for agents: transit state machine and movement.

mod access;
mod frontage;
mod lane_nav;
mod movement;
mod planning;
mod runtime;
mod schedule;
mod slices;
mod traffic;

use super::data::AgentSystem;
use super::{
    ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_INGRESS, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

use lane_nav::planned_next_connector;
use runtime::{PAR_THRESHOLD, dispatch_agents};
use slices::{MovementSlices, RawSlice};

// ---------------------------------------------------------------------------
// IDM (Intelligent Driver Model) constants — defined in config.rs.
// ---------------------------------------------------------------------------

use crate::config::{CAR_JUNCTION_SPEED_MS, CAR_LENGTH, DEFAULT_URBAN_ROAD_SPEED_MS, IDM_S_MIN};
use traffic::{
    LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_MIN_SPEED_GAIN_MS,
    braking_speed_for_distance, connector_turn_speed, idm_gap_bucket, idm_new_speed,
    lane_change_gap_clear, lane_entry_slot_clear, limit_speed_change, live_lane_bucket_transit,
    overtake_follow_gap, planned_lane_change_target,
};

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
