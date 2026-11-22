// SPDX-License-Identifier: GPL-3.0-or-later

//! The drop-off arrival (`PARK-34`): a moving-in household that cannot park its own car at home
//! is driven in, steps out at the stopped car's doors and walks in, and the car drives back to
//! a border and leaves the map; one with a car and no free home place parks farther away.

use super::family::{
    COMPOSITION, FRAME_S, SIZE, SPEED, continues_after_load, road_and_home, step_until,
};
use super::*;
use crate::nodes::sim::core::trips::TripEvent;
use crate::simulation::economy::agents::{AGE_CHILD, AgentHandle, TRANSIT_IN_BUILDING};
use crate::simulation::parking::ParkingTripPhase;

// A household of `size` leaving the border on the home's side, with no car when `carless`
// (else the carrier identity's own decision, a car at this low-density home), and its arrival
// check scheduled as the paced departure schedules it.
fn arriving(mut c: SimCore, border: u32, size: u16, carless: bool) -> (SimCore, AgentHandle) {
    let pos = c.region_graph.node(border).pos;
    let i = c
        .agents
        .spawn_household_arrival_carrier(0, size, COMPOSITION, border, pos.x, pos.z);
    if carless {
        c.agents.parking_trip[i].arrival_ownership = true;
    }
    assert!(c.agents.start_parking_move_in(
        i,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    c.agents.release_departure(i);
    c.households.recount_pending_arrivals(&mut c.agents);
    c.time.speed_multiplier = SPEED;
    let carrier = c.agents.agent_handle(i);
    c.schedule_trip(
        c.time.absolute_slot() + 1,
        TripEvent::ArrivalCheck { carrier },
    );
    (c, carrier)
}

fn home_household(c: &SimCore) -> Option<usize> {
    (0..c.households.households.len()).find(|&h| c.households.households[h].home_building_id == 0)
}

// The household's members in member (spawn) order.
fn family(c: &SimCore, h: usize) -> Vec<usize> {
    let mut members: Vec<_> = c.agents.household_members(h).collect();
    members.sort_unstable();
    members
}

fn all_home(c: &SimCore, members: &[usize]) -> bool {
    members
        .iter()
        .all(|&i| c.agents.transit[i] == TRANSIT_IN_BUILDING && c.agents.current_building[i] == 0)
}

// Runs until the drop-off car is gone, checking it never holds a household again.
fn until_the_car_left(c: &mut SimCore, carrier: AgentHandle) {
    step_until(c, 40_000, |c| c.agents.resolve_agent(carrier).is_none());
    assert!(
        c.agents
            .parking_trip
            .iter()
            .zip(c.agents.household_id.iter())
            .all(|(trip, &h)| !trip.drop_off || h != usize::MAX),
        "nothing of the drop-off car remains"
    );
}

#[test]
fn a_carless_household_of_six_steps_out_at_a_free_place_and_the_car_leaves_the_map() {
    let (c, border) = road_and_home();
    let home = c.allocator.building_handles.handle(0).to_bits();
    let place = c.parking.first_free_candidate(home).unwrap().place;
    let (mut c, carrier) = arriving(c, border, SIZE, true);
    let i = c.agents.resolve_agent(carrier).unwrap();
    let trip = c.agents.parking_trip[i];
    assert!(trip.drop_off && trip.phase == ParkingTripPhase::Drive);
    assert_eq!(
        trip.destination, place,
        "it stops at the catchment's first free place"
    );
    assert_eq!(
        c.parking.first_free_candidate(home).map(|c| c.place),
        Some(place),
        "without reserving it"
    );
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    let h = home_household(&c).unwrap();
    let members = family(&c, h);
    assert_eq!(
        members.len(),
        usize::from(SIZE),
        "the whole household steps out"
    );
    let driver = c.agents.resolve_agent(carrier).unwrap();
    assert!(!members.contains(&driver), "the driver is no member");
    for (m, &i) in members.iter().enumerate() {
        let [x, z] = c
            .parking
            .car_exit_point(place, m, crate::config::LEFT_HAND_DRIVE)
            .unwrap();
        assert_eq!(
            (c.agents.pos_x[i], c.agents.pos_y[i]),
            (x, z),
            "member {m} at its door"
        );
        assert_eq!(
            c.agents.age_group[i],
            COMPOSITION.member_age_group(m as u16)
        );
        assert_eq!(c.agents.parking_trip[i].phase, ParkingTripPhase::WalkToDoor);
    }
    let a = &c.agents;
    assert_eq!(a.household_id[driver], usize::MAX);
    assert_eq!(a.home_building[driver], usize::MAX);
    assert_eq!(a.age_group[driver], AGE_CHILD, "never a worker");
    assert_eq!(a.pending_household_size[driver], 0);
    assert!(a.parking_trip[driver].drop_off);
    assert_eq!(a.parking_trip[driver].phase, ParkingTripPhase::PullOut);
    assert_eq!(c.households.pending_arrivals.households, 0);
    until_the_car_left(&mut c, carrier);
    let members = family(&c, h);
    step_until(&mut c, 8000, |c| all_home(c, &members));
    assert_eq!(
        c.agents.len(),
        usize::from(SIZE),
        "six residents and nothing else"
    );
    assert_eq!(
        c.parking.first_free_candidate(home).map(|c| c.place),
        Some(place),
        "the place stays free"
    );
}

#[test]
fn with_every_place_taken_the_car_stops_at_the_kerb_by_the_door() {
    let (mut c, border) = road_and_home();
    c.parking.fill_test_fleet();
    let kerb = crate::simulation::economy::agents::tick::kerb_stop_entrance(
        &c.allocator.entrances[0],
        &c.region_graph,
    )
    .unwrap();
    let (mut c, carrier) = arriving(c, border, 4, true);
    let i = c.agents.resolve_agent(carrier).unwrap();
    assert!(
        c.agents.parking_trip[i].destination.is_none(),
        "no place: the kerb"
    );
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    let driver = c.agents.resolve_agent(carrier).unwrap();
    let stop = godot::prelude::Vector2::new(c.agents.pos_x[driver], c.agents.pos_y[driver]);
    assert!(
        stop.distance_to(kerb.door_pos) < 1e-3,
        "stopped at the kerb stop"
    );
    assert_eq!(
        c.agents.current_lane_id[driver],
        usize::MAX,
        "out of the driving lane"
    );
    let h = home_household(&c).unwrap();
    let members = family(&c, h);
    assert_eq!(members.len(), 4);
    for &m in &members {
        let at = godot::prelude::Vector2::new(c.agents.pos_x[m], c.agents.pos_y[m]);
        assert!(at.distance_to(stop) < 3.5, "member {m} beside the car");
    }
    until_the_car_left(&mut c, carrier);
    // The car's removal moved an agent's index: the members again.
    let members = family(&c, h);
    step_until(&mut c, 8000, |c| all_home(c, &members));
}

#[test]
fn a_car_finding_no_home_place_parks_at_the_nearest_free_place_beyond() {
    let (mut c, border) = road_and_home();
    c.parking.fill_test_fleet();
    // A second lot 500 m along the road: past the home catchment's 400 m walk.
    let id = register_lot(&mut c);
    c.place_service_building_internal(&id, 500.0, -30.0)
        .unwrap();
    c.publish_pending_building_site_changes();
    let (mut c, carrier) = arriving(c, border, 3, false);
    let i = c.agents.resolve_agent(carrier).unwrap();
    let trip = c.agents.parking_trip[i];
    assert!(
        !trip.drop_off && !trip.vehicle.is_none(),
        "it drives its own car"
    );
    let place = c
        .parking
        .place_entrance(
            trip.destination,
            &c.allocator,
            &c.transit_network,
            &c.region_graph,
        )
        .unwrap();
    assert!(
        place.door_pos.x > 400.0,
        "a place of the far lot, at {:?}",
        place.door_pos
    );
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    let members = family(&c, home_household(&c).unwrap());
    step_until(&mut c, 40_000, |c| all_home(c, &members));
}

#[test]
fn with_no_free_place_in_reach_the_car_stays_stored_and_the_household_is_dropped_off() {
    let (mut c, border) = road_and_home();
    c.parking.fill_test_fleet();
    let (mut c, carrier) = arriving(c, border, 2, false);
    let i = c.agents.resolve_agent(carrier).unwrap();
    let trip = c.agents.parking_trip[i];
    assert!(trip.drop_off, "dropped off");
    let car = trip.vehicle;
    assert!(!car.is_none(), "with its own car decided");
    assert_eq!(c.parking.vehicle_place(car), None, "stored off the map");
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    let h = home_household(&c).unwrap();
    c.step_frame(FRAME_S);
    let owner = c.households.handles.handle(h).to_bits();
    assert_eq!(
        c.parking.household_car(owner),
        car,
        "the household owns its car"
    );
    until_the_car_left(&mut c, carrier);
}

#[test]
fn a_home_removed_on_the_way_turns_the_car_back_with_the_household_unhoused() {
    let (c, border) = road_and_home();
    let (mut c, carrier) = arriving(c, border, SIZE, true);
    for _ in 0..60 {
        c.step_frame(FRAME_S);
    }
    let households = c.households.households.len();
    assert!(c.bulldoze_building(0));
    assert_eq!(
        c.households.households.len(),
        households + 1,
        "the household forms unhoused"
    );
    let h = c.households.households.len() - 1;
    assert_eq!(c.households.households[h].home_building_id, usize::MAX);
    assert_eq!(
        c.agents.household_members(h).count(),
        usize::from(SIZE),
        "nobody lost"
    );
    let driver = c.agents.resolve_agent(carrier).unwrap();
    assert_eq!(
        c.agents.household_id[driver],
        usize::MAX,
        "the driver is no member"
    );
    until_the_car_left(&mut c, carrier);
}

#[test]
fn a_home_removed_after_the_step_out_leaves_the_family_unhoused_and_the_car_leaves() {
    let (c, border) = road_and_home();
    let (mut c, carrier) = arriving(c, border, SIZE, true);
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    let household = c.households.handles.handle(home_household(&c).unwrap());
    c.step_frame(FRAME_S);
    assert!(c.bulldoze_building(0));
    let h = c.households.handles.resolve(household).unwrap();
    assert_eq!(c.households.households[h].home_building_id, usize::MAX);
    assert_eq!(c.agents.household_members(h).count(), usize::from(SIZE));
    until_the_car_left(&mut c, carrier);
}

#[test]
fn a_road_edit_on_the_way_back_and_the_car_still_leaves() {
    let (c, border) = road_and_home();
    let (mut c, carrier) = arriving(c, border, 3, true);
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    step_until(&mut c, 8000, |c| {
        let i = c.agents.resolve_agent(carrier).unwrap();
        c.agents.parking_trip[i].phase == ParkingTripPhase::None
    });
    // A side road joins the road the car drives on: the edge splits under it.
    commit(&mut c, (-200.0, 0.0), (-200.0, 120.0));
    until_the_car_left(&mut c, carrier);
    let members = family(&c, home_household(&c).unwrap());
    step_until(&mut c, 40_000, |c| all_home(c, &members));
}

#[test]
fn a_save_on_the_way_in_and_one_on_the_way_back_continue_as_the_unsaved_run() {
    let (c, border) = road_and_home();
    let (mut c, carrier) = arriving(c, border, SIZE, true);
    for _ in 0..60 {
        c.step_frame(FRAME_S);
    }
    continues_after_load(&mut c, "drop-off-on-the-way-in", 60);
    step_until(&mut c, 8000, |c| home_household(c).is_some());
    for _ in 0..30 {
        c.step_frame(FRAME_S);
    }
    let i = c.agents.resolve_agent(carrier).unwrap();
    assert!(c.agents.parking_trip[i].drop_off, "on its way back");
    continues_after_load(&mut c, "drop-off-on-the-way-back", 120);
    until_the_car_left(&mut c, carrier);
}

#[test]
fn a_format_126_save_with_a_carrier_walking_in_loads_and_the_household_forms_at_home() {
    let (mut c, border) = road_and_home();
    let pos = c.region_graph.node(border).pos;
    let i = c
        .agents
        .spawn_household_arrival_carrier(0, 3, COMPOSITION, border, pos.x, pos.z);
    c.agents.parking_trip[i].arrival_ownership = true;
    // The walk from the border as the game before `PARK-34` set one out.
    assert!(c.agents.start_border_walk_for_test(
        i,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    c.agents.release_departure(i);
    c.households.recount_pending_arrivals(&mut c.agents);
    let carrier = c.agents.agent_handle(i);
    c.schedule_trip(
        c.time.absolute_slot() + 1,
        TripEvent::ArrivalCheck { carrier },
    );
    c.time.speed_multiplier = SPEED;
    for _ in 0..30 {
        c.step_frame(FRAME_S);
    }
    let path = temp_save_path("drop-off-format-126-walker");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "UPDATE save_meta SET version = ?1",
            [crate::simulation::save::schema::PRE_DROP_OFF_SAVE_VERSION],
        )
        .unwrap();
    }
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    c.time.speed_multiplier = SPEED;
    let i = c.agents.resolve_agent(carrier).unwrap();
    assert_eq!(
        c.agents.transit_mode[i],
        crate::simulation::economy::agents::MODE_WALK,
        "still walking in"
    );
    step_until(&mut c, 200_000, |c| home_household(c).is_some());
    let members = family(&c, home_household(&c).unwrap());
    assert_eq!(members.len(), 3);
    assert!(all_home(&c, &members), "formed inside the home");
}
