// SPDX-License-Identifier: GPL-3.0-or-later

//! The line before the gate (`PARK-36`, the owner's "solution A"): trucks that find every dock
//! held wait at the kerb just before the freight gate, against the traffic, and a truck granted a
//! dock pulls forward into the gate. The owner's example puts the gate upstream of the door, at
//! the frontage's first end: the line at the door's kerb stop lay past the gate, so a granted truck
//! drove round the block to come back to it. Now the line lies before the gate, here in front of
//! the neighbouring lot, and no truck passes its gate.

use super::*;

// The owner's example: the grocery's gate 20 m west of its door, upstream of it on the forward
// lane its trucks arrive by.
const UPSTREAM_GATE: f32 = -20.0;

// Truck `i`'s moves from now until it stands in its dock, summed tick by tick, asserting each
// tick that it is not past the gate's road connection along the road (`gate_x`).
fn driven_to_dock(yard: &mut Yard, i: usize, gate_x: f32) -> f32 {
    let mut driven = 0.0;
    let mut last = at(yard, i);
    yard.tick_until(|y| {
        let now = at(y, i);
        assert!(now.x <= gate_x + 0.05, "past the gate at {now:?}");
        driven += now.distance_to(last);
        last = now;
        y.phase(i) == ParkingTripPhase::AtDock
    });
    driven
}

// The way a truck granted at place 0 drives at most: along the kerb to the gate's road connection
// and the dock's route from there to the stop.
fn place_to_dock(yard: &Yard) -> f32 {
    let route = yard.parking.dock_route(yard.dock(0)).unwrap();
    let way: f32 = route.windows(2).map(|w| Vector2::new(w[0][0], w[0][1]).distance_to(Vector2::new(w[1][0], w[1][1]))).sum();
    line_place(yard, 1, 0).distance_to(Vector2::new(route[0][0], route[0][1])) + way
}

// Three trucks for a one-dock yard whose gate lies upstream of its door: the second and third
// find the dock held, wait in the line before the gate in the order they reached it, in front of
// the lot upstream, and never pass the gate. Each in turn, granted the dock,
// pulls forward into the gate: its drive from the line to the dock is the few metres to the gate
// and the dock's route, never a trip round the block.
#[test]
fn trucks_wait_before_an_upstream_gate_and_pull_forward_into_it() {
    let mut yard = yard_with_gate(0, 1, UPSTREAM_GATE);
    let gate_x = YARD_X + UPSTREAM_GATE;
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    yard.tick_until(|y| y.phase(second) == ParkingTripPhase::AtKerb);
    let third = yard.dispatch(2);
    for (i, slot) in [(second, 0), (third, 1)] {
        yard.tick_until(|y| {
            for j in [second, third] {
                assert!(at(y, j).x <= gate_x + 0.05, "past the gate at {:?}", at(y, j));
            }
            y.phase(i) == ParkingTripPhase::AtKerb && at(y, i).distance_to(line_place(y, 1, slot)) < 0.05
        });
    }
    let stamp = |y: &Yard, i: usize| y.agents.agents.journey_start_time[i];
    assert!(stamp(&yard, second) < stamp(&yard, third), "the second reached the line first");
    // Both stand before the gate, upstream along the kerb, the line reaching past the frontage's
    // first end, before the lot upstream.
    let door_x = YARD_X;
    assert!(at(&yard, second).x < gate_x - 4.0 && at(&yard, third).x < at(&yard, second).x);
    assert!(at(&yard, third).x < door_x + UPSTREAM_GATE - 12.0);

    // The first leaves; the second is granted the dock and pulls in without passing the gate.
    assert!(yard.check(0));
    yard.tick_until(|y| y.phase(second) == ParkingTripPhase::PullIn);
    assert_eq!(yard.phase(third), ParkingTripPhase::AtKerb);
    let bound = place_to_dock(&yard) + 0.5;
    let driven = driven_to_dock(&mut yard, second, gate_x);
    assert!(driven <= bound, "drove {driven} m from the line to the dock, at most {bound}");
    // The third moves up to the first place, forward, and takes the dock after the second.
    yard.tick_until(|y| at(y, third).distance_to(line_place(y, 1, 0)) < 0.05);
    assert!(yard.check(1));
    yard.tick_until(|y| y.phase(third) == ParkingTripPhase::PullIn);
    let driven = driven_to_dock(&mut yard, third, gate_x);
    assert!(driven <= bound, "drove {driven} m from the line to the dock, at most {bound}");
}

// A truck that a save from before the line moved to the gate left waiting at the door's kerb stop
// past the gate (owner's example: the gate upstream of the door) is still served: it stays where
// it stands while it waits, and granted the dock it drives to the gate as it was planned then.
#[test]
fn a_truck_waiting_at_the_door_past_the_gate_is_still_served() {
    let mut yard = yard_with_gate(0, 1, UPSTREAM_GATE);
    let first = yard.dispatch(0);
    yard.tick_until(|y| y.phase(first) == ParkingTripPhase::AtDock);
    let second = yard.dispatch(1);
    yard.tick_until(|y| y.phase(second) == ParkingTripPhase::AtKerb);
    // The older layout: standing at the door's kerb stop, its lane point beside it, past the
    // gate's.
    let (_, lane, gate_d) = gate_lane(&yard, 1);
    let door = kerb_stop(&yard, 1);
    let geometry = &yard.network.lane_system.lanes[lane].geometry;
    let door_d = BuildingAllocator::project_point_to_polyline_s(geometry, door);
    assert!(door_d > gate_d + 10.0);
    let a = &mut yard.agents.agents;
    (a.pos_x[second], a.pos_y[second]) = (door.x, door.y);
    a.planned_detach_lane_d[second] = door_d;
    for _ in 0..50 {
        yard.tick();
        assert!(at(&yard, second).distance_to(door) < 1e-4, "it stays where it stands");
    }
    assert!(yard.check(0));
    yard.tick_until(|y| {
        assert!(!y.inside_grocery(second), "no truck takes the door");
        y.phase(second) == ParkingTripPhase::AtDock
    });
    assert!(yard.check(1));
    assert_eq!(yard.status(1), ShipmentStatus::Returning);
}

