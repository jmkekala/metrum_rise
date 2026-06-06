//! Junction connector entry, wait, zero-hop, and exit handling.

use super::super::super::super::{
    ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::super::super::lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane, lane_origin_node,
};
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::{
    ConnectorEntry, claim_connector_entry, junction_car_speed, junction_entry_speed,
};
use super::super::NETWORK_REPLAN_DELAY_S;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use rand::Rng;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32};

thread_local! {
    static VALID_CONNS: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

/// Control flow requested after a lane-end transition.
pub(super) enum LaneEndAction {
    /// Keep executing the movement loop normally.
    KeepMoving,
    /// Restart the loop immediately after entering a connector.
    Continue,
    /// Stop movement for this tick.
    Break,
}

/// Handles the transition after an agent reaches the end of a network or connector lane.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn handle_lane_end<R: Rng + ?Sized>(
    i: usize,
    lane_id: usize,
    speed: f32,
    sim_time: f32,
    remaining_dist: &mut f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_attach_claimed: &[AtomicBool],
    rng: &mut R,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_cur_e = &slices.cur_e;

        let lane = &transit_network.lane_system.lanes[lane_id];
        if lane.edge_id == usize::MAX {
            return exit_connector_lane(i, lane_id, *remaining_dist, transit_network, slices);
        }

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
        let should_hold_access_tail_idx = access_plan_valid && path_idx_before_lane_end >= path_len;
        if !should_hold_frontage_idx && !should_hold_access_tail_idx {
            *s_path_idx.get_mut(i) += 1;
        }
        let path_idx = *s_path_idx.get(i);

        if path_idx < path_len {
            let next_node = s_path.get(i)[path_idx];
            let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) else {
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
                return LaneEndAction::Break;
            };

            let connector_entry = VALID_CONNS.with(|v| {
                let mut connector_candidates = v.borrow_mut();
                let any_routing_valid = collect_connector_lanes_to_edge(
                    lane_id,
                    best_e,
                    transit_network,
                    &mut connector_candidates,
                );
                claim_connector_entry(
                    &mut connector_candidates,
                    any_routing_valid,
                    rng,
                    lane_buckets,
                    lane_attach_claimed,
                )
            });
            match connector_entry {
                ConnectorEntry::Enter(chosen_conn) => {
                    *s_lane_id.get_mut(i) = chosen_conn;
                    *s_lane_d.get_mut(i) = 0.0;
                    *s_transit.get_mut(i) = TRANSIT_INTERSECTION;
                    *s_cur_e.get_mut(i) = usize::MAX;
                    apply_connector_entry_speed(
                        i,
                        chosen_conn,
                        speed,
                        remaining_dist,
                        transit_network,
                        slices,
                    );
                    if crate::debug::is_traffic_enabled() {
                        let conn_lane = &transit_network.lane_system.lanes[chosen_conn];
                        let target_lane =
                            conn_lane.next_lanes.first().copied().unwrap_or(usize::MAX);
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
                            *remaining_dist,
                            *s_path_idx.get(i),
                            path_len,
                        );
                    }
                    LaneEndAction::KeepMoving
                }
                ConnectorEntry::ClaimedThisTick => {
                    *s_path_idx.get_mut(i) = path_idx_before_lane_end;
                    *s_lane_d.get_mut(i) = lane.length;
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
                    LaneEndAction::Break
                }
                ConnectorEntry::Occupied => {
                    *s_path_idx.get_mut(i) = path_idx_before_lane_end;
                    *s_lane_d.get_mut(i) = lane.length;
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
                    LaneEndAction::Break
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
                    // No connection lane exists for this turn. Clear the path so the next
                    // replan can route around the restricted junction.
                    s_path.get_mut(i).clear();
                    *s_lane_id.get_mut(i) = usize::MAX;
                    LaneEndAction::Break
                }
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
                        let connector_entry = VALID_CONNS.with(|v| {
                            let mut connector_candidates = v.borrow_mut();
                            let any_routing_valid = collect_connector_lanes_to_lane(
                                lane_id,
                                detach_lane_id,
                                transit_network,
                                &mut connector_candidates,
                            );
                            claim_connector_entry(
                                &mut connector_candidates,
                                any_routing_valid,
                                rng,
                                lane_buckets,
                                lane_attach_claimed,
                            )
                        });
                        match connector_entry {
                            ConnectorEntry::Enter(conn_lane_id) => {
                                *s_lane_id.get_mut(i) = conn_lane_id;
                                *s_lane_d.get_mut(i) = 0.0;
                                *s_transit.get_mut(i) = TRANSIT_INTERSECTION;
                                *s_cur_e.get_mut(i) = usize::MAX;
                                apply_connector_entry_speed(
                                    i,
                                    conn_lane_id,
                                    speed,
                                    remaining_dist,
                                    transit_network,
                                    slices,
                                );
                                if crate::debug::is_traffic_enabled() {
                                    let conn_lane =
                                        &transit_network.lane_system.lanes[conn_lane_id];
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
                                        *remaining_dist,
                                        *s_path_idx.get(i),
                                        path_len,
                                    );
                                }
                                return LaneEndAction::Continue;
                            }
                            ConnectorEntry::Occupied => {
                                *s_path_idx.get_mut(i) = path_idx.min(path_len);
                                *s_lane_d.get_mut(i) = lane.length;
                                *s_speed.get_mut(i) = 0.0;
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
                                return LaneEndAction::Break;
                            }
                            ConnectorEntry::ClaimedThisTick => {
                                *s_path_idx.get_mut(i) = path_idx.min(path_len);
                                *s_lane_d.get_mut(i) = lane.length;
                                *s_speed.get_mut(i) = 0.0;
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
                                return LaneEndAction::Break;
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
                        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                        *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                        *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
                        *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                        *s_access_flags.get_mut(i) = replan.access_flags;
                        *s_next_replan_time.get_mut(i) = 0.0;
                    } else {
                        *s_path_idx.get_mut(i) = 0;
                        *s_speed.get_mut(i) = 0.0;
                        *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                    }
                } else {
                    *s_path_idx.get_mut(i) = 0;
                    *s_speed.get_mut(i) = 0.0;
                }
            }
            LaneEndAction::Break
        }
    }
}

unsafe fn apply_connector_entry_speed(
    i: usize,
    conn_lane_id: usize,
    speed: f32,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;

        if *s_tmode.get(i) == MODE_CAR {
            let turn_speed = transit_network
                .lane_system
                .lanes
                .get(conn_lane_id)
                .map(|conn_lane| junction_entry_speed(*s_speed.get(i), conn_lane))
                .unwrap_or_else(|| junction_car_speed(*s_speed.get(i)));
            let time_left = if speed > 1.0e-5 {
                *remaining_dist / speed
            } else {
                0.0
            };
            *remaining_dist = turn_speed * time_left;
            *s_speed.get_mut(i) = turn_speed;
        }
    }
}

unsafe fn exit_connector_lane(
    i: usize,
    lane_id: usize,
    remaining_dist: f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_cur_e = &slices.cur_e;

        let lane = &transit_network.lane_system.lanes[lane_id];
        if !lane.next_lanes.is_empty() {
            let tgt_road_lane = lane.next_lanes[0];
            if tgt_road_lane < transit_network.lane_system.lanes.len() {
                let target_edge = transit_network.lane_system.lanes[tgt_road_lane].edge_id;
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
                return LaneEndAction::KeepMoving;
            }
            traffic_log!(
                "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} next_lane={} reason=invalid-target-lane",
                i,
                lane.node_id,
                lane_id,
                lane.length,
                tgt_road_lane,
            );
        } else {
            traffic_log!(
                "[JUNCTION_MISSING_EXIT] agent={} node={} conn_lane={} conn_len={:.2} reason=no-next-lane",
                i,
                lane.node_id,
                lane_id,
                lane.length,
            );
        }
        s_path.get_mut(i).clear();
        *s_lane_id.get_mut(i) = usize::MAX;
        LaneEndAction::Break
    }
}
