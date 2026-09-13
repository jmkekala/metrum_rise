// SPDX-License-Identifier: GPL-2.0-only

//! IDM speed update phase for live vehicle traffic.

use super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use super::lane_nav::planned_next_connector;
use super::runtime::dispatch_agents;
use super::slices::RawSlice;
use super::traffic::{
    LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_MIN_SPEED_GAIN_MS, braking_speed_for_distance,
    connector_turn_speed, idm_gap_bucket, idm_new_speed, lane_change_gap_clear,
    lane_entry_slot_clear, limit_speed_change, live_lane_bucket_transit, overtake_follow_gap,
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
            let buckets: &Vec<Vec<(f32, usize)>> = &self.lane_buckets;

            dispatch_agents(n, |i| unsafe {
                let cur_spd = *s_speed_idm.get(i);
                let transit = *s_transit_idm.get(i);
                let tmode = *s_tmode_idm.get(i);
                let cooldown = (*s_overtake_cooldown_idm.get(i) - delta).max(0.0);
                *s_overtake_cooldown_idm.get_mut(i) = cooldown;

                if !live_lane_bucket_transit(transit) || tmode != MODE_CAR {
                    *s_overtake_blocked_idm.get_mut(i) = 0.0;
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
                                        if lane_entry_slot_clear(connector_id, buckets) {
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

                // This phase reads only this agent's speed; the shared lane snapshot holds distances.
                *s_speed_idm.get_mut(i) = limit_speed_change(cur_spd, target_speed, delta);
            });
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

#[inline(always)]
fn junction_brake_lookahead_m(speed: f32) -> f32 {
    const JUNCTION_LOOKAHEAD_MARGIN_M: f32 = 20.0;
    let speed = speed.max(0.0);
    speed * speed / (2.0 * IDM_B) + JUNCTION_LOOKAHEAD_MARGIN_M
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::{MODE_WALK, TRANSIT_IN_BUILDING};
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::lanes::{Lane, LaneType};
    use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    fn speed_fixture(
        count: usize,
        mode: &str,
        reverse: bool,
    ) -> (AgentSystem, TransitNetwork, RegionGraph, usize) {
        let length = count as f32 * 16.0 + 1_000.0;
        let geometry = vec![Vector3::ZERO, Vector3::new(length, 0.0, 0.0)];
        let mut graph = RegionGraph::new();
        let start = graph.add_node(geometry[0], NodeType::Junction);
        let end = graph.add_node(geometry[1], NodeType::Junction);
        let edge = graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            fwd_lanes: 1,
            width: 7.0,
            speed_limit: 20.0,
            physical_length: length,
            geometry: geometry.clone(),
            physical_geometry: geometry.clone(),
            ..Default::default()
        });
        let mut network = TransitNetwork::new();
        for edge_id in [edge, usize::MAX] {
            network.lane_system.lanes.push(Lane {
                geometry: geometry.clone(),
                length,
                edge_id,
                lane_type: LaneType::Vehicle,
                ..Default::default()
            });
        }
        let mut agents = AgentSystem::new();
        for slot in 0..count {
            let idx = if reverse { count - 1 - slot } else { slot };
            let i = agents.spawn_housed_agent(usize::MAX, idx as f32 * 16.0, 0.0);
            let kind = if mode == "mixed" { idx % 4 } else { 0 };
            agents.transit[i] = if mode == "idle" || kind == 2 {
                TRANSIT_IN_BUILDING
            } else if kind == 3 {
                TRANSIT_INTERSECTION
            } else {
                TRANSIT_NETWORK
            };
            agents.transit_mode[i] = if kind == 1 { MODE_WALK } else { MODE_CAR };
            agents.current_lane_id[i] = usize::from(kind == 3);
            agents.current_edge[i] = if kind == 3 { usize::MAX } else { edge };
            agents.lane_distance[i] = idx as f32 * 16.0;
            agents.speed[i] = 4.0 + (idx % 16) as f32;
            agents.overtake_blocked_time_s[i] = 5.0;
            agents.overtake_cooldown_s[i] = 0.5;
        }
        let (_, live) = agents.prepare_lane_buckets_for_tick(&network, count);
        (agents, network, graph, live)
    }

    #[test]
    fn speed_updates_are_independent_of_storage_order_and_worker_count() {
        for mode in ["road", "mixed", "idle"] {
            let mut reference = None;
            for workers in [1, 8] {
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(workers)
                    .build()
                    .unwrap();
                for reverse in [false, true] {
                    let (mut agents, network, graph, live) = speed_fixture(32_768, mode, reverse);
                    let initial_speeds = agents.speed.clone();
                    pool.install(|| {
                        for _ in 0..6 {
                            agents.update_idm_speeds(0.1, &network, &graph, 32_768, live);
                        }
                    });
                    assert!(agents.overtake_cooldown_s.iter().all(|&value| value == 0.0));
                    if mode == "idle" {
                        assert_eq!(agents.speed, initial_speeds);
                        assert!(
                            agents
                                .overtake_blocked_time_s
                                .iter()
                                .all(|&value| value == 0.0)
                        );
                    } else {
                        assert_ne!(agents.speed, initial_speeds);
                    }
                    let mut state: Vec<_> = agents
                        .speed
                        .iter()
                        .zip(&agents.overtake_blocked_time_s)
                        .zip(&agents.overtake_cooldown_s)
                        .map(|((&speed, &blocked), &cooldown)| {
                            (speed.to_bits(), blocked.to_bits(), cooldown.to_bits())
                        })
                        .collect();
                    if reverse {
                        state.reverse();
                    }
                    if let Some(expected) = &reference {
                        assert_eq!(
                            &state, expected,
                            "mode={mode} workers={workers} reverse={reverse}"
                        );
                    } else {
                        reference = Some(state);
                    }
                }
            }
        }
    }

    #[test]
    #[ignore = "manual matched release timing of the isolated speed-update phase"]
    fn benchmark_speed_update() {
        use std::{hint::black_box, time::Instant};
        for count in [1_024, 16_384, 131_072, 1_048_576] {
            for mode in ["road", "mixed", "idle"] {
                let (mut agents, network, graph, live) = speed_fixture(count, mode, false);
                for _ in 0..3 {
                    agents.update_idm_speeds(0.1, &network, &graph, count, live);
                }
                let mut samples = [0.0; 21];
                for sample in &mut samples {
                    let start = Instant::now();
                    for _ in 0..8 {
                        agents.update_idm_speeds(
                            black_box(0.1),
                            black_box(&network),
                            black_box(&graph),
                            black_box(count),
                            black_box(live),
                        );
                    }
                    *sample = start.elapsed().as_secs_f64() * 1_000.0 / 8.0;
                }
                let checksum = agents
                    .speed
                    .iter()
                    .chain(&agents.overtake_blocked_time_s)
                    .chain(&agents.overtake_cooldown_s)
                    .fold(0_u64, |hash, value| {
                        hash.wrapping_mul(31)
                            .wrapping_add(u64::from(value.to_bits()))
                    });
                black_box(&agents);
                samples.sort_by(f64::total_cmp);
                eprintln!(
                    "speed_update agents={count} mode={mode} median_ms={:.6} checksum={checksum}",
                    samples[10]
                );
            }
        }
    }
}
