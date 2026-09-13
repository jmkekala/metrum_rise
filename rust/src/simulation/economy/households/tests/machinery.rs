// SPDX-License-Identifier: GPL-2.0-only

//! Machinery recipe, maintenance stock and area-production regressions.

use super::support::*;
use super::*;
use crate::simulation::agriculture::{AgricultureSystem, FieldSite};
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
use crate::simulation::extraction::{ExtractorSite, ResourceExtractionSystem};

#[test]
fn machinery_factory_consumes_imported_materials_and_its_own_upkeep() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let steel = catalog.resource_runtime_id_for_id("steel").unwrap();
    let metals = catalog.resource_runtime_id_for_id("metals").unwrap();
    let mut allocator = BuildingAllocator::new();
    let mut factory = make_building(0.0, ZoneType::Industrial, "test:machinery", 0.0);
    factory.economy_profile_runtime_id = profile.runtime_id;
    factory.worker_count = 4;
    for port in &profile.inputs {
        factory.set_inventory_units(port.resource_runtime_id, 80.0);
    }
    allocator.buildings.push(factory);
    let mut households = HouseholdSystem::new();
    for _ in 0..24 {
        households.run_building_economy(&mut allocator, true);
    }
    let factory = &allocator.buildings[0];
    assert!((factory.inventory_units(machinery) - 120.0).abs() < 0.001);
    assert!((factory.inventory_units(steel) - 68.0).abs() < 0.001);
    assert!((factory.inventory_units(metals) - 76.0).abs() < 0.001);
    allocator.buildings[0].set_inventory_units(steel, 0.0);
    let stock = allocator.buildings[0].resource_inventory.clone();
    households.run_building_economy(&mut allocator, true);
    assert_eq!(
        allocator.buildings[0].resource_inventory, stock,
        "no materials means no production or upkeep consumption"
    );
}

#[test]
fn farm_upkeep_follows_area_staffing_and_available_stock() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("grain_farm_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let grain = catalog.resource_runtime_id_for_id("grain").unwrap();
    let mut allocator = BuildingAllocator::new();
    let mut farm = make_building(0.0, ZoneType::None, "test:farm", 0.0);
    farm.economy_profile_runtime_id = profile.runtime_id;
    farm.worker_count = 1;
    farm.commercial_activity_floor_scale = 1.0;
    farm.set_inventory_units(machinery, 0.5 / 24.0);
    allocator.buildings.push(farm);
    let mut agriculture = AgricultureSystem::from_sites(vec![FieldSite {
        building_idx: 0,
        resource_id: "grain".to_owned(),
        polygon_world: Vec::new(),
        area_m2: 20_000.0,
    }]);
    agriculture.apply_work_area_scales(&mut allocator);
    agriculture.produce_hourly(&mut allocator, &catalog);
    assert!(allocator.buildings[0].inventory_units(machinery) < 0.0001);
    assert!((allocator.buildings[0].inventory_units(grain) - 290.0 * 0.5 / 24.0).abs() < 0.001);
    let stock = allocator.buildings[0].resource_inventory.clone();
    agriculture.produce_hourly(&mut allocator, &catalog);
    assert_eq!(allocator.buildings[0].resource_inventory, stock);
    allocator.buildings[0].set_inventory_units(machinery, 80.0);
    allocator.buildings[0].worker_count = 0;
    agriculture.produce_hourly(&mut allocator, &catalog);
    assert_eq!(allocator.buildings[0].inventory_units(machinery), 80.0);
}

#[test]
fn mine_upkeep_is_charged_once_and_only_for_remaining_reserve() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("coal_mine_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let coal = catalog.resource_runtime_id_for_id("coal").unwrap();
    let mut allocator = BuildingAllocator::new();
    let mut mine = make_building(0.0, ZoneType::None, "test:mine", 0.0);
    mine.economy_profile_runtime_id = profile.runtime_id;
    mine.worker_count = 10;
    mine.commercial_activity_floor_scale = 1.0;
    mine.set_inventory_units(machinery, 80.0);
    allocator.buildings.push(mine);
    let mut extraction = ResourceExtractionSystem::from_sites(vec![ExtractorSite {
        building_idx: 0,
        resource_id: "coal".to_owned(),
        polygon_world: Vec::new(),
        area_m2: 20_000.0,
        total_reserve_units: 5.0,
        extracted_units: 0.0,
    }]);
    extraction.apply_work_area_scales(&mut allocator);
    assert_eq!(
        operating_input_demand_scale(&catalog, &allocator.buildings[0], profile),
        2.0
    );
    HouseholdSystem::new().run_building_economy(&mut allocator, true);
    assert_eq!(allocator.buildings[0].inventory_units(machinery), 80.0);
    extraction.produce_hourly(&mut allocator, &catalog);
    assert_eq!(allocator.buildings[0].inventory_units(coal), 5.0);
    assert!(
        (allocator.buildings[0].inventory_units(machinery) - (80.0 - 4.0 / 24.0)).abs() < 0.001
    );
    let stock = allocator.buildings[0].resource_inventory.clone();
    extraction.produce_hourly(&mut allocator, &catalog);
    assert_eq!(allocator.buildings[0].resource_inventory, stock);
    assert_eq!(
        physical_worker_capacity_for_profile(&allocator.buildings[0], profile),
        0
    );
    assert_eq!(
        operating_input_demand_scale(&catalog, &allocator.buildings[0], profile),
        0.0
    );
    assert_eq!(
        scaled_input_inventory_targets_for_building(
            &catalog,
            &allocator.buildings[0],
            profile,
            profile.input_port(machinery).unwrap(),
        ),
        (0.0, 0.0, 0.0),
    );
    // Persistence rebuilds the same productive-area cache without discarding the pit or its stock.
    allocator.buildings[0].set_work_area_scale(2.0);
    let restored = ResourceExtractionSystem::from_sites(extraction.sites().to_vec());
    restored.apply_work_area_scales(&mut allocator);
    assert_eq!(allocator.buildings[0].work_area_scale, 0.0);
    assert_eq!(restored.sites()[0].area_m2, 20_000.0);
    assert_eq!(allocator.buildings[0].resource_inventory, stock);
}

#[test]
fn machinery_inventory_fill_counts_shared_stock_once() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let mut factory = make_building(0.0, ZoneType::Industrial, "test:machinery", 0.0);
    factory.economy_profile_runtime_id = profile.runtime_id;
    for input in &profile.inputs {
        factory.set_inventory_units(input.resource_runtime_id, 80.0);
    }
    // Two 80-unit material stores and one shared 168-unit Machinery store.
    for stock in [0.0, 80.0, 168.0] {
        factory.set_inventory_units(machinery, stock);
        let fill = building_inventory_fill_ratio(&catalog, &factory, profile).unwrap();
        assert!((fill - (160.0 + stock) / 328.0).abs() < 0.0001);
    }
}

#[test]
fn machinery_liquidation_preserves_upkeep_and_shipment_reservations() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let mut factory = make_building(0.0, ZoneType::Industrial, "test:machinery", 0.0);
    factory.economy_profile_runtime_id = profile.runtime_id;
    factory.operating_budget = 0.0;
    factory.set_inventory_units(machinery, 120.0);
    let mut reserved = vec![0.0; catalog.resource_count()];
    reserved[ShipmentSystem::reservation_slot_for_building(
        0,
        machinery,
        catalog.resource_count(),
    )
    .unwrap()] = 20.0;
    for (target, expected_stock, expected_budget) in
        [(50.0, 110.0, 50.0), (f32::INFINITY, 100.0, 100.0)]
    {
        super::super::building_economy::liquidate_outputs_until_budget(
            0,
            &mut factory,
            &catalog,
            &reserved,
            catalog.resource_count(),
            tuning.owa_distress_liquidation_multiplier,
            target,
        );
        assert_eq!(factory.inventory_units(machinery), expected_stock);
        assert_eq!(factory.operating_budget, expected_budget);
    }
}

#[test]
fn small_upkeep_buffers_reorder_before_empty_and_zero_area_never_orders() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    for (profile_id, scale) in [
        ("grain_farm_basic", 0.01),
        ("coal_mine_basic", 0.01),
        ("water_plant_basic", 1.0),
    ] {
        let profile = catalog.profile_for_id(profile_id).unwrap();
        let port = profile.input_port(machinery).unwrap();
        let (target, reorder, _) = profile.input_inventory_targets(port, scale);
        assert_eq!(target, 80.0);
        assert!(target - profile.min_shipment_units > port.units_per_day * scale);
        assert!(reorder == 0.0 || reorder >= 40.0);
        assert_eq!(profile.input_inventory_targets(port, 0.0), (0.0, 0.0, 0.0));
        assert!(profile.initial_input_import_cost(&catalog, scale, 1.75) >= 2_800.0);
    }
}

#[test]
fn same_resource_upkeep_frees_storage_during_production() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("machinery_factory_basic").unwrap();
    let machinery = catalog.resource_runtime_id_for_id("machinery").unwrap();
    let steel = catalog.resource_runtime_id_for_id("steel").unwrap();
    let mut allocator = BuildingAllocator::new();
    let mut factory = make_building(0.0, ZoneType::Industrial, "test:machinery", 0.0);
    factory.economy_profile_runtime_id = profile.runtime_id;
    factory.worker_count = 4;
    for input in &profile.inputs {
        factory.set_inventory_units(input.resource_runtime_id, 80.0);
    }
    // Enough room for exactly one hour's net 40/day, including the two units reused daily.
    let capacity =
        profile.output_buffer_capacity_units_for(profile.output_port(machinery).unwrap());
    factory.set_inventory_units(machinery, capacity - 40.0 / 24.0);
    allocator.buildings.push(factory);
    HouseholdSystem::new().run_building_economy(&mut allocator, true);
    assert!((allocator.buildings[0].inventory_units(machinery) - capacity).abs() < 0.001);
    assert!((allocator.buildings[0].inventory_units(steel) - 79.5).abs() < 0.001);
}

#[test]
fn small_store_stock_targets_keep_two_batches_after_activity_scaling() {
    let catalog = load_runtime_economy_catalog().unwrap();
    let profile = catalog.profile_for_id("grocery_basic").unwrap();
    let store = make_building(0.0, ZoneType::Commercial, "test:grocery", 0.0);
    let (target, reorder, _) =
        scaled_input_inventory_targets_for_building(&catalog, &store, profile, &profile.inputs[0]);
    assert_eq!(target, 80.0);
    assert_eq!(reorder, 40.0);
    assert!(target - reorder >= profile.min_shipment_units);
}

#[test]
fn supported_producers_can_cover_imported_machinery_wages_and_taxes() {
    use crate::simulation::economy::fiscal::{CityFiscalPolicy, daily_property_tax, tax_amount};
    let catalog = load_runtime_economy_catalog().unwrap();
    let tuning = load_runtime_economy_tuning().unwrap();
    let fiscal = CityFiscalPolicy::from_runtime_tuning(&tuning);
    // Existing grain/coal chains supply locally. New machinery, steel and metals use OWA.
    // Utility examples have sufficient customers; production examples sell their net output.
    for (id, zone, service_sales) in [
        ("grain_farm_basic", ZoneType::None, 0.0),
        ("coal_mine_basic", ZoneType::None, 0.0),
        ("food_processor_basic", ZoneType::Industrial, 0.0),
        ("machinery_factory_basic", ZoneType::Industrial, 0.0),
        ("power_plant_basic", ZoneType::None, 1200.0),
        ("water_plant_basic", ZoneType::None, 200.0),
        ("wastewater_treatment_basic", ZoneType::None, 250.0),
    ] {
        let profile = catalog.profile_for_id(id).unwrap();
        let building = make_building(0.0, zone, "test:balance", 0.0);
        let workers = physical_worker_capacity_for_profile(&building, profile);
        let sales = service_sales
            + profile
                .outputs
                .iter()
                .map(|port| profile.net_output_units_per_day(port))
                .sum::<f32>();
        let input_cost: f32 = profile
            .inputs
            .iter()
            .filter(|port| profile.output_port(port.resource_runtime_id).is_none())
            .map(|port| {
                let resource = catalog
                    .resource_id_for_runtime_id(port.resource_runtime_id)
                    .unwrap();
                let multiplier = if matches!(resource, "machinery" | "steel" | "metals") {
                    tuning.owa_import_price_multiplier
                } else {
                    1.0
                };
                port.units_per_day
                    * catalog
                        .unit_price_for_resource(port.resource_runtime_id)
                        .unwrap()
                    * multiplier
            })
            .sum();
        let utility_cost: f32 = [
            "power_plant_basic",
            "water_plant_basic",
            "wastewater_treatment_basic",
        ]
        .iter()
        .map(|id| {
            catalog.profile_for_id(id).unwrap().unit_price_currency
                * tuning.owa_import_price_multiplier
        })
        .sum();
        let pretax_profit = sales * profile.unit_price_currency
            - input_cost
            - workers as f32 * profile.wage_max_currency_per_day
            - utility_cost
            - daily_property_tax(
                if matches!(
                    profile.kind,
                    EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
                ) {
                    ZoneType::Industrial
                } else {
                    zone
                },
                1,
                &fiscal,
            );
        let after_tax = pretax_profit - tax_amount(pretax_profit, fiscal.business_profit_tax_rate);
        assert!(
            after_tax > 0.0,
            "{id} cannot cover its supported full-operation costs: {after_tax}"
        );
    }
}
