// SPDX-License-Identifier: GPL-2.0-only

//! Service-sales pool regressions (`SIM-02-I`): money paid at household phases and drawn at
//! store phases an hour later is conserved, also when a store leaves before its draw.

use super::support::*;
use super::*;

// One home with a large household and two fully staffed barbers the census funds in full.
fn barber_street() -> (HouseholdSystem, BuildingAllocator) {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("personal_service_small").unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_asset(&mut allocator, "test", "pool_home", ZoneClass::Residential);
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let barber = register_test_commercial_asset_with_profile(
        &mut allocator,
        "test",
        "pool_barber",
        "personal_service_small",
    );
    let mut households = HouseholdSystem::new();
    for idx in 1..=2 {
        let mut store = make_building(10.0 * idx as f32, ZoneType::Commercial, &barber, 0.0);
        store.economy_profile_runtime_id = profile.runtime_id;
        store.worker_count = profile.worker_capacity;
        store.operating_budget = 0.0;
        allocator.buildings.push(store);
        households.job_funding.set_activity(idx, 1.0);
    }
    let mut household = make_household(0, 100, 0.0, 0.0);
    household.budget = 1_000.0;
    households.households.push(household);
    (households, allocator)
}

// Household and store cash and the pool between them.
fn city_cash(households: &HouseholdSystem, allocator: &BuildingAllocator) -> f64 {
    let household_cash: f64 = households
        .households
        .iter()
        .map(|household| f64::from(household.budget))
        .sum();
    let building_cash: f64 = allocator
        .buildings
        .iter()
        .map(|building| f64::from(building.operating_budget))
        .sum();
    household_cash + building_cash + households.service_sales.money()
}

#[test]
fn service_sales_pool_conserves_money_when_a_store_leaves_before_its_draw() {
    let (mut households, mut allocator) = barber_street();
    let initial = city_cash(&households, &allocator);
    // The first hour prices the sales, the second pays into the pool, the third draws it.
    for _ in 0..3 {
        households.run_building_hour_for_test(&mut allocator);
        assert!((city_cash(&households, &allocator) - initial).abs() < 1e-6);
    }
    let first_draw = allocator.buildings[1].revenue;
    assert!(first_draw > 0.0);
    assert_eq!(allocator.buildings[2].revenue, first_draw);

    // The second barber recorded this hour's capacity and leaves before drawing on it; its
    // cash leaves the city with it. Minimal fixture: the store is last.
    let removed = allocator.buildings.pop().unwrap();
    let removed_cash = f64::from(removed.operating_budget);
    let pool = households.service_sales.money();
    households.run_building_hour_for_test(&mut allocator);
    assert!((city_cash(&households, &allocator) + removed_cash - initial).abs() < 1e-6);
    // The remaining barber draws only its own share; the other carries into the next pool.
    let second_draw = allocator.buildings[1].revenue - first_draw;
    assert!((f64::from(second_draw) - pool / 2.0).abs() < 1e-4);

    // As the only store, it then draws the whole pool, the carried share included.
    let carried_and_paid = households.service_sales.money();
    households.run_building_hour_for_test(&mut allocator);
    let third_draw = allocator.buildings[1].revenue - first_draw - second_draw;
    assert!((f64::from(third_draw) - carried_and_paid).abs() < 1e-4);
    assert!((city_cash(&households, &allocator) + removed_cash - initial).abs() < 1e-6);
}
