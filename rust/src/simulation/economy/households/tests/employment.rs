//! Employment search, claims, wages, and founding assignment tests.

use super::support::*;
use super::*;

#[test]
fn no_car_agent_can_take_walk_reachable_job() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_foot_only_edge(n0, n1));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "walk_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "walk_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = false;

    households.recount_worker_assignments(&agents, &mut allocator);
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
}

#[test]
fn children_and_elders_do_not_take_jobs() {
    let (graph, network) = simple_work_graph();
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));
    households.households[0].child_count = 1;
    households.households[0].adult_count = 0;
    households.households[0].elder_count = 1;
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "age_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "age_job_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let child = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[child] = 0;
    agents.age_group[child] = AGE_CHILD;
    agents.transit[child] = TRANSIT_IN_BUILDING;
    agents.current_building[child] = 0;
    let elder = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[elder] = 0;
    agents.age_group[elder] = AGE_ELDER;
    agents.transit[elder] = TRANSIT_IN_BUILDING;
    agents.current_building[elder] = 0;

    households.recount_worker_assignments(&agents, &mut allocator);
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[child], usize::MAX);
    assert_eq!(agents.work_building[elder], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
}

#[test]
fn worker_can_take_far_reachable_job() {
    let (graph, network) = work_graph_to(6_500.0);
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "far_job_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        6_000.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;

    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
}

#[test]
fn workplace_claim_falls_back_to_next_ranked_job_when_best_fills() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let daily_wage = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "fallback_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "fallback_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        30.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = daily_wage;
    allocator.buildings[2].operating_budget = daily_wage;
    allocator.rebuild_zone_index();
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for agent in [a0, a1] {
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.has_car[agent] = true;
    }

    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.work_building[a1], 2);
    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(allocator.buildings[2].worker_count, 1);
}

#[test]
fn missing_entrance_cache_does_not_use_straight_line_work_fallback() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "missing_cache_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "missing_cache_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;

    let (graph, network) = simple_work_graph();
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
}

#[test]
fn deserted_employer_is_ejected_before_wages() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let daily_wage = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_wage_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_wage_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = daily_wage;
    allocator.buildings[1].worker_count = 1;
    allocator.buildings[1].is_deserted = true;

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(allocator.buildings[1].operating_budget, daily_wage);
}

#[test]
fn insolvent_self_fire_decrements_worker_count() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "insolvent_wage_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "insolvent_wage_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].operating_budget = 0.0;
    allocator.buildings[1].worker_count = 1;

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);
    agents.consecutive_unpaid_days[agent] = 1;

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(allocator.buildings[1].operating_budget, 0.0);
}

#[test]
fn full_current_workplace_is_scored_before_switching() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    let worker_capacity = grocery.worker_capacity;
    let daily_wage = grocery.average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));
    households.households[0].budget = 0.0;
    households.households[0].stock = 0.0;
    households.households[0].stock_days = 0.0;

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "stay_job_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stay_job_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        200.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[1].worker_count = worker_capacity;
    allocator.buildings[1].operating_budget = daily_wage * worker_capacity as f32;
    allocator.buildings[2].operating_budget = daily_wage;
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;
    agents.assign_work_building(agent, 1, 0);
    agents.job_lock_days[agent] = 0;

    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(allocator.buildings[1].worker_count, worker_capacity);
    assert_eq!(allocator.buildings[2].worker_count, 0);
}

#[test]
fn immigrant_household_assigns_nearby_work_during_founding() {
    let mut households = HouseholdSystem::new();
    let catalog = load_runtime_economy_catalog().expect("catalog");
    let tuning = load_runtime_economy_tuning().expect("tuning");
    let hid = households.admit_immigrant_household(&catalog, &tuning, 0, 2);
    households.households[hid].budget = 0.0;
    households.households[hid].stock = 1.0;
    households.households[hid].stock_days = 0.5;

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "res_house", ZoneClass::Residential);
    let industrial_asset =
        register_test_asset(&mut allocator, "test", "ind_shop", ZoneClass::Industrial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Industrial,
        &industrial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();
    let (graph, network) = simple_work_graph();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.household_id[a] = hid;
        agents.transit[a] = TRANSIT_IN_BUILDING;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.current_node[a] = 0;
        agents.has_car[a] = true;
    }

    households.consume_household_stock(&mut agents);
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.planned_activity[a0], 0);
    assert_eq!(agents.planned_target_building[a0], usize::MAX);
}

#[test]
fn operational_hour_tick_rebuilds_household_and_worker_counts_together() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 0, 30.0, 30.0);
    household.stock = 100.0;
    household.stock_days = 100.0;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "fused_tick_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "fused_tick_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.work_building[agent] = 1;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;

    let mut logistics = ShipmentSystem::new();
    let network = TransitNetwork::new();
    let graph = RegionGraph::new();
    let mut treasury_balance = 0.0;
    households.operational_hour_tick(
        &mut agents,
        &mut allocator,
        &mut logistics,
        &network,
        &graph,
        0,
        0,
        &mut treasury_balance,
        &[],
        &crate::simulation::economy::fiscal::CityFiscalPolicy::default(),
    );

    assert_eq!(households.households[0].member_count, 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
}
