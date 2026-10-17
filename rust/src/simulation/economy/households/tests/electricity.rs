// SPDX-License-Identifier: GPL-2.0-only

//! Physical power conservation, upkeep and cash-independent household delivery regressions.

use super::support::*;
use super::*;
use crate::simulation::economy::households::ElectricityProfiles;

fn town(workers: u32, cash: f32) -> (HouseholdSystem, BuildingAllocator, usize) {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "electricity",
        "home",
        ZoneClass::Residential,
    );
    let mut households = HouseholdSystem::new();
    for idx in 0..85 {
        allocator.buildings.push(make_building(
            idx as f32 * 10.0,
            ZoneType::Residential,
            &home_asset,
            0.0,
        ));
        let mut household = make_household(idx, 2, 0.0, 0.0);
        household.budget = cash;
        households.households.push(household);
    }
    let plant_asset =
        register_test_utility_asset(&mut allocator, "electricity", "plant", "power_plant_basic");
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let mut plant = make_building(1000.0, ZoneType::None, &plant_asset, 0.0);
    plant.economy_profile_runtime_id = profile.runtime_id;
    plant.worker_count = workers;
    for port in &profile.inputs {
        plant.set_inventory_units(port.resource_runtime_id, port.units_per_day);
    }
    let idx = allocator.buildings.len();
    allocator.buildings.push(plant);
    (households, allocator, idx)
}

// An hour whose totals give the measured hours their dispatch and coverage, which phases read
// one hour old (`SIM-02-I`); the day's meters, generation and coal start clean after it.
fn warm_up(households: &mut HouseholdSystem, allocator: &mut BuildingAllocator) {
    households.run_electricity_hour(allocator);
    households.reset_daily_ledgers();
    allocator.reset_daily_input_accumulators();
    households.day_totals = Default::default();
}

#[test]
fn power_input_buffers_follow_staffing_and_preserve_minimum_freight_batches() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    for workers in [0, 1, 2, 3] {
        let (_, allocator, plant) = town(workers, 0.0);
        for input in &profile.inputs {
            let targets = scaled_input_inventory_targets_for_building(
                &catalog,
                &allocator.buildings[plant],
                profile,
                input,
            );
            if workers == 0 {
                assert_eq!(targets, (0.0, 0.0, 0.0));
            } else if input.resource_runtime_id == coal {
                assert!((targets.0 - 1620.0 * workers as f32).abs() < 0.001);
                assert!((targets.1 - 810.0 * workers as f32).abs() < 0.001);
            } else {
                assert_eq!(targets.0, 2.0 * input.shipment_batch_kg);
            }
        }
    }
}

#[test]
fn fractional_hourly_utility_charges_reconcile_with_household_cash() {
    let mut household = make_household(0, 2, 0.0, 100.0);
    household.budget = 100_000.0;
    let (mut households, _, mut agents, _, _) =
        setup_replenishment_world(household, "utility_home", "utility_store", 50.0, 20.0);
    let before = households.households[0].budget;
    // Utilities are charged in each household's own hour (`SIM-02-F`).
    for _ in 0..24 {
        households.consume_household_stock(&mut agents);
    }
    let ledger = &households.daily_ledgers()[0];
    assert_eq!(
        before - households.households[0].budget,
        ledger.water_consumption_cost + ledger.sewage_consumption_cost,
    );
}

#[test]
fn electricity_dispatch_tracks_delivered_demand_and_fixed_staffed_upkeep() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    for workers in [1, 2, 3] {
        let (mut households, mut allocator, plant) = town(workers, 100.0);
        warm_up(&mut households, &mut allocator);
        let before_coal = allocator.buildings[plant].inventory_units(coal);
        let before_machinery = allocator.buildings[plant].inventory_units(machinery);
        for _ in 0..24 {
            households.run_electricity_hour(&mut allocator);
        }
        let expected_generation = 935.0 + 10.0 + 20.0 * workers as f32;
        let b = &allocator.buildings[plant];
        assert!((b.daily_power_service_units - expected_generation).abs() < 0.01);
        assert!((before_coal - b.inventory_units(coal) - expected_generation * 0.45).abs() < 0.1);
        assert!(
            (before_machinery - b.inventory_units(machinery) - 0.2 * workers as f32).abs() < 0.001
        );
        let mut treasury = 0.0;
        let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
        assert!((summary.demand_units - expected_generation).abs() < 0.01);
        assert!((summary.coverage - 1.0).abs() < 0.0001);
        assert!((households.daily_ledgers()[0].power_consumption_cost - 2.2).abs() < 0.001);
        // Own local auxiliary bill cancels in the treasury; only customers supply external cash.
        assert!((treasury - 187.0).abs() < 0.01);
    }
}

#[test]
fn electricity_delivery_survives_empty_wallets_and_unpaid_bills_remain_visible() {
    let (mut households, mut allocator, _) = town(1, 0.0);
    warm_up(&mut households, &mut allocator);
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
    let mut treasury = 0.0;
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 965.0).abs() < 0.01);
    assert_eq!(summary.household_local_revenue, 0.0);
    assert!(treasury.abs() < 0.001);
    assert!((households.households[0].power_arrears.total() - 2.2).abs() < 0.001);
    assert!((households.daily_ledgers()[0].power_unpaid_cost - 2.2).abs() < 0.001);
    households.reset_daily_ledgers();
    allocator.reset_daily_input_accumulators();
    households.households[0].budget = 10.0;
    households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!(households.households[0].power_arrears.total() < 0.0001);
    assert!((households.households[0].budget - 7.8).abs() < 0.001);
}

#[test]
fn demolished_plant_preserves_delivered_power_fuel_and_customer_bills() {
    let (mut households, mut allocator, plant) = town(1, 100.0);
    warm_up(&mut households, &mut allocator);
    let mut treasury = -1000.0;
    for _ in 0..12 {
        households.run_electricity_hour(&mut allocator);
    }
    households.close_building_electricity(&mut allocator.buildings[plant], true, &mut treasury);
    allocator.buildings.pop(); // Minimal fixture: plant is last, with no agent/routing references.
    for _ in 0..12 {
        households.run_electricity_hour(&mut allocator);
    }
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 950.0).abs() < 0.01);
    assert!((summary.supply_units - 482.5).abs() < 0.01);
    // The first hour without the plant still meters the last hour's coverage (`SIM-02-I`).
    assert!((summary.served_units - (482.5 + 935.0 / 24.0)).abs() < 0.01);
    assert!((treasury - (-1000.0 + 93.5 * 13.0 / 12.0)).abs() < 0.01);
    let bill = (13.0 * 0.2 + 11.0 * 0.25) * 11.0 / 24.0;
    assert!((households.daily_ledgers()[0].power_consumption_cost - bill).abs() < 0.001);
    // The coal burned before the removal stays in the day's report (`SIM-02-G`).
    assert!((households.last_day_totals.coal_consumed_kg - 217.125).abs() < 0.01);
}

#[test]
fn removal_after_its_daily_phase_does_not_charge_a_plant_twice() {
    let (mut households, mut allocator, plant) = town(1, 100.0);
    warm_up(&mut households, &mut allocator);
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
    let mut treasury = 0.0;
    households.settle_daily_electricity(&mut allocator, &mut treasury);
    // The plant's daily phase settled its window and rolled its meter.
    allocator.roll_daily_accumulators(plant);
    let paid = treasury;
    households.close_building_electricity(&mut allocator.buildings[plant], true, &mut treasury);
    assert_eq!(treasury, paid);
    assert_eq!(households.removed_electricity.settlement.supply_units, 0.0);
    assert!((households.last_day_totals.coal_consumed_kg - 434.25).abs() < 0.01);
}

#[test]
fn removed_workplace_pays_its_meter_without_losing_residential_delivery() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let (mut households, mut allocator, plant) = town(1, 100.0);
    allocator.buildings[0].zone_type = ZoneType::Mixed;
    allocator.buildings[0].economy_profile_runtime_id =
        catalog.profile_for_id("grocery_basic").unwrap().runtime_id;
    allocator.buildings[0].operating_budget = 100.0;
    warm_up(&mut households, &mut allocator);
    for _ in 0..12 {
        households.run_electricity_hour(&mut allocator);
    }
    let mut treasury = 0.0;
    households.invalidate_building(0, &mut allocator);
    households.close_building_electricity(&mut allocator.buildings[0], false, &mut treasury);
    assert!((allocator.buildings[0].operating_budget - 99.2).abs() < 0.001);
    allocator.buildings[0].broken = true; // Keep IDs in this isolated meter fixture.
    for _ in 0..12 {
        households.run_electricity_hour(&mut allocator);
    }
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 963.5).abs() < 0.01);
    assert!((summary.served_units - summary.demand_units).abs() < 0.01);
    assert!((households.daily_ledgers()[0].power_consumption_cost - 1.1).abs() < 0.001);
    assert!((summary.private_local_revenue - 0.8).abs() < 0.001);
    assert!((treasury - 186.7).abs() < 0.01);
    assert!((allocator.buildings[plant].revenue - 186.7).abs() < 0.01);
}

#[test]
fn electricity_imports_cover_fuel_shortfall_without_extra_routing() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let (mut households, mut allocator, plant) = town(1, 100.0);
    allocator.buildings[plant].set_inventory_units(coal, 4.5); // Exactly 10 kWh.
    warm_up(&mut households, &mut allocator);
    households.run_electricity_hour(&mut allocator);
    assert!((allocator.buildings[plant].daily_power_service_units - 10.0).abs() < 0.0001);
    assert!(allocator.buildings[plant].inventory_units(coal) < 0.0001);
    let mut treasury = 0.0;
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 965.0 / 24.0).abs() < 0.001);
    assert!((summary.served_units - 10.0).abs() < 0.001);
    assert!(summary.coverage < 1.0);
    let cost = households.daily_ledgers()[0].power_consumption_cost;
    assert!(cost > 2.2 / 24.0 && cost < 2.75 / 24.0);
}

#[test]
fn shared_mixed_use_home_bills_each_load_once() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let (mut households, mut allocator, _) = town(0, 100.0);
    households.households.truncate(2);
    households.households[1].home_building_id = 0;
    households.households[1].member_count = 4;
    households.households[1].adult_count = 4;
    allocator.buildings.truncate(1);
    let building = &mut allocator.buildings[0];
    building.occupancy = 2;
    building.zone_type = ZoneType::Mixed;
    building.economy_profile_runtime_id =
        catalog.profile_for_id("grocery_basic").unwrap().runtime_id;
    building.operating_budget = 100.0;
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
    let mut treasury = 0.0;
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 39.0).abs() < 0.001); // 31 home + 8 idle shop.
    assert!((allocator.buildings[0].operating_budget - 98.0).abs() < 0.001);
    // The home's standby kWh splits equally between its two household places (`SIM-02-I`).
    let ledgers = households.daily_ledgers();
    assert!((ledgers[0].power_consumption_cost - 10.5 * 0.25).abs() < 0.001);
    assert!((ledgers[1].power_consumption_cost - 20.5 * 0.25).abs() < 0.001);
    assert_eq!(summary.served_units, 0.0);
}

#[test]
fn multiple_plants_share_dispatch_and_collected_revenue_by_generation() {
    let (mut households, mut allocator, plant) = town(1, 100.0);
    let mut second = allocator.buildings[plant].clone();
    second.worker_count = 2;
    allocator.buildings.push(second);
    warm_up(&mut households, &mut allocator);
    for _ in 0..24 {
        households.run_electricity_hour(&mut allocator);
    }
    let mut treasury = 0.0;
    let summary = households.settle_daily_electricity(&mut allocator, &mut treasury);
    assert!((summary.demand_units - 1015.0).abs() < 0.01);
    assert!((summary.served_units - summary.demand_units).abs() < 0.01);
    let first = &allocator.buildings[plant];
    let second = &allocator.buildings[plant + 1];
    assert!((first.daily_power_service_units - 1015.0 / 3.0).abs() < 0.01);
    assert!((second.daily_power_service_units - 2030.0 / 3.0).abs() < 0.01);
    assert!((first.revenue - 187.0 / 3.0).abs() < 0.01);
    assert!((second.revenue - 374.0 / 3.0).abs() < 0.01);
    assert!((treasury - 187.0).abs() < 0.01);
}

#[test]
fn phase_totals_match_the_hourly_dispatch_over_unchanged_state() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("power_plant_basic").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let workers = 2;
    let (mut households, mut allocator, plant) = town(workers, 100.0);
    // A shared home: its two households split its standby load by household place.
    households.households[1].home_building_id = 0;
    allocator.buildings[0].occupancy = 2;
    warm_up(&mut households, &mut allocator);
    let stock = f64::from(allocator.buildings[plant].inventory_units(coal));

    // The citywide hourly dispatch's totals for this state: 83 single homes, the shared home
    // and the plant's own load, against its staffed capacity.
    let staffing = workers as f32 / profile.worker_capacity as f32;
    let plant_kwh = profile.electricity_demand_kwh_per_day(1.0, staffing, 0) / 24.0;
    let demand = f64::from((83.0 * 11.0 + 21.0) / 24.0 + plant_kwh);
    let capacity = f64::from(profile.power_capacity_kw * staffing);
    let fuel_per_kwh = f64::from(
        profile.input_port(coal).unwrap().units_per_day / profile.base_rate_units_per_day,
    );
    let generated = demand.min(capacity);
    let burned = capacity * fuel_per_kwh * generated / capacity;

    let profiles = ElectricityProfiles::new(&catalog);
    for building_idx in 0..allocator.buildings.len() {
        households.run_building_electricity(&profiles, &mut allocator, building_idx);
    }
    for household_id in 0..households.households.len() {
        households.record_household_electricity(&profiles, household_id, &mut allocator);
    }
    let hour = households.electricity_hour;
    let close = |a: f64, b: f64| (a - b).abs() < 1e-4 * b.max(1.0);
    assert!(
        close(hour.demand_kwh, demand),
        "{} vs {demand}",
        hour.demand_kwh
    );
    assert!(close(hour.capacity_kwh, capacity));
    assert!(close(hour.coal_burned_kg, burned));
    assert!(close(hour.coal_stock_kg, stock - burned));
    let b = &allocator.buildings[plant];
    assert!(close(f64::from(b.daily_power_service_units), generated));
    assert!(close(
        f64::from(b.daily_electricity.workplace.local_kwh),
        f64::from(plant_kwh)
    ));
    let home = allocator.buildings[0].daily_electricity.residential;
    assert!((home.consumed_kwh - 21.0 / 24.0).abs() < 1e-5);
    assert_eq!(home.local_kwh, home.consumed_kwh);
    for ledger in &households.daily_ledgers()[..2] {
        assert!((ledger.electricity.consumed_kwh - 10.5 / 24.0).abs() < 1e-5);
    }

    households.close_electricity_hour();
    let hour = households.electricity_hour;
    assert!(close(f64::from(hour.dispatch), generated / capacity));
    assert_eq!(hour.coverage, 1.0);
    assert!(close(households.day_totals.coal_consumed_kg, burned));
    assert!(close(households.coal_inventory_kg, stock - burned));
    assert_eq!(hour.demand_kwh, 0.0);
}

#[test]
fn coverage_and_dispatch_lag_exactly_one_hour() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let (mut households, mut allocator, plant) = town(1, 100.0);
    warm_up(&mut households, &mut allocator);
    let stock = allocator.buildings[plant].inventory_units(coal);
    let meter = |households: &HouseholdSystem| households.daily_ledgers()[0].electricity;
    let generation =
        |allocator: &BuildingAllocator| allocator.buildings[plant].daily_power_service_units;

    // Out of coal, the plant generates nothing at once, but meters keep the last hour's
    // coverage for this hour.
    allocator.buildings[plant].set_inventory_units(coal, 0.0);
    households.run_electricity_hour(&mut allocator);
    let outage = meter(&households);
    assert!(outage.consumed_kwh > 0.0);
    assert_eq!(outage.local_kwh, outage.consumed_kwh);
    assert_eq!(generation(&allocator), 0.0);
    let hour = households.electricity_hour;
    assert_eq!((hour.dispatch, hour.coverage), (0.0, 0.0));
    households.run_electricity_hour(&mut allocator);
    assert_eq!(meter(&households).local_kwh, outage.local_kwh);

    // Restocked, it offers capacity at once but generates only an hour later, at that hour's
    // dispatch.
    allocator.buildings[plant].set_inventory_units(coal, stock);
    households.run_electricity_hour(&mut allocator);
    assert_eq!(generation(&allocator), 0.0);
    assert_eq!(meter(&households).local_kwh, outage.local_kwh);
    let hour = households.electricity_hour;
    assert!(hour.dispatch > 0.0);
    assert_eq!(hour.coverage, 1.0);
    households.run_electricity_hour(&mut allocator);
    assert!(generation(&allocator) > 0.0);
    let restored = meter(&households);
    assert!((restored.local_kwh - outage.local_kwh - 11.0 / 24.0).abs() < 1e-5);
}
