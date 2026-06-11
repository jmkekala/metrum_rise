//! Network lane-entry bootstrap for agents without an active lane.

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_WALK,
    TRANSIT_NETWORK,
};
use super::super::super::lane_nav::lane_origin_node;
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::deterministic_choice_index;
use super::super::NETWORK_REPLAN_DELAY_S;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use std::cell::RefCell;
use std::sync::atomic::AtomicU32;

thread_local! {
    static VALID_LANES: RefCell<Vec<usize>> = RefCell::new(Vec::with_capacity(8));
}

/// Control flow requested by lane-entry preparation.
pub(super) enum LaneEntryAction {
    /// Continue movement along the current lane in this loop iteration.
    Ready,
    /// Restart the movement loop after successful zero-hop or empty-path repair.
    Continue,
    /// Stop movement for this tick.
    Break,
}

/// Prepares an agent with no active lane by repairing its path or choosing a first lane.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn prepare_lane_entry(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> LaneEntryAction {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_cur_b = &slices.cur_b;
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
                        return LaneEntryAction::Continue;
                    }
                }
            }

            if access_plan_valid {
                if (*s_access_flags.get(i) & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
                    *s_speed.get_mut(i) = 0.0;
                    return LaneEntryAction::Break;
                }
                if sim_time < *s_next_replan_time.get(i) {
                    *s_speed.get_mut(i) = 0.0;
                    return LaneEntryAction::Break;
                }
                let cur_n = *s_cur_n.get(i);
                if cur_n == u32::MAX {
                    *s_speed.get_mut(i) = 0.0;
                    *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                    return LaneEntryAction::Break;
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
                    return LaneEntryAction::Break;
                }
                if s_path.get(i).is_empty() {
                    return LaneEntryAction::Continue;
                }
            } else {
                *s_speed.get_mut(i) = 0.0;
                *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                return LaneEntryAction::Break;
            }
        }

        if *s_lane_id.get(i) == usize::MAX {
            let path = s_path.get(i);
            let idx = *s_path_idx.get(i);
            if idx < path.len() {
                let next_node = path[idx];
                if let Some(best_e) = graph.get_edge_between_nodes(*s_cur_n.get(i), next_node) {
                    let edge = graph.edge(best_e);
                    let is_fwd = edge.start_node == *s_cur_n.get(i);
                    if let Some(edge_lanes) = transit_network.lane_system.edge_lanes.get(&best_e) {
                        VALID_LANES.with(|v| {
                            let mut valid_lanes = v.borrow_mut();
                            valid_lanes.clear();
                            for &l_id in edge_lanes {
                                let lane = &transit_network.lane_system.lanes[l_id];
                                if lane.is_fwd == is_fwd {
                                    if *s_tmode.get(i) == MODE_WALK {
                                        if lane.lane_type == LaneType::Foot {
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
                                    } else if lane.lane_type == LaneType::Vehicle {
                                        valid_lanes.push(l_id);
                                    }
                                }
                            }
                            if !valid_lanes.is_empty() {
                                let choice_seed =
                                    lane_entry_choice_seed(i, *s_cur_n.get(i), next_node, best_e);
                                let chosen = valid_lanes
                                    [deterministic_choice_index(choice_seed, valid_lanes.len())];
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
                            return LaneEntryAction::Break;
                        }
                    } else {
                        s_path.get_mut(i).clear();
                        return LaneEntryAction::Break;
                    }
                } else {
                    s_path.get_mut(i).clear();
                    return LaneEntryAction::Break;
                }
            } else {
                return LaneEntryAction::Break;
            }
        }

        LaneEntryAction::Ready
    }
}

#[inline(always)]
fn lane_entry_choice_seed(agent_idx: usize, from_node: u32, to_node: u32, edge_id: usize) -> u64 {
    (agent_idx as u64)
        ^ ((from_node as u64) << 17)
        ^ ((to_node as u64) << 33)
        ^ ((edge_id as u64) << 1)
}
