//! Commercial bootstrap capacity and production tests.

use super::support::*;
use super::*;

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
fn household_shortage_floor_opens_commercial_workers_without_sales() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
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
        households.households.push(make_household(0, 2, 10.0, 0.0));
    }

    refresh_commercial_activity_floor(&catalog, &households.households, &mut allocator);
    let store = &allocator.buildings[1];
    let profile = catalog
        .profile_by_runtime_id(store.economy_profile_runtime_id)
        .expect("grocery runtime profile");

    assert_eq!(
        active_worker_capacity_for_profile(&catalog, store, profile),
        3,
        "14 residents with empty household stock should open a demand floor above the one-worker bootstrap"
    );
}

#[test]
fn explicit_work_area_capacity_scales_without_double_staffing_penalty() {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let profile = catalog
        .profile_for_id("grain_farm_basic")
        .expect("grain farm runtime profile");
    let mut building = make_building(0.0, ZoneType::None, "test:farm", 0.0);
    building.economy_profile_runtime_id = profile.runtime_id;
    building.work_area_scale = 0.2731;
    building.worker_count = 3;
    let output_port = profile.outputs.first().expect("grain output");
    let output_capacity = profile.output_buffer_capacity_units_for(output_port);
    building.set_inventory_units(output_port.resource_runtime_id, output_capacity - 2.0);

    assert_eq!(
        active_worker_capacity_for_profile(&catalog, &building, profile),
        3
    );
    let factors = building_operation_factors(&catalog, &building, profile);
    assert_eq!(factors.active_worker_capacity, 3);
    assert_eq!(factors.effective_workers, 3);
    assert!(
        (factors.throughput_factor - 1.0).abs() < 0.001,
        "area-scaled full staffing should not divide throughput by the authored one-hectare worker count"
    );
    assert!(
        (factors.output_headroom_factor - 1.0).abs() < 0.001,
        "area-scaled producers should compare storage headroom to scaled hourly output"
    );
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
    households.run_building_economy(&mut allocator);

    let output_units = allocator.buildings[0].inventory_units(household_supply_resource);
    let expected_units = profile.outputs[0].units_per_day / 24.0 / profile.worker_capacity as f32;
    assert!(
        (output_units - expected_units).abs() < 0.001,
        "one active worker with more than one hourly input need should produce at full one-worker rate: got={output_units:.3} expected={expected_units:.3}"
    );
}
