//! Junction lane-end orchestration for network movement.

mod enter;
mod exit;
mod zero_hop;

use super::super::super::super::{ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID};
use super::super::super::claims::LaneClaimContext;
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::MovementSlices;
use super::super::NETWORK_REPLAN_DELAY_S;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use std::sync::atomic::AtomicU32;

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
