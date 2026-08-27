//! Network, immigration, and junction movement state orchestration.

mod detach;
mod junction;
mod lane_change;
mod lane_entry;
mod pose;
mod replan;

use super::super::super::{MODE_CAR, TRANSIT_INTERSECTION};
use super::super::claims::LaneClaimContext;
use super::super::slices::MovementSlices;
use super::super::traffic::{connector_turn_speed, junction_car_speed};
use crate::config::CAR_JUNCTION_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use detach::try_network_detach;
use junction::{LaneEndAction, handle_lane_end};
use lane_change::prepare_lane_change_and_overtake;
use lane_entry::{LaneEntryAction, prepare_lane_entry};
use pose::update_network_pose;
use replan::prepare_network_replan;
use std::sync::atomic::AtomicU32;

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
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) {
    unsafe {
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;

        if !prepare_network_replan(
            i,
            sim_time,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            slices,
        ) {
            return;
        }

        let speed = network_movement_speed(i, transit_network, slices);
        let mut remaining_dist = speed * delta;
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

            lane_id =
                prepare_lane_change_and_overtake(i, lane_id, lane_buckets, transit_network, slices);

            let lane = &transit_network.lane_system.lanes[lane_id];
            let dist_to_end = lane.length - *s_lane_d.get(i);
            if remaining_dist < dist_to_end {
                *s_lane_d.get_mut(i) += remaining_dist;
                remaining_dist = 0.0;

                if try_network_detach(i, lane_id, allocator, transit_network, lane_claims, slices) {
                    break;
                }
            } else {
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

        update_network_pose(i, transit_network, slices);
    }
}

unsafe fn network_movement_speed(
    i: usize,
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) -> f32 {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_lane_id = &slices.lane_id;

        if *s_tmode.get(i) != MODE_CAR {
            return 4.0;
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
