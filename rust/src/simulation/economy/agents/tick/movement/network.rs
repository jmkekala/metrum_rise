// SPDX-License-Identifier: GPL-3.0-or-later

//! Network, immigration, and junction movement state orchestration.

mod detach;
mod junction;
mod lane_change;
mod lane_entry;
mod pose;
mod replan;
mod walk_home;

use super::super::super::{MODE_CAR, TRANSIT_INTERSECTION};
use super::super::access::ACCESS_LANE_D_TOLERANCE_M;
use super::super::claims::LaneClaimContext;
use super::super::slices::MovementSlices;
use super::super::traffic::{connector_turn_speed, junction_car_speed};
use crate::config::CAR_JUNCTION_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::economy::agents::walker_crowd::{Opposing, walker_lane_speed};
use crate::simulation::economy::agents::walker_gait::walker_speed;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use detach::try_network_detach;
use junction::{LaneEndAction, handle_lane_end};
use lane_change::prepare_lane_change_and_overtake;
use lane_entry::{LaneEntryAction, prepare_lane_entry, try_pending_lane_attach};
use pose::update_network_pose;
pub(super) use replan::apply_network_replan;
use replan::prepare_network_replan;
use std::sync::atomic::AtomicU32;
pub(in crate::simulation::economy::agents::tick) use walk_home::retry_after_s;
pub(super) use walk_home::walker_lost;

/// Handles network, immigration, and active junction connector movement.
///
/// Its per-agent steps (replan gates, lane entry, lane change, lane end and the junction entries
/// under it) live in sibling modules and carry `#[inline]`, which compiles them in this
/// function's codegen unit: without it, whether they inline here follows rustc's partition of
/// the whole crate into 16 units, and unrelated added code can move them out of line (roads.md
/// question 36; `ROAD-94` measured 6-17% on the movement pass from that alone).
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
    lane_buckets: &[Vec<LaneEntry>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) {
    unsafe {
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let walker = *slices.tmode.get(i) != MODE_CAR;
        // A walker's speed is what it walked this tick: 0 until it steps, so the walkers behind
        // one held anywhere below queue behind it (`walker_crowd`).
        if walker {
            *slices.speed.get_mut(i) = 0.0;
        }

        if try_pending_lane_attach(
            i,
            delta,
            transit_network,
            graph,
            lane_buckets,
            lane_claims,
            slices,
        )
            || walk_home::walk_to_sidewalk(
                i,
                delta,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            )
        {
            return;
        }
        walk_home::stand_on_sidewalk(i, sim_time, allocator, transit_network, graph, slices);
        // A walker that sets off for a sidewalk here, or in a stranded walker's retry, takes its
        // first step next tick.
        if walk_home::walking_to_sidewalk(i, slices)
            || !prepare_network_replan(
                i,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            )
            || walk_home::walking_to_sidewalk(i, slices)
        {
            return;
        }

        let speed = network_movement_speed(i, transit_network, lane_buckets, slices);
        let mut remaining_dist = speed * delta;
        let step = remaining_dist;
        let mut allow_zero_speed_network_bootstrap =
            remaining_dist <= 0.0 && *s_lane_id.get(i) == usize::MAX;

        while remaining_dist > 0.0 || allow_zero_speed_network_bootstrap {
            allow_zero_speed_network_bootstrap = false;
            if *s_lane_id.get(i) == usize::MAX {
                match prepare_lane_entry(
                    i,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    lane_buckets,
                    lane_claims,
                    slices,
                ) {
                    LaneEntryAction::Ready => {}
                    LaneEntryAction::Continue => continue,
                    LaneEntryAction::Break => break,
                }
            }

            let mut lane_id = *s_lane_id.get(i);
            if lane_id >= transit_network.lane_system.lanes.len() {
                reset_invalid_lane(i, slices);
                break;
            }

            lane_id = prepare_lane_change_and_overtake(
                i,
                lane_id,
                lane_buckets,
                transit_network,
                graph,
                lane_claims,
                slices,
            );

            let lane = &transit_network.lane_system.lanes[lane_id];
            let dist_to_end = lane.length - *s_lane_d.get(i);
            if remaining_dist < dist_to_end {
                let start_d = *s_lane_d.get(i);
                *s_lane_d.get_mut(i) += remaining_dist;
                remaining_dist = 0.0;

                if try_network_detach(
                    i,
                    lane_id,
                    start_d,
                    *s_lane_d.get(i),
                    delta,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    lane_buckets,
                    lane_claims,
                    slices,
                ) {
                    break;
                }
            } else {
                // A step that reaches the lane's end passes every detach point left on the lane,
                // the end itself included; detach there instead of leaving the lane (`TRAFFIC-15`).
                // The speed pass does not brake for a detach point, mid-lane or here: the car
                // detaches in the step that reaches it.
                if try_network_detach(
                    i,
                    lane_id,
                    *s_lane_d.get(i),
                    lane.length + ACCESS_LANE_D_TOLERANCE_M,
                    delta,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    lane_buckets,
                    lane_claims,
                    slices,
                ) {
                    break;
                }
                remaining_dist -= dist_to_end;
                match handle_lane_end(
                    i,
                    lane_id,
                    speed,
                    sim_time,
                    &mut remaining_dist,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    lane_buckets,
                    lane_claims,
                    slices,
                ) {
                    LaneEndAction::KeepMoving => {}
                    LaneEndAction::Continue => continue,
                    LaneEndAction::Break => break,
                }
            }
        }

        if walker && delta > 0.0 {
            *slices.speed.get_mut(i) = (step - remaining_dist).max(0.0) / delta;
        }
        update_network_pose(i, transit_network, slices);
    }
}

unsafe fn network_movement_speed(
    i: usize,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<LaneEntry>],
    slices: &MovementSlices,
) -> f32 {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_lane_id = &slices.lane_id;

        if *s_tmode.get(i) != MODE_CAR {
            return walker_network_speed(i, transit_network, lane_buckets, slices);
        }

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
    }
}

// Walker `i`'s speed on its lane: its pace, or the slower walker's ahead that it cannot pass.
// Reads the tick's buckets and other walkers' seeds, which no worker writes. O(log bucket).
unsafe fn walker_network_speed(
    i: usize,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<LaneEntry>],
    slices: &MovementSlices,
) -> f32 {
    unsafe {
        let pace_speed = |agent: usize| walker_speed(*slices.schedule_seed.get(agent));
        let own = pace_speed(i);
        let lane_id = *slices.lane_id.get(i);
        let lanes = &transit_network.lane_system;
        let (Some(lane), Some(bucket)) = (lanes.lanes.get(lane_id), lane_buckets.get(lane_id))
        else {
            return own;
        };
        if lane.lane_type != LaneType::Foot {
            return own;
        }
        let opposing = || {
            let id = lanes.opposing_foot_lane(lane_id)?;
            Some(Opposing {
                bucket: lane_buckets.get(id)?,
                length: lanes.lanes[id].length,
            })
        };
        walker_lane_speed(
            own,
            *slices.lane_d.get(i),
            i,
            lane.length,
            bucket,
            opposing,
            pace_speed,
        )
    }
}

unsafe fn reset_invalid_lane(i: usize, slices: &MovementSlices) {
    unsafe {
        let s_path = &slices.path;
        let s_lane_id = &slices.lane_id;
        let s_lane_change_from_lane = &slices.lane_change_from_lane;
        let s_lane_change_start_d = &slices.lane_change_start_d;
        let s_lane_change_length = &slices.lane_change_length;

        *s_lane_id.get_mut(i) = usize::MAX;
        *s_lane_change_from_lane.get_mut(i) = u32::MAX;
        *s_lane_change_start_d.get_mut(i) = 0.0;
        *s_lane_change_length.get_mut(i) = 0.0;
        s_path.get_mut(i).clear();
    }
}
