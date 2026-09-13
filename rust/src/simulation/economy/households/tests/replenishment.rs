// SPDX-License-Identifier: GPL-2.0-only

//! Household shopping, reservation, shortage, and cancellation tests.

use super::support::*;
use super::*;

#[test]
fn household_replenishment_uses_one_visible_shopper_trip() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "replenish_res", "replenish_com", 50.0, 20.0);

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    let shopper = households.households[0].shopping_agent_id;
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        40.0
    );
    assert_eq!(households.households[0].budget, 30.0);
    assert_eq!(agents.planned_target_building[shopper], 1);
    assert_eq!(agents.planned_activity[shopper], 2);

    arrive_agent_at_building(&mut agents, shopper, 1, 2);
    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_RETURNING
    );
    assert_eq!(households.households[0].stock, 0.0);
    assert!((allocator.buildings[1].revenue - 250.0).abs() < 0.001);
    assert_eq!(agents.planned_target_building[shopper], 0);
    assert_eq!(agents.planned_activity[shopper], 0);

    arrive_agent_at_building(&mut agents, shopper, 0, 0);
    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FULFILLED
    );
    assert_eq!(households.households[0].stock, 10.0);
}

#[test]
fn household_replenishment_reservation_uses_live_policy_vat() {
    let household = Household {
        home_building_id: 0,
        budget: 1_000.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "live_vat_res", "live_vat_store", 50.0, 20.0);

    let live_household_vat_rate = 0.20;
    households.run_household_operational_hour(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        0,
        live_household_vat_rate,
    );

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let unit_price = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .unit_price_currency;
    let reserved_amount = households.households[0].reserved_amount;
    let expected_reserved_total = reserved_amount * unit_price * (1.0 + live_household_vat_rate);
    assert!((reserved_amount - 10.0).abs() < 0.001);
    assert!(
        (households.households[0].reserved_total_cost - expected_reserved_total).abs() < 0.001,
        "reserved_total_cost should be priced with live VAT"
    );
}

#[test]
fn child_at_home_does_not_carry_household_shopping_trip() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 1,
        adult_count: 1,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "child_shop_res", "child_shop_com", 50.0, 20.0);
    let child = 0;
    agents.age_group[child] = AGE_CHILD;
    let adult = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[adult] = 0;
    agents.current_building[adult] = 1;
    agents.transit[adult] = TRANSIT_IN_BUILDING;
    agents.activity[adult] = 2;

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_WAITING_FOR_SHOPPER
    );
    assert_eq!(households.households[0].shopping_agent_id, usize::MAX);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        50.0
    );
}

#[test]
fn elder_can_carry_household_shopping_trip() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 1,
        child_count: 0,
        adult_count: 0,
        elder_count: 1,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "elder_shop_res", "elder_shop_com", 50.0, 20.0);
    agents.age_group[0] = AGE_ELDER;

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    assert_eq!(households.households[0].shopping_agent_id, 0);
}

#[test]
fn zero_stock_household_bypasses_replenishment_stagger_when_store_has_supply() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 5,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) = setup_replenishment_world(
        household,
        "urgent_replenish_res",
        "urgent_replenish_com",
        50.0,
        20.0,
    );

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
}

#[test]
fn household_can_restock_from_far_store() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 5,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "far_store_res", "far_store_com", 50.0, 6_000.0);

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    assert_eq!(households.households[0].reserved_store_building_id, 1);
}

#[test]
fn replenishment_search_cursor_reaches_next_store_window() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 1,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 24,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let (graph, network) = work_graph_to(6_300.0);
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "cursor_res", ZoneClass::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "cursor_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    for i in 0..409 {
        let x = if i == 24 {
            1_000.0
        } else if i < 24 {
            20.0 + i as f32 * 10.0
        } else {
            2_000.0 + i as f32 * 10.0
        };
        allocator.buildings.push(make_building(
            x,
            ZoneType::Commercial,
            &commercial_asset,
            50.0,
        ));
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let shopper = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[shopper] = 0;
    agents.current_building[shopper] = 0;
    agents.transit[shopper] = TRANSIT_IN_BUILDING;
    agents.activity[shopper] = 0;

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    assert_eq!(households.households[0].reserved_store_building_id, 25);
}

#[test]
fn replenishment_search_cursor_window_wraps_supplier_index() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 2,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 390,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let (graph, network) = work_graph_to(4_300.0);
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "cursor_wrap_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "cursor_wrap_com",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    for i in 0..400 {
        allocator.buildings.push(make_building(
            20.0 + i as f32 * 10.0,
            ZoneType::Commercial,
            &commercial_asset,
            50.0,
        ));
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let shopper = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[shopper] = 0;
    agents.current_building[shopper] = 0;
    agents.transit[shopper] = TRANSIT_IN_BUILDING;
    agents.activity[shopper] = 0;

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    assert_eq!(households.households[0].reserved_store_building_id, 1);
}

#[test]
fn unreachable_store_does_not_reserve_household_supplies() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, mut network, _graph) =
        setup_replenishment_world(
            household,
            "unreachable_res",
            "unreachable_com",
            50.0,
            1_000.0,
        );

    let mut graph = RegionGraph::new();
    let h0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let h1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let s0 = graph.add_node(Vector3::new(900.0, 0.0, 0.0), NodeType::Junction);
    let s1 = graph.add_node(Vector3::new(1_100.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(h0, h1, -100.0, 100.0));
    graph.add_edge(make_road_edge(s0, s1, 900.0, 1_100.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    allocator.buildings[0].edge_idx = 0;
    allocator.buildings[1].edge_idx = 1;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(households.households[0].replenishment_failure_count, 1);
    assert_eq!(
        households.households[0].reserved_store_building_id,
        usize::MAX
    );
    assert_eq!(households.households[0].budget, 300.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        50.0
    );
}

#[test]
fn deserted_store_cannot_sell_household_supplies() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "deserted_res", "deserted_store", 50.0, 20.0);
    allocator.buildings[1].is_deserted = true;
    allocator.rebuild_zone_index();

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(
        households.households[0].reserved_store_building_id,
        usize::MAX
    );
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        50.0
    );
}

#[test]
fn repeated_replenishment_failures_become_terminal_shortage() {
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: tuning
            .operational_clock
            .household_replenishment_terminal_failure_count
            .saturating_sub(1),
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "terminal_res", "terminal_store", 0.0, 20.0);

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FAILED_TERMINAL
    );
    assert_eq!(households.households[0].cooldown_hours, 0);
}

#[test]
fn household_waits_without_reservation_when_no_member_is_home() {
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household, "waiting_res", "waiting_store", 50.0, 20.0);
    agents.transit[0] = TRANSIT_ACCESS_INGRESS;
    agents.current_building[0] = usize::MAX;

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_WAITING_FOR_SHOPPER
    );
    assert_eq!(households.households[0].budget, 300.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        50.0
    );

    arrive_agent_at_building(&mut agents, 0, 0, 0);
    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 1);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
}

#[test]
fn canceled_shopping_to_store_restores_reserved_store_inventory() {
    let mut household = Household {
        home_building_id: 0,
        budget: 0.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_SHOPPING_TO_STORE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        shopping_agent_id: 0,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 8,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household.clone(), "cancel_res", "cancel_store", 0.0, 20.0);
    household.shopping_agent_schedule_seed = agents.schedule_seed[0];
    households.households[0] = household;
    allocator.buildings[1].is_deserted = true;
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(households.households[0].budget, 10.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        5.0
    );
    assert_eq!(agents.planned_target_building[0], usize::MAX);
}

#[test]
fn shopping_timeout_restores_pre_pickup_reservation() {
    let mut household = Household {
        home_building_id: 0,
        budget: 0.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_SHOPPING_TO_STORE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        shopping_agent_id: 0,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 1,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) =
        setup_replenishment_world(household.clone(), "timeout_res", "timeout_store", 0.0, 20.0);
    household.shopping_agent_schedule_seed = agents.schedule_seed[0];
    households.households[0] = household;

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(households.households[0].replenishment_failure_count, 1);
    assert_eq!(households.households[0].budget, 10.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        5.0
    );
}

#[test]
fn invalidating_home_restores_pre_pickup_store_reservation() {
    let mut household = Household {
        home_building_id: 0,
        budget: 0.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_SHOPPING_TO_STORE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        shopping_agent_id: 0,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 8,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, agents, _network, _graph) = setup_replenishment_world(
        household.clone(),
        "home_removed_res",
        "home_removed_store",
        0.0,
        20.0,
    );
    household.shopping_agent_schedule_seed = agents.schedule_seed[0];
    households.households[0] = household;

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    households.invalidate_building(0, &mut allocator);

    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(households.households[0].replenishment_failure_count, 1);
    assert_eq!(households.households[0].budget, 10.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        5.0
    );
}

#[test]
fn terminal_replenishment_shortage_retries_on_normal_cadence() {
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let household = Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_FAILED_TERMINAL,
        cooldown_hours: 0,
        replenishment_failure_count: tuning
            .operational_clock
            .household_replenishment_terminal_failure_count,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) = setup_replenishment_world(
        household,
        "terminal_retry_res",
        "terminal_retry_com",
        50.0,
        20.0,
    );

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
}

#[test]
fn store_losing_household_supply_profile_before_pickup_restores_reservation() {
    let mut household = Household {
        home_building_id: 0,
        budget: 0.0,
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_SHOPPING_TO_STORE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        shopping_agent_id: 0,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 8,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) = setup_replenishment_world(
        household.clone(),
        "profile_lost_res",
        "profile_lost_com",
        0.0,
        20.0,
    );
    household.shopping_agent_schedule_seed = agents.schedule_seed[0];
    households.households[0] = household;
    allocator.buildings[1].economy_profile_runtime_id = 0;

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_COOLDOWN
    );
    assert_eq!(households.households[0].budget, 10.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        5.0
    );
}

#[test]
fn low_stock_household_can_buy_affordable_partial_restock() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let unit_price = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile")
        .unit_price_currency;
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let partial_units = 5.0;

    let household = Household {
        home_building_id: 0,
        budget: partial_units * unit_price * (1.0 + tuning.fiscal.household_vat_rate),
        stock: 0.0,
        member_count: 2,
        child_count: 0,
        adult_count: 2,
        elder_count: 0,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    };
    let (mut households, mut allocator, mut agents, network, graph) = setup_replenishment_world(
        household,
        "partial_restock_res",
        "partial_restock_com",
        50.0,
        20.0,
    );

    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE
    );
    assert!((households.households[0].reserved_amount - partial_units).abs() < f32::EPSILON);
    assert_eq!(households.households[0].budget, 0.0);

    let shopper = households.households[0].shopping_agent_id;
    arrive_agent_at_building(&mut agents, shopper, 1, 2);
    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);
    arrive_agent_at_building(&mut agents, shopper, 0, 0);
    households.run_household_replenishment(&mut agents, &mut allocator, &network, &graph, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FULFILLED
    );
    assert!((households.households[0].stock - partial_units).abs() < f32::EPSILON);
    assert!((households.households[0].stock_days - 2.5).abs() < f32::EPSILON);
}

#[test]
fn shopping_candidates_remain_bounded_without_growing_reserved_storage() {
    use super::super::replenishment::insert_shopping_candidate;
    let mut allocator = BuildingAllocator::new();
    for idx in 0..32 {
        allocator.buildings.push(make_building(
            (idx / 2) as f32,
            ZoneType::Commercial,
            "test:shopping_candidates",
            0.0,
        ));
    }
    for limit in [0, 8, 24] {
        let mut candidates = Vec::with_capacity(limit);
        let capacity = candidates.capacity();
        for idx in (0..allocator.buildings.len()).rev() {
            insert_shopping_candidate(&mut candidates, limit, idx, 0.0, 0.0, &allocator);
            assert!(candidates.len() <= limit);
            assert_eq!(
                candidates.capacity(),
                capacity,
                "limit={limit}, candidate={idx}"
            );
        }
        assert_eq!(candidates, (0..limit).collect::<Vec<_>>());
        insert_shopping_candidate(&mut candidates, limit, 0, 0.0, 0.0, &allocator);
        insert_shopping_candidate(&mut candidates, limit, usize::MAX, 0.0, 0.0, &allocator);
        assert_eq!(candidates, (0..limit).collect::<Vec<_>>());
    }
}

#[test]
#[ignore = "manual matched release timing of bounded shopping destination selection"]
fn benchmark_shopping_candidate_selection() {
    use super::super::replenishment::insert_shopping_candidate;
    use std::{hint::black_box, time::Instant};
    for (limit, scanned) in [(8, 128), (24, 384)] {
        let mut allocator = BuildingAllocator::new();
        for idx in 0..scanned {
            allocator.buildings.push(make_building(
                (idx / 2) as f32,
                ZoneType::Commercial,
                "test:shopping_candidate_benchmark",
                0.0,
            ));
        }
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let mut candidates = Vec::with_capacity(limit);
            let start = Instant::now();
            for _ in 0..1_000 {
                candidates.clear();
                for idx in (0..scanned).rev() {
                    insert_shopping_candidate(
                        &mut candidates,
                        limit,
                        black_box(idx),
                        0.0,
                        0.0,
                        &allocator,
                    );
                }
                black_box(&candidates);
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0;
            assert_eq!(candidates, (0..limit).collect::<Vec<_>>());
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "shopping_candidates limit={limit} scanned={scanned} queries=1000 median_ms={:.3}",
            samples[5]
        );
    }
}
