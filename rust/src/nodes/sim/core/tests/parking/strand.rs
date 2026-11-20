// SPDX-License-Identifier: GPL-3.0-or-later

//! `ECON-29` (round STRAND's diagnosis, 2026-11-13): a household moving in by car whose new home
//! is demolished on its way in, the car case of the owner's stranded walkers.

use super::*;
use crate::simulation::economy::agents::{HouseholdAgeComposition, MODE_WALK, TRANSIT_IN_BUILDING};
use crate::simulation::network::graph::RoadParking;
use crate::simulation::parking::ParkingTripPhase;

// A road with kerb parking both sides, two homes of four places (building 0 at x 0, building 1
// at x 400) and the carrier of a two-adult family bound for building 0, held at the border it
// sets out from.
fn city_with_a_family_moving_in() -> (SimCore, usize) {
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
    // Homes in the residential vacancy index, which the housing queue searches.
    for building in &mut c.allocator.buildings {
        building.zone_type = crate::simulation::zoning::ZoneType::Residential;
    }
    c.allocator.rebuild_zone_index();
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
    (c, carrier)
}

// The bulldozer makes the family an unhoused household on the road: the carrier, now its first
// member, keeps the car decided at the border, which its household adopts. Its trip gone, the
// drive's recovery settles it where it stands for its household's next home, its car off the map
// (`settle_without_car`), as an evicted driver does. The housing queue rehouses the household in
// the other home, and the driver goes in. Before the fix it stayed a household on its way in, in
// no building for good, counted by the demand as a pending arrival.
#[test]
fn a_moving_in_driver_whose_home_is_demolished_becomes_an_unhoused_household_and_is_rehoused() {
    let (mut c, carrier) = city_with_a_family_moving_in();
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
    assert_eq!(c.agents.pending_household_size[i], 0);
    assert_eq!(c.agents.household_id[i], 0);
    assert_eq!(c.agents.home_building[i], usize::MAX);
    assert_eq!(c.agents.target_building[i], usize::MAX);
    assert_eq!(c.households.households.len(), 1);
    assert_eq!(c.households.households[0].home_building_id, usize::MAX);
    assert_eq!(c.households.households[0].member_count, 2);
    c.households.recount_pending_arrivals(&mut c.agents);
    assert_eq!(
        c.households.pending_arrivals.households, 0,
        "no longer arriving"
    );
    // Six game minutes on.
    for _ in 0..3_000 {
        c.step_frame(1.0 / 60.0);
    }
    let i = c.agents.resolve_agent(handle).unwrap();
    let a = &c.agents;
    assert_eq!(a.transit[i], TRANSIT_IN_BUILDING);
    assert_eq!(a.current_building[i], usize::MAX, "settled in no building");
    assert_eq!(a.transit_mode[i], MODE_WALK);
    assert_eq!(a.parking_trip[i].phase, ParkingTripPhase::None);
    assert_eq!(a.parking_trip[i].vehicle, car);
    assert!(
        !a.parking_trip[i].arrival_ownership,
        "its household adopted the car"
    );
    let owner = c.households.handles.handle(0).to_bits();
    assert_eq!(c.parking.household_car(owner), car);
    assert!(c.households.households[0].home_building_id == usize::MAX);
    // The housing queue gives it the other home once its reserves afford it (an immigrant's
    // starting budget alone does not).
    c.households.households[0].budget = 1_000.0;
    let slot = c.time.absolute_slot();
    c.households
        .queue_every_household_for_test(slot, &mut c.agents, &c.allocator);
    c.households
        .serve_housing_requests(8, &mut c.agents, &mut c.allocator);
    assert_eq!(c.households.households[0].home_building_id, 0, "rehoused");
    for _ in 0..600 {
        c.step_frame(1.0 / 60.0);
    }
    let i = c.agents.resolve_agent(handle).unwrap();
    assert_eq!(c.agents.transit[i], TRANSIT_IN_BUILDING);
    assert_eq!(c.agents.current_building[i], 0, "it goes in");
    assert!(
        c.agents
            .household_members(0)
            .all(|m| c.agents.current_building[m] == 0),
        "the whole family is home"
    );
}

// Rows of the family that a save must keep: each member's place and trip, and the household.
fn family_rows(c: &SimCore) -> Vec<String> {
    let a = &c.agents;
    let mut rows: Vec<String> = c
        .households
        .households
        .iter()
        .map(|h| {
            format!(
                "household home {} members {} budget {}",
                h.home_building_id as i64, h.member_count, h.budget
            )
        })
        .collect();
    for h in 0..c.households.households.len() {
        for i in a.household_members(h) {
            rows.push(format!(
                "member {i}: transit {} mode {} building {} lane {} d {} target {} trip {}",
                a.transit[i],
                a.transit_mode[i],
                a.current_building[i] as i64,
                a.current_lane_id[i] as i64,
                a.lane_distance[i],
                a.target_building[i] as i64,
                serde_json::to_value(a.parking_trip[i]).unwrap(),
            ));
        }
    }
    rows
}

// `ECON-29`: the unhoused household a demolition makes of a family driving in saves and loads as
// one, saved before its car's adoption is published, and the loaded game continues as the running
// one does, frame for frame.
#[test]
fn an_unhoused_arrival_household_continues_exactly_across_a_save() {
    let (mut c, carrier) = city_with_a_family_moving_in();
    let handle = c.agents.agent_handle(carrier);
    assert!(c.agents.start_parking_move_in(
        carrier,
        &mut c.parking,
        &c.allocator,
        &c.transit_network,
        &c.region_graph
    ));
    c.time.speed_multiplier = 8.0;
    for _ in 0..2_000 {
        c.step_frame(1.0 / 60.0);
        let i = c.agents.resolve_agent(handle).unwrap();
        if c.agents.current_lane_id[i] != usize::MAX {
            break;
        }
    }
    assert!(c.bulldoze_building(0));
    assert_eq!(c.households.households.len(), 1);
    let path = temp_save_path("econ29_unhoused");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    let before = family_rows(&c);
    let mut straight = Vec::new();
    for _ in 0..600 {
        c.step_frame(1.0 / 60.0);
        straight.push(family_rows(&c));
    }
    let speed = c.time.speed_multiplier;
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    // A loaded game starts paused.
    c.time.speed_multiplier = speed;
    assert_eq!(family_rows(&c), before, "it loads as it was saved");
    for (frame, rows) in straight.iter().enumerate() {
        c.step_frame(1.0 / 60.0);
        assert_eq!(&family_rows(&c), rows, "frame {frame} after the load");
    }
}

// `ECON-29`: a save from before the fix holds a family on its way in whose home a demolition
// took: its carrier keeps its household's size with no home and no household. The load makes it
// the unhoused household the running game now makes (before, the loader dropped its size and
// left a walker with no household that nothing ever removed).
#[test]
fn a_family_left_homeless_on_its_way_in_by_an_older_save_loads_as_an_unhoused_household() {
    let (mut c, carrier) = city_with_a_family_moving_in();
    // The state the old evacuation left: home and destination cleared, the household kept.
    c.agents.agents.home_building[carrier] = usize::MAX;
    c.agents.agents.target_building[carrier] = usize::MAX;
    c.allocator.release_vacancy(0);
    let path = temp_save_path("econ29_ghost");
    c.save_game_settled_at(path.to_str().unwrap(), None, 0)
        .unwrap();
    c.load_game_internal(path.to_str().unwrap()).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_eq!(c.households.households.len(), 1);
    let family = &c.households.households[0];
    assert_eq!(family.home_building_id, usize::MAX);
    assert_eq!((family.member_count, family.adult_count), (2, 2));
    assert_eq!(c.agents.len(), 2);
    for member in 0..2 {
        assert_eq!(c.agents.household_id[member], 0);
        assert_eq!(c.agents.pending_household_size[member], 0);
        assert_eq!(c.agents.home_building[member], usize::MAX);
        assert_eq!(c.agents.current_building[member], usize::MAX);
    }
    c.households.recount_pending_arrivals(&mut c.agents);
    assert_eq!(c.households.pending_arrivals.households, 0);
    // It waits unhoused and is rehoused by the housing rule (the fixture's homes are service
    // lots a load does not list as residential: listed again here).
    for building in &mut c.allocator.buildings {
        building.zone_type = crate::simulation::zoning::ZoneType::Residential;
    }
    c.allocator.rebuild_zone_index();
    c.households.households[0].budget = 1_000.0;
    let slot = c.time.absolute_slot();
    c.households
        .queue_every_household_for_test(slot, &mut c.agents, &c.allocator);
    c.households
        .serve_housing_requests(8, &mut c.agents, &mut c.allocator);
    assert_ne!(
        c.households.households[0].home_building_id,
        usize::MAX,
        "rehoused"
    );
}
