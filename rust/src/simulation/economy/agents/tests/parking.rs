// SPDX-License-Identifier: GPL-2.0-only

//! Resident parking itinerary regressions on a minimal straight-road fixture.

use super::support::{create_test_building, register_test_asset, setup_straight_road};
use super::*;
use crate::simulation::parking::{Bay, ParkingSystem, ParkingTripPhase, PlaceSource};

/// Minimal resident/lot fixture shared with the blocked-ring parking integration test.
pub(in crate::simulation::economy::agents) fn fixture() -> (
    AgentSystem,
    ParkingSystem,
    BuildingAllocator,
    TransitNetwork,
    RegionGraph,
) {
    fixture_at(45.0, 455.0, 455.0)
}

fn fixture_at(
    home_x: f32,
    work_x: f32,
    driveway_x: f32,
) -> (
    AgentSystem,
    ParkingSystem,
    BuildingAllocator,
    TransitNetwork,
    RegionGraph,
) {
    let (network, graph, _, _) = setup_straight_road();
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(&mut allocator, "parking", "home", ZoneClass::Residential);
    let work_asset = register_test_asset(&mut allocator, "parking", "work", ZoneClass::Commercial);
    for (x, asset, zone) in [
        (home_x, home_asset, ZoneType::Residential),
        (work_x, work_asset, ZoneType::Commercial),
    ] {
        let mut b = create_test_building(0, -1);
        b.center_x = x;
        b.center_y = -12.0;
        b.facing_dir = Vector2::new(0.0, -1.0);
        b.asset_id = asset;
        b.zone_type = zone;
        allocator.buildings.push(b);
    }
    allocator
        .building_handles
        .register_appended(allocator.buildings.len());
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut parking = ParkingSystem::default();
    for b in 0..2 {
        let e = &allocator.entrances[b];
        let pos = e.door_pos + Vector2::new(5.0, 0.0);
        let curb_x = if b == 1 { driveway_x } else { e.curb_pos.x };
        parking.add_place(
            PlaceSource::Building {
                building: allocator.building_handles.handle(b).to_bits(),
                anchor: 0,
            },
            Bay {
                identity: [0, b as u64],
                position: [pos.x, 0.0, pos.y],
                path: [
                    [curb_x, e.curb_pos.y],
                    [pos.x, e.curb_pos.y],
                    [pos.x, pos.y + 3.0],
                    [pos.x, pos.y],
                ],
                heading: 0.0,
                width: 2.5,
                length: 5.0,
            },
        );
    }
    parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
    let home = allocator.building_handles.handle(0).to_bits();
    let car = parking.assign_household(
        0,
        crate::simulation::zoning::profiles::ZoneDensity::Low,
        [home_x, -12.0],
    );
    parking.park_new_car(car, home);
    parking.seed_fixture_car(car, home);
    assert!(parking.vehicle_place(car).is_some());
    let mut agents = AgentSystem::new();
    for _ in 0..2 {
        let i = agents.spawn_housed_agent(0, home_x, -12.0);
        agents.agents.parking_trip[i].vehicle = car;
        agents.agents.planned_target_building[i] = 1;
        agents.agents.planned_activity[i] = 1;
    }
    (agents, parking, allocator, network, graph)
}

#[test]
fn resident_itinerary_never_drives_to_a_door_and_one_borrower() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    let mut phases = [false; 5];
    let clock = test_clock(0, 0);
    for _ in 0..8000 {
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        let phase = agents.agents.parking_trip[0].phase;
        match phase {
            ParkingTripPhase::WalkToCar => phases[0] = true,
            ParkingTripPhase::PullOut => phases[1] = true,
            ParkingTripPhase::Drive => phases[2] = true,
            ParkingTripPhase::PullIn => phases[3] = true,
            ParkingTripPhase::WalkToDoor => phases[4] = true,
            ParkingTripPhase::None => {}
        }
        assert_ne!(
            agents.agents.transit_mode[1], MODE_CAR,
            "second borrower drove"
        );
        for i in 0..2 {
            if agents.agents.transit[i] == TRANSIT_IN_BUILDING
                && agents.agents.current_building[i] == 1
            {
                assert_eq!(
                    agents.agents.transit_mode[i], MODE_WALK,
                    "car leg ended at a door"
                );
            }
        }
        if agents.agents.current_building[0] == 1 && agents.agents.transit[0] == TRANSIT_IN_BUILDING
        {
            break;
        }
    }
    assert_eq!(
        phases,
        [true; 5],
        "missing itinerary leg: {phases:?}, phase={:?} transit={} lane={} pos=({}, {})",
        agents.agents.parking_trip[0].phase,
        agents.agents.transit[0],
        agents.agents.current_lane_id[0],
        agents.agents.pos_x[0],
        agents.agents.pos_y[0]
    );
    assert_eq!(agents.agents.current_building[0], 1);
    let car = agents.agents.parking_trip[0].vehicle;
    assert!(parking.vehicle_place(car).is_some());
}

#[test]
fn full_destination_falls_back_to_walking_without_relocation() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    parking.fill_test_fleet();
    let before = (agents.agents.pos_x[0], agents.agents.pos_y[0]);
    agents.tick_with_parking(
        &allocator,
        &mut network,
        &mut graph,
        0.1,
        &test_clock(0, 0),
        &mut parking,
    );
    assert_eq!(agents.agents.transit_mode[0], MODE_WALK);
    assert_eq!(agents.agents.parking_trip[0].phase, ParkingTripPhase::None);
    let e = &allocator.entrances[0];
    assert!(
        (agents.agents.pos_x[0] - e.door_pos.x).hypot(agents.agents.pos_y[0] - e.door_pos.y) < 0.01
    );
    assert!((agents.agents.pos_x[0] - before.0).abs() < 2.0);
}

#[test]
fn removing_reserved_supply_replans_at_the_actual_position() {
    for phase in [
        ParkingTripPhase::WalkToCar,
        ParkingTripPhase::PullOut,
        ParkingTripPhase::Drive,
        ParkingTripPhase::PullIn,
    ] {
        let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
        let e = &allocator.entrances[1];
        let pos = e.door_pos + Vector2::new(13.0, 0.0);
        let replacement = parking.add_place(
            PlaceSource::Building {
                building: allocator.building_handles.handle(1).to_bits(),
                anchor: 1,
            },
            Bay {
                identity: [0, 9],
                position: [pos.x, 0.0, pos.y],
                path: [
                    [e.curb_pos.x + 8.0, e.curb_pos.y],
                    [pos.x, e.curb_pos.y],
                    [pos.x, pos.y + 3.0],
                    [pos.x, pos.y],
                ],
                heading: 0.0,
                width: 2.5,
                length: 5.0,
            },
        );
        parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
        let clock = test_clock(0, 0);
        for _ in 0..8000 {
            agents.tick_with_parking(
                &allocator,
                &mut network,
                &mut graph,
                0.1,
                &clock,
                &mut parking,
            );
            if agents.parking_trip[0].phase == phase {
                break;
            }
        }
        assert_eq!(agents.parking_trip[0].phase, phase);
        let removed = agents.parking_trip[0].destination;
        assert_ne!(removed, replacement);
        parking.remove_place(removed);
        parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
        let car = agents.parking_trip[0].vehicle;
        let mut reached = false;
        for _ in 0..10000 {
            let before = Vector2::new(agents.pos_x[0], agents.pos_y[0]);
            agents.tick_with_parking(
                &allocator,
                &mut network,
                &mut graph,
                0.1,
                &clock,
                &mut parking,
            );
            let after = Vector2::new(agents.pos_x[0], agents.pos_y[0]);
            assert!(
                before.distance_to(after) < 8.0,
                "recovery relocated a driver from {before:?} to {after:?} in {phase:?}"
            );
            if agents.current_building[0] == 1 && agents.transit[0] == TRANSIT_IN_BUILDING {
                reached = true;
                break;
            }
        }
        assert!(
            reached,
            "recovery stalled from {phase:?}: {:?}",
            agents.parking_trip[0]
        );
        assert_eq!(parking.vehicle_place(car), Some(replacement));
        assert_eq!(agents.transit_mode[0], MODE_WALK);
    }
}

#[test]
fn towing_before_borrowing_walks_to_the_cars_actual_new_place() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    let e = &allocator.entrances[0];
    let pos = e.door_pos + Vector2::new(13.0, 0.0);
    let replacement = parking.add_place(
        PlaceSource::Building {
            building: allocator.building_handles.handle(0).to_bits(),
            anchor: 1,
        },
        Bay {
            identity: [0, 9],
            position: [pos.x, 0.0, pos.y],
            path: [
                [e.curb_pos.x + 8.0, e.curb_pos.y],
                [pos.x, e.curb_pos.y],
                [pos.x, pos.y + 3.0],
                [pos.x, pos.y],
            ],
            heading: 0.0,
            width: 2.5,
            length: 5.0,
        },
    );
    parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
    let clock = test_clock(0, 0);
    agents.tick_with_parking(
        &allocator,
        &mut network,
        &mut graph,
        0.1,
        &clock,
        &mut parking,
    );
    assert_eq!(agents.parking_trip[0].phase, ParkingTripPhase::WalkToCar);
    let car = agents.parking_trip[0].vehicle;
    let old = agents.parking_trip[0].source;
    parking.remove_place(old);
    assert_eq!(parking.vehicle_place(car), Some(replacement));
    parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
    let mut retrieved = false;
    for _ in 0..10000 {
        let before = Vector2::new(agents.pos_x[0], agents.pos_y[0]);
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        assert!(before.distance_to(Vector2::new(agents.pos_x[0], agents.pos_y[0])) < 8.0);
        if agents.parking_trip[0].source == replacement && agents.transit_mode[0] == MODE_CAR {
            retrieved = true;
        }
        if agents.current_building[0] == 1 && agents.transit[0] == TRANSIT_IN_BUILDING {
            break;
        }
    }
    assert!(
        retrieved,
        "borrower did not retrieve the towed car: {:?}",
        agents.parking_trip[0]
    );
    assert_eq!(agents.current_building[0], 1);
}

#[test]
fn candidate_walk_bound_uses_network_time_and_publication_rebuilds_it() {
    let (agents, mut parking, mut allocator, network, graph) = fixture();
    allocator.entrances[0].door_pos = Vector2::new(45.0, -100.0);
    let home = allocator.building_handles.handle(0).to_bits();
    let remote = parking.add_place(
        PlaceSource::Building {
            building: allocator.building_handles.handle(1).to_bits(),
            anchor: 8,
        },
        Bay {
            identity: [0, 8],
            position: [300.0, 0.0, -100.0],
            path: [
                [300.0, -4.0],
                [300.0, -20.0],
                [300.0, -95.0],
                [300.0, -100.0],
            ],
            heading: 0.0,
            width: 2.5,
            length: 5.0,
        },
    );
    assert!(
        allocator.entrances[0]
            .door_pos
            .distance_to(Vector2::new(300.0, -100.0))
            < 400.0
    );
    parking.rebuild_candidates(&[0], &allocator, &network, &graph, None);
    assert!(
        parking.walk_to_car_seconds(home, remote).is_none(),
        "straight-line bound admitted a walk over 400 m at walking pace"
    );
    let near = parking.add_place(
        PlaceSource::Building {
            building: home,
            anchor: 9,
        },
        Bay {
            identity: [0, 9],
            position: [75.0, 0.0, -100.0],
            path: [[75.0, -4.0], [75.0, -20.0], [75.0, -95.0], [75.0, -100.0]],
            heading: 0.0,
            width: 2.5,
            length: 5.0,
        },
    );
    assert!(parking.first_free_candidate(home).is_none());
    parking.rebuild_candidates(&[0], &allocator, &network, &graph, None);
    assert_eq!(parking.first_free_candidate(home).unwrap().place, near);
    parking.remove_place(near);
    parking.rebuild_candidates(&[0], &allocator, &network, &graph, None);
    assert!(parking.first_free_candidate(home).is_none());
    assert!(
        parking
            .vehicle_place(agents.parking_trip[0].vehicle)
            .is_some()
    );
}

#[test]
fn children_cannot_borrow_a_free_household_car() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    agents.age_group.fill(AGE_CHILD);
    agents.tick_with_parking(
        &allocator,
        &mut network,
        &mut graph,
        0.1,
        &test_clock(0, 0),
        &mut parking,
    );
    for i in 0..agents.len() {
        assert!(!agents.has_car[i]);
        assert_eq!(agents.transit_mode[i], MODE_WALK);
        assert_eq!(agents.parking_trip[i].phase, ParkingTripPhase::None);
    }
    assert!(
        parking
            .borrowed_by(agents.parking_trip[0].vehicle)
            .is_none()
    );
}

#[test]
fn resident_estimate_uses_the_complete_departure_quote_without_claiming() {
    use crate::simulation::economy::agents::tick::{
        estimate_parking_trip_seconds, quote_parking_trip,
    };
    let (mut agents, mut parking, allocator, network, graph) = fixture();
    let car = agents.parking_trip[0].vehicle;
    let quote = quote_parking_trip(
        car,
        agents.agent_handle(0),
        true,
        0,
        1,
        &parking,
        &allocator,
        &network,
        &graph,
        &agents.pathfind_count,
        None,
    )
    .unwrap();
    let estimate = estimate_parking_trip_seconds(
        0,
        1,
        car,
        true,
        &parking,
        &allocator,
        &network,
        &graph,
        &agents.pathfind_count,
    )
    .unwrap();
    assert_eq!(estimate, quote.seconds.ceil() as u16);
    assert!(parking.borrowed_by(car).is_none());
    assert!(
        parking
            .first_free_candidate(allocator.building_handles.handle(1).to_bits())
            .is_some()
    );
    parking.fill_test_fleet();
    assert!(
        quote_parking_trip(
            car,
            agents.agent_handle(0),
            true,
            0,
            1,
            &parking,
            &allocator,
            &network,
            &graph,
            &agents.pathfind_count,
            None
        )
        .is_none()
    );
    let walk = estimate_parking_trip_seconds(
        0,
        1,
        car,
        false,
        &parking,
        &allocator,
        &network,
        &graph,
        &agents.pathfind_count,
    )
    .unwrap();
    assert_eq!(
        estimate_parking_trip_seconds(
            0,
            1,
            car,
            true,
            &parking,
            &allocator,
            &network,
            &graph,
            &agents.pathfind_count
        ),
        Some(walk)
    );
}

#[test]
fn a_walking_return_releases_custody_of_a_car_towed_home() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    agents.planned_target_building[1] = usize::MAX;
    let clock = test_clock(0, 0);
    for _ in 0..8000 {
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        if agents.current_building[0] == 1 && agents.transit[0] == TRANSIT_IN_BUILDING {
            break;
        }
    }
    assert_eq!(agents.current_building[0], 1);
    let car = agents.parking_trip[0].vehicle;
    assert_eq!(parking.borrowed_by(car), Some(agents.agent_handle(0)));
    parking.remove_place(parking.vehicle_place(car).unwrap());
    parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
    assert!(parking.vehicle_place(car).is_some());
    agents.planned_target_building[0] = 0;
    agents.planned_activity[0] = 0;
    for _ in 0..8000 {
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        if agents.current_building[0] == 0 && agents.transit[0] == TRANSIT_IN_BUILDING {
            break;
        }
    }
    assert_eq!(agents.current_building[0], 0);
    assert_eq!(agents.transit_mode[0], MODE_WALK);
    assert!(parking.borrowed_by(car).is_none());
    assert!(parking.available_to(car, agents.agent_handle(1)));
}

#[test]
fn a_resident_can_walk_farther_than_the_destination_catchment_to_the_actual_car() {
    use crate::simulation::economy::agents::tick::{ResidentMobility, quote_parking_trip};
    let (mut agents, mut parking, allocator, network, graph) = fixture();
    let car = agents.parking_trip[0].vehicle;
    let source = parking.vehicle_place(car).unwrap();
    let home = allocator.building_handles.handle(0).to_bits();
    let work = allocator.building_handles.handle(1).to_bits();
    let away = parking.first_free_candidate(work).unwrap().place;
    assert!(parking.reserve(car, away));
    parking.release(car);
    assert!(parking.occupy(car, away));
    assert!(parking.walk_to_car_seconds(home, away).is_none());
    let quote = quote_parking_trip(
        car,
        agents.agent_handle(0),
        true,
        0,
        0,
        &parking,
        &allocator,
        &network,
        &graph,
        &agents.pathfind_count,
        None,
    )
    .unwrap();
    assert_eq!(quote.source, away);
    assert_eq!(quote.destination, source);
    assert!(quote.to_car.seconds() > 400.0 / crate::config::AGENT_WALK_SPEED_MS);
    let mobility = ResidentMobility::for_agent(&parking, &agents, 0);
    assert!(mobility.round_trip(0, 1, &allocator, &network, &graph, &agents.pathfind_count));
    assert!(
        parking.borrowed_by(car).is_none(),
        "estimates must not claim custody"
    );
}

#[test]
fn shared_economy_quotes_match_resident_mode_choice_and_respect_custody() {
    use crate::simulation::economy::agents::tick::{
        ResidentMobility, SharedNetworkQuotes, TripEstimate, TripQueryScratch,
        estimate_parking_trip_seconds_within,
    };
    let (mut agents, mut parking, allocator, network, graph) = fixture();
    let car = agents.parking_trip[0].vehicle;
    let mobility = ResidentMobility::for_agent(&parking, &agents, 0);
    let expected = mobility.seconds(0, 1, &allocator, &network, &graph, &agents.pathfind_count);
    let mut shared = SharedNetworkQuotes::default();
    shared.sync(&network);
    let mut query = TripQueryScratch::default();
    query.prepare_deferred_quotes(&network, &graph, &shared);
    let actual = TripQueryScratch::with_owned(&mut query, || {
        estimate_parking_trip_seconds_within(
            0,
            1,
            car,
            true,
            &parking,
            &allocator,
            &network,
            &graph,
            &agents.pathfind_count,
            f32::INFINITY,
            Some(&shared),
        )
    });
    assert!(matches!(actual, TripEstimate::Seconds(seconds) if seconds == expected));
    assert!(mobility.round_trip(0, 1, &allocator, &network, &graph, &agents.pathfind_count));
    let target = parking
        .first_free_candidate(allocator.building_handles.handle(1).to_bits())
        .unwrap()
        .place;
    assert!(parking.reserve_trip(car, agents.agent_handle(0), target));
    assert!(!ResidentMobility::for_agent(&parking, &agents, 1).eligible);
}

/// Services only route-buffer preparation events at the next tick boundary of these isolated
/// fixtures, matching SimCore's schedule/retry boundary without constructing a gameplay world.
pub(in crate::simulation::economy::agents) fn prepare_requested_routes(agents: &mut AgentSystem) {
    let requests = std::mem::take(&mut agents.parking_retries);
    for (handle, seconds) in requests {
        assert_eq!(
            seconds, 0,
            "fixture unexpectedly needs a timed route/supply retry"
        );
        let i = agents.resolve_agent(handle).unwrap();
        let trip = agents.parking_trip[i];
        trip.prepare_route_buffer(&mut agents.agents.current_path[i]);
        agents.agents.parking_trip[i].retry_pending = false;
        agents.agents.next_replan_time[i] = 0.0;
    }
}

// Run the real itinerary up to its driveway handoff, observing both lane visits and position.
fn assert_pull_in_station(home_x: f32, work_x: f32, driveway_x: f32, loops: bool) {
    let (mut agents, mut parking, allocator, mut network, mut graph) =
        fixture_at(home_x, work_x, driveway_x);
    let mut returning = false;
    let clock = test_clock(0, 0);
    for _ in 0..12_000 {
        prepare_requested_routes(&mut agents);
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        let lane = agents.current_lane_id[0];
        if agents.parking_trip[0].phase == ParkingTripPhase::Drive
            && lane != usize::MAX
            && !network.lane_system.lanes[lane].is_fwd
            && network.lane_system.lanes[lane].edge_id == 0
        {
            returning = true;
        }
        if agents.parking_trip[0].phase == ParkingTripPhase::PullIn {
            assert_eq!(returning, loops, "detached on the wrong visit");
            let lane = agents.planned_detach_lane_id[0] as usize;
            let distance = agents.planned_detach_lane_d[0];
            let point =
                BuildingAllocator::sample_pos_on_lane(&network.lane_system.lanes[lane], distance);
            assert!((point.x - driveway_x).abs() < 0.01);
            assert_eq!(Vector2::new(agents.pos_x[0], agents.pos_y[0]), point);
            assert!(parking.trip_reserved(
                agents.parking_trip[0].vehicle,
                agents.parking_trip[0].destination,
            ));
            return;
        }
    }
    panic!("never pulled in: {:?}", agents.parking_trip[0]);
}

#[test]
fn parking_pull_in_reaches_the_driveway_at_the_lane_end() {
    assert_pull_in_station(45.0, 495.0, 500.0, false);
}

#[test]
fn parking_pull_in_waits_for_the_last_destination_edge_visit() {
    assert_pull_in_station(455.0, 45.0, 45.0, true);
}

#[test]
fn parking_gridlock_excludes_parked_and_stored_cars() {
    let (mut agents, mut parking, allocator, network, graph) = fixture();
    let parked = agents.parking_trip[0].vehicle;
    assert!(parking.vehicle_place(parked).is_some());
    let stored = parking.assign_household(
        1,
        crate::simulation::zoning::profiles::ZoneDensity::Low,
        [45.0, -12.0],
    );
    assert!(!stored.is_none());
    assert!(parking.vehicle_place(stored).is_none());
    agents.parking_trip[1].vehicle = stored;
    agents.prepare_lane_buckets_for_test(&network);
    assert!(agents.lane_buckets.iter().all(Vec::is_empty));
    for now in [1, 2] {
        agents.check_gridlock(&allocator, &network, &graph, now, Some(&parking));
        let report = agents.gridlock.report();
        assert_eq!(report.waiting_lanes, 0);
        assert_eq!(report.cycles, 0);
        assert_eq!(report.saturation_reports, 0);
    }
    let saved = agents.gridlock.save(&network);
    assert!(saved.heads.is_empty());
    assert!(saved.cycle_cars.is_empty());
}

#[test]
fn parking_gridlock_treats_a_held_pull_out_as_an_entry_wait() {
    let (mut agents, mut parking, allocator, mut network, mut graph) = fixture();
    let clock = test_clock(0, 0);
    for _ in 0..8_000 {
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        if agents.parking_trip[0].phase == ParkingTripPhase::PullOut {
            break;
        }
    }
    assert_eq!(agents.parking_trip[0].phase, ParkingTripPhase::PullOut);
    let lane = agents.planned_attach_lane_id[0] as usize;
    let distance = agents.planned_attach_lane_d[0];
    let blocker = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0);
    agents.transit_mode[blocker] = MODE_CAR;
    agents
        .agents
        .enter_lane(blocker, &network.lane_system, lane, distance, 0);
    agents.speed[blocker] = 0.0;
    // Hold the entry slot full while the real pull-out reaches the driveway.
    for _ in 0..1_000 {
        agents.tick_with_parking(
            &allocator,
            &mut network,
            &mut graph,
            0.1,
            &clock,
            &mut parking,
        );
        if agents.access_flags[0] & ACCESS_LANE_ATTACH_PENDING != 0 {
            break;
        }
        agents
            .agents
            .enter_lane(blocker, &network.lane_system, lane, distance, 0);
        agents.speed[blocker] = 0.0;
        agents.lane_bucket_snapshot_valid = false;
    }
    assert_ne!(agents.access_flags[0] & ACCESS_LANE_ATTACH_PENDING, 0);
    assert_eq!(agents.current_lane_id[0], usize::MAX);
    // The swept pull-out completed; network entry is still held at the driveway.
    assert_eq!(agents.parking_trip[0].phase, ParkingTripPhase::Drive);
    for now in [1, 2] {
        agents.check_gridlock(&allocator, &network, &graph, now, Some(&parking));
        let report = agents.gridlock.report();
        assert_eq!(report.cycles, 0);
        assert_eq!(report.saturation_reports, 0);
    }
    let saved = agents.gridlock.save(&network);
    let borrower = agents.agent_handle(0).to_bits();
    assert!(saved.heads.iter().all(|&(_, car, _)| car != borrower));
}

#[test]
fn parking_move_in_parks_before_walking_to_door() {
    run_parking_move_in(false);
}

#[test]
fn parking_move_in_without_space_walks_and_keeps_car_off_map() {
    run_parking_move_in(true);
}

fn run_parking_move_in(full: bool) {
    let (_, mut parking, allocator, mut network, mut graph) = fixture();
    parking.remove_household_car(0);
    if full { parking.fill_test_fleet(); }
    let mut agents = AgentSystem::new();
    let border = graph.edge(0).start_node;
    let pos = graph.node(border).pos;
    let i = agents.spawn_household_arrival_carrier(1, 1,
        HouseholdAgeComposition { adult_count: 1, ..Default::default() }, border, pos.x, pos.z);
    agents.prepare_lane_buckets_for_test(&network);
    assert!(agents.start_parking_move_in(i, &mut parking, &allocator, &network, &graph));
    agents.prepare_lane_buckets_for_test(&network);
    let car = agents.parking_trip[i].vehicle;
    assert!(!car.is_none());
    assert_eq!(agents.transit_mode[i], if full { MODE_WALK } else { MODE_CAR });
    assert!(parking.vehicle_place(car).is_none(), "departure cannot create occupancy");
    let clock = test_clock(0, 0);
    let mut phases = [false; 3];
    let mut restored = false;
    for _ in 0..15000 {
        agents.tick_with_parking(&allocator, &mut network, &mut graph, 0.1, &clock, &mut parking);
        prepare_requested_routes(&mut agents);
        match agents.parking_trip[i].phase {
            ParkingTripPhase::Drive => phases[0] = true,
            ParkingTripPhase::PullIn => phases[1] = true,
            ParkingTripPhase::WalkToDoor => phases[2] = true,
            _ => {}
        }
        if !full && phases[1] && !restored {
            parking = ParkingSystem::load_json(&parking.save_json(|s| s).unwrap()).unwrap();
            parking.rebuild_candidates(&[0, 1], &allocator, &network, &graph, None);
            let saved = serde_json::to_string(&agents.parking_trip[i]).unwrap();
            agents.agents.parking_trip[i] = serde_json::from_str(&saved).unwrap();
            restored = true;
        }
        if agents.transit[i] == TRANSIT_IN_BUILDING {
            assert_eq!(agents.transit_mode[i], MODE_WALK, "move-in car reached a door");
            break;
        }
    }
    assert_eq!(agents.transit[i], TRANSIT_IN_BUILDING);
    assert_eq!(agents.current_building[i], 1);
    if full {
        assert!(parking.vehicle_place(car).is_none());
        assert_eq!(phases, [false; 3]);
        assert_eq!(parking.counts().2, 1);
    } else {
        assert_eq!(phases, [true; 3]);
        assert!(parking.vehicle_place(car).is_some());
        assert!(restored);
    }
}

#[test]
fn parking_resident_border_recovery_cannot_create_a_car_door_leg() {
    let (_, mut parking, allocator, mut network, mut graph) = fixture();
    parking.remove_household_car(0);
    let mut agents = AgentSystem::new();
    let border = graph.edge(0).start_node;
    let pos = graph.node(border).pos;
    let car = parking.arrival_car(0, crate::simulation::zoning::profiles::ZoneDensity::Low, [455.0, -12.0]);
    parking.store_arrival_car(car, allocator.building_handles.handle(1).to_bits());
    let i = agents.spawn_border_arrival_agent(1, border, pos.x, pos.z);
    agents.agents.parking_trip[i].vehicle = car;
    let clock = test_clock(0, 0);
    for _ in 0..10000 {
        agents.tick_with_parking(&allocator, &mut network, &mut graph, 0.1, &clock, &mut parking);
        assert_eq!(agents.transit_mode[i], MODE_WALK);
        if agents.transit[i] == TRANSIT_IN_BUILDING { break; }
    }
    assert_eq!(agents.current_building[i], 1);
    assert_eq!(agents.transit[i], TRANSIT_IN_BUILDING);
    assert!(parking.vehicle_place(car).is_none());
}
