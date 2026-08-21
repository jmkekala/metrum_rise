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
    let (graph, network) = simple_work_graph();
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(0, 1000, 10.0, 10.0));
    let mut agents = AgentSystem::new();
    let visitor = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[visitor] = 0;

    households.run_visible_service_visits_for_test(&mut agents, &allocator, &network, &graph, 0);

    assert_eq!(agents.planned_target_building[visitor], 1);
    assert_eq!(agents.planned_activity[visitor], ACTIVITY_SHOPPING);
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
    agents.household_id[visitor] = 0;

    households.run_visible_service_visits_for_test(&mut agents, &allocator, &network, &graph, 0);

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
    agents.household_id[visitor] = 0;

    households.run_visible_service_visits_for_test(&mut agents, &allocator, &network, &graph, 0);

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
    agents.household_id[visitor] = 0;
    arrive_agent_at_building(&mut agents, visitor, 1, ACTIVITY_SHOPPING);

    households.run_visible_service_visits_for_test(&mut agents, &allocator, &network, &graph, 1);

    assert_eq!(agents.planned_target_building[visitor], 0);
    assert_eq!(agents.planned_activity[visitor], ACTIVITY_HOME);
}
