// SPDX-License-Identifier: GPL-2.0-only

//! Network lane-entry bootstrap for agents without an active lane.

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_WALK,
};
use super::super::super::claims::LaneClaimContext;
use super::super::super::lane_nav::lane_origin_node;
use super::super::super::planning::{plan_border_network_replan, plan_network_replan};
use super::super::super::slices::{MovementSlices, enter_lane};
use super::super::super::traffic::{claim_lane_entry, lane_entry_slot_clear};
use super::super::super::trip_failures::FailureCause;
use super::super::NETWORK_REPLAN_DELAY_S;
use super::super::replan_watchdog::{
    delay_or_recover_after_network_replan_failure, has_recoverable_network_trip,
    reset_network_replan_watchdog,
};
use super::apply_network_replan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::determinism::stable_index;
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
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> LaneEntryAction {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_cur_b = &slices.cur_b;
        let s_tgt_b = &slices.tgt_b;
        let s_freight_target_border_node = &slices.freight_target_border_node;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_lane_id = &slices.lane_id;
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
                && let Some(detach_origin) =
                    lane_origin_node(planned_detach_lane_id, transit_network, graph)
                && detach_origin == *s_plan_detach_n.get(i)
            {
                if *s_tmode.get(i) != MODE_WALK
                    && (!lane_entry_slot_clear(planned_detach_lane_id, lane_buckets)
                        || !claim_lane_entry(i, planned_detach_lane_id, lane_claims))
                {
                    *s_speed.get_mut(i) = 0.0;
                    return LaneEntryAction::Break;
                }
                let parent_edge = transit_network.lane_system.lanes[planned_detach_lane_id].edge_id;
                enter_lane!(
                    slices,
                    i,
                    transit_network,
                    planned_detach_lane_id,
                    0.0,
                    *s_cur_n.get(i)
                );
                if *s_speed.get(i) == 0.0 {
                    *s_speed.get_mut(i) = graph.edge(parent_edge).speed_limit;
                }
                reset_network_replan_watchdog(i, slices);
                return LaneEntryAction::Continue;
            }

            if access_plan_valid {
                if (*s_access_flags.get(i) & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
                    let border_node = *s_freight_target_border_node.get(i);
                    if *s_cur_n.get(i) == border_node {
                        *s_speed.get_mut(i) = 0.0;
                        return LaneEntryAction::Break;
                    }
                    if sim_time < *s_next_replan_time.get(i) {
                        *s_speed.get_mut(i) = 0.0;
                        return LaneEntryAction::Break;
                    }
                    if let Some(replan) = plan_border_network_replan(
                        *s_cur_n.get(i),
                        *s_cur_e.get(i),
                        border_node,
                        graph,
                        transit_network,
                        pathfind_count,
                    ) {
                        apply_network_replan(i, replan, slices);
                        return LaneEntryAction::Continue;
                    }
                    delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        graph,
                        "freight-border-lane-entry",
                        FailureCause::NoBorderRoute {
                            from: *s_cur_n.get(i),
                            border: border_node,
                        },
                        slices,
                    );
                    return LaneEntryAction::Break;
                }
                if sim_time < *s_next_replan_time.get(i) {
                    *s_speed.get_mut(i) = 0.0;
                    return LaneEntryAction::Break;
                }
                let cur_n = *s_cur_n.get(i);
                if cur_n == u32::MAX {
                    delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        graph,
                        "missing-current-node-lane-entry",
                        FailureCause::NoPosition,
                        slices,
                    );
                    return LaneEntryAction::Break;
                }
                match plan_network_replan(
                    cur_n,
                    *s_cur_e.get(i),
                    *s_tgt_b.get(i),
                    *s_tmode.get(i),
                    *s_access_flags.get(i),
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    Some((i, "network-lane-entry")),
                ) {
                    Ok(replan) => apply_network_replan(i, replan, slices),
                    Err(failure) => {
                        delay_or_recover_after_network_replan_failure(
                            i,
                            sim_time,
                            allocator,
                            graph,
                            "network-lane-entry",
                            FailureCause::NoRoute(failure),
                            slices,
                        );
                        return LaneEntryAction::Break;
                    }
                }
                if s_path.get(i).is_empty() {
                    return LaneEntryAction::Continue;
                }
            } else {
                if has_recoverable_network_trip(i, slices) {
                    delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        graph,
                        "missing-access-plan-lane-entry",
                        FailureCause::NoPlan,
                        slices,
                    );
                } else {
                    *s_speed.get_mut(i) = 0.0;
                    *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
                    reset_network_replan_watchdog(i, slices);
                }
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
                                let start = stable_index(choice_seed, valid_lanes.len());
                                let chosen = (0..valid_lanes.len())
                                    .map(|offset| valid_lanes[(start + offset) % valid_lanes.len()])
                                    .find(|&lane| {
                                        *s_tmode.get(i) == MODE_WALK
                                            || (lane_entry_slot_clear(lane, lane_buckets)
                                                && claim_lane_entry(i, lane, lane_claims))
                                    });
                                let Some(chosen) = chosen else {
                                    // A valid route waits at its node, preserving cargo, target,
                                    // schedule and retry state. Occupancy is never overfilled.
                                    *s_speed.get_mut(i) = 0.0;
                                    return;
                                };
                                enter_lane!(
                                    slices,
                                    i,
                                    transit_network,
                                    chosen,
                                    0.0,
                                    *s_cur_n.get(i)
                                );
                                *s_cur_b.get_mut(i) = usize::MAX;
                                // Seed speed from edge limit on first lane entry.
                                if *s_speed.get(i) == 0.0 {
                                    *s_speed.get_mut(i) = graph.edge(best_e).speed_limit;
                                }
                                reset_network_replan_watchdog(i, slices);
                            } else {
                                s_path.get_mut(i).clear();
                            }
                        });
                        if s_path.get(i).is_empty() || *s_lane_id.get(i) == usize::MAX {
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
