// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: speed.rs
//  script_path: rust/src/simulation/economy/agents/tick/speed.rs
//  module_name: speed
//  version: 0.1.0
//  description: IDM speed update phase for live vehicle traffic.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! IDM speed update phase for live vehicle traffic.

use super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use super::lane_nav::planned_next_connector;
use super::runtime::dispatch_agents;
use super::slices::RawSlice;
use super::traffic::{
    LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_MIN_SPEED_GAIN_MS, braking_speed_for_distance,
    conflicting_movements_clear, connector_turn_speed, idm_gap_bucket, idm_new_speed,
    lane_change_gap_clear, lane_entry_slot_clear, limit_speed_change, live_lane_bucket_transit,
    overtake_follow_gap,
    planned_lane_change_target,
};
use crate::config::{CAR_JUNCTION_SPEED_MS, DEFAULT_URBAN_ROAD_SPEED_MS, IDM_B};
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    /// Updates car speeds with IDM, turn braking, lane-change gating, and overtake timers.
    pub(super) fn update_idm_speeds(
        &mut self,
        delta: f32,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        n: usize,
        live_lane_agent_count: usize,
    ) {
        if live_lane_agent_count == 0 {
            self.reset_inactive_traffic_timers(delta, n);
            return;
        }

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

                // A car already inside a junction is committed. Its speed comes
                // from the turn geometry and the car ahead of it on the same
                // connector, never from a conflict rule: holding it here strands
                // a vehicle in the middle of the box, blocking every movement
                // that crosses it, while traffic that does not test it drives
                // through. Yielding happens before entry, not after.
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
                        let dist_to_end = (lane.length - my_d).max(0.0);
                        if lane.edge_id != usize::MAX
                            && dist_to_end <= junction_brake_lookahead_m(cur_spd.max(v_max))
                        {
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
                                        // Braking must ask the same question entry
                                        // asks. Checking only this connector's own
                                        // bucket makes a car roll up to the mouth
                                        // expecting to go, then be refused by the
                                        // conflict rule, so it halts on the line
                                        // and never restarts while traffic that
                                        // never tests it drives past.
                                        let clear = lane_entry_slot_clear(connector_id, buckets)
                                            && conflicting_movements_clear(
                                                connector.node_id,
                                                connector_id,
                                                &transit_network.lane_system,
                                                buckets,
                                                |a| *s_speed_idm.get(a),
                                            );
                                        if clear {
                                            connector_turn_speed(connector)
                                        } else {
                                            0.0
                                        }
                                    })
                                    .unwrap_or(CAR_JUNCTION_SPEED_MS);
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
    }

    fn reset_inactive_traffic_timers(&mut self, delta: f32, n: usize) {
        let s_overtake_blocked = RawSlice::new(&mut self.agents.overtake_blocked_time_s);
        let s_overtake_cooldown = RawSlice::new(&mut self.agents.overtake_cooldown_s);

        dispatch_agents(n, |i| unsafe {
            if *s_overtake_blocked.get(i) != 0.0 {
                *s_overtake_blocked.get_mut(i) = 0.0;
            }
            let cooldown = *s_overtake_cooldown.get(i);
            if cooldown > 0.0 {
                *s_overtake_cooldown.get_mut(i) = (cooldown - delta).max(0.0);
            }
        });
    }
}

// ========================================================================
// BRAKING DISTANCE
// ========================================================================

#[inline(always)]
fn junction_brake_lookahead_m(speed: f32) -> f32 {
    const JUNCTION_LOOKAHEAD_MARGIN_M: f32 = 20.0;
    let speed = speed.max(0.0);
    speed * speed / (2.0 * IDM_B) + JUNCTION_LOOKAHEAD_MARGIN_M
}
