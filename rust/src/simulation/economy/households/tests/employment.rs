// SPDX-License-Identifier: GPL-2.0-only

//! Employment search, claims, wages, and founding assignment tests.

use super::support::*;
use super::*;

#[test]
fn local_stock_repricing_preserves_the_funded_import_alternative() {
    use crate::simulation::economy::operation::InputRequirement;
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 1.2;
    tuning.owa_export_price_multiplier = 1.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_utility_asset(&mut allocator, "test", "stock_grain", "grain_farm_basic");
    let factory_asset = register_test_asset(&mut allocator, "test", "stock_food", ZoneClass::Industrial);
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    let farm_workers = physical_worker_capacity_for_profile(&farm, farm_profile);
    allocator.buildings.push(farm);
    let mut factory = make_building(40.0, ZoneType::Industrial, &factory_asset, 0.0);
    factory.operating_budget = 0.0;
    factory.worker_count = factory_profile.worker_capacity;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    let mut retained_import = false;
    // Sweep the narrow capital boundary: excess grain finances payroll, so cheaper
    // replacement prices can lower available assets more than opening requirements.
    for stock in (800..=1_400).step_by(5) {
        allocator.buildings[1].set_inventory_units(grain, stock as f32);
        allocator.buildings[0].worker_count = 0;
        funding.refresh(&allocator, &[], &mut logistics, &network, &graph,
            &catalog, &tuning, &policy, &[], 100_000.0, 420);
        let imported = funding.capacities[1];
        allocator.buildings[0].worker_count = farm_workers;
        funding.refresh(&allocator, &[], &mut logistics, &network, &graph,
            &catalog, &tuning, &policy, &[], 100_000.0, 420);
        assert!(funding.capacities[1] >= imported,
            "optional local supply erased funded jobs at {stock} grain units");
        if imported == factory_profile.worker_capacity {
            let quote = logistics.quote_recurring_input(&mut funding.input_supply,
                InputRequirement { resource: grain, daily_units: 200.0, buffer_units: 800.0,
                    replenishment_batch_units: Some(400.0) },
                Some(1), &allocator.entrances[1], None, &allocator, &catalog, &tuning,
                &network, &graph, 1.0);
            retained_import |= quote.goods.is_finite();
            funding.input_supply.reject(&allocator, &graph);
        }
    }
    assert!(retained_import,
        "the import alternative must release the rejected local claim in full");
}

#[test]
fn unfinished_service_keeps_fiscal_ceiling_without_advertising_live_jobs() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (graph, mut network) = simple_work_graph();
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_utility_asset(&mut allocator, "test", "pending_power", "power_plant_basic");
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let mut plant = make_building(40.0, ZoneType::None, &asset, 0.0);
    plant.economy_profile_runtime_id = profile.runtime_id;
    plant.construction_remaining_hours = 24;
    plant.construction_total_hours = 24;
    allocator.buildings.push(plant);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    for fraction in [0.0, 0.5, 1.0] {
        funding.refresh(&allocator, &[], &mut logistics, &network, &graph, &catalog, &tuning,
            &policy, &[fraction], 100_000.0, 420);
        assert_eq!(funding.capacities, [0]);
        assert_eq!(funding.service_capacities, [service_funded_worker_capacity(
            profile.worker_capacity, profile, 0, &[fraction],
        )]);
    }
    allocator.buildings[0].construction_remaining_hours = 0;
    funding.refresh(&allocator, &[], &mut logistics, &network, &graph, &catalog, &tuning,
        &policy, &[0.5], 100_000.0, 420);
    assert_eq!(funding.capacities, funding.service_capacities);
    assert!(funding.capacities[0] > 0);
    let mut borders = Vec::new();
    crate::simulation::economy::logistics::collect_connected_border_nodes(&graph, &mut borders);
    assert!(funding.has_current_input_supply(&allocator, &network, &borders, 420));
    assert!(!funding.has_current_input_supply(&allocator, &network, &borders,
        crate::simulation::economy::logistics::freight_price_phase(&tuning, 480)),
        "a new receiving-price interval cannot reuse earlier profitability");
    network.frontage_delay_revision += 1;
    assert!(!funding.has_current_input_supply(&allocator, &network, &borders, 420),
        "unchanged road geometry cannot validate stale recurring freight prices");
}

#[test]
fn fiscal_service_inputs_share_live_supply_without_turning_payroll_into_private_sales() {
    use crate::simulation::economy::operation::InputRequirement;

    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    for reachable in [true, false] {
        let mut allocator = BuildingAllocator::new();
        let source_asset = register_test_utility_asset(
            &mut allocator,
            "test",
            "service_machinery",
            "machinery_factory_basic",
        );
        let service_asset = register_test_utility_asset(
            &mut allocator,
            "test",
            "service_water",
            "water_plant_basic",
        );
        let source_profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
        let service_profile = catalog.profile_for_id("water_plant_basic").unwrap();
        let mut source = make_building(20.0, ZoneType::Industrial, &source_asset, 0.0);
        source.economy_profile_runtime_id = source_profile.runtime_id;
        source.worker_count = source_profile.worker_capacity;
        source.operating_budget = 100_000.0;
        allocator.buildings.push(source);
        let mut service = make_building(40.0, ZoneType::None, &service_asset, 0.0);
        service.economy_profile_runtime_id = service_profile.runtime_id;
        service.worker_count = service_profile.worker_capacity;
        service.operating_budget = 0.0;
        allocator.buildings.push(service);
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let proposed_door = allocator.entrances[1].clone();
        if !reachable {
            allocator.entrances[1].car_lane_fwd = usize::MAX;
            allocator.entrances[1].car_lane_bkw = usize::MAX;
        }
        let mut households = HouseholdSystem::new();
        let mut logistics = ShipmentSystem::new();
        households.job_funding.refresh(
            &allocator,
            &[],
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0, 420,
        );
        assert_eq!(
            households.job_funding.capacities,
            [4, 3],
            "public payroll follows fiscal funding, not a private-sales profitability test"
        );
        let resource = catalog.resource_runtime_id_for_id("machinery").unwrap();
        for (units, expected) in [(39.5, true), (40.0, !reachable)] {
            let book = &mut households.job_funding.input_supply;
            let costs = logistics.quote_recurring_input(
                book,
                InputRequirement {
                    resource,
                    daily_units: units,
                    buffer_units: units * 2.0,
                    replenishment_batch_units: Some(10.0),
                },
                None,
                &proposed_door,
                None,
                &allocator,
                &catalog,
                &tuning,
                &network,
                &graph, 1.0,
            );
            assert_eq!(
                costs.goods.is_finite(),
                expected,
                "only a reachable water plant reserves its 0.5 daily machinery input"
            );
            book.reject(&allocator, &graph);
        }
        assert_eq!(allocator.buildings[0].operating_budget, 100_000.0);
        assert_eq!(allocator.buildings[1].operating_budget, 0.0);
    }
}

#[test]
fn post_assignment_sync_refreshes_employment_without_census_member_links() {
    let mut agents = AgentSystem::new();
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 3, 1_000.0, 5.0));
    households.households.push(make_household(usize::MAX, 1, 1_000.0, 5.0));
    for hid in [0, 0, 0, 1] {
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[agent] = hid;
    }
    // The census predates these job assignments. No membership links are available here.
    agents.assign_work_building(0, 1, 0);
    agents.assign_work_building(3, 1, 0);
    households.sync_agent_money_from_households(&mut agents);
    assert_eq!(households.households[0].employed_adults, 1);
    assert_eq!(households.households[1].employed_adults, 1);
    agents.assign_work_building(0, usize::MAX, 0);
    agents.assign_work_building(1, 2, 0);
    agents.assign_work_building(2, 2, 0);
    households.sync_agent_money_from_households(&mut agents);
    assert_eq!(households.households[0].employed_adults, 2);
}

#[test]
fn live_hiring_shares_finite_local_grain_and_releases_rejected_capital_claims() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_import_price_multiplier = 2.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_utility_asset(&mut allocator, "test", "finite_grain", "grain_farm_basic");
    let factory_asset = register_test_asset(&mut allocator, "test", "finite_food", ZoneClass::Industrial);
    let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 10.0;
    farm.operating_budget = 100_000.0;
    farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
    allocator.buildings.push(farm);
    for x in [40.0, 60.0] {
        let mut factory = make_building(x, ZoneType::Industrial, &factory_asset, 0.0);
        factory.operating_budget = 100_000.0;
        allocator.buildings.push(factory);
    }
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let refresh = |households: &mut HouseholdSystem, logistics: &mut ShipmentSystem, allocator: &BuildingAllocator| {
        households.job_funding.refresh(allocator, &[], logistics, &network, &graph, &catalog,
            &tuning, &policy, &[], 100_000.0, 420);
    };
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(households.job_funding.capacities[1], factory_profile.worker_capacity);
    assert_eq!(households.job_funding.capacities[2], 0,
        "a second factory cannot reuse the farm's 200 daily grain units");
    allocator.buildings[1].operating_budget = 0.0;
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(households.job_funding.capacities[1], 0);
    assert_eq!(households.job_funding.capacities[2], factory_profile.worker_capacity,
        "capital rejection must release every staged local input claim");
    assert_eq!(allocator.buildings[2].operating_budget, 100_000.0);
    allocator.buildings[0].worker_count = 0;
    allocator.buildings[0].set_inventory_units(farm_profile.outputs[0].resource_runtime_id, 1_000_000.0);
    refresh(&mut households, &mut logistics, &allocator);
    assert_eq!(households.job_funding.capacities[2], 0,
        "stock without active production cannot promise recurring staffing");
}

#[test]
fn locally_funded_staffed_processor_supplies_the_next_funded_shop() {
    for source_before_shop in [true, false] {
        let catalog = load_runtime_economy_catalog().unwrap();
        let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
        tuning.owa_import_price_multiplier = 2.0;
        let policy =
            crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
        let (mut graph, network) = simple_work_graph();
        graph.set_node_type(0, NodeType::Border);
        let mut allocator = BuildingAllocator::new();
        let farm_asset =
            register_test_utility_asset(&mut allocator, "test", "chain_grain", "grain_farm_basic");
        let factory_asset =
            register_test_asset(&mut allocator, "test", "chain_food", ZoneClass::Industrial);
        let shop_asset =
            register_test_asset(&mut allocator, "test", "chain_shop", ZoneClass::Commercial);
        let home_asset =
            register_test_asset(&mut allocator, "test", "chain_home", ZoneClass::Residential);
        let farm_profile = catalog.profile_for_id("grain_farm_basic").unwrap();
        let factory_profile = catalog.profile_for_id("food_processor_basic").unwrap();
        let shop_profile = catalog.profile_for_id("grocery_basic").unwrap();
        let mut farm = make_building(20.0, ZoneType::None, &farm_asset, 0.0);
        farm.economy_profile_runtime_id = farm_profile.runtime_id;
        farm.work_area_scale = 10.0;
        farm.worker_count = physical_worker_capacity_for_profile(&farm, farm_profile);
        farm.operating_budget = 100_000.0;
        allocator.buildings.push(farm);
        for (x, zone, asset, workers) in [
            (
                40.0,
                ZoneType::Industrial,
                &factory_asset,
                factory_profile.worker_capacity,
            ),
            (
                60.0,
                ZoneType::Commercial,
                &shop_asset,
                shop_profile.worker_capacity,
            ),
        ] {
            let mut building = make_building(x, zone, asset, 0.0);
            building.worker_count = workers;
            building.operating_budget = 100_000.0;
            allocator.buildings.push(building);
        }
        allocator
            .buildings
            .push(make_building(80.0, ZoneType::Residential, &home_asset, 0.0));
        let (factory, shop) = if source_before_shop {
            (1, 2)
        } else {
            allocator.buildings.swap(1, 2);
            (2, 1)
        };
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        let mut households = HouseholdSystem::new();
        let mut customer = make_household(3, 200, 100.0, 1.0);
        customer.gross_daily_wages = 100_000.0;
        households.households.push(customer);
        let mut logistics = ShipmentSystem::new();
        let refresh = |households: &mut HouseholdSystem,
                       logistics: &mut ShipmentSystem,
                       allocator: &BuildingAllocator| {
            households.job_funding.refresh(
                allocator,
                &households.households,
                logistics,
                &network,
                &graph,
                &catalog,
                &tuning,
                &policy,
                &[],
                100_000.0,
                420,
            );
        };
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[factory],
            factory_profile.worker_capacity
        );
        assert_eq!(
            households.job_funding.capacities[shop], shop_profile.worker_capacity,
            "the locally funded processor's staffed production must reach the shop"
        );
        assert!(
            !households
                .job_funding
                .input_supply
                .offers_inputs(shop_profile),
            "the shop must reserve the processor's entire finite output"
        );
        allocator.buildings[factory].worker_count = 0;
        allocator.buildings[factory]
            .set_inventory_units(factory_profile.outputs[0].resource_runtime_id, 1_000_000.0);
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[factory],
            factory_profile.worker_capacity
        );
        assert_eq!(
            households.job_funding.capacities[shop], 0,
            "unfilled funded jobs and existing stock cannot publish recurring production"
        );
        allocator.buildings[factory].worker_count = factory_profile.worker_capacity;
        allocator.buildings[factory]
            .set_inventory_units(factory_profile.outputs[0].resource_runtime_id, 0.0);
        allocator.buildings[factory].operating_budget = 0.0;
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(households.job_funding.capacities[factory], 0);
        assert_eq!(
            households.job_funding.capacities[shop], 0,
            "a rejected producer cannot publish stock or leave a funded downstream operation"
        );
        allocator.buildings[factory].operating_budget = 100_000.0;
        refresh(&mut households, &mut logistics, &allocator);
        assert_eq!(
            households.job_funding.capacities[shop],
            shop_profile.worker_capacity
        );
    }
}

// Commute/claim tests need funded jobs independent of a tiny fixture's retail market.
// City-funded workplaces keep the real job search and service-funding path under test.
fn fund_route_test_workplaces(allocator: &mut BuildingAllocator) {
    let asset = register_test_utility_asset(
        allocator,
        "test",
        "route_test_employer",
        "power_plant_basic",
    );
    let profile = load_runtime_economy_catalog()
        .unwrap()
        .profile_for_id("power_plant_basic")
        .unwrap()
        .runtime_id;
    for building in &mut allocator.buildings {
        if building.zone_type == ZoneType::Commercial {
            building.zone_type = ZoneType::None;
            building.asset_id = asset.clone();
            building.economy_profile_runtime_id = profile;
        }
    }
}

#[test]
fn live_hiring_and_admission_keep_jobs_after_paid_stock_and_payroll() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "funded_factory",
        ZoneClass::Industrial,
    );
    let mut factory = make_building(50.0, ZoneType::Industrial, &asset, 0.0);
    let capital = profile.zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier);
    factory.operating_budget = capital;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    let mut check = |allocator: &BuildingAllocator, logistics: &mut ShipmentSystem, expected| {
        households.job_funding.refresh(
            allocator,
            &households.households,
            logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0, 420,
        );
        assert_eq!(households.job_funding.capacities[0], expected);
        let mut demand = crate::simulation::economy::demand::DemandSystem::new();
        let zoning = crate::simulation::zoning::ZoningSystem::new(
            &crate::simulation::core::config::WorldConfig::default(),
        );
        let terrain = crate::simulation::terrain::TerrainSystem::new(4, 4);
        let mut surface = crate::simulation::network::surface::RoadSurfaceSystem::new(512.0);
        surface.compile_dirty(&graph, &terrain);
        demand.run_hourly_pass_with_policy(allocator, &households, &graph, &zoning,
            100_000.0, &policy, &network, logistics,
            crate::simulation::buildings::allocator::BuildingSiteEnvironment {
                road_surface: &surface, terrain: &terrain,
            }, 420);
        assert_eq!(
            demand.scenario_diagnostics().0.move_in_job_slots,
            expected,
            "admission and workplace assignment must consume the same funded jobs"
        );
    };
    check(&allocator, &mut logistics, profile.worker_capacity);
    let payroll = profile.worker_capacity as f32 * profile.average_daily_wage();
    let paid_goods = capital - payroll - 8.0;
    allocator.buildings[0].operating_budget = payroll;
    logistics.shipments.push(Shipment {
        id: 1,
        resource_runtime_id: profile.inputs[0].resource_runtime_id,
        amount: paid_goods
            / (catalog
                .unit_price_for_resource(profile.inputs[0].resource_runtime_id)
                .unwrap()
                * tuning.owa_import_price_multiplier),
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: 7,
        total_cost: paid_goods + 8.0,
        freight_cost: 8.0,
        eta_hours: 1,
        queued_hours: 0,
    });
    check(&allocator, &mut logistics, profile.worker_capacity);
    allocator.buildings[0].operating_budget -= payroll;
    check(&allocator, &mut logistics, profile.worker_capacity);
    allocator.buildings[0].set_inventory_units(
        profile.inputs[0].resource_runtime_id,
        logistics.shipments[0].amount,
    );
    logistics.shipments[0].status = ShipmentStatus::Fulfilled;
    check(&allocator, &mut logistics, profile.worker_capacity);
}

#[test]
fn owner_money_and_unprofitable_exports_do_not_create_live_jobs() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(
        &mut allocator,
        "test",
        "unfunded_factory",
        ZoneClass::Industrial,
    );
    let mut factory = make_building(50.0, ZoneType::Industrial, &asset, 0.0);
    factory.operating_budget = 1_000_000.0;
    allocator.buildings.push(factory);
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    let mut logistics = ShipmentSystem::new();
    for outside_bid in [0.0, 0.01] {
        tuning.owa_export_price_multiplier = outside_bid;
        households.job_funding.refresh(
            &allocator,
            &[],
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0, 420,
        );
        assert_eq!(
            households.job_funding.capacities[0], 0,
            "startup money cannot substitute for a market that covers recurring costs"
        );
    }
    tuning.owa_export_price_multiplier = 1.0;
    allocator.buildings[0].operating_budget = 0.0;
    households.job_funding.refresh(
        &allocator,
        &[],
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0, 420,
    );
    assert_eq!(
        households.job_funding.capacities[0], 0,
        "profitable sales still need working capital"
    );
    allocator.buildings[0].operating_budget = 1_000_000.0;
    allocator.buildings[0].construction_remaining_hours = 1;
    households.job_funding.refresh(
        &allocator,
        &[],
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0, 420,
    );
    assert_eq!(
        households.job_funding.capacities[0], 0,
        "future production cannot pay arrivals"
    );
    allocator.buildings[0].construction_remaining_hours = 0;
    for (role, expected) in [
        (NodeType::Border, 10),
        (NodeType::Junction, 0),
        (NodeType::Border, 10),
    ] {
        graph.set_node_type(0, role);
        households.job_funding.refresh(
            &allocator,
            &[],
            &mut logistics,
            &network,
            &graph,
            &catalog,
            &tuning,
            &policy,
            &[],
            100_000.0, 420,
        );
        assert_eq!(
            households.job_funding.capacities[0], expected,
            "gateway role changes must invalidate outside quotes even without a CCH rebuild"
        );
    }
}

#[test]
fn profitable_local_staffing_survives_an_unprofitable_full_export_plan() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut tuning = (*load_runtime_economy_tuning().unwrap()).clone();
    tuning.owa_export_price_multiplier = 0.01;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::from_runtime_tuning(&tuning);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
    let mut allocator = BuildingAllocator::new();
    let factory_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_factory",
        ZoneClass::Industrial,
    );
    let shop_asset =
        register_test_asset(&mut allocator, "test", "local_buyer", ZoneClass::Commercial);
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_customers",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Industrial,
        &factory_asset,
        0.0,
    ));
    allocator
        .buildings
        .push(make_building(40.0, ZoneType::Commercial, &shop_asset, 0.0));
    allocator
        .buildings
        .push(make_building(60.0, ZoneType::Residential, &home_asset, 0.0));
    allocator.buildings[0].operating_budget = 100_000.0;
    allocator.buildings[1].operating_budget = 100_000.0;
    allocator.buildings[1].worker_count = 8;
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(2, 100, 30.0, 30.0));
    let mut logistics = ShipmentSystem::new();
    households.job_funding.refresh(
        &allocator,
        &households.households,
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0, 420,
    );
    assert_eq!(
        households.job_funding.capacities[1], 0,
        "customer savings and owner capital cannot promise recurring shop payroll"
    );
    households.households[0].gross_daily_wages = 10_000.0;
    households.households[0].employed_adults = 100;
    households.job_funding.refresh(
        &allocator,
        &households.households,
        &mut logistics,
        &network,
        &graph,
        &catalog,
        &tuning,
        &policy,
        &[],
        100_000.0, 420,
    );
    assert!(households.job_funding.capacities[1] > 0);
    let jobs = households.job_funding.capacities[0];
    assert!(
        jobs > 0
            && jobs
                < catalog
                    .profile_for_id("food_processor_basic")
                    .unwrap()
                    .worker_capacity,
        "local buyers support a smaller operation even though full export production loses money: {jobs}"
    );
}

#[test]
#[ignore = "manual matched release timing of payroll plus final money publication"]
fn benchmark_payroll_phase() {
    use std::{hint::black_box, time::Instant};
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    for count in [1_024_usize, 8_192, 65_536] {
        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(make_building(0.0, ZoneType::Residential, "test:home", 0.0));
        let capacity = profile.worker_capacity as usize;
        for _ in 0..count.div_ceil(capacity) {
            let mut factory = make_building(30.0, ZoneType::Industrial, "test:factory", 0.0);
            factory.operating_budget = 1_000_000.0;
            for input in &profile.inputs {
                factory.set_inventory_units(input.resource_runtime_id, 1_000.0);
            }
            allocator.buildings.push(factory);
        }
        let mut agents = AgentSystem::new();
        let mut households = HouseholdSystem::new();
        for idx in 0..count {
            households
                .households
                .push(make_household(0, 1, 1_000.0, 5.0));
            let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
            agents.household_id[agent] = idx;
            let work = 1 + idx / capacity;
            agents.assign_work_building(agent, work, 0);
            allocator.buildings[work].worker_count += 1;
        }
        let logistics = ShipmentSystem::new();
        let mut treasury = 0.0;
        let mut phase = || {
            black_box(households.pay_daily_wages_with_service_funding(
                &mut agents,
                &mut allocator,
                0.12,
                &mut treasury,
                &[],
                &logistics,
                true,
            ));
            households.sync_agent_money_from_households(&mut agents);
        };
        phase();
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            let start = Instant::now();
            for _ in 0..4 {
                phase();
            }
            *sample = start.elapsed().as_secs_f64() * 1_000.0 / 4.0;
        }
        assert!(agents.work_building.iter().all(|&work| work != usize::MAX));
        assert!(
            agents
                .money
                .iter()
                .zip(&households.households)
                .all(|(&money, home)| money == home.budget)
        );
        samples.sort_by(f64::total_cmp);
        eprintln!("payroll_phase workers={count} median_ms={:.3}", samples[5]);
    }
}

#[test]
fn no_car_unemployed_adult_can_take_reachable_job_even_with_household_savings() {
    for (budget, stock_days) in [(0.0, 0.0), (100_000.0, 7.0)] {
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
        households.households[0].budget = budget;
        households.households[0].stock = stock_days;
        households.households[0].stock_days = stock_days;

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
        fund_route_test_workplaces(&mut allocator);
        allocator.rebuild_zone_index();
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);

        let mut agents = AgentSystem::new();
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.has_car[agent] = false;

        households.rebuild_household_and_worker_counts(&agents, &mut allocator);
        households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

        assert_eq!(agents.work_building[agent], 1);
        assert_eq!(allocator.buildings[1].worker_count, 1);
    }
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
    fund_route_test_workplaces(&mut allocator);
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

    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
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
    fund_route_test_workplaces(&mut allocator);
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
    let worker_capacity = catalog
        .profile_for_id("power_plant_basic")
        .expect("funded service profile")
        .worker_capacity;

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
    fund_route_test_workplaces(&mut allocator);
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

    households.assign_agent_workplaces_with_service_funding(
        &mut agents,
        &mut allocator,
        &network,
        &graph,
        &[
            0.0,
            1.0 / worker_capacity as f32,
            1.0 / worker_capacity as f32,
        ],
        false,
        &mut ShipmentSystem::new(),
        &crate::simulation::economy::fiscal::CityFiscalPolicy::default(),
        100_000.0, 420,
    );

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
    fund_route_test_workplaces(&mut allocator);
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
fn wage_payment_liquidates_output_inventory_before_unpaid() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let processor = catalog
        .profile_for_id("food_processor_basic")
        .expect("processor starter profile");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let daily_wage = processor.average_daily_wage();

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "inventory_wage_res",
        ZoneClass::Residential,
    );
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "inventory_wage_ind",
        ZoneClass::Industrial,
    );
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
    allocator.buildings[1].operating_budget = 0.0;
    allocator.buildings[1].worker_count = 1;
    allocator.buildings[1].set_inventory_units(packaged_food, 100.0);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(agents.consecutive_unpaid_days[agent], 0);
    assert!((households.households[0].budget - daily_wage).abs() <= 0.001);
    assert!(allocator.buildings[1].inventory_units(packaged_food) < 100.0);
    assert!(allocator.buildings[1].operating_budget.abs() <= 0.001);
}

#[test]
fn full_current_workplace_is_scored_before_switching() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let employer = catalog
        .profile_for_id("power_plant_basic")
        .expect("funded service profile");
    let worker_capacity = employer.worker_capacity;

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
    fund_route_test_workplaces(&mut allocator);
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
    let hid = households.admit_immigrant_household(
        &catalog,
        &tuning,
        0,
        2,
        crate::simulation::economy::agents::household_age_composition(0, 0, 2),
    );
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
    allocator.buildings[1].operating_budget = catalog
        .profile_for_id("food_processor_basic")
        .unwrap()
        .zoned_startup_budget(&catalog, tuning.owa_import_price_multiplier);
    let (mut graph, network) = simple_work_graph();
    graph.set_node_type(0, NodeType::Border);
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
        |_, _| {},
    );

    assert_eq!(households.households[0].member_count, 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
}

#[test]
fn consumer_market_uses_each_familys_income_instead_of_wallet_balance() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "consumer_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut families = vec![
        make_household(0, 1, 0.0, 0.0),
        make_household(0, 1, 0.0, 0.0),
    ];
    families[0].gross_daily_wages = 100_000.0;
    families[0].employed_adults = 1;
    families[1].budget = 1_000_000.0;
    let buyers = |families: &[Household],
                  policy: &crate::simulation::economy::fiscal::CityFiscalPolicy| {
        affordable_consumer_residents(families, &allocator, &catalog, &tuning, policy, 100_000.0)
    };
    assert_eq!(
        buyers(&families, &policy),
        1.0,
        "one rich family cannot finance another's recurring purchases; savings are finite"
    );
    families[0].budget = 0.0;
    assert_eq!(
        buyers(&families, &policy),
        1.0,
        "paying a bill cannot erase recurring wages"
    );
    families[1].gross_daily_wages = 30.0;
    families[1].employed_adults = 1;
    let partial = buyers(&families, &policy);
    assert!(partial > 1.0 && partial < 2.0, "{partial}");
    policy.household_vat_rate = 1.0;
    assert!(
        buyers(&families, &policy) < partial,
        "VAT must not become store revenue"
    );
}

#[test]
fn consumer_market_reserves_transfers_once_and_excludes_temporary_income() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "pension_home",
        ZoneClass::Residential,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut pensioner = make_household(0, 1, 0.0, 0.0);
    pensioner.adult_count = 0;
    pensioner.elder_count = 1;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        pension_per_elder_per_day: 100.0,
        unemployment_benefit_per_adult_per_day: 100.0,
        ..Default::default()
    };
    let buyers = |families: &[Household], treasury| {
        affordable_consumer_residents(families, &allocator, &catalog, &tuning, &policy, treasury)
    };
    assert_eq!(buyers(&[pensioner.clone(), pensioner.clone()], 100.0), 1.0);
    assert_eq!(buyers(&[pensioner.clone(), pensioner.clone()], 0.0), 0.0);
    let unemployed = make_household(0, 1, 100.0, 0.0);
    assert_eq!(
        buyers(&[unemployed.clone()], 100.0),
        0.0,
        "time-limited transfers cannot promise ongoing customer revenue"
    );
    assert_eq!(
        buyers(&[unemployed, pensioner], 100.0),
        0.0,
        "earlier temporary claims still consume city cash before later permanent claims"
    );
}

#[test]
fn consumer_reservations_keep_family_money_quantities_and_exact_rollback_separate() {
    use crate::simulation::economy::customers::ConsumerBudgets;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "consumer_claim_home", ZoneClass::Residential);
    allocator.buildings.push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let mut families = vec![make_household(0, 1, 0.0, 0.0), make_household(0, 1, 100_000.0, 0.0)];
    families[0].gross_daily_wages = 100.0;
    families[0].employed_adults = 1;
    let resource = catalog.resource_runtime_id_for_id("household_supplies").unwrap();
    let mut customers = ConsumerBudgets::default();
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 100_000.0);
    assert_eq!(customers.affordable_residents(), affordable_consumer_residents(
        &families, &allocator, &catalog, &tuning, &policy, 100_000.0));
    assert_eq!(customers.available(1, resource, 1.0), 0.0,
        "savings and a neighbour's wages cannot promise recurring purchases");
    assert_eq!(customers.available(0, resource, 1.0), 1.0,
        "cheap goods cannot create more than the family's actual daily need");
    let expensive = customers.available(0, resource, 500.0);
    assert!(expensive > 0.0 && expensive < 1.0);
    assert_eq!(customers.claim(0, resource, expensive * 0.25, 500.0), expensive * 0.25);
    assert_eq!(customers.claim(0, resource, expensive * 0.25, 500.0), expensive * 0.25);
    customers.reject();
    assert_eq!(customers.available(0, resource, 500.0), expensive,
        "multiple staged claims must restore the original spending bits");
    assert_eq!(customers.claim(0, resource, 10.0, 1.0), 1.0);
    customers.accept();
    assert_eq!(customers.available(0, resource, 1.0), 0.0);
    customers.reject();
    assert_eq!(customers.available(0, resource, 500.0), 0.0,
        "rejecting a later quote cannot restore an accepted household's quantity");
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 100_000.0);
    assert_eq!(customers.available(0, resource, 500.0), expensive);
    customers.clear();
    assert_eq!(customers.available(0, resource, 1.0), 0.0);
}

#[test]
fn routed_consumer_claims_restore_eligibility_and_exclude_disconnected_money() {
    use crate::simulation::economy::customers::ConsumerMarket;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let a = graph.add_node(Vector3::new(900.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(1300.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(a, b, 900.0, 1300.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "routed_customer_home", ZoneClass::Residential);
    let shop = register_test_asset(&mut allocator, "test", "routed_customer_shop", ZoneClass::Commercial);
    allocator.buildings.push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator.buildings.push(make_building(40.0, ZoneType::Commercial, &shop, 0.0));
    let mut remote = make_building(1000.0, ZoneType::Residential, &home, 0.0);
    remote.edge_idx = 1;
    allocator.buildings.push(remote);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut families = vec![make_household(0, 1, 0.0, 0.0), make_household(2, 100, 0.0, 0.0)];
    for family in &mut families {
        family.gross_daily_wages = 100_000.0;
        family.employed_adults = 1;
        family.shopping_has_car = true;
    }
    let resource = catalog.resource_runtime_id_for_id("household_supplies").unwrap();
    let mut customers = ConsumerMarket::default();
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    let quote = |customers: &mut ConsumerMarket| customers.claim_reachable(
        &allocator.entrances[1], resource, 200.0, 25.0, &allocator, &network, &graph);
    assert_eq!(quote(&mut customers), 1.0,
        "neither the disconnected 100 residents nor duplicate foot/car entries add sales");
    assert_eq!(quote(&mut customers), 0.0);
    customers.reject();
    assert_eq!(quote(&mut customers), 1.0, "rollback must restore bucket eligibility too");
    customers.accept_for(1);
    customers.reject();
    assert_eq!(quote(&mut customers), 0.0);
    let mut construction = ConsumerMarket::default();
    construction.copy_reservations_from(&customers);
    assert_eq!(quote(&mut construction), 0.0);
    construction.release_owner(1);
    assert_eq!(quote(&mut construction), 1.0);
    construction.reject();
    assert_eq!(quote(&mut construction), 0.0,
        "failed replacement restores the incumbent's exact existing claim");
    construction.release_owner(1);
    assert_eq!(construction.claim_reachable(&allocator.entrances[1], resource, 0.5, 25.0,
        &allocator, &network, &graph), 0.5);
    construction.accept_for(1);
    assert_eq!(quote(&mut construction), 0.5,
        "accepted replacement retires the old receipt and retains its actual new purchases");
    construction.reject();
    construction.copy_reservations_from(&customers);
    assert_eq!(quote(&mut construction), 0.0, "a warm fork must restore the live book");
    assert_eq!(quote(&mut customers), 0.0, "construction cannot mutate the live accounts");
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    assert_eq!(quote(&mut customers), 1.0, "warm census refills exhausted trees");
    customers.reject();
    families[0].adult_count = 0;
    families[0].elder_count = 0;
    families[0].child_count = 1;
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    assert_eq!(quote(&mut customers), 0.0, "a child-only family has no eligible shopper");
    customers.clear();
    assert_eq!(quote(&mut customers), 0.0);
}

#[test]
fn routed_consumer_claims_require_the_return_trip_and_the_households_mode() {
    use crate::simulation::economy::customers::ConsumerMarket;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "consumer_mode_home", ZoneClass::Residential);
    let shop = register_test_asset(&mut allocator, "test", "consumer_mode_shop", ZoneClass::Commercial);
    allocator.buildings.push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator.buildings.push(make_building(40.0, ZoneType::Commercial, &shop, 0.0));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    for entrance in &mut allocator.entrances {
        entrance.foot_lane_fwd = usize::MAX;
        entrance.foot_lane_bkw = usize::MAX;
    }
    let mut families = vec![make_household(0, 1, 0.0, 0.0)];
    families[0].gross_daily_wages = 1000.0;
    families[0].employed_adults = 1;
    let resource = catalog.resource_runtime_id_for_id("household_supplies").unwrap();
    let mut customers = ConsumerMarket::default();
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    assert_eq!(customers.claim_reachable(&allocator.entrances[1], resource, 1.0, 25.0,
        &allocator, &network, &graph), 0.0, "car-only frontage cannot serve a family without a car");
    families[0].shopping_has_car = true;
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    assert_eq!(customers.claim_reachable(&allocator.entrances[1], resource, 1.0, 25.0,
        &allocator, &network, &graph), 1.0);
    customers.reject();
    // A proposed forward-only door is reachable outward along the shared frontage,
    // but cannot return to the home's forward-only door through the dead-end road.
    graph.edge_mut(0).bkw_lanes = 0;
    graph.edge_mut(0).allowed_types = TransitFlags::CAR;
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    customers.reset(&families, &allocator, &catalog, &tuning, &policy, 0.0, &network, &graph);
    let mut proposed = allocator.entrances[1].clone();
    proposed.car_lane_bkw = usize::MAX;
    let paths = std::sync::atomic::AtomicU32::new(0);
    let home_origin = crate::simulation::economy::agents::tick::BuildingTripEnds::origin(0, &allocator, &network, &graph);
    let destination = crate::simulation::economy::agents::tick::BuildingTripEnds::at_entrance(&proposed, false, &network, &graph);
    assert!(matches!(crate::simulation::economy::agents::tick::estimate_trip_seconds_between(
        &home_origin, &destination, true, &network, &graph, &paths, f32::INFINITY, f32::INFINITY),
        crate::simulation::economy::agents::tick::TripEstimate::Seconds(Some(_))));
    assert_eq!(customers.claim_reachable(&proposed, resource, 1.0, 25.0,
        &allocator, &network, &graph), 0.0, "one outward trip cannot promise a shopping round trip");
}

#[test]
fn walking_component_proofs_follow_directed_lane_connectors() {
    use crate::simulation::economy::agents::tick::{
        BuildingTripEnds, PedestrianReachability, building_origin_trip_is_feasible,
    };
    use crate::simulation::network::lanes::LaneType;
    let (mut graph, mut network) = simple_work_graph();
    let end = graph.add_node(Vector3::new(700.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(1, end, 300.0, 700.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset = register_test_asset(&mut allocator, "test", "walk_proof", ZoneClass::Residential);
    for (x, edge) in [(0.0, 0), (100.0, 0), (400.0, 1), (500.0, 1)] {
        let mut building = make_building(x, ZoneType::Residential, &asset, 0.0);
        building.edge_idx = edge;
        allocator.buildings.push(building);
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let reachability = PedestrianReachability::build(&network, &graph);
    let queries = std::sync::atomic::AtomicU32::new(0);
    let mut proven = 0;
    for from in 0..4 {
        for to in 0..4 {
            if from == to { continue; }
            let origin = BuildingTripEnds::origin(from, &allocator, &network, &graph);
            let destination = BuildingTripEnds::destination(to, &allocator, &network, &graph);
            if origin.has_proven_walk_to(&destination, &reachability) {
                proven += 1;
                assert!(building_origin_trip_is_feasible(from, to, 0, false,
                    &allocator, &network, &graph, &queries), "false lane proof for {from} -> {to}");
            }
        }
    }
    assert!(proven > 0, "connected pedestrian lanes must avoid repeated path searches");
    allocator.entrances[0].foot_lane_bkw = usize::MAX;
    allocator.entrances[3].foot_lane_bkw = usize::MAX;
    let origin = BuildingTripEnds::origin(0, &allocator, &network, &graph);
    let destination = BuildingTripEnds::destination(3, &allocator, &network, &graph);
    assert!(!origin.has_proven_walk_to(&destination, &reachability),
        "shared-node ends retain the ordinary direct-connector check");
    assert!(building_origin_trip_is_feasible(0, 3, 0, false,
        &allocator, &network, &graph, &queries));
    for lane in &mut network.lane_system.lanes {
        if lane.lane_type == LaneType::Foot && lane.edge_id == usize::MAX {
            lane.next_lanes.clear();
        }
    }
    let disconnected = PedestrianReachability::build(&network, &graph);
    let origin = BuildingTripEnds::origin(0, &allocator, &network, &graph);
    let destination = BuildingTripEnds::destination(3, &allocator, &network, &graph);
    assert!(!origin.has_proven_walk_to(&destination, &disconnected),
        "weak road connectivity is insufficient after pedestrian connectors disappear");
    assert!(!building_origin_trip_is_feasible(0, 3, 0, false,
        &allocator, &network, &graph, &queries));
}

#[test]
fn live_consumer_funding_rejects_unreachable_sales_and_releases_failed_capital_claims() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let (mut graph, mut network) = simple_work_graph();
    let a = graph.add_node(Vector3::new(900.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(1300.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(a, b, 900.0, 1300.0));
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "live_customer_home", ZoneClass::Residential);
    let shop = register_test_asset(&mut allocator, "test", "live_customer_shop", ZoneClass::Commercial);
    let profile = catalog.profile_for_id("personal_service_small").unwrap();
    allocator.buildings.push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    for x in [40.0, 80.0] {
        let mut business = make_building(x, ZoneType::Commercial, &shop, 0.0);
        business.economy_profile_runtime_id = profile.runtime_id;
        business.operating_budget = 100_000.0;
        allocator.buildings.push(business);
    }
    let mut remote = make_building(1000.0, ZoneType::Residential, &home, 0.0);
    remote.edge_idx = 1;
    allocator.buildings.push(remote);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut families = vec![make_household(3, 200, 0.0, 0.0)];
    families[0].gross_daily_wages = 100_000.0;
    families[0].employed_adults = 1;
    let mut funding = crate::simulation::economy::hiring::JobFunding::default();
    let mut logistics = ShipmentSystem::new();
    funding.refresh(&allocator, &families, &mut logistics, &network, &graph,
        &catalog, &tuning, &policy, &[], 100_000.0, 420);
    assert_eq!(&funding.capacities[1..3], &[0, 0],
        "a rich disconnected community cannot finance either local shop");
    families[0].home_building_id = 0;
    allocator.buildings[1].operating_budget = 0.0;
    funding.refresh(&allocator, &families, &mut logistics, &network, &graph,
        &catalog, &tuning, &policy, &[], 100_000.0, 420);
    assert_eq!(funding.capacities[1], 0);
    assert!(funding.capacities[2] > 0, "a failed capital check must release reached family claims");
    allocator.buildings[1].operating_budget = 100_000.0;
    funding.refresh(&allocator, &families, &mut logistics, &network, &graph,
        &catalog, &tuning, &policy, &[], 100_000.0, 420);
    assert!(funding.capacities[1] > 0);
    assert_eq!(funding.capacities[2], 0,
        "the same household need cannot fund both shops after the first commits its capacity");
}

#[test]
fn household_wage_snapshot_counts_only_active_paid_adult_assignments() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("food_processor_basic").unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "wage_home", ZoneClass::Residential);
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "wage_factory",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Industrial, &factory, 0.0));
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));
    let mut agents = AgentSystem::new();
    for _ in 0..2 {
        let member = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[member] = 0;
        agents.assign_work_building(member, 1, 0);
    }
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert_eq!(households.households[0].employed_adults, 2);
    assert_eq!(
        households.households[0].gross_daily_wages,
        2.0 * profile.average_daily_wage()
    );
    agents.consecutive_unpaid_days[0] = 1;
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert_eq!(
        households.households[0].employed_adults, 2,
        "an unpaid worker is still assigned"
    );
    assert_eq!(
        households.households[0].gross_daily_wages,
        profile.average_daily_wage()
    );
    allocator.buildings[1].construction_remaining_hours = 1;
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert_eq!(households.households[0].gross_daily_wages, 0.0);
    assert_eq!(households.households[0].employed_adults, 2);
}

#[test]
fn shopping_car_census_uses_eligible_residents_and_resets_removed_access() {
    use crate::simulation::economy::agents::{AGE_ADULT, AGE_CHILD, AGE_ELDER};
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(usize::MAX, 2, 0.0, 0.0));
    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    for age in [AGE_ADULT, AGE_CHILD] {
        let index = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.household_id[index] = 0;
        agents.age_group[index] = age;
        agents.has_car[index] = age == AGE_CHILD;
    }
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert!(!households.households[0].shopping_has_car,
        "a child's car cannot supply an eligible household shopper");
    agents.has_car[0] = true;
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert!(households.households[0].shopping_has_car);
    agents.age_group[0] = AGE_ELDER;
    households.households[0].shopping_has_car = false;
    households.rebuild_household_shopping_access(&agents);
    assert!(households.households[0].shopping_has_car,
        "the load census must include elders without relying on saved household flags");
    agents.household_id[0] = usize::MAX;
    households.rebuild_household_shopping_access(&agents);
    assert!(!households.households[0].shopping_has_car);
    households.households[0].shopping_has_car = true;
    households.rebuild_household_and_worker_counts(&agents, &mut allocator);
    assert!(!households.households[0].shopping_has_car,
        "a removed resident cannot leave cached car access behind");
}

#[test]
#[ignore = "manual release scaling of recurring household income and purchasing forecasts"]
fn benchmark_consumer_income_forecast() {
    use std::hint::black_box;
    use std::time::Instant;
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(
        &mut allocator,
        "test",
        "income_bench_home",
        ZoneClass::Residential,
    );
    let factory = register_test_asset(
        &mut allocator,
        "test",
        "income_bench_work",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    allocator
        .buildings
        .push(make_building(20.0, ZoneType::Industrial, &factory, 0.0));
    // Isolate indexed aggregation: repeated references avoid gameplay placement/routing setup.
    for count in [1_024, 8_192, 65_536] {
        let mut households = HouseholdSystem::new();
        households.households = vec![make_household(0, 2, 0.0, 0.0); count];
        let mut agents = AgentSystem::new();
        for hid in 0..count {
            for _ in 0..2 {
                let index = agents.spawn_housed_agent(0, 0.0, 0.0);
                agents.household_id[index] = hid;
                agents.assign_work_building(index, 1, 0);
            }
        }
        let mut samples = [0.0; 9];
        for round in 0..12 {
            let start = Instant::now();
            households.refresh_household_wage_snapshot(black_box(&agents), &allocator);
            let buyers = affordable_consumer_residents(
                &households.households,
                &allocator,
                &catalog,
                &tuning,
                &policy,
                100_000.0,
            );
            assert_eq!(black_box(buyers), (count * 2) as f64);
            if round >= 3 {
                samples[round - 3] = start.elapsed().as_secs_f64() * 1_000.0;
            }
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "consumer_income_forecast households={count} agents={} median_ms={:.3}",
            count * 2,
            samples[4]
        );
    }
}
