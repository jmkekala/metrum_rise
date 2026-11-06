// SPDX-License-Identifier: GPL-2.0-only

//! Junction lane-end orchestration for network movement.

mod enter;
mod exit;
mod walk;
mod zero_hop;

pub(super) use walk::witness_lane;

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK,
};
use super::super::super::claims::LaneClaimContext;
use super::super::super::lane_nav::preferred_entry_lanes;
use super::super::super::planning::{plan_border_network_replan, plan_network_replan};
use super::super::super::slices::MovementSlices;
use super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::super::super::walk_lanes::WalkFrom;
use super::super::replan_watchdog::{
    delay_or_recover_after_network_replan_failure, has_recoverable_network_trip,
};
use super::apply_network_replan;
use super::replan::replan_after_missed_turn;
use super::walk_home::walker_lost;
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

/// A walker's planned connector that does not exist (`cause`): the lane end replans it from its
/// own lane ([`walk::replan_from_lane`]).
pub(super) struct WalkerStranded(FailureCause);

/// Handles the transition after an agent reaches the end of a network or connector lane.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
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
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_lane_id = &slices.lane_id;
        let s_cur_e = &slices.cur_e;

        let lane = &transit_network.lane_system.lanes[lane_id];
        if lane.edge_id == usize::MAX {
            return exit::exit_connector_lane(
                i,
                lane_id,
                *remaining_dist,
                transit_network,
                lane_buckets,
                lane_claims,
                slices,
            );
        }

        *s_cur_n.get_mut(i) = if lane.is_fwd {
            graph.edge(lane.edge_id).end_node
        } else {
            graph.edge(lane.edge_id).start_node
        };

        // A stranded walker waits at the end of its lane; its replan gate plans home on its
        // backoff (`ROAD-74`).
        if *s_tmode.get(i) == MODE_WALK && super::walk_home::is_stranded(i, slices) {
            *slices.lane_d.get_mut(i) = lane.length;
            *remaining_dist = 0.0;
            return LaneEndAction::Break;
        }

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
                record_failure(
                    i,
                    FailureCause::NoRoad {
                        node: *s_cur_n.get(i),
                        next_node,
                    },
                    FailureOutcome::Replans,
                    slices,
                );
                s_path.get_mut(i).clear();
                *s_lane_id.get_mut(i) = usize::MAX;
                return LaneEndAction::Break;
            };

            let node = *s_cur_n.get(i);
            // A walker takes the sidewalk its route's witness names (`ROAD-74`).
            let walk_lane = (*s_tmode.get(i) == MODE_WALK).then(|| {
                walk::witness_lane(
                    i,
                    WalkFrom::Lane(lane_id),
                    node,
                    path_idx.saturating_sub(1),
                    best_e,
                    transit_network,
                    graph,
                    slices,
                )
            });
            // A car entering a multi-lane road takes the lane its route's next lane-selective
            // movement needs (`preferred_entry_lanes`); O(1) when the road has one lane.
            let prefer = (*s_tmode.get(i) == MODE_CAR)
                .then(|| {
                    preferred_entry_lanes(
                        best_e,
                        node,
                        s_path.get(i),
                        path_idx + 1,
                        *s_access_flags.get(i),
                        *slices.planned_detach_n.get(i),
                        *slices.planned_detach_lane.get(i) as usize,
                        transit_network,
                        graph,
                    )
                })
                .flatten();
            let entry = match walk_lane {
                Some(None) => Err(WalkerStranded(FailureCause::NoTurnToEdge {
                    node,
                    from_lane: lane_id,
                    from_edge: lane.edge_id,
                    to_edge: best_e,
                })),
                _ => enter::enter_next_edge_connector(
                    i,
                    lane_id,
                    lane.edge_id,
                    best_e,
                    walk_lane.flatten(),
                    node,
                    path_idx_before_lane_end,
                    path_len,
                    speed,
                    remaining_dist,
                    transit_network,
                    lane_buckets,
                    lane_claims,
                    slices,
                    prefer,
                ),
            };
            replan_if_missed_turn(
                i,
                lane_id,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
            return entry.unwrap_or_else(|WalkerStranded(cause)| {
                walk::replan_from_lane(
                    i,
                    lane_id,
                    node,
                    path_idx_before_lane_end,
                    Some(cause),
                    sim_time,
                    remaining_dist,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                )
            });
        }

        if let Some(entry) = zero_hop::try_zero_hop_access_connector(
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
            replan_if_missed_turn(
                i,
                lane_id,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
            return entry.unwrap_or_else(|WalkerStranded(cause)| {
                walk::replan_from_lane(
                    i,
                    lane_id,
                    *s_cur_n.get(i),
                    path_idx,
                    Some(cause),
                    sim_time,
                    remaining_dist,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                )
            });
        }

        #[cfg(test)]
        probe_end_of_path(
            i,
            lane_id,
            access_plan_valid,
            sim_time,
            transit_network,
            graph,
            slices,
        );
        // A walker whose path ends short of its exit keeps its lane (`TRAFFIC-19`) and plans
        // again from its end: walks on, walks home or is stranded there. One with no plan waits
        // there for its replan gate; one with a trip but no destination is lost (`walker_lost`),
        // and one with no trip at all walks home, or with no home waits (`TRAFFIC-24`).
        if *s_tmode.get(i) == MODE_WALK {
            if access_plan_valid {
                return walk::replan_from_lane(
                    i,
                    lane_id,
                    *s_cur_n.get(i),
                    path_idx,
                    None,
                    sim_time,
                    remaining_dist,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
            *slices.lane_d.get_mut(i) = lane.length;
            *remaining_dist = 0.0;
            if *s_tgt_b.get(i) != usize::MAX {
                // Its replan gate plans from here.
            } else if has_recoverable_network_trip(i, slices) {
                walker_lost(
                    i,
                    FailureCause::NoPlan,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            } else {
                super::walk_home::head_home_without_trip(
                    i,
                    *s_cur_n.get(i),
                    lane_id,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
            return LaneEndAction::Break;
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
                        None,
                    ) {
                        apply_network_replan(i, replan, slices);
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
                        transit_network,
                        graph,
                        pathfind_count,
                        "freight-border-junction",
                        FailureCause::NoBorderRoute {
                            from: *s_cur_n.get(i),
                            border: border_node,
                        },
                        slices,
                    );
                }
                *s_path_idx.get_mut(i) = 0;
                *s_speed.get_mut(i) = 0.0;
                return LaneEndAction::Break;
            }
            if sim_time >= *s_next_replan_time.get(i) {
                match plan_network_replan(
                    *s_cur_n.get(i),
                    *s_cur_e.get(i),
                    None,
                    *s_tgt_b.get(i),
                    *s_tmode.get(i),
                    *s_access_flags.get(i),
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    Some((i, "network-junction")),
                    None,
                    slices.parking.and_then(|p| p.replan_entrance(*slices.parking_trip.get(i), allocator, transit_network, graph)),
                ) {
                    Ok(replan) => apply_network_replan(i, replan, slices),
                    Err(failure) => delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        "network-junction",
                        FailureCause::NoRoute(failure),
                        slices,
                    ),
                }
            } else {
                *s_path_idx.get_mut(i) = 0;
                *s_speed.get_mut(i) = 0.0;
            }
        }
        LaneEndAction::Break
    }
}

// After a lane-end entry from road lane `lane_id`: a car that took a missed turn
// (`enter::follow_missed_turn`) is on a new lane with its node path dropped, and replans now from
// the road it entered. A zero-hop entry also has an empty path; `replan_after_missed_turn` leaves
// it alone. O(1) for every other entry.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
unsafe fn replan_if_missed_turn(
    i: usize,
    lane_id: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        if *slices.tmode.get(i) == MODE_CAR
            && slices.path.get(i).is_empty()
            && *slices.lane_id.get(i) != lane_id
        {
            replan_after_missed_turn(
                i,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
        }
    }
}

// The `TRAFFIC-19` probe's count of route ends short of the exit, by agent kind, and a sample of
// each walker's state there.
#[cfg(test)]
unsafe fn probe_end_of_path(
    i: usize,
    lane_id: usize,
    access_plan_valid: bool,
    sim_time: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) {
    use super::super::super::lane_nav::{lane_origin_node, lane_terminal_node};
    use super::super::super::trip_failures::probe_tally;
    unsafe {
        let walker = *slices.tmode.get(i) == MODE_WALK;
        let kind = if walker { "walker" } else { "vehicle" };
        let plan = if access_plan_valid {
            "plan valid"
        } else {
            "no plan"
        };
        probe_tally::add_event("end of path", kind, plan);
        if !walker {
            return;
        }
        probe_tally::sample(|| {
            let lane = &transit_network.lane_system.lanes[lane_id];
            let detach = *slices.planned_detach_lane.get(i) as usize;
            format!(
                "END-OF-PATH agent {i} t {sim_time:.1} node {} lane {lane_id} edge {} fwd {} \
                 idx {} path {:?} path_idx {} detach_n {} detach_lane {detach} detach_origin {:?} \
                 detach_terminal {:?} flags {:#x} target {} activity {} next_replan {:.1} \
                 failures {}",
                *slices.cur_n.get(i),
                lane.edge_id,
                lane.is_fwd,
                lane.lane_idx,
                slices.path.get(i),
                *slices.path_idx.get(i),
                *slices.planned_detach_n.get(i),
                lane_origin_node(detach, transit_network, graph),
                lane_terminal_node(detach, transit_network, graph),
                *slices.access_flags.get(i),
                *slices.tgt_b.get(i),
                *slices.activity.get(i),
                *slices.next_replan_time.get(i),
                *slices.network_replan_failures.get(i),
            )
        });
    }
}
