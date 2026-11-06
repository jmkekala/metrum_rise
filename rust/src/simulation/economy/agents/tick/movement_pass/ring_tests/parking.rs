// SPDX-License-Identifier: GPL-2.0-only

//! Reservation-preserving parking detours through the existing confirmed wait-for ring.

use super::*;
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::economy::agents::tests::parking::{fixture, prepare_requested_routes};
use crate::simulation::economy::agents::{TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING};
use crate::simulation::parking::{Bay, ParkingSystem, ParkingTrip, ParkingTripPhase, PlaceSource};
use godot::prelude::Vector2;

#[test]
fn parking_gridlock_detour_keeps_the_reserved_driveway_and_never_teleports() {
    let mut ring = ring(12.0, 120, 0);
    run_ring(&mut ring, 4_000, 1, 0.5);
    add_escapes(&mut ring);
    // Only the destination road needs sidewalks for the last walk of this isolated trip.
    let foot_fwd = ring.network.lane_system.lanes.len();
    let foot_bkw = foot_fwd + 1;
    for is_fwd in [true, false] {
        let mut sidewalk = ring.network.lane_system.lanes[2].clone();
        sidewalk.lane_type = crate::simulation::network::lanes::LaneType::Foot;
        sidewalk.is_fwd = is_fwd;
        // The hosts' side -1 sidewalk, indexed as the lane builder indexes it.
        sidewalk.lane_idx = -100;
        sidewalk.next_lanes = Default::default();
        if !is_fwd {
            sidewalk.geometry.reverse();
        }
        ring.network.lane_system.lanes.push(sidewalk);
    }
    ring.network
        .lane_system
        .edge_lanes
        .get_mut(&2)
        .unwrap()
        .extend([foot_fwd, foot_bkw]);
    // The topology gained only empty lanes. Rebuild the ordinary occupancy snapshot.
    ring.agents.lane_bucket_snapshot_valid = false;
    ring.agents.prepare_lane_buckets_for_test(&ring.network);
    let head = ring.agents.lane_buckets[0].last().unwrap().1;
    let (_, _, mut allocator, _, _) = fixture();
    // Both hosts are on the ring's opposite long road. The driveway is 80 m away from its
    // destination door: replanning to the door would keep custody but detach at a wrong station.
    for (b, x) in [300.0, 180.0].into_iter().enumerate() {
        allocator.buildings[b].edge_idx = 2;
        allocator.entrances[b] = BuildingEntrance {
            edge_idx: 2,
            side: -1,
            entrance_s_m: 400.0 - x,
            door_pos: Vector2::new(x, 62.0),
            curb_pos: Vector2::new(x, 54.0),
            car_lane_fwd: 2,
            foot_lane_fwd: foot_fwd,
            foot_lane_bkw: foot_bkw,
            ..BuildingEntrance::default()
        };
    }
    let mut parking = ParkingSystem::default();
    let mut places = Vec::new();
    for (b, x) in [300.0, 100.0].into_iter().enumerate() {
        places.push(parking.add_place(
            PlaceSource::Building {
                building: allocator.building_handles.handle(b).to_bits(),
                anchor: 0,
            },
            Bay {
                identity: [0, b as u64],
                position: [x, 0.0, 62.0],
                path: [[x, 54.0], [x + 4.0, 54.0], [x + 4.0, 59.0], [x, 62.0]],
                heading: 0.0,
                width: 2.5,
                length: 5.0,
            },
        ));
    }
    parking.rebuild_candidates(&[0, 1], &allocator, &ring.network, &ring.graph, None);
    let car = parking.assign_household(
        0,
        crate::simulation::zoning::profiles::ZoneDensity::Low,
        [300.0, 62.0],
    );
    assert!(parking.occupy(car, places[0]));
    let borrower = ring.agents.agent_handle(head);
    assert!(parking.reserve_trip(car, borrower, places[1]));
    parking.release(car);
    parking.set_moving(car, true);
    ring.agents.parking_trip[head] = ParkingTrip {
        vehicle: car,
        source: places[0],
        destination: places[1],
        target: allocator.building_handles.handle(1).to_bits(),
        phase: ParkingTripPhase::Drive,
        route_capacity: 100,
        ..Default::default()
    };
    ring.agents.freight_shipment_id[head] = u64::MAX;
    ring.agents.home_building[head] = 0;
    ring.agents.target_building[head] = 1;
    ring.agents.access_flags[head] = ACCESS_PLAN_VALID;
    ring.agents.planned_detach_lane_id[head] = 2;
    ring.agents.planned_detach_lane_d[head] = 290.0;
    ring.agents.planned_detach_node[head] = 2;
    let before = (
        ring.agents.current_lane_id[head],
        ring.agents.lane_distance[head].to_bits(),
        ring.agents.pos_x[head].to_bits(),
        ring.agents.pos_y[head].to_bits(),
    );
    for now in [1, 2] {
        ring.agents
            .check_gridlock(&allocator, &ring.network, &ring.graph, now, Some(&parking));
    }
    let report = ring.agents.gridlock.report();
    assert_eq!(report.confirmed, 1, "{report:?}");
    assert_eq!(report.detours, 1, "{report:?}");
    assert_eq!(ring.agents.current_path[head][..2], [1, 4]);
    assert_eq!(ring.agents.planned_detach_lane_id[head], 2);
    assert_eq!(ring.agents.planned_detach_lane_d[head], 290.0);
    assert!(ring.agents.parking_trip[head].route_capacity <= ring.agents.current_path[head].len());
    assert!(parking.trip_reserved(car, places[1]));
    assert_eq!(parking.borrowed_by(car), Some(borrower));
    assert_eq!(
        before,
        (
            ring.agents.current_lane_id[head],
            ring.agents.lane_distance[head].to_bits(),
            ring.agents.pos_x[head].to_bits(),
            ring.agents.pos_y[head].to_bits(),
        )
    );
    // Release background queues after proving the detour, so this test owns only the chosen
    // car's route and arrival, rather than the refill/saturation contract tested by the ring.
    for i in 0..ring.agents.len() {
        if i != head {
            ring.agents.current_lane_id[i] = usize::MAX;
            ring.agents.transit[i] = TRANSIT_IN_BUILDING;
            ring.agents.next_replan_time[i] = f32::MAX;
        }
    }
    ring.agents.lane_bucket_snapshot_valid = false;
    let clock = TimeSystem::new();
    let mut pulled_in = false;
    for _ in 0..12_000 {
        prepare_requested_routes(&mut ring.agents);
        let before = Vector2::new(ring.agents.pos_x[head], ring.agents.pos_y[head]);
        ring.agents.tick_with_parking(
            &allocator,
            &mut ring.network,
            &mut ring.graph,
            0.1,
            &clock,
            &mut parking,
        );
        let after = Vector2::new(ring.agents.pos_x[head], ring.agents.pos_y[head]);
        assert!(
            before.distance_to(after) < 8.0,
            "detour relocated {before:?} to {after:?}"
        );
        if ring.agents.parking_trip[head].phase == ParkingTripPhase::PullIn {
            if !pulled_in {
                assert_eq!(ring.agents.transit[head], TRANSIT_ACCESS_INGRESS);
                assert!((after.x - 100.0).abs() < 0.01);
            }
            pulled_in = true;
        }
        if ring.agents.parking_trip[head].phase == ParkingTripPhase::WalkToDoor {
            assert!(pulled_in);
            assert_eq!(parking.vehicle_place(car), Some(places[1]));
            return;
        }
    }
    panic!(
        "parking detour did not arrive: {:?}",
        ring.agents.parking_trip[head]
    );
}
