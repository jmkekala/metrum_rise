// SPDX-License-Identifier: GPL-2.0-only

//! Walkers at a junction follow their route's sidewalks (`ROAD-74`).
//!
//! A walker reaching a node takes the connector into the sidewalk its route's lane witness
//! ([`fill_walk_lanes`]) names for the next road, never any connector onto it. Where that
//! connector no longer exists (a crossing removed under it), the walker waits at the mouth and
//! plans again from its own lane through the normal planner; with no route it gives the trip up
//! for a walk home or is stranded (`walk_home`). It never leaves its lane to do so and no
//! crossing is made for it.

use super::super::super::super::super::{ACCESS_PLAN_VALID, MODE_WALK};
use super::super::super::super::planning::plan_network_replan;
use super::super::super::super::slices::MovementSlices;
use super::super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::super::super::super::walk_lanes::{NO_WITNESS, PASSED_HOP, WalkFrom, fill_walk_lanes};
use super::super::apply_network_replan;
use super::super::walk_home::give_up_trip;
use super::LaneEndAction;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use std::sync::atomic::AtomicU32;

/// The sidewalk lane walker `i`'s witness takes on hop `hop` of its path, the road from `node`
/// (its `path[hop]`) to the next node over `to_edge`, when a foot connector from road lane
/// `from_lane` leads into it. A route without a witness gets one first, from `from_lane`
/// (`WalkFrom::Lane`) or, for a walker with no lane yet, from the node. `None` when the witness
/// has no step here. O(connectors of `from_lane`), plus the witness once per route.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(in super::super) unsafe fn witness_lane(
    i: usize,
    from: WalkFrom,
    node: u32,
    hop: usize,
    to_edge: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Option<usize> {
    unsafe {
        let path = slices.path.get(i);
        if path.get(hop) != Some(&node) || hop + 1 >= path.len() {
            return None;
        }
        let walk_lanes = slices.walk_lanes.get_mut(i);
        if walk_lanes.len() + 1 != path.len() || walk_lanes[hop] == PASSED_HOP {
            let detach = if *slices.access_flags.get(i) & ACCESS_PLAN_VALID != 0 {
                match *slices.planned_detach_lane.get(i) {
                    u32::MAX => usize::MAX,
                    lane => lane as usize,
                }
            } else {
                usize::MAX
            };
            fill_walk_lanes(
                path,
                hop,
                from,
                detach,
                *slices.schedule_seed.get(i),
                transit_network,
                graph,
                walk_lanes,
            );
        }
        let lane_idx = walk_lanes[hop];
        if lane_idx == NO_WITNESS {
            return None;
        }
        let lanes = &transit_network.lane_system.lanes;
        let is_target = |lane: usize| {
            lanes.get(lane).is_some_and(|value| {
                value.edge_id == to_edge
                    && value.lane_type == LaneType::Foot
                    && value.lane_idx == lane_idx
            })
        };
        match from {
            WalkFrom::Lane(from_lane) => lanes
                .get(from_lane)?
                .next_lanes
                .iter()
                .filter_map(|&conn| {
                    let conn = lanes.get(conn)?;
                    (conn.edge_id == usize::MAX && conn.lane_type == LaneType::Foot)
                        .then(|| conn.next_lanes.first().copied())
                        .flatten()
                })
                .find(|&target| is_target(target)),
            WalkFrom::Node => transit_network
                .lane_system
                .edge_lanes
                .get(&to_edge)?
                .iter()
                .copied()
                .find(|&lane| {
                    is_target(lane)
                        && super::super::super::super::lane_nav::lane_origin_node(
                            lane,
                            transit_network,
                            graph,
                        ) == Some(node)
                }),
        }
    }
}

/// Walker `i` stands at the end of road lane `lane_id` at `node`, where its witness has no step
/// (`cause`) or its path ends short of its exit (`None`, `TRAFFIC-19`): it waits at the mouth and
/// plans again from its own lane. A new route is walked from the next tick (one `cause` record,
/// outcome `Replans`; none at a path's end); with none it gives the trip up ([`give_up_trip`]).
/// No teleport and no crossing of its own.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn replan_from_lane(
    i: usize,
    lane_id: usize,
    node: u32,
    path_idx_before_lane_end: usize,
    cause: Option<FailureCause>,
    sim_time: f32,
    remaining_dist: &mut f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> LaneEndAction {
    unsafe {
        let lane = &transit_network.lane_system.lanes[lane_id];
        *slices.path_idx.get_mut(i) = path_idx_before_lane_end;
        *slices.lane_d.get_mut(i) = lane.length;
        *remaining_dist = 0.0;
        if sim_time < *slices.next_replan_time.get(i) {
            return LaneEndAction::Break;
        }
        match plan_network_replan(
            node,
            lane.edge_id,
            Some(lane_id),
            *slices.tgt_b.get(i),
            MODE_WALK,
            *slices.access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "walk-witness")),
        ) {
            Ok(replan) => {
                if let Some(cause) = cause {
                    record_failure(i, cause, FailureOutcome::Replans, slices);
                }
                apply_network_replan(i, replan, slices);
            }
            Err(failure) => {
                give_up_trip(
                    i,
                    node,
                    lane_id,
                    FailureCause::NoRoute(failure),
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
        }
        LaneEndAction::Break
    }
}
