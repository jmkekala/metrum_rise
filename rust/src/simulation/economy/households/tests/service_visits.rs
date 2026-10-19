// SPDX-License-Identifier: GPL-2.0-only

//! Representative personal-service visit tests.

use super::support::*;
use super::*;
use crate::simulation::economy::agents::{ACTIVITY_HOME, ACTIVITY_SHOPPING};

#[test]
fn personal_service_demand_schedules_visible_barber_visit() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_visit_home",
        ZoneClass::Residential,
    );
    let barber_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "service_visit_barber",
        "personal_service_small",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut barber = make_building(30.0, ZoneType::Commercial, &barber_asset, 0.0);
    barber.economy_profile_runtime_id = personal_service_profile.runtime_id;
    barber.worker_count = 1;
    barber.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(barber);
    let (mut graph, mut network) = simple_work_graph();
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(0, 1000, 10.0, 10.0));
    let mut agents = AgentSystem::new();
    let visitor = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(visitor, 0);

    households.run_visible_service_visits_for_test(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        0,
    );

    assert_eq!(agents.planned_target_building[visitor], 1);
    assert_eq!(agents.planned_activity[visitor], ACTIVITY_SHOPPING);

    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    households.job_funding.customers.refresh(
        &households.households,
        &allocator,
        &catalog,
        &tuning,
        &policy,
        1_000_000.0,
        &network,
        &graph,
    );
    assert!(
        households
            .job_funding
            .customers
            .walking_proofs(&network)
            .is_some()
    );
    // Appending a building and publishing fresh entrances leave lane connectivity intact.
    // Unchanged home geometry remains reusable; changed or invalid entrances need fresh work.
    allocator.buildings.push(make_building(
        90.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    for invalidate in [false, true] {
        if invalidate {
            network.cch_graph.metric_revision += 1;
        }
        assert_eq!(
            households
                .job_funding
                .customers
                .walking_proofs(&network)
                .is_none(),
            invalidate
        );
        for invalid_entrance in [false, true] {
            let entrance = allocator.entrances[0].clone();
            if invalid_entrance {
                allocator.entrances[0] = Default::default();
            }
            let cached = households
                .job_funding
                .customers
                .household_walking_access(0, 0, &allocator, &network);
            assert_eq!(cached.is_some(), !invalidate && !invalid_entrance);
            if let Some(cached) = cached {
                let walking = households
                    .job_funding
                    .customers
                    .walking_proofs(&network)
                    .unwrap();
                assert_eq!(
                    cached,
                    crate::simulation::economy::agents::tick::BuildingWalkingAccess::at_entrance(
                        &allocator.entrances[0],
                        &network,
                        &graph,
                        walking
                    )
                );
            }
            assert!(
                households
                    .job_funding
                    .customers
                    .household_walking_access(0, 2, &allocator, &network)
                    .is_none()
            );
            assert!(
                households
                    .job_funding
                    .customers
                    .household_walking_access(1, 0, &allocator, &network)
                    .is_none()
            );
            agents.planned_target_building[visitor] = usize::MAX;
            agents.planned_activity[visitor] = ACTIVITY_HOME;
            households.run_visible_service_visits_for_test(
                &mut agents,
                &mut allocator,
                &network,
                &graph,
                0,
            );
            assert_eq!(
                agents.planned_target_building[visitor],
                if invalid_entrance { usize::MAX } else { 1 }
            );
            assert_eq!(
                agents.planned_activity[visitor],
                if invalid_entrance {
                    ACTIVITY_HOME
                } else {
                    ACTIVITY_SHOPPING
                }
            );
            allocator.entrances[0] = entrance;
        }
    }
    households.job_funding.customers.refresh(
        &households.households,
        &allocator,
        &catalog,
        &tuning,
        &policy,
        1_000_000.0,
        &network,
        &graph,
    );
    assert!(
        households
            .job_funding
            .customers
            .walking_proofs(&network)
            .is_some()
    );
    // Terrain authoring may rebuild the same lane count without rebuilding CCH.
    let mut copied = crate::simulation::economy::customers::ConsumerMarket::default();
    copied.copy_reservations_from(&households.job_funding.customers);
    let count = network.lane_system.lanes.len();
    network.lane_system.rebuild(&mut graph);
    assert_eq!(network.lane_system.lanes.len(), count);
    assert!(
        households
            .job_funding
            .customers
            .walking_proofs(&network)
            .is_none()
    );
    households.job_funding.customers.refresh(
        &households.households,
        &allocator,
        &catalog,
        &tuning,
        &policy,
        1_000_000.0,
        &network,
        &graph,
    );
    copied.copy_reservations_from(&households.job_funding.customers);
    assert!(
        copied.walking_proofs(&network).is_some(),
        "construction forks follow the lane revision"
    );
}

#[test]
fn personal_service_visit_waits_when_household_needs_replenishment() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_wait_home",
        ZoneClass::Residential,
    );
    let barber_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "service_wait_barber",
        "personal_service_small",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut barber = make_building(30.0, ZoneType::Commercial, &barber_asset, 0.0);
    barber.economy_profile_runtime_id = personal_service_profile.runtime_id;
    barber.worker_count = 1;
    barber.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(barber);
    let (graph, network) = simple_work_graph();
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 1000, 10.0, 0.5);
    household.replenishment_state = REPLENISHMENT_NEEDS;
    households.households.push(household);
    let mut agents = AgentSystem::new();
    let visitor = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(visitor, 0);

    households.run_visible_service_visits_for_test(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        0,
    );

    assert_eq!(agents.planned_target_building[visitor], usize::MAX);
}

#[test]
fn health_essentials_service_does_not_spawn_pharmacy_visit_yet() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let health_profile = catalog
        .profile_for_id("health_essentials_small")
        .expect("health essentials profile");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "pharmacy_visit_home",
        ZoneClass::Residential,
    );
    let pharmacy_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "pharmacy_visit_store",
        "health_essentials_small",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut pharmacy = make_building(30.0, ZoneType::Commercial, &pharmacy_asset, 0.0);
    pharmacy.economy_profile_runtime_id = health_profile.runtime_id;
    pharmacy.worker_count = 1;
    pharmacy.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(pharmacy);
    let (graph, network) = simple_work_graph();
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(0, 1000, 10.0, 10.0));
    let mut agents = AgentSystem::new();
    let visitor = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(visitor, 0);

    households.run_visible_service_visits_for_test(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        0,
    );

    assert_eq!(agents.planned_target_building[visitor], usize::MAX);
}

#[test]
fn personal_service_visitor_returns_home_on_next_household_pass() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_return_home",
        ZoneClass::Residential,
    );
    let barber_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "service_return_barber",
        "personal_service_small",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut barber = make_building(30.0, ZoneType::Commercial, &barber_asset, 0.0);
    barber.economy_profile_runtime_id = personal_service_profile.runtime_id;
    barber.worker_count = 1;
    barber.commercial_activity_floor_scale = 1.0;
    allocator.buildings.push(barber);
    let (graph, network) = simple_work_graph();
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 10.0, 10.0));
    let mut agents = AgentSystem::new();
    let visitor = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(visitor, 0);
    arrive_agent_at_building(&mut agents, visitor, 1, ACTIVITY_SHOPPING);

    households.run_visible_service_visits_for_test(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        1,
    );

    assert_eq!(agents.planned_target_building[visitor], 0);
    assert_eq!(agents.planned_activity[visitor], ACTIVITY_HOME);
}

#[test]
#[ignore = "manual matched release timing of retained household walking access"]
fn benchmark_retained_household_walking_access() {
    use crate::simulation::economy::agents::tick::BuildingWalkingAccess;
    use crate::simulation::economy::customers::ConsumerMarket;
    use std::hint::black_box;
    use std::time::Instant;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (graph, network) = simple_work_graph();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "retained_walk",
        ZoneClass::Residential,
    );
    for x in [0.0, 120.0] {
        allocator
            .buildings
            .push(make_building(x, ZoneType::Residential, &asset, 0.0));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let household = make_household(0, 2, 10.0, 10.0);
    for count in [1_024, 65_536] {
        let households: Vec<_> = (0..count)
            .map(|index| {
                let mut household = household.clone();
                household.home_building_id = index % 2;
                household
            })
            .collect();
        let mut market = ConsumerMarket::default();
        market.refresh(
            &households,
            &allocator,
            &catalog,
            &tuning,
            &policy,
            1_000_000.0,
            &network,
            &graph,
        );
        let walking = market.walking_proofs(&network).unwrap();
        let expected = std::array::from_fn::<_, 2, _>(|home| {
            BuildingWalkingAccess::at_entrance(
                &allocator.entrances[home],
                &network,
                &graph,
                walking,
            )
        });
        for (index, household) in households.iter().enumerate() {
            assert_eq!(
                market.household_walking_access(
                    index,
                    household.home_building_id,
                    &allocator,
                    &network
                ),
                Some(expected[household.home_building_id])
            );
        }
        for pair in 0..11 {
            for retained in if pair % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = Instant::now();
                for _ in 0..4 {
                    for (index, household) in households.iter().enumerate() {
                        let home = black_box(household.home_building_id);
                        let proof = if retained {
                            market
                                .household_walking_access(
                                    black_box(index),
                                    home,
                                    &allocator,
                                    &network,
                                )
                                .unwrap()
                        } else {
                            BuildingWalkingAccess::at_entrance(
                                &allocator.entrances[home],
                                &network,
                                &graph,
                                walking,
                            )
                        };
                        black_box(proof);
                    }
                }
                let query_ns = start.elapsed().as_secs_f64() * 1e9 / (4 * count) as f64;
                eprintln!(
                    "HOME_WALK {{\"households\":{count},\"pair\":{pair},\"retained\":{retained},\"query_ns\":{query_ns}}}"
                );
            }
        }
    }
}
