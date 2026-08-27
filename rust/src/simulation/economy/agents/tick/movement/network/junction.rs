//! Junction lane-end orchestration for network movement.

mod enter;
mod exit;
mod zero_hop;

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR,
};
use super::super::super::claims::LaneClaimContext;
use super::super::super::planning::{
    REROUTE_INTERVAL_S, plan_border_network_replan, plan_network_replan, price_node_path,
    reroute_is_worthwhile,
};
use super::super::super::slices::MovementSlices;
use super::super::replan_watchdog::{
    delay_or_recover_after_network_replan_failure, reset_network_replan_watchdog,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use std::sync::atomic::AtomicU32;

/// Reconsiders an agent's route against observed congestion.
///
/// Called at a junction while the agent still holds a valid path. Prices the
/// remainder of the current route with the live congested metric, asks the
/// router for a fresh one, and swaps only when the candidate beats the
/// remainder by [`REROUTE_IMPROVEMENT_FRACTION`]. Anything less is left alone,
/// because two routes of near-equal cost would otherwise trade the vehicle back
/// and forth every time they were compared.
///
/// Rate limited per agent by [`REROUTE_INTERVAL_S`]. A failed pathfind changes
/// nothing: the agent keeps the route it has, which is the correct outcome when
/// the alternative is unknown.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
unsafe fn try_congestion_reroute(
    i: usize,
    sim_time: f32,
    path_idx: usize,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let s_next_reroute = &slices.next_reroute_time;
        if sim_time < *s_next_reroute.get(i) {
            return;
        }
        *s_next_reroute.get_mut(i) = sim_time + REROUTE_INTERVAL_S;

        // Only a vehicle can take a different road. A pedestrian's route is its
        // own network, and freight to a border holds a plan this must not touch.
        if *slices.tmode.get(i) != MODE_CAR
            || (*slices.access_flags.get(i) & ACCESS_FREIGHT_BORDER_DESTINATION) != 0
        {
            return;
        }

        let Some(current_cost) = price_node_path(slices.path.get(i), path_idx, graph) else {
            return;
        };
        if current_cost <= 0.0 {
            return;
        }

        let Some(replan) = plan_network_replan(
            *slices.cur_n.get(i),
            *slices.cur_e.get(i),
            *slices.tgt_b.get(i),
            *slices.tmode.get(i),
            *slices.access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            None,
        ) else {
            return;
        };

        let Some(candidate_cost) = price_node_path(&replan.current_path, 0, graph) else {
            return;
        };
        if !reroute_is_worthwhile(current_cost, candidate_cost) {
            return;
        }

        traffic_log!(
            "[CONGESTION_REROUTE] agent={} node={} current_cost={:.1} candidate_cost={:.1} old_len={} new_len={}",
            i,
            *slices.cur_n.get(i),
            current_cost,
            candidate_cost,
            slices.path.get(i).len(),
            replan.current_path.len(),
        );

        // The new path starts at the current node, so the agent resumes from
        // index 1 exactly as it would after a recovery replan.
        *slices.path.get_mut(i) = replan.current_path;
        *slices.path_idx.get_mut(i) = if slices.path.get(i).len() >= 2 { 1 } else { 0 };
        *slices.planned_detach_n.get_mut(i) = replan.planned_detach_node;
        *slices.planned_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
        *slices.planned_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
        *slices.access_flags.get_mut(i) = replan.access_flags;
    }
}

/// Holds an agent at a controlled junction until its control admits it.
///
/// Returns `true` when the agent must wait this tick. The agent is pinned at
/// the end of its approach lane and its path index is rewound, which is the
/// same hold a connector-occupied wait performs, so a signal and a queue are
/// indistinguishable to everything downstream.
///
/// A signal holds without a duration and is re-tested every tick. A priority
/// sign holds for a fixed delay stamped on arrival, so a stop arm pays its
/// halt once rather than on every tick it spends waiting.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
unsafe fn hold_at_junction_control(
    i: usize,
    lane_id: usize,
    from_edge: usize,
    node_id: u32,
    sim_time: f32,
    path_idx_before_lane_end: usize,
    path_len: usize,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        // Resolve merged node ids before reading control. Building a junction
        // splits and merges nodes, and the id an agent carries can be one that
        // was merged away; `node_aliases` chains it to the survivor. Reading the
        // raw id lands on a different node, which reports uncontrolled, and
        // every car sails through a red light.
        let node_id = graph.get_valid_node(node_id);
        if (node_id as usize) >= graph.node_count() {
            return false;
        }
        let control = &graph.node(node_id).control;
        if control.is_uncontrolled() {
            return false;
        }

        let s_release = &slices.junction_release_time;
        let held_release = *s_release.get(i);

        // A delay stamped on a previous tick runs to completion first.
        if held_release > f32::MIN && sim_time < held_release {
            hold_agent_at_stop_line(
                i,
                lane_id,
                path_idx_before_lane_end,
                remaining_dist,
                transit_network,
                slices,
            );
            traffic_log!(
                "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} path_idx={}/{} reason=priority-delay",
                i,
                node_id,
                lane_id,
                from_edge,
                path_idx_before_lane_end,
                path_len,
            );
            return true;
        }

        match control.entry_hold_s(from_edge, sim_time) {
            // Red or amber: no fixed duration, re-tested next tick.
            None => {
                *s_release.get_mut(i) = f32::MIN;
                hold_agent_at_stop_line(
                    i,
                    lane_id,
                    path_idx_before_lane_end,
                    remaining_dist,
                    transit_network,
                    slices,
                );
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} path_idx={}/{} reason=signal-red",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    path_idx_before_lane_end,
                    path_len,
                );
                true
            }
            // Already served its delay, or never owed one.
            Some(delay) if delay <= 0.0 || held_release > f32::MIN => {
                *s_release.get_mut(i) = f32::MIN;
                false
            }
            // First arrival at a yield or stop arm: stamp the release time.
            Some(delay) => {
                *s_release.get_mut(i) = sim_time + delay;
                hold_agent_at_stop_line(
                    i,
                    lane_id,
                    path_idx_before_lane_end,
                    remaining_dist,
                    transit_network,
                    slices,
                );
                traffic_log!(
                    "[JUNCTION_WAIT] agent={} node={} lane={} from_edge={} path_idx={}/{} reason=priority-arrive",
                    i,
                    node_id,
                    lane_id,
                    from_edge,
                    path_idx_before_lane_end,
                    path_len,
                );
                true
            }
        }
    }
}

/// Pins an agent at the end of its approach lane without advancing its path.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
unsafe fn hold_agent_at_stop_line(
    i: usize,
    lane_id: usize,
    path_idx_before_lane_end: usize,
    remaining_dist: &mut f32,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) {
    unsafe {
        *slices.path_idx.get_mut(i) = path_idx_before_lane_end;
        *slices.lane_d.get_mut(i) = transit_network.lane_system.lanes[lane_id].length;
        *remaining_dist = 0.0;
    }
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
pub(super) unsafe fn handle_lane_end(
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
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_tgt_b = &slices.tgt_b;
        let s_freight_target_border_node = &slices.freight_target_border_node;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_lane_id = &slices.lane_id;
        let s_cur_e = &slices.cur_e;

        let lane = &transit_network.lane_system.lanes[lane_id];
        if lane.edge_id == usize::MAX {
            return exit::exit_connector_lane(i, lane_id, *remaining_dist, transit_network, slices);
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
            // A route chosen at departure is a prediction. Revising it here is
            // what lets congestion push back on the traffic feeding it.
            try_congestion_reroute(
                i,
                sim_time,
                path_idx,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
            let path_len = s_path.get(i).len();
            if path_idx >= path_len {
                *s_lane_id.get_mut(i) = usize::MAX;
                return LaneEndAction::Break;
            }

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

            if hold_at_junction_control(
                i,
                lane_id,
                lane.edge_id,
                *s_cur_n.get(i),
                sim_time,
                path_idx_before_lane_end,
                path_len,
                remaining_dist,
                transit_network,
                graph,
                slices,
            ) {
                return LaneEndAction::Break;
            }

            return enter::enter_next_edge_connector(
                i,
                lane_id,
                lane.edge_id,
                best_e,
                *s_cur_n.get(i),
                path_idx_before_lane_end,
                path_len,
                speed,
                remaining_dist,
                transit_network,
                lane_buckets,
                lane_claims,
                slices,
            );
        }

        if let Some(action) = zero_hop::try_zero_hop_access_connector(
            i,
            lane_id,
            lane.edge_id,
            path_idx,
            path_len,
            speed,
            remaining_dist,
            transit_network,
            graph,
            lane_buckets,
            lane_claims,
            slices,
        ) {
            return action;
        }

        s_path.get_mut(i).clear();
        *s_lane_id.get_mut(i) = usize::MAX;
        if access_plan_valid {
            if (*s_access_flags.get(i) & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
                let border_node = *s_freight_target_border_node.get(i);
                if *s_cur_n.get(i) != border_node && sim_time >= *s_next_replan_time.get(i) {
                    if let Some(replan) = plan_border_network_replan(
                        *s_cur_n.get(i),
                        lane.edge_id,
                        border_node,
                        graph,
                        transit_network,
                        pathfind_count,
                    ) {
                        *s_path.get_mut(i) = replan.current_path;
                        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                        *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                        *s_plan_detach_lane.get_mut(i) = u32::MAX;
                        *s_plan_detach_lane_d.get_mut(i) = 0.0;
                        *s_access_flags.get_mut(i) = replan.access_flags;
                        *s_next_replan_time.get_mut(i) = 0.0;
                        reset_network_replan_watchdog(i, slices);
                        traffic_log!(
                            "[FREIGHT_BORDER_REPLAN] agent={} node={} border_node={} incoming_edge={} path_idx={}/{} path={:?}",
                            i,
                            *s_cur_n.get(i),
                            border_node,
                            lane.edge_id,
                            *s_path_idx.get(i),
                            s_path.get(i).len(),
                            s_path.get(i),
                        );
                        return LaneEndAction::Break;
                    }
                    delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        graph,
                        "freight-border-junction",
                        slices,
                    );
                }
                *s_path_idx.get_mut(i) = 0;
                *s_speed.get_mut(i) = 0.0;
                return LaneEndAction::Break;
            }
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
                    Some((i, "network-junction")),
                ) {
                    *s_path.get_mut(i) = replan.current_path;
                    *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                    *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                    *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
                    *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                    *s_access_flags.get_mut(i) = replan.access_flags;
                    *s_next_replan_time.get_mut(i) = 0.0;
                    reset_network_replan_watchdog(i, slices);
                } else {
                    delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        graph,
                        "network-junction",
                        slices,
                    );
                }
            } else {
                *s_path_idx.get_mut(i) = 0;
                *s_speed.get_mut(i) = 0.0;
            }
        }
        LaneEndAction::Break
    }
}
