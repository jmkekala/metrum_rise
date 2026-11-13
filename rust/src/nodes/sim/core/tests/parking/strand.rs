// SPDX-License-Identifier: GPL-2.0-only

//! STRAND diagnosis (2026-11-13): a household moving in by car whose new home is demolished on
//! its way in, the car case of the owner's stranded walkers.

use super::*;
use crate::simulation::economy::agents::{HouseholdAgeComposition, MODE_WALK, TRANSIT_IN_BUILDING};
use crate::simulation::network::graph::RoadParking;
use crate::simulation::parking::ParkingTripPhase;

// The evacuation clears the carrier's home and destination but keeps its household on the way
// in (`pending_household_size`); the drive's recovery then finds no destination, no home and no
// building (`parking_trip_target`) and settles the driver where it stands, its car off the map,
// "for its household's next home" (`settle_without_car`), which a household not yet admitted
// never gets: it stays in no building for good, never arrives, and the demand keeps counting it
// as a pending arrival.
#[test]
fn strand_a_moving_in_driver_whose_home_is_demolished_settles_nowhere_for_good() {
    let mut c = SimCore::new(
        WorldConfig::new(2048.0, 1024.0, 16.0, 10.0)
            .with_chunking(128.0, 0.0)
            .with_terrain_resolution(2.0),
    );
    c.treasury.balance = 10_000_000.0;
    commit_with(
        &mut c,
        (-600.0, 0.0),
        (600.0, 0.0),
        RoadParking {
            left: true,
            right: true,
        },
    );
    let id = register_lot(&mut c);
    let mut manifest = c.allocator.registry.get(&id).unwrap().manifest.clone();
    manifest.building.as_mut().unwrap().household_capacity = Some(4);
    c.allocator
        .registry
        .register("parking", manifest, String::new());
    c.place_service_building_internal(&id, 0.0, -30.0).unwrap();
    c.place_service_building_internal(&id, 400.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let border = if c.allocator.entrances[0].side == -1 {
        c.region_graph.edge(0).start_node
    } else {
        c.region_graph.edge(0).end_node
    };
    let pos = c.region_graph.node(border).pos;
    c.allocator.claim_vacancy(0);
    let carrier = c.agents.spawn_household_arrival_carrier(
        0,
        2,
        HouseholdAgeComposition {
            adult_count: 2,
            ..Default::default()
        },
        border,
        pos.x,
        pos.z,
    );
    let handle = c.agents.agent_handle(carrier);
    assert!(c.agents.start_parking_move_in(
        carrier,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    let car = c.agents.parking_trip[carrier].vehicle;
    assert!(!car.is_none());
    c.time.speed_multiplier = 8.0;
    let mut on_road = false;
    for _ in 0..2_000 {
        c.step_frame(1.0 / 60.0);
        let i = c.agents.resolve_agent(handle).unwrap();
        if c.agents.parking_trip[i].phase == ParkingTripPhase::Drive
            && c.agents.current_lane_id[i] != usize::MAX
        {
            on_road = true;
            break;
        }
    }
    assert!(on_road, "it drives in");
    // The player bulldozes its new home; a road taking it evacuates through the same
    // `evacuate_referencing`.
    assert!(c.bulldoze_building(0));
    let i = c.agents.resolve_agent(handle).unwrap();
    assert_eq!(c.agents.pending_household_size[i], 2);
    assert_eq!(c.agents.home_building[i], usize::MAX);
    assert_eq!(c.agents.target_building[i], usize::MAX);
    // Six game minutes on.
    for _ in 0..3_000 {
        c.step_frame(1.0 / 60.0);
    }
    let i = c.agents.resolve_agent(handle).unwrap();
    let a = &c.agents;
    eprintln!(
        "STRAND car carrier: transit {} mode {} building {} lane {} node {} target {} home {} pending {} trip {:?} stored {:?} lines {:#?}",
        a.transit[i],
        a.transit_mode[i],
        a.current_building[i] as i64,
        a.current_lane_id[i] as i64,
        a.current_node[i] as i64,
        a.target_building[i] as i64,
        a.home_building[i] as i64,
        a.pending_household_size[i],
        a.parking_trip[i],
        c.parking.stored_car_home(car),
        a.trip_failures.lines,
    );
    assert_eq!(a.transit[i], TRANSIT_IN_BUILDING);
    assert_eq!(a.current_building[i], usize::MAX, "in no building");
    assert_eq!(a.transit_mode[i], MODE_WALK);
    assert_eq!(
        a.pending_household_size[i], 2,
        "still a household on its way in"
    );
    assert_eq!(a.target_building[i], usize::MAX);
    assert_eq!(a.parking_trip[i].phase, ParkingTripPhase::None);
    assert!(
        c.parking.stored_car_home(car).is_some(),
        "its car is off the map"
    );
    assert!(c.households.households.is_empty(), "it never arrives");
    c.households.recount_pending_arrivals(&mut c.agents);
    assert_eq!(
        c.households.pending_arrivals.households, 1,
        "the demand still counts it as arriving"
    );
}
