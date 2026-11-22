// SPDX-License-Identifier: GPL-3.0-or-later

//! Access-egress and access-ingress movement state handling.

mod egress;
mod ingress;

use super::super::access::{advance_along_local_access_path, step_out_along_local_access_path};
use super::super::slices::MovementSlices;
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, FREIGHT_YARD_SPEED_MS};
use crate::simulation::economy::agents::walker_gait::walker_speed;
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, MODE_CAR, TRANSIT_IN_BUILDING,
};
use godot::prelude::Vector2;

pub(super) use egress::handle_access_egress;
pub(super) use ingress::handle_access_ingress;

// Point `k` of a walked polyline for the traffic log, the origin past its end, as the fixed
// arrays' unused entries read.
fn point(path: &[Vector2], k: usize) -> Vector2 {
    path.get(k).copied().unwrap_or(Vector2::ZERO)
}

/// Agent `i`'s step this tick along the polyline `points` it walks or drives off the lanes (a
/// door's access path, a bay's or a dock's manoeuvre, the way to a sidewalk): a vehicle at the
/// driveway speed, a carrier on its dock's route at the yard speed, a walker at its own walking
/// speed. The one place an off-lane step's length is decided. Writes the position and returns
/// where the agent stood, where it stands now and whether it reached the polyline's end.
/// `beside_car`: the agent may stand off the polyline and first heads for the polyline's second
/// point: a walker beside the car it has just left (`PARK-35`), a truck leaving its place in the
/// line at a kerb stop or the gate it was sent on from (`PARK-36`). O(points), no allocation.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline]
pub(super) unsafe fn step_along_access_path(
    i: usize,
    points: &[Vector2],
    delta: f32,
    beside_car: bool,
    slices: &MovementSlices,
) -> (Vector2, Vector2, bool) {
    unsafe {
        let current = Vector2::new(*slices.pos_x.get(i), *slices.pos_y.get(i));
        let step = if *slices.tmode.get(i) != MODE_CAR {
            walker_speed(*slices.schedule_seed.get(i))
        } else if on_a_docks_route(i, slices) {
            FREIGHT_YARD_SPEED_MS
        } else {
            AGENT_DRIVEWAY_SPEED_MS
        } * delta;
        let (next, reached) = if beside_car {
            step_out_along_local_access_path(current, points, step)
        } else {
            advance_along_local_access_path(current, points, step)
        };
        *slices.pos_x.get_mut(i) = next.x;
        *slices.pos_y.get_mut(i) = next.y;
        (current, next, reached)
    }
}

/// Clears the authoritative access plan scalars for one agent.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn clear_access_plan(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.planned_attach_n.get_mut(i) = u32::MAX;
        *slices.planned_detach_n.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane_d.get_mut(i) = 0.0;
        *slices.planned_detach_lane_d.get_mut(i) = 0.0;
        *slices.access_flags.get_mut(i) = 0;
    }
}

/// Resets an invalid access plan back to the specified building-local position.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn reset_invalid_access_plan(
    i: usize,
    current_building: usize,
    pos: Vector2,
    next_replan_time: f32,
    slices: &MovementSlices,
) {
    unsafe {
        // A carrier whose building went while it pulled out of one of its docks (`PARK-20`):
        // its yard trip ends with the building, which frees the dock. A pull-out with its
        // building standing waits in the yard instead (`stand_in_yard`). One byte test on this
        // failure path.
        if slices.parking_trip.get(i).freight_gate {
            *slices.parking_trip.get_mut(i) = Default::default();
        }
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.y;
        *slices.cur_b.get_mut(i) = current_building;
        *slices.tgt_b.get_mut(i) = usize::MAX;
        clear_access_plan(i, slices);
        clear_network_state(i, slices);
        *slices.transit.get_mut(i) = TRANSIT_IN_BUILDING;
        *slices.next_replan_time.get_mut(i) = next_replan_time;
        *slices.network_replan_failures.get_mut(i) = 0;
    }
}

/// Whether agent `i` is a truck backing out of a dock (`PARK-20`), which a lost or stale access
/// plan never moves: it stops where it is on its route, keeps the dock, and the parking phase
/// plans its way out again ([`stand_in_yard`]). One byte test of a row the step has read.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn pulls_out_of_a_dock(i: usize, slices: &MovementSlices) -> bool {
    unsafe {
        let trip = slices.parking_trip.get(i);
        trip.freight_gate && trip.phase == crate::simulation::parking::ParkingTripPhase::PullOut
    }
}

/// Whether agent `i` is a freight carrier on its dock's route, pulling in or out (`PARK-20`): it
/// moves at the yard speed. One byte test of a row the step has read.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
unsafe fn on_a_docks_route(i: usize, slices: &MovementSlices) -> bool {
    unsafe {
        let trip = slices.parking_trip.get(i);
        trip.freight_gate
            && matches!(
                trip.phase,
                crate::simulation::parking::ParkingTripPhase::PullIn
                    | crate::simulation::parking::ParkingTripPhase::PullOut
            )
    }
}

/// Stops truck `i` in its yard, frozen for the parking phase's ordered commit to plan its way out
/// of the dock again (`PARK-20`; `yard.rs`). O(1).
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn stand_in_yard(i: usize, slices: &MovementSlices) {
    unsafe {
        let trip = slices.parking_trip.get_mut(i);
        trip.ready = true;
        trip.replan_needed = true;
        *slices.speed.get_mut(i) = 0.0;
    }
}

/// Completes an access-ingress trip and places the agent inside the destination building.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn arrive_in_building(
    i: usize,
    building_id: usize,
    pos: Vector2,
    sim_time: f32,
    slices: &MovementSlices,
) {
    unsafe {
        debug_assert!(*slices.tmode.get(i) != crate::simulation::economy::agents::MODE_CAR
            || (slices.parking_trip.get(i).vehicle.is_none() && *slices.pending_household_size.get(i) == 0),
            "resident/move-in car leg must end at parking, never a door: agent={i} trip={:?} pending={} activity={} home={} target={building_id}",
            slices.parking_trip.get(i), *slices.pending_household_size.get(i), *slices.activity.get(i), *slices.home.get(i));
        *slices.pos_x.get_mut(i) = pos.x;
        *slices.pos_y.get_mut(i) = pos.y;
        *slices.cur_b.get_mut(i) = building_id;
        *slices.tgt_b.get_mut(i) = usize::MAX;
        *slices.transit.get_mut(i) = TRANSIT_IN_BUILDING;

        if !slices.parking_trip.get(i).vehicle.is_none() {
            // Arrival is an event for ordered custody release, including a wholly walking return.
            slices.parking_trip.get_mut(i).ready = true;
        }
        // A carrier that came through its gate, or was sent to the door from it, has ended its
        // yard trip (`PARK-20`). One byte test per arrival.
        if slices.parking_trip.get(i).freight_gate {
            *slices.parking_trip.get_mut(i) = Default::default();
        }
        let home = *slices.home.get(i);
        let activity = *slices.activity.get(i);
        *slices.activity.get_mut(i) = if building_id == home && activity != ACTIVITY_SHOPPING {
            ACTIVITY_HOME
        } else {
            // Preserve shopping through store pickup and the household's hourly delivery
            // acknowledgement. Merely arriving home must not resume a work departure.
            activity
        };

        clear_access_plan(i, slices);
        clear_network_state(i, slices);
        *slices.next_replan_time.get_mut(i) = 0.0;
        *slices.network_replan_failures.get_mut(i) = 0;

        let commute_time = sim_time - *slices.jstart.get(i);
        *slices.happiness.get_mut(i) =
            (*slices.happiness.get(i) - commute_time / 60.0).clamp(0.0, 100.0);
    }
}

/// Clears lane, path and maneuver state while retaining the access plan.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn clear_network_state(i: usize, slices: &MovementSlices) {
    unsafe {
        *slices.cur_n.get_mut(i) = u32::MAX;
        *slices.cur_e.get_mut(i) = usize::MAX;
        *slices.lane_id.get_mut(i) = usize::MAX;
        *slices.lane_d.get_mut(i) = 0.0;
        *slices.lane_change_from_lane.get_mut(i) = u32::MAX;
        *slices.lane_change_start_d.get_mut(i) = 0.0;
        *slices.lane_change_length.get_mut(i) = 0.0;
        *slices.overtake_blocked_time.get_mut(i) = 0.0;
        *slices.overtake_cooldown.get_mut(i) = 0.0;
        *slices.speed.get_mut(i) = 0.0;
        slices.path.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
    }
}
