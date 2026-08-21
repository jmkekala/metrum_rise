//! Business solvency, taxation, utility, and service-funding tests.

use super::support::*;
use super::*;

#[test]
fn unemployment_timer_advances_when_treasury_is_empty_and_requires_valid_home() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "benefit_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.work_building[agent] = usize::MAX;

    let mut treasury = 0.0;
    households.pay_unemployment_benefits(&agents, &allocator, &mut treasury);
    assert_eq!(households.households[0].unemployment_days_elapsed, 1);

    households.households[0].unemployment_days_elapsed = 0;
    allocator.buildings[0].broken = true;
    treasury = 1_000.0;
    households.pay_unemployment_benefits(&agents, &allocator, &mut treasury);
    assert_eq!(households.households[0].unemployment_days_elapsed, 0);
}

#[test]
fn single_elder_receives_pension_without_work_or_unemployment() {
    let mut households = HouseholdSystem::new();
    let mut elder_household = make_household(0, 1, 0.0, 1.0);
    elder_household.child_count = 0;
    elder_household.adult_count = 0;
    elder_household.elder_count = 1;
    households.households.push(elder_household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "pension_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));

    let agents = AgentSystem::new();
    let mut treasury = 1_000.0;
    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    policy.unemployment_benefit_per_adult_per_day = 30.0;
    policy.pension_per_elder_per_day = 30.0;
    policy.child_support_per_child_per_day = 0.0;

    households.pay_household_transfers(&agents, &allocator, &mut treasury, &policy);

    assert_eq!(households.households[0].budget, 30.0);
    assert_eq!(
        households.daily_ledgers()[0].unemployment_benefit_income,
        0.0
    );
    assert_eq!(households.daily_ledgers()[0].pension_income, 30.0);
    assert_eq!(households.daily_ledgers()[0].child_support_income, 0.0);
    assert_eq!(households.households[0].unemployment_days_elapsed, 0);
}

#[test]
fn daily_residential_property_tax_debits_occupied_household_budget() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "daily_property_tax_home",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    allocator.buildings.push(home);

    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.0, 0.0);
    household.budget = 10.0;
    households.households.push(household);

    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    policy.residential_property_tax_per_home_per_day = 2.0;
    policy.property_tax_level_multiplier = 1.75;

    let revenue = households.settle_daily_property_tax(&mut allocator, &policy);

    assert!((revenue.residential_property_tax - 3.5).abs() < 0.001);
    assert_eq!(revenue.commercial_property_tax, 0.0);
    assert_eq!(revenue.industrial_property_tax, 0.0);
    assert!((households.households[0].budget - 6.5).abs() < 0.001);
    assert!((households.daily_ledgers()[0].property_tax_paid - 3.5).abs() < 0.001);
}

#[test]
fn daily_nonresidential_property_tax_debits_private_building_budgets() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "daily_property_tax_store",
        ZoneClass::Commercial,
    );
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "daily_property_tax_factory",
        ZoneClass::Industrial,
    );
    let mut commercial = make_building(0.0, ZoneType::Commercial, &commercial_asset, 0.0);
    commercial.level = 2;
    commercial.operating_budget = 100.0;
    allocator.buildings.push(commercial);
    let mut industrial = make_building(20.0, ZoneType::Industrial, &industrial_asset, 0.0);
    industrial.operating_budget = 100.0;
    allocator.buildings.push(industrial);

    let mut households = HouseholdSystem::new();
    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    policy.commercial_property_tax_per_building_per_day = 25.0;
    policy.industrial_property_tax_per_building_per_day = 35.0;
    policy.property_tax_level_multiplier = 2.0;

    let revenue = households.settle_daily_property_tax(&mut allocator, &policy);

    assert_eq!(revenue.residential_property_tax, 0.0);
    assert!((revenue.commercial_property_tax - 50.0).abs() < 0.001);
    assert!((revenue.industrial_property_tax - 35.0).abs() < 0.001);
    assert!((allocator.buildings[0].operating_budget - 50.0).abs() < 0.001);
    assert!((allocator.buildings[1].operating_budget - 65.0).abs() < 0.001);
}

#[test]
fn explicit_work_area_property_tax_uses_industrial_rate() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_asset(
        &mut allocator,
        "test",
        "explicit_property_tax_farm",
        ZoneClass::Industrial,
    );
    let mut farm = make_building(0.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.operating_budget = 100.0;
    allocator.buildings.push(farm);

    let mut households = HouseholdSystem::new();
    let mut policy = crate::simulation::economy::fiscal::CityFiscalPolicy::default();
    policy.industrial_property_tax_per_building_per_day = 35.0;

    let revenue = households.settle_daily_property_tax(&mut allocator, &policy);

    assert_eq!(revenue.commercial_property_tax, 0.0);
    assert!((revenue.industrial_property_tax - 35.0).abs() < 0.001);
    assert!((allocator.buildings[0].operating_budget - 65.0).abs() < 0.001);
}

#[test]
fn ensure_agent_households_does_not_materialize_missing_household_ids() {
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = usize::MAX;

    households.ensure_agent_households(&mut agents);

    assert!(households.households.is_empty());
    assert_eq!(agents.household_id[agent], usize::MAX);
}

#[test]
fn forced_liquidation_sells_only_unreserved_inventory() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "liquidation_store",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        &commercial_asset,
        50.0,
    ));
    allocator.buildings[0].operating_budget = -10.0;

    let mut logistics = ShipmentSystem::new();
    logistics.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: household_supplies,
        amount: 20.0,
        source: ShipmentEndpoint::Building(0),
        destination: ShipmentEndpoint::OwaBorder(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert_eq!(
        allocator.buildings[0].inventory_units(household_supplies),
        20.0
    );
    let household_supply_unit_price = catalog
        .unit_price_for_resource(household_supplies)
        .expect("household supplies unit price");
    let no_provider_utility_cost = [
        "power_plant_basic",
        "water_plant_basic",
        "wastewater_treatment_basic",
    ]
    .iter()
    .map(|profile_id| {
        catalog
            .profile_for_id(profile_id)
            .expect("utility profile")
            .unit_price_currency
    })
    .sum::<f32>()
        * tuning.owa_import_price_multiplier;
    let expected_budget = -10.0 - no_provider_utility_cost
        + 30.0 * household_supply_unit_price * tuning.owa_distress_liquidation_multiplier;
    assert!(
        (allocator.buildings[0].operating_budget - expected_budget).abs() < 0.001,
        "forced liquidation should use distress multiplier, not scheduled export multiplier"
    );
}

#[test]
fn business_profit_tax_charges_only_positive_active_business_growth() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "profit_store",
        ZoneClass::Commercial,
    );
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "profit_factory",
        ZoneClass::Industrial,
    );
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "profit_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        10.0,
        ZoneType::Industrial,
        &industrial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings[0].profit_tax_budget_baseline = 500.0;
    allocator.buildings[0].operating_budget = 650.0;
    allocator.buildings[1].profit_tax_budget_baseline = 500.0;
    allocator.buildings[1].operating_budget = 700.0;
    allocator.buildings[1].is_deserted = true;
    allocator.buildings[2].profit_tax_budget_baseline = 500.0;
    allocator.buildings[2].operating_budget = 900.0;

    let mut households = HouseholdSystem::new();
    let tax = households.settle_business_profit_tax(&mut allocator, 0.10);

    assert!((tax - 15.0).abs() < 0.001);
    assert!((allocator.buildings[0].operating_budget - 635.0).abs() < 0.001);
    assert!((allocator.buildings[0].profit_tax_budget_baseline - 635.0).abs() < 0.001);
    assert!((allocator.buildings[0].last_day_profit - 150.0).abs() < 0.001);
    assert_eq!(allocator.buildings[1].operating_budget, 700.0);
    assert_eq!(allocator.buildings[1].profit_tax_budget_baseline, 700.0);
    assert_eq!(allocator.buildings[1].last_day_profit, 200.0);
    assert_eq!(allocator.buildings[2].operating_budget, 900.0);
    assert_eq!(allocator.buildings[2].profit_tax_budget_baseline, 900.0);
    assert_eq!(allocator.buildings[2].last_day_profit, 0.0);

    let second_tax = households.settle_business_profit_tax(&mut allocator, 0.10);
    assert_eq!(second_tax, 0.0);
    assert!((allocator.buildings[0].operating_budget - 635.0).abs() < 0.001);
}

#[test]
fn business_profit_tax_tracks_explicit_work_area_profiles_outside_zones() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let coal_profile = catalog
        .profile_for_id("coal_mine_basic")
        .expect("coal mine profile");
    let mut allocator = BuildingAllocator::new();
    let explicit_asset = register_test_asset(
        &mut allocator,
        "test",
        "profit_coal_pit",
        ZoneClass::Industrial,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::None, &explicit_asset, 0.0));
    allocator.buildings[0].economy_profile_runtime_id = coal_profile.runtime_id;
    allocator.buildings[0].profit_tax_budget_baseline = 100.0;
    allocator.buildings[0].operating_budget = 180.0;

    let mut households = HouseholdSystem::new();
    let tax = households.settle_business_profit_tax(&mut allocator, 0.10);

    assert!((tax - 8.0).abs() < 0.001);
    assert!((allocator.buildings[0].operating_budget - 172.0).abs() < 0.001);
    assert!((allocator.buildings[0].profit_tax_budget_baseline - 172.0).abs() < 0.001);
    assert!((allocator.buildings[0].last_day_profit - 80.0).abs() < 0.001);
}

#[test]
fn business_profit_tax_does_not_push_recovering_business_negative() {
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "recovering_profit_store",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[0].profit_tax_budget_baseline = -100.0;
    allocator.buildings[0].operating_budget = 5.0;

    let mut households = HouseholdSystem::new();
    let tax = households.settle_business_profit_tax(&mut allocator, 0.10);

    assert_eq!(tax, 5.0);
    assert_eq!(allocator.buildings[0].operating_budget, 0.0);
    assert_eq!(allocator.buildings[0].profit_tax_budget_baseline, 0.0);
    assert_eq!(allocator.buildings[0].last_day_profit, 105.0);
    assert!(!allocator.buildings[0].budget_distress);
}

#[test]
fn service_store_sales_use_staffed_aggregate_household_demand() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");
    let personal_services = catalog
        .resource_runtime_id_for_id("personal_services")
        .expect("personal service resource");

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let service_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "half_staffed_barber",
        "personal_service_small",
    );
    let mut service_building = make_building(10.0, ZoneType::Commercial, &service_asset, 0.0);
    service_building.economy_profile_runtime_id = personal_service_profile.runtime_id;
    service_building.worker_count = 2;
    service_building.operating_budget = 0.0;
    service_building.profit_tax_budget_baseline = 0.0;
    allocator.buildings.push(service_building);

    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 100, 0.0, 0.0);
    household.budget = 10.0;
    households.households.push(household);

    households.run_building_economy(&mut allocator, true);

    let service_unit_price = catalog
        .unit_price_for_resource(personal_services)
        .expect("personal service unit price");
    let expected_revenue = 100.0 * 0.03 / 24.0 * service_unit_price;
    assert!(
        (allocator.buildings[1].revenue - expected_revenue).abs() < 0.001,
        "service revenue should be demand-capped and capacity-share credited"
    );
    assert!(
        (allocator.buildings[1].operating_budget - expected_revenue).abs() < 0.001,
        "service revenue should fund the commercial building budget"
    );
    assert!(
        (households.households[0].budget - (10.0 - expected_revenue)).abs() < 0.001,
        "aggregate service sales should debit household budget"
    );
    assert_eq!(
        allocator.buildings[1].inventory_units(personal_services),
        0.0,
        "service capacity must not accumulate as physical inventory"
    );
}

#[test]
fn service_store_active_worker_slots_scale_from_aggregate_demand() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_activity_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let service_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "activity_barber",
        "personal_service_small",
    );
    let mut service_building = make_building(10.0, ZoneType::Commercial, &service_asset, 0.0);
    service_building.economy_profile_runtime_id = personal_service_profile.runtime_id;
    allocator.buildings.push(service_building);

    let empty_households = HouseholdSystem::new();
    refresh_commercial_activity_floor(&catalog, &empty_households.households, &mut allocator, true);
    assert_eq!(
        active_worker_capacity_for_profile(
            &catalog,
            &allocator.buildings[1],
            personal_service_profile
        ),
        0,
        "service stores should offer no active worker slots without resident demand"
    );

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 70, 0.0, 0.0));

    refresh_commercial_activity_floor(&catalog, &households.households, &mut allocator, true);

    assert_eq!(
        active_worker_capacity_for_profile(
            &catalog,
            &allocator.buildings[1],
            personal_service_profile
        ),
        1,
        "70 residents should create only one active barber slot, not the full four-worker profile"
    );
    allocator.buildings[1].worker_count = 1;
    let factors =
        building_operation_factors(&catalog, &allocator.buildings[1], personal_service_profile);
    assert!(
        (factors.throughput_factor - 0.25).abs() < 0.001,
        "service output should still scale against authored worker capacity"
    );
}

#[test]
fn service_store_wage_pass_sheds_workers_above_active_demand_capacity() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 70, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_shed_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let service_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "overstaffed_barber",
        "personal_service_small",
    );
    let mut service_building = make_building(10.0, ZoneType::Commercial, &service_asset, 0.0);
    service_building.economy_profile_runtime_id = personal_service_profile.runtime_id;
    service_building.worker_count = 4;
    service_building.operating_budget = 1_000.0;
    allocator.buildings.push(service_building);

    let mut agents = AgentSystem::new();
    let workers = [
        agents.spawn_housed_agent(0, 0.0, 0.0),
        agents.spawn_housed_agent(0, 0.0, 0.0),
        agents.spawn_housed_agent(0, 0.0, 0.0),
        agents.spawn_housed_agent(0, 0.0, 0.0),
    ];
    for agent in workers {
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
    }

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(agents.work_building[workers[0]], 1);
    assert_eq!(agents.work_building[workers[1]], usize::MAX);
    assert_eq!(agents.work_building[workers[2]], usize::MAX);
    assert_eq!(agents.work_building[workers[3]], usize::MAX);
}

#[test]
fn service_store_fake_inventory_cannot_liquidate_for_payroll() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let personal_service_profile = catalog
        .profile_for_id("personal_service_small")
        .expect("personal service profile");
    let personal_services = catalog
        .resource_runtime_id_for_id("personal_services")
        .expect("personal_services resource");

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "service_liquidation_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let service_asset = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "fake_inventory_barber",
        "personal_service_small",
    );
    let mut service_building = make_building(10.0, ZoneType::Commercial, &service_asset, 0.0);
    service_building.economy_profile_runtime_id = personal_service_profile.runtime_id;
    service_building.worker_count = 1;
    service_building.operating_budget = 0.0;
    service_building.set_inventory_units(personal_services, 100.0);
    allocator.buildings.push(service_building);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(agents.work_building[agent], 1);
    assert_eq!(agents.consecutive_unpaid_days[agent], 1);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(
        allocator.buildings[1].inventory_units(personal_services),
        100.0
    );
    assert_eq!(allocator.buildings[1].operating_budget, 0.0);
}

#[test]
fn explicit_work_area_wage_pass_keeps_owa_backed_area_workers() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm profile");

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 8, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "farm_shed_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let farm_asset = register_test_asset(
        &mut allocator,
        "test",
        "overstaffed_area_farm",
        ZoneClass::Industrial,
    );
    let mut farm = make_building(10.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 1.0;
    farm.commercial_activity_floor_scale = 1.0;
    farm.worker_count = 8;
    farm.operating_budget = 10_000.0;
    allocator.buildings.push(farm);

    let mut agents = AgentSystem::new();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
        workers.push(agent);
    }

    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(
        allocator.buildings[1].worker_count, 8,
        "OWA-capable farms should keep their full area-scaled worker slots active"
    );
    for &agent in &workers {
        assert_eq!(agents.work_building[agent], 1);
        assert_eq!(agents.consecutive_unpaid_days[agent], 0);
    }
}

#[test]
fn same_day_bankruptcy_preserves_yesterday_profit_for_inspector() {
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "bankrupt_profit_factory",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Industrial,
        &industrial_asset,
        0.0,
    ));
    allocator.buildings[0].profit_tax_budget_baseline = 500.0;
    allocator.buildings[0].operating_budget = -50.0;
    allocator.buildings[0].budget_distress = true;

    let mut households = HouseholdSystem::new();
    households.run_bankruptcy_check(&mut allocator);
    let tax = households.settle_business_profit_tax(&mut allocator, 0.10);

    assert_eq!(tax, 0.0);
    assert!(allocator.buildings[0].is_deserted);
    assert_eq!(allocator.buildings[0].last_day_profit, -550.0);
    assert_eq!(allocator.buildings[0].profit_tax_budget_baseline, -50.0);
}

#[test]
fn city_service_buildings_do_not_enter_private_bankruptcy() {
    let mut allocator = BuildingAllocator::new();
    let utility_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "municipal_power",
        "power_plant_basic",
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::None, &utility_asset, 0.0));
    allocator.buildings[0].operating_budget = -50.0;
    allocator.buildings[0].budget_distress = true;

    let mut households = HouseholdSystem::new();
    households.run_bankruptcy_check(&mut allocator);

    assert!(!allocator.buildings[0].is_deserted);
    assert!(allocator.buildings[0].budget_distress);
    assert_eq!(allocator.buildings[0].operating_budget, -50.0);
}

#[test]
fn nearby_building_search_sorts_before_truncating_candidates() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "nearby_res", ZoneClass::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "nearby_com", ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        10.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        -10.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    let chunk = RegionGraph::get_chunk_coords(Vector3::new(0.0, 0.0, 0.0));
    allocator.building_chunks.insert(chunk, vec![2, 1, 0]);

    let candidates =
        allocator.find_nearby_buildings_by_zones(0.0, 0.0, &[ZoneType::Commercial], 0, 1);

    assert_eq!(candidates, vec![1]);
}

#[test]
fn utility_provider_must_have_workers_before_receiving_service_revenue() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "utility_consumer",
        ZoneClass::Commercial,
    );
    let power_asset =
        register_test_utility_asset(&mut allocator, "test", "power", "power_plant_basic");
    let water_asset =
        register_test_utility_asset(&mut allocator, "test", "water", "water_plant_basic");
    let sewage_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "sewage",
        "wastewater_treatment_basic",
    );

    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    for (asset_id, profile_id, x) in [
        (&power_asset, "power_plant_basic", 20.0),
        (&water_asset, "water_plant_basic", 40.0),
        (&sewage_asset, "wastewater_treatment_basic", 60.0),
    ] {
        let mut building = make_building(x, ZoneType::None, asset_id, 0.0);
        building.economy_profile_runtime_id = catalog
            .profile_for_id(profile_id)
            .expect("utility profile")
            .runtime_id;
        allocator.buildings.push(building);
    }

    let logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    assert_eq!(allocator.buildings[2].revenue, 0.0);
    assert_eq!(allocator.buildings[3].revenue, 0.0);
    assert_eq!(treasury_balance, 0.0);

    allocator.buildings[0].operating_budget = 500.0;
    for idx in 1..=3 {
        allocator.buildings[idx].worker_count = 1;
    }
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    assert!(allocator.buildings[2].revenue > 0.0);
    assert!(allocator.buildings[3].revenue > 0.0);
    let treasury_after_unfueled_power = treasury_balance;

    allocator.buildings[0].operating_budget = 500.0;
    let coal = catalog
        .resource_runtime_id_for_id("coal")
        .expect("coal resource");
    allocator.buildings[1].set_inventory_units(coal, 10.0);
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    assert!(treasury_balance < treasury_after_unfueled_power);

    allocator.buildings[0].operating_budget = 500.0;
    allocator.buildings[1].daily_power_service_units = 1.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert!(allocator.buildings[1].revenue > 0.0);
    assert!(allocator.buildings[2].revenue > 0.0);
    assert!(allocator.buildings[3].revenue > 0.0);
}

#[test]
fn private_nonresidential_utility_bill_uses_unified_service_prices() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "unified_utility_store",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        &commercial_asset,
        0.0,
    ));
    allocator.buildings[0].operating_budget = 100.0;

    let logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    let expected_owa_utility_cost = [
        "power_plant_basic",
        "water_plant_basic",
        "wastewater_treatment_basic",
    ]
    .iter()
    .map(|profile_id| {
        catalog
            .profile_for_id(profile_id)
            .expect("utility profile")
            .unit_price_currency
    })
    .sum::<f32>()
        * tuning.owa_import_price_multiplier;
    assert!(
        (allocator.buildings[0].operating_budget - (100.0 - expected_owa_utility_cost)).abs()
            < 0.001
    );
    assert_eq!(treasury_balance, 0.0);
    assert_eq!(
        households.last_power_settlement().private_local_revenue,
        0.0
    );
}

#[test]
fn explicit_work_area_utility_bill_uses_private_business_prices() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_asset(
        &mut allocator,
        "test",
        "explicit_utility_farm",
        ZoneClass::Industrial,
    );
    let mut farm = make_building(0.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.operating_budget = 100.0;
    allocator.buildings.push(farm);

    let logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    let expected_owa_utility_cost = [
        "power_plant_basic",
        "water_plant_basic",
        "wastewater_treatment_basic",
    ]
    .iter()
    .map(|profile_id| {
        catalog
            .profile_for_id(profile_id)
            .expect("utility profile")
            .unit_price_currency
    })
    .sum::<f32>()
        * tuning.owa_import_price_multiplier;
    assert!(
        (allocator.buildings[0].operating_budget - (100.0 - expected_owa_utility_cost)).abs()
            < 0.001
    );
    assert_eq!(treasury_balance, 0.0);
}

#[test]
fn explicit_work_area_negative_budget_enters_distress() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_asset(
        &mut allocator,
        "test",
        "explicit_distress_farm",
        ZoneClass::Industrial,
    );
    let mut farm = make_building(0.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.operating_budget = -10.0;
    allocator.buildings.push(farm);

    let logistics = ShipmentSystem::new();
    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert!(allocator.buildings[0].budget_distress);
    households.run_bankruptcy_check(&mut allocator);
    assert!(allocator.buildings[0].is_deserted);
}

#[test]
fn household_utility_owa_surcharge_uses_unified_multiplier() {
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "unified_utility_home",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));

    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 1, 0.0, 0.0);
    household.budget = 100.0;
    households.households.push(household);
    households.ensure_daily_ledger_len();
    households.daily_ledgers[0].power_consumption_cost = 3.0;
    households.daily_ledgers[0].water_consumption_cost = 2.0;
    households.daily_ledgers[0].sewage_consumption_cost = 1.5;
    households.daily_ledgers[0].utility_stock_consumption_cost = 6.5;

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    let expected_power_bill = 3.0 * tuning.owa_import_price_multiplier;
    let expected_water_bill = 2.0 * tuning.owa_import_price_multiplier;
    let expected_sewage_bill = 1.5 * tuning.owa_import_price_multiplier;
    let expected_total_bill = expected_power_bill + expected_water_bill + expected_sewage_bill;
    assert!(
        (households.daily_ledgers()[0].power_consumption_cost - expected_power_bill).abs() < 0.001
    );
    assert!(
        (households.daily_ledgers()[0].water_consumption_cost - expected_water_bill).abs() < 0.001
    );
    assert!(
        (households.daily_ledgers()[0].sewage_consumption_cost - expected_sewage_bill).abs()
            < 0.001
    );
    assert!(
        (households.households[0].budget - (100.0 - (expected_total_bill - 6.5))).abs() < 0.001
    );
    assert!(
        (households.daily_ledgers()[0].utility_stock_consumption_cost - expected_total_bill).abs()
            < 0.001
    );
    assert_eq!(treasury_balance, 0.0);
    assert_eq!(
        households.last_power_settlement().household_local_revenue,
        0.0
    );
}

#[test]
fn household_utility_payment_flows_to_fueled_power_provider() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "powered_household_home",
        ZoneClass::Residential,
    );
    let power_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "powered_household_plant",
        "power_plant_basic",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut power_building = make_building(20.0, ZoneType::None, &power_asset, 0.0);
    let power_profile = catalog
        .profile_for_id("power_plant_basic")
        .expect("power profile");
    power_building.economy_profile_runtime_id = power_profile.runtime_id;
    power_building.worker_count = power_profile.worker_capacity;
    power_building.daily_power_service_units = 42.0;
    allocator.buildings.push(power_building);

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 20, 0.0, 0.0));
    households.ensure_daily_ledger_len();
    households.daily_ledgers[0].power_consumption_cost = 60.0;
    households.daily_ledgers[0].utility_stock_consumption_cost = 60.0;

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert!((allocator.buildings[1].revenue - 63.0).abs() < 0.001);
    assert!((allocator.buildings[1].daily_power_served_units - 21.0).abs() < 0.001);
    let missing_city_utility_owa = ["water_plant_basic", "wastewater_treatment_basic"]
        .iter()
        .map(|profile_id| {
            catalog
                .profile_for_id(profile_id)
                .expect("utility profile")
                .unit_price_currency
        })
        .sum::<f32>()
        * tuning.owa_import_price_multiplier;
    assert!((treasury_balance - (60.0 - missing_city_utility_owa as f64)).abs() < 0.001);
    let summary = households.last_power_settlement();
    assert!((summary.utility_local_revenue - 63.0).abs() < 0.001);
    assert!((summary.city_service_utility_local_cost - 3.0).abs() < 0.001);
    assert!((summary.city_service_utility_owa_cost - missing_city_utility_owa).abs() < 0.001);
}

#[test]
fn power_settlement_uses_recorded_output_after_coal_is_consumed() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "recorded_power_home",
        ZoneClass::Residential,
    );
    let power_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "recorded_power_plant",
        "power_plant_basic",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut power_building = make_building(20.0, ZoneType::None, &power_asset, 0.0);
    let power_profile = catalog
        .profile_for_id("power_plant_basic")
        .expect("power profile");
    power_building.economy_profile_runtime_id = power_profile.runtime_id;
    power_building.worker_count = 0;
    power_building.daily_power_service_units = 60.0;
    allocator.buildings.push(power_building);

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 20, 0.0, 0.0));
    households.ensure_daily_ledger_len();
    households.daily_ledgers[0].power_consumption_cost = 60.0;
    households.daily_ledgers[0].utility_stock_consumption_cost = 60.0;

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert!((allocator.buildings[1].revenue - 60.0).abs() < 0.001);
    assert!((treasury_balance - 60.0).abs() < 0.001);
}

#[test]
fn city_service_wages_debit_treasury_not_building_budget() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "city_service_wage_home",
        ZoneClass::Residential,
    );
    let power_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "city_service_wage_power",
        "power_plant_basic",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut service_building = make_building(20.0, ZoneType::None, &power_asset, 0.0);
    let power_profile = catalog
        .profile_for_id("power_plant_basic")
        .expect("power profile");
    service_building.economy_profile_runtime_id = power_profile.runtime_id;
    service_building.worker_count = 1;
    service_building.operating_budget = 0.0;
    allocator.buildings.push(service_building);

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let mut treasury_balance = 1_000.0;
    let wage = power_profile.average_daily_wage();
    let income_tax =
        households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(income_tax, 0.0);
    assert!((treasury_balance - (1_000.0 - wage as f64)).abs() < 0.001);
    assert!((households.households[0].budget - wage).abs() < 0.001);
    assert_eq!(allocator.buildings[1].operating_budget, 0.0);
    assert_eq!(agents.consecutive_unpaid_days[agent], 0);
}

#[test]
fn power_service_funding_sheds_workers_to_funded_capacity() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 2, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "funding_shed_home",
        ZoneClass::Residential,
    );
    let power_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "funding_shed_power",
        "power_plant_basic",
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut service_building = make_building(20.0, ZoneType::None, &power_asset, 0.0);
    let power_profile = catalog
        .profile_for_id("power_plant_basic")
        .expect("power profile");
    service_building.economy_profile_runtime_id = power_profile.runtime_id;
    service_building.worker_count = 2;
    allocator.buildings.push(service_building);

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for agent in [a0, a1] {
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
    }

    households.enforce_service_funding_staffing(&mut agents, &mut allocator, &[1.0, 0.05], true);

    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.work_building[a1], usize::MAX);
}
