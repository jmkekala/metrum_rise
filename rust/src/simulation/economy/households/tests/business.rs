// SPDX-License-Identifier: GPL-2.0-only

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
    agents.assign_household_id(agent, 0);
    agents.work_building[agent] = usize::MAX;

    let mut treasury = 0.0;
    households.pay_unemployment_benefits(&allocator, &mut treasury);
    assert_eq!(households.households[0].unemployment_days_elapsed, 1);
    assert_eq!(households.daily_ledgers()[0].transfer_claim, 30.0);
    assert_eq!(households.daily_ledgers()[0].transfer_income(), 0.0);

    households.households[0].unemployment_days_elapsed = 0;
    allocator.buildings[0].broken = true;
    treasury = 1_000.0;
    households.pay_unemployment_benefits(&allocator, &mut treasury);
    assert_eq!(households.households[0].unemployment_days_elapsed, 0);
}

#[test]
fn adults_not_paid_a_wage_in_the_window_draw_unemployment_benefit() {
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "paid_window_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    let mut households = HouseholdSystem::new();
    for paid in [0, 1, 2] {
        let mut household = make_household(0, 2, 0.0, 0.0);
        household.adult_count = 2;
        household.child_count = 0;
        // The employed count still lists both adults; only paid wages count.
        household.employed_adults = 2;
        households.households.push(household);
        households.restore_paid_workers(paid, paid as u16);
    }
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        unemployment_benefit_per_adult_per_day: 30.0,
        pension_per_elder_per_day: 0.0,
        child_support_per_child_per_day: 0.0,
        ..Default::default()
    };
    let mut treasury = 1_000.0;
    for household_id in 0..3 {
        households.pay_household_transfer(household_id, &allocator, &mut treasury, &policy);
    }
    let benefit = |id: usize| households.daily_ledgers()[id].unemployment_benefit_income;
    assert_eq!([benefit(0), benefit(1), benefit(2)], [60.0, 30.0, 0.0]);
    assert_eq!(households.households[2].unemployment_days_elapsed, 0);
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

    let mut treasury = 1_000.0;
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        unemployment_benefit_per_adult_per_day: 30.0,
        pension_per_elder_per_day: 30.0,
        child_support_per_child_per_day: 0.0,
        ..Default::default()
    };

    households.pay_household_transfers(&allocator, &mut treasury, &policy);

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

    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        residential_property_tax_per_home_per_day: 2.0,
        property_tax_level_multiplier: 1.75,
        ..Default::default()
    };

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
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        commercial_property_tax_per_building_per_day: 25.0,
        industrial_property_tax_per_building_per_day: 35.0,
        property_tax_level_multiplier: 2.0,
        ..Default::default()
    };

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
    let policy = crate::simulation::economy::fiscal::CityFiscalPolicy {
        industrial_property_tax_per_building_per_day: 35.0,
        ..Default::default()
    };

    let revenue = households.settle_daily_property_tax(&mut allocator, &policy);

    assert_eq!(revenue.commercial_property_tax, 0.0);
    assert!((revenue.industrial_property_tax - 35.0).abs() < 0.001);
    assert!((allocator.buildings[0].operating_budget - 65.0).abs() < 0.001);
}

#[test]
fn membership_rebuild_does_not_materialize_missing_household_ids() {
    let mut households = HouseholdSystem::new();
    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(agent, usize::MAX);

    households.recount_every_household(&agents, &BuildingAllocator::new());

    assert!(households.households.is_empty());
    assert_eq!(agents.household_id[agent], usize::MAX);
}

#[test]
fn forced_liquidation_sells_stock_on_hand_not_loaded_cargo() {
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
        30.0,
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
        carrier: crate::simulation::economy::agents::AgentHandle::NONE,
        total_cost: 0.0,
        freight_cost: 0.0,
        eta_hours: 1,
        leg_started_hour: 0,
        trip_event_slot: 0,
        // The logistics hour loads every shipment it plans, so a building's daily phase only
        // sees cargo that has already left its stock (`SIM-02-G`).
        cargo_loaded: true,
    });

    let mut households = HouseholdSystem::new();
    let mut treasury_balance = 0.0;
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert_eq!(
        allocator.buildings[0].inventory_units(household_supplies),
        0.0
    );
    assert_eq!(logistics.shipments[0].amount, 20.0);
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
            * if *profile_id == "power_plant_basic" {
                8.0
            } else {
                1.0
            }
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
    // The census funded the two staffed workers' half of the barber's output.
    households.job_funding.set_activity(1, 0.5);

    // The first hour prices the sale, the second's household phase pays into the pool and the
    // third's store phase draws it (`SIM-02-I`).
    households.run_building_hour_for_test(&mut allocator);
    households.run_building_hour_for_test(&mut allocator);

    let service_unit_price = catalog
        .unit_price_for_resource(personal_services)
        .expect("personal service unit price");
    let demand_rate = catalog
        .profile_for_id("personal_service_demand")
        .unwrap()
        .consumption_rate_per_resident;
    let expected_revenue = 100.0 * demand_rate / 24.0 * service_unit_price;
    assert!(
        (households.households[0].budget - (10.0 - expected_revenue)).abs() < 0.001,
        "aggregate service sales should debit household budget"
    );
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    households.run_building_hour_for_test(&mut allocator);
    assert!(
        (allocator.buildings[1].revenue - expected_revenue).abs() < 0.001,
        "service revenue should be demand-capped and capacity-share credited"
    );
    assert!(
        (allocator.buildings[1].operating_budget - expected_revenue).abs() < 0.001,
        "service revenue should fund the commercial building budget"
    );
    assert_eq!(
        allocator.buildings[1].inventory_units(personal_services),
        0.0,
        "service capacity must not accumulate as physical inventory"
    );
}

#[test]
fn service_store_active_worker_slots_scale_from_census_activity() {
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

    refresh_commercial_activity_floor(&catalog, &mut allocator, &[]);
    assert_eq!(
        active_worker_capacity_for_profile(
            &catalog,
            &allocator.buildings[1],
            personal_service_profile
        ),
        0,
        "service stores should offer no active worker slots without funded customers"
    );

    // The census funded a fifth of the barber's output for the customers it reserved.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[0.0, 0.2]);

    assert_eq!(
        active_worker_capacity_for_profile(
            &catalog,
            &allocator.buildings[1],
            personal_service_profile
        ),
        1,
        "a fifth of the output opens one active barber slot, not the full four-worker profile"
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
fn service_store_wage_pass_sheds_workers_above_funded_capacity() {
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
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
    }

    households.job_funding.capacities = vec![0, 1];
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
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    households.job_funding.capacities = vec![0, 1];
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
fn explicit_work_area_wages_keep_funded_workers_and_release_them_when_area_shrinks() {
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
    farm.work_area_scale = 80.0;
    farm.commercial_activity_floor_scale = 1.0;
    farm.worker_count = 8;
    farm.operating_budget = 10_000.0;
    allocator.buildings.push(farm);

    let mut agents = AgentSystem::new();
    let mut workers = Vec::new();
    for _ in 0..8 {
        let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
        workers.push(agent);
    }

    households.job_funding.capacities = vec![0, 8];
    let mut treasury_balance = 0.0;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(allocator.buildings[1].worker_count, 6);
    for &agent in &workers[..6] {
        assert_eq!(agents.work_building[agent], 1);
    }
    for &agent in &workers[6..] {
        assert_eq!(agents.work_building[agent], usize::MAX);
    }

    allocator.buildings[1].set_work_area_scale(1.0);
    let budget_before = allocator.buildings[1].operating_budget;
    households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);
    assert_eq!(allocator.buildings[1].worker_count, 2);
    assert_eq!(
        allocator.buildings[1].operating_budget,
        budget_before - 180.0
    );
    assert_eq!(agents.work_building[workers[0]], 1);
    for &agent in &workers[2..] {
        assert_eq!(agents.work_building[agent], usize::MAX);
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
    // Staffing alone is insufficient when maintenance stock is empty.
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert_eq!(allocator.buildings[2].revenue, 0.0);
    assert_eq!(allocator.buildings[3].revenue, 0.0);
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    for idx in 1..=3 {
        allocator.buildings[idx].set_inventory_units(machinery, 80.0);
    }
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    assert!(allocator.buildings[2].revenue > 0.0);
    assert!(allocator.buildings[3].revenue > 0.0);

    allocator.buildings[0].operating_budget = 500.0;
    let coal = catalog
        .resource_runtime_id_for_id("coal")
        .expect("coal resource");
    allocator.buildings[1].set_inventory_units(coal, 10.0);
    // Plants generate at the last hour's dispatch (`SIM-02-I`).
    households.run_electricity_hour(&mut allocator);
    households.run_electricity_hour(&mut allocator);
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
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
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
            * if *profile_id == "power_plant_basic" {
                8.0
            } else {
                1.0
            }
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
fn idle_work_areas_pay_area_scaled_imported_utilities() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    for id in ["grain_farm_basic", "coal_mine_basic"] {
        for area in [0.0, 1.0, 4.0, 100.0] {
            let farm_profile = catalog.profile_for_id(id).unwrap();
            let mut allocator = BuildingAllocator::new();
            let farm_asset = register_test_asset(
                &mut allocator,
                "test",
                "explicit_utility_farm",
                ZoneClass::Industrial,
            );
            let mut farm = make_building(0.0, ZoneType::None, &farm_asset, 0.0);
            farm.economy_profile_runtime_id = farm_profile.runtime_id;
            farm.set_work_area_scale(area);
            farm.worker_count = 0; // Unstaffed and empty of inputs: maintenance still consumes utilities.
            farm.operating_budget = 1_000_000.0;
            allocator.buildings.push(farm);

            let logistics = ShipmentSystem::new();
            let mut households = HouseholdSystem::new();
            let mut treasury_balance = 0.0;
            for _ in 0..24 {
                households.run_electricity_hour(&mut allocator);
            }
            households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

            let expected_owa_utility_cost = [
                "power_plant_basic",
                "water_plant_basic",
                "wastewater_treatment_basic",
            ]
            .iter()
            .enumerate()
            .map(|(service, profile_id)| {
                catalog
                    .profile_for_id(profile_id)
                    .expect("utility profile")
                    .unit_price_currency
                    * if service == 0 {
                        farm_profile.electricity_demand_kwh_per_day(area, 0.0, 0)
                    } else {
                        1.0 + area * farm_profile.area_utility_units_per_hectare[service]
                    }
            })
            .sum::<f32>()
                * tuning.owa_import_price_multiplier;
            assert!(
                (allocator.buildings[0].operating_budget
                    - (1_000_000.0 - expected_owa_utility_cost))
                    .abs()
                    < 0.1
            );
            assert!(
                (households.last_power_settlement().demand_units
                    - farm_profile.electricity_demand_kwh_per_day(area, 0.0, 0))
                .abs()
                    < 0.02
            );
            assert_eq!(households.last_power_settlement().coverage, 0.0);
            assert_eq!(treasury_balance, 0.0);

            // A local plant covers half the enlarged electricity demand; the remainder
            // stays imported. Both the producer bill and treasury receipts must agree.
            let power_asset = register_test_utility_asset(
                &mut allocator,
                "test",
                "area_power",
                "power_plant_basic",
            );
            let power = catalog.profile_for_id("power_plant_basic").unwrap();
            let demand = farm_profile.electricity_demand_kwh_per_day(area, 0.0, 0);
            let mut provider = make_building(20.0, ZoneType::None, &power_asset, 0.0);
            provider.economy_profile_runtime_id = power.runtime_id;
            provider.daily_power_service_units = demand * 0.5;
            provider.worker_count = 0; // Production was already recorded; no municipal consumer load.
            allocator.reset_daily_input_accumulators();
            allocator.buildings[0]
                .daily_electricity
                .workplace
                .record(demand, 0.5);
            allocator.buildings.push(provider);
            allocator.buildings[0].operating_budget = 1_000_000.0;
            households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
            let local_receipt = demand * 0.5 * power.unit_price_currency;
            let saving = local_receipt * (tuning.owa_import_price_multiplier - 1.0);
            assert_eq!(households.last_power_settlement().coverage, 0.5);
            assert!(
                (allocator.buildings[0].operating_budget
                    - (1_000_000.0 - expected_owa_utility_cost + saving))
                    .abs()
                    < 0.1
            );
            assert!((treasury_balance - f64::from(local_receipt)).abs() < 0.01);
        }
    }
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
fn an_employer_that_cannot_pay_wages_two_windows_running_deserts() {
    for unpaid_again in [false, true] {
        let mut allocator = BuildingAllocator::new();
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "unpaid_payroll_store",
            ZoneClass::Commercial,
        );
        let mut store = make_building(0.0, ZoneType::Commercial, &commercial_asset, 0.0);
        // Positive cash, yet a wage went unpaid in the window for lack of it.
        store.operating_budget = 1_000.0;
        store.unpaid_cash_wages_in_window = 1;
        allocator.buildings.push(store);

        let logistics = ShipmentSystem::new();
        let mut households = HouseholdSystem::new();
        let mut treasury_balance = 0.0;
        households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);
        assert!(allocator.buildings[0].budget_distress);
        allocator.roll_daily_accumulators(0);
        assert_eq!(allocator.buildings[0].unpaid_cash_wages_in_window, 0);

        allocator.buildings[0].unpaid_cash_wages_in_window = u32::from(unpaid_again);
        households.run_bankruptcy_check(&mut allocator);
        assert_eq!(allocator.buildings[0].is_deserted, unpaid_again);
    }
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
    households.daily_ledgers[0].electricity.record(6.0, 0.0);
    allocator.buildings[0]
        .daily_electricity
        .residential
        .record(6.0, 0.0);
    households.daily_ledgers[0].water_consumption_cost = 2.0;
    households.daily_ledgers[0].sewage_consumption_cost = 1.5;

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    let expected_power_bill = 6.0 * 0.2 * tuning.owa_import_price_multiplier;
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
        (households.households[0].budget - (100.0 - (expected_total_bill - 3.5))).abs() < 0.001
    );
    assert_eq!(treasury_balance, 0.0);
    assert_eq!(
        households.last_power_settlement().household_local_revenue,
        0.0
    );
}

#[test]
fn household_utility_revenue_cannot_exceed_cash_actually_paid() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let hourly_cost = tuning.households.water_sewage_cost_per_member_per_day / 24.0;
    for initial_budget in [0.0, hourly_cost / 4.0] {
        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            "test:cash_limited_home",
            0.0,
        ));
        let power_asset = register_test_utility_asset(
            &mut allocator,
            "test",
            "cash_limited_plant",
            "power_plant_basic",
        );
        let mut plant = make_building(20.0, ZoneType::None, &power_asset, 0.0);
        plant.economy_profile_runtime_id = catalog
            .profile_for_id("power_plant_basic")
            .unwrap()
            .runtime_id;
        // Unstaffed plant has standby demand, but earns no power revenue.
        allocator.buildings.push(plant);
        let mut households = HouseholdSystem::new();
        let mut household = make_household(0, 1, 0.0, 0.0);
        household.budget = initial_budget;
        households.households.push(household);
        households.run_electricity_hour(&mut allocator);
        households.consume_household_stock(&mut AgentSystem::new());
        let mut treasury = 0.0;
        households.settle_daily_utilities(&mut allocator, &ShipmentSystem::new(), &mut treasury);

        let ledger = &households.daily_ledgers()[0];
        let paid = ledger.power_consumption_cost
            + ledger.water_consumption_cost
            + ledger.sewage_consumption_cost;
        assert!(
            (paid - initial_budget).abs() < 0.00001,
            "paid={paid}, available={initial_budget}"
        );
        assert_eq!(households.households[0].budget, 0.0);
        assert!((treasury + 10.0 * 0.25 / 24.0).abs() < 0.00001);
        assert!(households.households[0].power_arrears.total() > 0.0);
        assert_eq!(allocator.buildings[1].revenue, 0.0);
    }
}

#[test]
fn household_utility_surcharge_records_only_the_remaining_cash() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(usize::MAX, 1, 0.0, 0.0);
    household.budget = 0.25;
    households.households.push(household);
    households.ensure_daily_ledger_len();
    households.daily_ledgers[0].water_consumption_cost = 3.0;
    households.settle_daily_utilities(
        &mut BuildingAllocator::new(),
        &ShipmentSystem::new(),
        &mut 0.0,
    );
    assert_eq!(households.households[0].budget, 0.0);
    assert!((households.daily_ledgers()[0].water_consumption_cost - 3.25).abs() < 0.00001);
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
    power_building.worker_count = 1;
    power_building.daily_power_service_units = 131.0;
    power_building.daily_power_served_units = 131.0;
    power_building.daily_electricity.workplace.record(30.0, 1.0);
    allocator.buildings.push(power_building);

    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 20, 0.0, 0.0));
    households.ensure_daily_ledger_len();
    households.households[0].budget = 1000.0;
    households.daily_ledgers[0].electricity.record(101.0, 1.0);
    allocator.buildings[0]
        .daily_electricity
        .residential
        .record(101.0, 1.0);

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert!((allocator.buildings[1].revenue - 20.2).abs() < 0.001);
    assert!((allocator.buildings[1].daily_power_served_units - 131.0).abs() < 0.001);
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
    assert!((treasury_balance - (20.2 - missing_city_utility_owa as f64)).abs() < 0.001);
    let summary = households.last_power_settlement();
    assert!((summary.utility_local_revenue - 26.2).abs() < 0.001);
    assert!((summary.city_service_utility_local_cost - 6.0).abs() < 0.001);
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
    households.households[0].budget = 1000.0;
    households.daily_ledgers[0].electricity.record(60.0, 1.0);
    allocator.buildings[0]
        .daily_electricity
        .residential
        .record(60.0, 1.0);

    let logistics = ShipmentSystem::new();
    let mut treasury_balance = 0.0;
    households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury_balance);

    assert!((allocator.buildings[1].revenue - 12.0).abs() < 0.001);
    assert!((treasury_balance - 12.0).abs() < 0.001);
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
    agents.assign_household_id(agent, 0);
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.assign_work_building(agent, 1, 0);

    let water_asset = register_test_utility_asset(
        &mut allocator,
        "test",
        "city_service_wage_water",
        "water_plant_basic",
    );
    let water_profile = catalog.profile_for_id("water_plant_basic").unwrap();
    let mut water = make_building(40.0, ZoneType::None, &water_asset, 0.0);
    water.economy_profile_runtime_id = water_profile.runtime_id;
    water.worker_count = 1;
    water.operating_budget = 0.0;
    allocator.buildings.push(water);
    let water_worker = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.assign_household_id(water_worker, 0);
    agents.assign_work_building(water_worker, 2, 0);

    households.job_funding.capacities = vec![0, 1, 1];
    let mut treasury_balance = 1_000.0;
    let wage = power_profile.average_daily_wage() + water_profile.average_daily_wage();
    let income_tax =
        households.pay_daily_wages(&mut agents, &mut allocator, 0.0, &mut treasury_balance);

    assert_eq!(income_tax, 0.0);
    assert!((treasury_balance - (1_000.0 - wage as f64)).abs() < 0.001);
    assert!((households.households[0].budget - wage).abs() < 0.001);
    assert_eq!(allocator.buildings[1].operating_budget, 0.0);
    assert_eq!(allocator.buildings[2].operating_budget, 0.0);
    assert_eq!(households.last_city_service_wages().total, wage);
    assert_eq!(
        households.last_city_service_wages().power,
        power_profile.average_daily_wage()
    );
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
        agents.assign_household_id(agent, 0);
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.assign_work_building(agent, 1, 0);
    }

    households.enforce_service_funding_staffing(&mut agents, &mut allocator, &[1.0, 0.05]);

    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.work_building[a1], usize::MAX);
}

#[test]
#[ignore = "manual matched release timing of funded staffing limits"]
fn benchmark_funded_staffing() {
    use std::{hint::black_box, time::Instant};
    let catalog = load_runtime_economy_catalog().unwrap();
    for power in [false, true] {
        let profile = catalog
            .profile_for_id(if power {
                "power_plant_basic"
            } else {
                "food_processor_basic"
            })
            .unwrap();
        let capacity = profile.worker_capacity as usize;
        for count in [1_024_usize, 8_192, 65_536] {
            let mut allocator = BuildingAllocator::new();
            let mut template = make_building(20.0, ZoneType::Industrial, "test:funding_bench", 0.0);
            template.economy_profile_runtime_id = profile.runtime_id;
            allocator.buildings = vec![template; count.div_ceil(capacity)];
            let funding = vec![0.5; allocator.buildings.len()];
            let mut agents = AgentSystem::new();
            for _ in 0..count {
                agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            }
            let mut households = HouseholdSystem::new();
            let mut samples = [0.0; 11];
            for sample in &mut samples {
                for (idx, work) in agents.work_building.iter_mut().enumerate() {
                    *work = idx / capacity;
                }
                for building in &mut allocator.buildings {
                    building.worker_count = capacity as u32;
                }
                let start = Instant::now();
                households.enforce_service_funding_staffing(
                    black_box(&mut agents),
                    &mut allocator,
                    &funding,
                );
                *sample = start.elapsed().as_secs_f64() * 1_000.0;
            }
            let employed = agents
                .work_building
                .iter()
                .filter(|&&work| work != usize::MAX)
                .count();
            if power {
                assert!(employed > 0 && employed < count);
            } else {
                assert_eq!(employed, count);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "funded_staffing power={power} workers={count} median_ms={:.3}",
                samples[5]
            );
        }
    }
}

#[test]
#[ignore = "manual matched release timing of hourly household charges and daily utility settlement"]
fn benchmark_household_utility_settlement() {
    use std::{hint::black_box, time::Instant};
    for count in [1_024, 8_192, 65_536] {
        let mut households = HouseholdSystem::new();
        let mut household = make_household(0, 1, 0.0, 1_000.0);
        household.budget = 10_000.0;
        households.households = vec![household; count];
        let mut allocator = BuildingAllocator::new();
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            "test:utility_benchmark",
            0.0,
        ));
        let mut agents = AgentSystem::new();
        let logistics = ShipmentSystem::new();
        let mut treasury = 0.0;
        let mut samples = [0.0; 11];
        for sample in &mut samples {
            households.reset_daily_ledgers();
            let start = Instant::now();
            for _ in 0..24 {
                households.consume_household_stock(black_box(&mut agents));
            }
            households.settle_daily_utilities(&mut allocator, &logistics, &mut treasury);
            *sample = start.elapsed().as_secs_f64() * 1_000.0;
        }
        assert!(
            households
                .households
                .iter()
                .all(|home| home.budget > 0.0 && home.budget < 10_000.0)
        );
        assert_eq!(treasury, 0.0);
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "household_utility households={count} median_ms={:.3}",
            samples[5]
        );
    }
}
