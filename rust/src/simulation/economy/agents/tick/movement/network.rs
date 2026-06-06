//! Network, immigration, and junction movement state handling.

use super::super::super::{
    ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR, MODE_WALK, TRANSIT_ACCESS_INGRESS,
    TRANSIT_IMMIGRATING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::super::access::{local_access_point, local_access_side_label, planned_detach_is_legal};
use super::super::lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane, lane_origin_node,
    lane_terminal_node,
};
use super::super::planning::{plan_immigration_trip, plan_network_replan};
use super::super::slices::MovementSlices;
use super::super::traffic::{
    ConnectorEntry, LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_COOLDOWN_S,
    OVERTAKE_DETACH_BUFFER_M, OVERTAKE_EDGE_BUFFER_M, OVERTAKE_MIN_GAP_GAIN_M,
    OVERTAKE_RETURN_TARGET_GAP_M, OVERTAKE_STUCK_TIME_S, OVERTAKE_TARGET_AHEAD_GAP_M,
    claim_connector_entry, connector_turn_speed, cruise_lane_return_target, idm_gap_bucket,
    junction_car_speed, junction_entry_speed, lane_change_gap_clear, lane_change_length_for_speed,
    overtaking_lane_target, planned_detach_distance_on_current_edge, planned_lane_change_target,
};
use super::NETWORK_REPLAN_DELAY_S;
use crate::config::CAR_JUNCTION_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use godot::prelude::*;
use rand::Rng;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

thread_local! {
    static VALID_LANES: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

/// Handles network, immigration, and active junction connector movement.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn handle_network_movement(
    i: usize,
    delta: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    lane_buckets: &Vec<Vec<(f32, usize)>>,
    lane_attach_claimed: &Vec<AtomicBool>,
    slices: &MovementSlices,
) {
    let mut rng = rand::thread_rng();

    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_cur_b = &slices.cur_b;
        let s_tgt_b = &slices.tgt_b;
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
                    *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
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
                    } else {
                        s_path.get_mut(i).clear();
                        *s_path_idx.get_mut(i) = 0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
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
                let access_plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
                let zero_hop_node_path = (*s_access_flags.get(i) & ACCESS_ZERO_HOP_NODE_PATH) != 0;
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
                            let parent_edge =
                                transit_network.lane_system.lanes[planned_detach_lane_id].edge_id;
                            *s_cur_e.get_mut(i) = parent_edge;
                            *s_lane_id.get_mut(i) = planned_detach_lane_id;
                            *s_lane_d.get_mut(i) = 0.0;
                            if *s_speed.get(i) == 0.0 {
                                *s_speed.get_mut(i) = graph.edge(parent_edge).speed_limit;
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
                        *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
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
                    if let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) {
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
                                            if lane.lane_type
                                                == crate::simulation::network::lanes::LaneType::Foot
                                            {
                                                let b_idx = *s_cur_b.get(i);
                                                if b_idx != usize::MAX
                                                    && b_idx < allocator.buildings.len()
                                                {
                                                    let b_side = allocator.buildings[b_idx].side;
                                                    let lane_side =
                                                        if lane.lane_idx > 0 { 1 } else { -1 };
                                                    if lane_side == b_side {
                                                        valid_lanes.push(l_id);
                                                    }
                                                } else {
                                                    valid_lanes.push(l_id);
                                                }
                                            }
                                        } else if lane.lane_type
                                            == crate::simulation::network::lanes::LaneType::Vehicle
                                        {
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
                let finish_d = *s_lane_change_start_d.get(i) + *s_lane_change_length.get(i);
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
                    let target_lane = &transit_network.lane_system.lanes[target_lane_id];
                    let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                    let available_m = (source_lane.length - *s_lane_d.get(i))
                        .min(*s_plan_detach_lane_d.get(i) - *s_lane_d.get(i));
                    let maneuver_length = lane_change_length_for_speed(*s_speed.get(i)).min(
                        (available_m - LANE_CHANGE_FINISH_EPS_M).max(LANE_CHANGE_MIN_LENGTH_M),
                    );
                    let gap_clear = lane_buckets
                        .get(target_lane_id)
                        .map(|bucket| lane_change_gap_clear(bucket, lane_d, *s_speed.get(i)))
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
                let planned_target_pending = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
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

                    if source_lane.edge_id != usize::MAX && enough_road_left && far_from_detach {
                        let current_gap = lane_buckets
                            .get(lane_id)
                            .map(|bucket| idm_gap_bucket(bucket, *s_lane_d.get(i)))
                            .unwrap_or(f32::MAX);
                        let overtake_target =
                            if *s_overtake_blocked_time.get(i) >= OVERTAKE_STUCK_TIME_S {
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
                            let target_lane = &transit_network.lane_system.lanes[target_lane_id];
                            let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                            let target_gap = lane_buckets
                                .get(target_lane_id)
                                .map(|bucket| idm_gap_bucket(bucket, lane_d))
                                .unwrap_or(f32::MAX);
                            let target_clear = lane_buckets
                                .get(target_lane_id)
                                .map(|bucket| {
                                    lane_change_gap_clear(bucket, lane_d, *s_speed.get(i))
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

                let access_plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
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
                            let target_entrance = allocator.entrances.get(*s_tgt_b.get(i));
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
                    let access_plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
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
                                    *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                                    *s_plan_detach_lane.get_mut(i) =
                                        replan.planned_detach_lane_id as u32;
                                    *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
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
                            let target_edge =
                                transit_network.lane_system.lanes[tgt_road_lane].edge_id;
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
        if current_lane != usize::MAX && current_lane < transit_network.lane_system.lanes.len() {
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
                    let jitter = (f32::sin(i as f32 * 4.0) + f32::cos(i as f32 * 7.0)) * 0.7;
                    out += normal * jitter;
                }
                *s_pos_x.get_mut(i) = out.x;
                *s_pos_y.get_mut(i) = out.z;
            }
        }
    }
}
