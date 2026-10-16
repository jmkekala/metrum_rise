// SPDX-License-Identifier: GPL-2.0-only

//! Commercial bootstrap capacity and production tests.

use super::support::*;
use super::*;

#[test]
fn commercial_activity_is_bitwise_identical_across_worker_counts() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let mut allocator = BuildingAllocator::new();
    let home = register_test_residential_asset_with_capacity(
        &mut allocator,
        "test",
        "activity_home",
        10_000,
    );
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, &home, 0.0));
    let households: Vec<_> = (0..8_193)
        .map(|i| {
            let mut household = make_household(0, 2, 20.0, 0.0);
            household.stock = (i % 97) as f32 * 0.017;
            household.stock_days = household.stock / 2.0;
            household
        })
        .collect();
    let totals: Vec<_> = [1, 2, 7]
        .into_iter()
        .map(|workers| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(workers)
                .build()
                .unwrap()
                .install(|| {
                    commercial_activity_signal_for_city(&catalog, &households, &allocator)
                        .demand_units_per_day
                        .to_bits()
                })
        })
        .collect();
    assert!(
        totals.iter().all(|&total| total == totals[0]),
        "worker-dependent demand totals: {totals:?}"
    );
}

#[test]
fn zero_sales_commercial_active_worker_capacity_is_bootstrap_sized() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let building = make_building(0.0, ZoneType::Commercial, "test:grocery", 0.0);
    let profile = catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .expect("grocery runtime profile");

    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &building, profile),
        1
    );
}

#[test]
fn grocery_activity_follows_its_own_census_not_city_shortage() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, "test:home", 0.0));
    for x in [10.0, 20.0] {
        allocator
            .buildings
            .push(make_building(x, ZoneType::Commercial, "test:grocery", 0.0));
    }
    let mut households = HouseholdSystem::new();
    for _ in 0..7 {
        households.households.push(make_household(0, 2, 10.0, 0.0));
    }
    let profile = catalog
        .profile_by_runtime_id(allocator.buildings[1].economy_profile_runtime_id)
        .expect("grocery runtime profile");
    for store in &mut allocator.buildings[1..] {
        store.worker_count = profile.worker_capacity;
        for input in &profile.inputs {
            store.set_inventory_units(input.resource_runtime_id, input.units_per_day);
        }
    }

    // Empty pantries citywide do not open work at a store whose census funded none.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[]);
    let idle = building_operation_factors(&catalog, &allocator.buildings[1], profile);
    assert_eq!(idle.active_worker_capacity, 1, "bootstrap slot only");
    assert_eq!(idle.throughput_factor, 0.0);

    // The store its customers reserved runs at full funded activity; a city-wide demand ratio
    // must not also cap it.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[0.0, 1.0, 0.2]);
    let busy = building_operation_factors(&catalog, &allocator.buildings[1], profile);
    assert_eq!(busy.active_worker_capacity, profile.worker_capacity);
    assert_eq!(busy.throughput_factor, 1.0);
    let quiet = building_operation_factors(&catalog, &allocator.buildings[2], profile);
    assert_eq!(
        quiet.active_worker_capacity,
        (profile.worker_capacity as f32 * 0.2).ceil() as u32
    );
    assert!((quiet.throughput_factor - 0.2).abs() < 1e-6);
}

#[test]
fn rounded_store_staff_refill_the_shelf_only_up_to_target() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Commercial,
        "test:grocery",
        0.0,
    ));
    let profile = catalog
        .profile_by_runtime_id(allocator.buildings[0].economy_profile_runtime_id)
        .expect("grocery runtime profile");
    let output = &profile.outputs[0];
    let store = &mut allocator.buildings[0];
    store.worker_count = profile.worker_capacity;
    for input in &profile.inputs {
        store.set_inventory_units(input.resource_runtime_id, input.units_per_day);
    }
    // Funded sales take 78.5% of output; whole workers round that up to 12 of 15.
    let activity = 0.785;
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[activity]);
    let staffed = 12.0 / profile.worker_capacity as f32;
    let factors = building_operation_factors(&catalog, &allocator.buildings[0], profile);
    assert_eq!(factors.active_worker_capacity, 12);
    assert!(
        (factors.throughput_factor - staffed).abs() < 1e-6,
        "empty shelf: paid staff refill it"
    );

    let target = profile.net_output_units_per_day(output) * profile.stock_target_days * activity;
    allocator.buildings[0].set_inventory_units(output.resource_runtime_id, target);
    let factors = building_operation_factors(&catalog, &allocator.buildings[0], profile);
    assert!(
        (factors.throughput_factor - activity).abs() < 1e-6,
        "a full shelf produces only the funded sales"
    );
}

#[test]
fn grocery_stock_and_shopper_reservations_cover_pantry_recovery_before_hiring() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let resource = catalog
        .resource_runtime_id_for_id("household_supplies")
        .unwrap();
    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(make_building(0.0, ZoneType::Residential, "test:home", 0.0));
    allocator.buildings.push(make_building(
        10.0,
        ZoneType::Commercial,
        "test:grocery",
        0.0,
    ));
    let mut households = HouseholdSystem::new();
    for _ in 0..7 {
        households
            .households
            .push(make_household(0, 2, 1_000.0, 0.0));
    }
    allocator.buildings[1].set_inventory_units(resource, 70.0);
    let signal = commercial_activity_signal_for_city(&catalog, &households.households, &allocator);
    assert_eq!(signal.demand_units_per_day, 14.0);

    // Moving goods from the shelf into unpaid or paid shopper reservations must
    // not create another order to produce them.
    allocator.buildings[1].set_inventory_units(resource, 0.0);
    for (i, household) in households.households.iter_mut().enumerate() {
        household.reserved_amount = 10.0;
        household.replenishment_state = if i % 2 == 0 {
            REPLENISHMENT_SHOPPING_TO_STORE
        } else {
            REPLENISHMENT_SHOPPING_RETURNING
        };
    }
    let signal = commercial_activity_signal_for_city(&catalog, &households.households, &allocator);
    assert_eq!(signal.demand_units_per_day, 14.0);

    // One bulk purchase must not raise recurring payroll above the funded activity.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[0.0, 0.1]);
    allocator.buildings[1].daily_household_sales_value = 5_000.0;
    allocator.buildings[1].recent_household_sales_value = 5_000.0;
    let profile = catalog.profile_for_id("grocery_basic").unwrap();
    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &allocator.buildings[1], profile),
        2
    );
}

#[test]
fn grocery_production_and_inputs_follow_demand_below_rounded_worker_capacity() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("grocery_basic").unwrap();
    let mut grocery = make_building(0.0, ZoneType::Commercial, "test:grocery", 0.0);
    grocery.commercial_activity_floor_scale = 0.15; // 30 supplies/day, rounded to three workers.
    grocery.worker_count = 3;
    for input in &profile.inputs {
        grocery.set_inventory_units(input.resource_runtime_id, 100.0);
    }
    // A shelf at its three-day target leaves no refill for the rounded-up worker.
    grocery.set_inventory_units(profile.outputs[0].resource_runtime_id, 90.0);
    let factors = building_operation_factors(&catalog, &grocery, profile);
    assert_eq!(factors.active_worker_capacity, 3);
    assert!((factors.throughput_factor * 200.0 - 30.0).abs() < 0.001);
    let targets = scaled_input_inventory_targets_for_building(
        &catalog,
        &grocery,
        profile,
        &profile.inputs[0],
    );
    assert!(
        (targets.0 - 100.0).abs() < 0.001,
        "90 kg for three days retain the two-batch minimum, not 120 kg for rounded staffing"
    );
    assert!((funded_input_demand_scale(&grocery, profile, 0.15) - 0.15).abs() < 0.001);
}

#[test]
fn explicit_work_area_capacity_scales_without_double_staffing_penalty() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut building = make_building(0.0, ZoneType::None, "test:farm", 0.0);
    building.economy_profile_runtime_id = profile.runtime_id;
    building.set_inventory_units(
        catalog.resource_runtime_id_for_id("machinery").unwrap(),
        80.0,
    );
    building.work_area_scale = 0.2731;
    building.commercial_activity_floor_scale = 1.0;
    building.worker_count = 2;
    let output_port = profile.outputs.first().expect("grain output");
    let output_capacity =
        scaled_output_buffer_capacity_units_for_building(&building, profile, output_port);
    assert!(
        (output_capacity - profile.output_buffer_capacity_units_for(output_port) * 0.2731).abs()
            < 0.001,
        "explicit work-area output storage should scale with committed area"
    );
    let scaled_hourly_output =
        scaled_output_units_per_day_for_building(&building, profile, output_port) / 24.0;
    building.set_inventory_units(
        output_port.resource_runtime_id,
        output_capacity - scaled_hourly_output - 2.0,
    );

    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &building, profile),
        2
    );
    let factors = building_operation_factors(&catalog, &building, profile);
    assert_eq!(factors.active_worker_capacity, 2);
    assert_eq!(factors.effective_workers, 2);
    assert!(
        (factors.throughput_factor - 1.0).abs() < 0.001,
        "a small field should reach full throughput with its rounded worker capacity"
    );
    assert!(
        (factors.output_headroom_factor - 1.0).abs() < 0.001,
        "area-scaled producers should compare storage headroom to scaled hourly output"
    );
    building.worker_count = 1;
    assert!(
        (building_operation_factors(&catalog, &building, profile).throughput_factor - 0.5).abs()
            < 0.001
    );
}

#[test]
fn explicit_work_area_market_scale_limits_jobs_and_output() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut building = make_building(0.0, ZoneType::None, "test:farm", 0.0);
    building.economy_profile_runtime_id = profile.runtime_id;
    building.set_inventory_units(
        catalog.resource_runtime_id_for_id("machinery").unwrap(),
        80.0,
    );
    // Four physical slots make the quarter-market case exactly one whole worker.
    building.work_area_scale = 40.0;
    building.commercial_activity_floor_scale = 0.25;
    building.worker_count = 1;

    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &building, profile),
        1
    );
    let factors = building_operation_factors(&catalog, &building, profile);
    assert_eq!(factors.active_worker_capacity, 1);
    assert_eq!(factors.effective_workers, 1);
    assert!(
        (factors.throughput_factor - 0.25).abs() < 0.001,
        "market-capped explicit work areas should produce against physical area capacity"
    );
}

#[test]
fn explicit_work_area_activity_follows_its_census() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let farm_profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut allocator = BuildingAllocator::new();
    let farm_asset = register_test_asset(
        &mut allocator,
        "test",
        "market_scaled_farm",
        ZoneClass::Industrial,
    );
    let mut farm = make_building(0.0, ZoneType::None, &farm_asset, 0.0);
    farm.economy_profile_runtime_id = farm_profile.runtime_id;
    farm.work_area_scale = 2.0;
    farm.operating_budget = 10_000.0;
    allocator.buildings.push(farm);

    // Neither an `OWA` gateway nor local buyers open jobs the census did not fund.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[]);
    assert_eq!(allocator.buildings[0].commercial_activity_floor_scale, 0.0);
    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &allocator.buildings[0], farm_profile),
        0
    );

    // An export-funded census opens the whole area without a local-demand throttle.
    refresh_commercial_activity_floor(&catalog, &mut allocator, &[1.0]);
    assert!((allocator.buildings[0].commercial_activity_floor_scale - 1.0).abs() < 0.001);
    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &allocator.buildings[0], farm_profile),
        2
    );
}

#[test]
fn grain_farm_density_preserves_area_yield_and_startup_runway() {
    use crate::simulation::agriculture::{AgricultureSystem, FieldSite};
    use crate::simulation::work_area::top_up_explicit_work_area_startup_budget;
    use crate::simulation::work_area::{minimum_work_area_workers, work_area_worker_equivalent};

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm");
    let output = profile.outputs.first().expect("grain output");
    for (hectares, workers) in [
        (0.0, 0),
        (0.01, 2),
        (1.0, 2),
        (4.0, 2),
        (10.0, 2),
        (10.01, 2),
        (20.0, 2),
        (20.01, 3),
        (50.0, 5),
        (50.01, 6),
        (60.0, 6),
        (100.0, 6),
        (1000.0, 6),
    ] {
        let mut allocator = BuildingAllocator::new();
        let mut farm = make_building(0.0, ZoneType::None, "test:farm", 0.0);
        farm.economy_profile_runtime_id = profile.runtime_id;
        farm.commercial_activity_floor_scale = 1.0;
        farm.worker_count = workers;
        farm.operating_budget = 500.0;
        farm.profit_tax_budget_baseline = 500.0;
        allocator.buildings.push(farm);
        let agriculture = AgricultureSystem::from_sites(vec![FieldSite {
            building_idx: 0,
            resource_id: "grain".to_owned(),
            polygon_world: Vec::new(),
            area_m2: hectares * 10_000.0,
        }]);
        // Load reconstructs the hectare scale from saved sites using this same path.
        agriculture.apply_work_area_scales(&mut allocator);
        assert_eq!(allocator.worker_capacity_with_catalog(0, &catalog), workers);
        let farm = &mut allocator.buildings[0];
        top_up_explicit_work_area_startup_budget(farm, &catalog, hectares);
        assert_eq!(
            active_worker_capacity_for_profile(&catalog, farm, profile),
            workers
        );
        assert!(
            (work_area_worker_equivalent(
                profile.workers_per_hectare,
                farm.work_area_scale,
                minimum_work_area_workers(profile.kind),
                profile.area_worker_capacity_max,
            ) - if hectares > 0.0 {
                (hectares * 0.1).clamp(2.0, 6.0)
            } else {
                0.0
            })
            .abs()
                < 0.001
        );
        let daily_imported_utilities = profile
            .utility_demand_units_per_day(hectares)
            .into_iter()
            .zip(local_utility_service_costs(&catalog))
            .map(|(qty, price)| qty * price * tuning.owa_import_price_multiplier)
            .sum::<f32>();
        let expected_budget = (workers as f32 * 90.0 * 7.0).max(500.0)
            + profile.initial_input_import_cost(
                &catalog,
                hectares,
                tuning.owa_import_price_multiplier,
            )
            + daily_imported_utilities * 7.0;
        assert!(
            (farm.operating_budget - expected_budget).abs() <= expected_budget * f32::EPSILON * 2.0
        );
        assert_eq!(farm.profit_tax_budget_baseline, farm.operating_budget);
        assert!(
            (scaled_output_units_per_day_for_building(farm, profile, output) - 20.0 * hectares)
                .abs()
                < 0.01
        );

        // Startup capital buys inputs; yield assertions begin after the delivery.
        allocator.buildings[0].set_inventory_units(
            catalog.resource_runtime_id_for_id("machinery").unwrap(),
            80.0,
        );
        agriculture.produce_hourly(&mut allocator, &catalog);
        assert!(
            (allocator.buildings[0].inventory_units(output.resource_runtime_id)
                - 20.0 * hectares / 24.0)
                .abs()
                < 0.001
        );
    }
}

#[test]
fn one_worker_commercial_production_uses_hourly_input_need() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supply_resource = household_supply_resource_runtime_id(&catalog);
    let mut allocator = BuildingAllocator::new();
    let mut store = make_building(0.0, ZoneType::Commercial, "test:grocery", 0.0);
    let profile = catalog
        .profile_by_runtime_id(store.economy_profile_runtime_id)
        .expect("grocery runtime profile");
    let input_port = profile.inputs.first().expect("grocery has an input");
    store.worker_count = 1;
    store.resource_inventory[input_port.resource_runtime_id as usize - 1] = 28.5;
    allocator.buildings.push(store);

    let mut households = HouseholdSystem::new();
    allocator
        .buildings
        .push(make_building(10.0, ZoneType::Residential, "test:home", 0.0));
    for _ in 0..7 {
        households
            .households
            .push(make_household(1, 2, 1_000.0, 0.0));
    }
    // The census funded exactly one worker's share of the store.
    households
        .job_funding
        .set_activity(0, 1.0 / profile.worker_capacity as f32);
    households.run_building_hour_for_test(&mut allocator);

    let output_units = allocator.buildings[0].inventory_units(household_supply_resource);
    let expected_units = profile.outputs[0].units_per_day / 24.0 / profile.worker_capacity as f32;
    assert!(
        (output_units - expected_units).abs() < 0.001,
        "one active worker with more than one hourly input need should produce at full one-worker rate: got={output_units:.3} expected={expected_units:.3}"
    );
}
