// SPDX-License-Identifier: GPL-3.0-or-later

//! A truck reaching its freight gate on the far lane (`PARK-32`): a second processor east of the
//! grocery sends trucks west, on the lane across the road from the gate. With a dock free the truck
//! crosses at once by the gap rule, holding the near lane while it crosses. With every dock held it
//! does not wait across the road: it drives on, comes back on the gate's own side and waits in the
//! line before the gate in the order it reaches it.

use super::*;

// The second processor's centre, east of the grocery's gate.
const EAST_X: f32 = 450.0;

// The one-dock yard with a second processor (building 2) at `EAST_X` on the same side.
fn yard_with_east_source() -> Yard {
    let mut yard = yard(1);
    let mut east = yard.allocator.buildings[0].clone();
    east.center_x = EAST_X;
    yard.allocator.buildings.push(east);
    yard.allocator
        .building_handles
        .register_appended(yard.allocator.buildings.len());
    yard.allocator
        .rebuild_entrance_cache(&yard.graph, &yard.network.lane_system);
    yard
}

// A truck from the east, the dock free: its plan ends at the gate's far lane point, it crosses
// there holding the near lane, and stands in the dock.
#[test]
fn a_truck_on_the_far_lane_crosses_into_a_free_dock() {
    let mut yard = yard_with_east_source();
    let truck = yard.dispatch_from(0, 2);
    let (_, near, _) = gate_lane(&yard, 1);
    assert_ne!(
        yard.agents.agents.planned_detach_lane_id[truck] as usize, near,
        "it reaches the gate on the far lane"
    );
    yard.tick_until(|y| y.agents.agents.transit[truck] == TRANSIT_ACCESS_INGRESS);
    assert_eq!(
        yard.agents.agents.crossing_lane[truck] as usize, near,
        "it crosses holding the near lane"
    );
    yard.tick_until(|y| y.phase(truck) == ParkingTripPhase::AtDock);
}

// One dock held: a truck from the west waits first in the line before the gate; a truck from the
// east refused at the end of its own line on the far lane drives round, joins the line on the
// gate's side behind the first, never standing across the road, and each is served in the order
// it reached the line.
#[test]
fn a_far_side_truck_refused_a_dock_joins_the_line_on_the_gates_side() {
    let mut yard = yard_with_east_source();
    let holder = yard.dispatch(0);
    yard.tick_until(|y| y.phase(holder) == ParkingTripPhase::AtDock);
    let near_truck = yard.dispatch(1);
    yard.tick_until(|y| y.phase(near_truck) == ParkingTripPhase::AtKerb);
    let far_truck = yard.dispatch_from(2, 2);
    let (_, near, _) = gate_lane(&yard, 1);
    assert_ne!(
        yard.agents.agents.planned_detach_lane_id[far_truck] as usize,
        near
    );
    yard.tick_until(|y| {
        let a = &y.agents.agents;
        if y.phase(far_truck) == ParkingTripPhase::AtKerb {
            assert!(
                at(y, far_truck).y > 0.0,
                "it waits across the road at {:?}",
                at(y, far_truck)
            );
        }
        y.phase(far_truck) == ParkingTripPhase::AtKerb
            && a.planned_detach_lane_id[far_truck] as usize == near
            && at(y, far_truck).distance_to(line_place(y, 1, 1)) < 0.05
    });
    let stamp = |y: &Yard, i: usize| y.agents.agents.journey_start_time[i];
    assert!(stamp(&yard, near_truck) < stamp(&yard, far_truck));
    assert!(yard.check(0));
    yard.tick_until(|y| y.phase(near_truck) == ParkingTripPhase::AtDock);
    assert_eq!(yard.phase(far_truck), ParkingTripPhase::AtKerb);
    assert!(yard.check(1));
    yard.tick_until(|y| y.phase(far_truck) == ParkingTripPhase::AtDock);
}
