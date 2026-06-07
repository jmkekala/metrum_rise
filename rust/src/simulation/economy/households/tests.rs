//! Household module tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::households::metrics::household_supply_unit_price;
use crate::simulation::economy::logistics::{
    CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};

fn test_economy_runtime_id(zone_type: ZoneType) -> u16 {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    match zone_type {
        ZoneType::Commercial => {
            catalog
                .profile_for_id("grocery_basic")
                .expect("grocery starter profile")
                .runtime_id
        }
        ZoneType::Industrial => {
            catalog
                .profile_for_id("food_processor_basic")
                .expect("food processor starter profile")
                .runtime_id
        }
        _ => 0,
    }
}

fn make_building(center_x: f32, zone_type: ZoneType, asset_id: &str, stock: f32) -> Building {
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let runtime_id = test_economy_runtime_id(zone_type);
    let mut resource_inventory = vec![0.0; catalog.resource_count()];
    if stock > 0.0
        && let Some(profile) = catalog.profile_by_runtime_id(runtime_id)
        && let Some(output_port) = profile.outputs.first()
    {
        resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
    }
    Building {
        center_x,
        center_y: 0.0,
        width_cells: 2,
        depth_cells: 2,
        zone_profile_runtime_id: 0,
        parcel_id: 0,
        zone_type,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        is_deserted: false,
        budget_distress: false,
        edge_idx: 0,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: asset_id.to_owned(),
        level: 1,
        broken: false,
        economy_profile_runtime_id: runtime_id,
        economy_broken: false,
        resource_inventory,
        revenue: 0.0,
        operating_budget: 500.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

fn make_foot_only_edge(start_node: u32, end_node: u32) -> Edge {
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 2.0,
        fwd_lanes: 0,
        bkw_lanes: 0,
        speed_limit: 5.0,
        base_cost: 10.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    }
}

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
    };
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                flat_size_m2: if matches!(zone, ZoneClass::Residential | ZoneClass::Mixed) {
                    Some(80.0)
                } else {
                    None
                },
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: match zone {
                    ZoneClass::Commercial => Some("grocery_basic".to_owned()),
                    ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
                    _ => None,
                },
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

fn register_test_residential_asset_with_capacity(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    household_capacity: u32,
) -> String {
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Small Home".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                flat_size_m2: Some(80.0),
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(ZoneClass::Residential),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: Some(household_capacity),
                worker_capacity: None,
                service_class: None,
                economy_profile: None,
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

fn register_test_utility_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    profile_id: &str,
) -> String {
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Utility".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::Explicit,
                zone_type: None,
                density: None,
                lot_width_cells: 2,
                lot_depth_cells: 2,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: None,
                worker_capacity: Some(4),
                service_class: None,
                economy_profile: Some(profile_id.to_owned()),
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

#[test]
fn household_replenishment_flows_through_reserved_and_pickup_states() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "replenish_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "replenish_com",
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
        50.0,
    ));
    allocator.rebuild_zone_index();

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_RESERVED
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        40.0
    );
    assert_eq!(households.households[0].budget, 50.0);

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_PICKUP_PENDING
    );

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FULFILLED
    );
    assert_eq!(households.households[0].stock, 10.0);
    assert_eq!(allocator.buildings[1].revenue, 250.0);
}

#[test]
fn zero_stock_household_bypasses_replenishment_stagger_when_store_has_supply() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 5,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "urgent_replenish_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "urgent_replenish_com",
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
        50.0,
    ));
    allocator.rebuild_zone_index();

    households.run_household_replenishment(&mut allocator, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_RESERVED
    );
}

#[test]
fn deserted_store_cannot_sell_household_supplies() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_store",
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
        50.0,
    ));
    allocator.buildings[1].is_deserted = true;
    allocator.rebuild_zone_index();

    households.run_household_replenishment(&mut allocator, 0);

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
fn replenishment_respects_pickup_eta_above_one_hour() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 300.0,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_RESERVED,
        cooldown_hours: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        pickup_eta_hours: 3,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "eta_res", ZoneClass::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "test", "eta_store", ZoneClass::Commercial);
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

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_RESERVED
    );
    assert_eq!(households.households[0].pickup_eta_hours, 2);

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_RESERVED
    );
    assert_eq!(households.households[0].pickup_eta_hours, 1);

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_PICKUP_PENDING
    );

    households.run_household_replenishment(&mut allocator, 0);
    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FULFILLED
    );
    assert_eq!(households.households[0].stock, 5.0);
}

#[test]
fn canceled_pickup_restores_reserved_store_inventory() {
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 0.0,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_PICKUP_PENDING,
        cooldown_hours: 0,
        reserved_store_building_id: 1,
        reserved_amount: 5.0,
        reserved_total_cost: 10.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "cancel_res", ZoneClass::Residential);
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "cancel_store",
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
    allocator.buildings[1].is_deserted = true;
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");

    households.run_household_replenishment(&mut allocator, 0);

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
    let partial_units = 5.0;

    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: partial_units * unit_price,
        stock: 0.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 0.0,
        replenishment_state: REPLENISHMENT_NEEDS,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "partial_restock_res",
        ZoneClass::Residential,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "partial_restock_com",
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
        50.0,
    ));
    allocator.rebuild_zone_index();

    households.run_household_replenishment(&mut allocator, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_RESERVED
    );
    assert!((households.households[0].reserved_amount - partial_units).abs() < f32::EPSILON);
    assert_eq!(households.households[0].budget, 0.0);

    households.run_household_replenishment(&mut allocator, 0);
    households.run_household_replenishment(&mut allocator, 0);

    assert_eq!(
        households.households[0].replenishment_state,
        REPLENISHMENT_FULFILLED
    );
    assert!((households.households[0].stock - partial_units).abs() < f32::EPSILON);
    assert!((households.households[0].stock_days - 2.5).abs() < f32::EPSILON);
}

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
        resource_runtime_id: household_supplies,
        amount: 20.0,
        source: ShipmentEndpoint::Building(0),
        destination: ShipmentEndpoint::OwaBorder(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        total_cost: 0.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    let mut households = HouseholdSystem::new();
    households.settle_daily_utilities(&mut allocator, &logistics);

    assert_eq!(
        allocator.buildings[0].inventory_units(household_supplies),
        20.0
    );
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
    households.settle_daily_utilities(&mut allocator, &logistics);
    assert_eq!(allocator.buildings[1].revenue, 0.0);
    assert_eq!(allocator.buildings[2].revenue, 0.0);
    assert_eq!(allocator.buildings[3].revenue, 0.0);

    allocator.buildings[0].operating_budget = 500.0;
    for idx in 1..=3 {
        allocator.buildings[idx].worker_count = 1;
    }
    households.settle_daily_utilities(&mut allocator, &logistics);
    assert!(allocator.buildings[1].revenue > 0.0);
    assert!(allocator.buildings[2].revenue > 0.0);
    assert!(allocator.buildings[3].revenue > 0.0);
}

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

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for agent in [a0, a1] {
        agents.household_id[agent] = 0;
        agents.transit[agent] = TRANSIT_IN_BUILDING;
        agents.current_building[agent] = 0;
        agents.has_car[agent] = true;
    }

    let network = TransitNetwork::new();
    let graph = RegionGraph::new();
    households.assign_agent_workplaces(&mut agents, &mut allocator, &network, &graph);

    assert_eq!(agents.work_building[a0], 1);
    assert_eq!(agents.work_building[a1], 2);
    assert_eq!(allocator.buildings[1].worker_count, 1);
    assert_eq!(allocator.buildings[2].worker_count, 1);
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

    households.pay_daily_wages(&mut agents, &mut allocator);

    assert_eq!(agents.work_building[agent], usize::MAX);
    assert_eq!(allocator.buildings[1].worker_count, 0);
    assert_eq!(households.households[0].budget, 0.0);
    assert_eq!(allocator.buildings[1].operating_budget, daily_wage);
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

    let mut agents = AgentSystem::new();
    let agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[agent] = 0;
    agents.transit[agent] = TRANSIT_IN_BUILDING;
    agents.current_building[agent] = 0;
    agents.has_car[agent] = true;
    agents.assign_work_building(agent, 1, 0);
    agents.job_lock_days[agent] = 0;

    let network = TransitNetwork::new();
    let graph = RegionGraph::new();
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
    let network = TransitNetwork::new();
    let graph = RegionGraph::new();
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
    households.operational_hour_tick(
        &mut agents,
        &mut allocator,
        &mut logistics,
        &network,
        &graph,
        0,
        0,
    );

    assert_eq!(households.households[0].member_count, 1);
    assert_eq!(allocator.buildings[1].worker_count, 1);
}

fn make_household(
    home_building_id: usize,
    member_count: u16,
    reserve_days: f32,
    stock_days: f32,
) -> Household {
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    let tuning = load_runtime_economy_tuning()
        .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
    let consumption_rate = 1.0;
    let daily_supply_cost =
        member_count.max(1) as f32 * consumption_rate * household_supply_unit_price(&catalog);
    let daily_utility_cost =
        member_count.max(1) as f32 * tuning.households.utility_cost_per_member_per_day;
    let daily_essential_cost = daily_supply_cost + daily_utility_cost;
    Household {
        home_building_id,
        budget: reserve_days * daily_essential_cost,
        stock: stock_days * member_count.max(1) as f32 * consumption_rate,
        member_count,
        consumption_rate,
        stock_days,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        pickup_eta_hours: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    }
}

#[test]
fn demand_household_removal_prioritizes_unhoused_households() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.5, 1.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 5.0, 5.0));
    households.households.push(make_household(1, 1, 2.0, 2.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "removal_res_a",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let housed_a = agents.spawn_housed_agent(0, 0.0, 0.0);
    let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
    let housed_b = agents.spawn_housed_agent(1, 0.0, 0.0);
    agents.household_id[housed_a] = 0;
    agents.household_id[unhoused] = 1;
    agents.home_building[unhoused] = usize::MAX;
    agents.target_building[unhoused] = usize::MAX;
    agents.household_id[housed_b] = 2;
    agents.recalculate_occupancy(&mut allocator);

    households.execute_demand_household_removal(1, &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 2);
    assert_eq!(agents.len(), 2);
    assert!(
        agents
            .household_id
            .iter()
            .all(|&household_id| household_id < households.households.len())
    );
    assert!(agents.home_building.iter().all(|&home| home != usize::MAX));
}

#[test]
fn demand_household_removal_uses_weaker_housed_households_after_unhoused_pool() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 0.5, 0.5));
    households.households.push(make_household(1, 1, 5.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 4.0, 4.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "removal_res_b",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let weak_housed = agents.spawn_housed_agent(0, 0.0, 0.0);
    let strong_housed = agents.spawn_housed_agent(1, 0.0, 0.0);
    let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[weak_housed] = 0;
    agents.household_id[strong_housed] = 1;
    agents.household_id[unhoused] = 2;
    agents.home_building[unhoused] = usize::MAX;
    agents.target_building[unhoused] = usize::MAX;
    agents.recalculate_occupancy(&mut allocator);

    households.execute_demand_household_removal(2, &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 1);
    assert_eq!(agents.len(), 1);
    assert_eq!(households.households[0].home_building_id, 1);
    assert_eq!(agents.household_id[0], 0);
    assert_eq!(agents.home_building[0], 1);
    assert_eq!(allocator.buildings[0].occupancy, 0);
    assert_eq!(allocator.buildings[1].occupancy, 1);
}

#[test]
fn unhoused_household_rehouses_into_affordable_vacant_home() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(usize::MAX, 2, 12.0, 3.0);
    household.unhoused_days_elapsed = 4;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "rehouse_res",
        ZoneClass::Residential,
    );
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.household_id[a] = 0;
        agents.home_building[a] = usize::MAX;
        agents.current_building[a] = usize::MAX;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_ACCESS_INGRESS;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(households.households[0].unhoused_days_elapsed, 0);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(households.households.len(), 1);
    assert_eq!(agents.home_building[a1], 0);
    assert_eq!(agents.target_building[a0], 0);
    assert_eq!(agents.target_building[a1], 0);
}

#[test]
fn upgrade_search_does_not_consume_same_or_lower_level_vacancy() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 30.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 30.0, 5.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_residential_asset_with_capacity(
        &mut allocator,
        "test",
        "upgrade_no_burn_res",
        1,
    );
    let mut current_home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    current_home.level = 2;
    current_home.occupancy = 1;
    allocator.buildings.push(current_home);
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let housed_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[housed_agent] = 0;
    let unhoused_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[unhoused_agent] = 1;
    agents.home_building[unhoused_agent] = usize::MAX;
    agents.current_building[unhoused_agent] = usize::MAX;
    agents.target_building[unhoused_agent] = usize::MAX;
    agents.planned_target_building[unhoused_agent] = usize::MAX;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 0);
    assert_eq!(households.households[1].home_building_id, 1);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(allocator.buildings[1].occupancy, 1);
    assert_eq!(agents.home_building[unhoused_agent], 1);
}

#[test]
fn same_day_relocation_frees_home_for_later_household() {
    let mut households = HouseholdSystem::new();
    households.households.push(make_household(0, 1, 30.0, 5.0));
    households
        .households
        .push(make_household(usize::MAX, 1, 30.0, 5.0));

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_residential_asset_with_capacity(
        &mut allocator,
        "test",
        "same_day_free_res",
        1,
    );
    let mut old_home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    old_home.level = 1;
    old_home.occupancy = 1;
    let mut upgrade_home = make_building(20.0, ZoneType::Residential, &residential_asset, 0.0);
    upgrade_home.level = 2;
    allocator.buildings.push(old_home);
    allocator.buildings.push(upgrade_home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let upgrading_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[upgrading_agent] = 0;
    let unhoused_agent = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[unhoused_agent] = 1;
    agents.home_building[unhoused_agent] = usize::MAX;
    agents.current_building[unhoused_agent] = usize::MAX;
    agents.target_building[unhoused_agent] = usize::MAX;
    agents.planned_target_building[unhoused_agent] = usize::MAX;

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, 1);
    assert_eq!(households.households[1].home_building_id, 0);
    assert_eq!(allocator.buildings[0].occupancy, 1);
    assert_eq!(allocator.buildings[1].occupancy, 1);
    assert_eq!(agents.home_building[upgrading_agent], 1);
    assert_eq!(agents.home_building[unhoused_agent], 0);
}

#[test]
fn unrehouseable_unhoused_household_accumulates_unhoused_days() {
    let mut households = HouseholdSystem::new();
    households
        .households
        .push(make_household(usize::MAX, 2, 0.0, 0.0));

    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();

    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].unhoused_days_elapsed, 1);

    households.resolve_household_housing(&mut agents, &mut allocator);
    assert_eq!(households.households[0].unhoused_days_elapsed, 2);
}

#[test]
fn failed_stay_rule_evicts_household_when_no_affordable_home_exists() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.5, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", "evict_res", ZoneClass::Residential);
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.household_id[a] = 0;
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(households.households[0].stay_failure_days, 0);
    assert_eq!(allocator.buildings[0].occupancy, 0);
    assert_eq!(agents.home_building[a0], usize::MAX);
    assert_eq!(agents.home_building[a1], usize::MAX);
    assert_eq!(agents.current_building[a0], usize::MAX);
    assert_eq!(agents.current_building[a1], usize::MAX);
    assert_eq!(agents.transit[a0], TRANSIT_ACCESS_INGRESS);
    assert_eq!(agents.transit[a1], TRANSIT_ACCESS_INGRESS);
}

#[test]
fn evicted_unhoused_household_keeps_membership_until_demand_removal() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.5, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "tracked_unhoused_res",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.level = 2;
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.household_id[a] = 0;
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);
    households.ensure_agent_households(&mut agents);
    households.rebuild_household_membership(&agents);

    assert_eq!(households.households.len(), 1);
    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(households.households[0].member_count, 2);
    assert_eq!(agents.household_id[a0], 0);
    assert_eq!(agents.household_id[a1], 0);

    households.execute_demand_household_removal(1, &mut agents, &mut allocator);

    assert_eq!(households.households.len(), 0);
    assert_eq!(agents.len(), 0);
}

#[test]
fn failed_stay_rule_does_not_relocate_zero_reserve_household_to_level_one() {
    let mut households = HouseholdSystem::new();
    let mut household = make_household(0, 2, 0.0, 1.0);
    household.stay_failure_days = 1;
    households.households.push(household);

    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "zero_reserve_relocate_res",
        ZoneClass::Residential,
    );
    let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
    home.occupancy = 1;
    allocator.buildings.push(home);
    allocator.buildings.push(make_building(
        20.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.rebuild_zone_index();

    let mut agents = AgentSystem::new();
    let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
    let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
    for a in [a0, a1] {
        agents.household_id[a] = 0;
        agents.home_building[a] = 0;
        agents.current_building[a] = 0;
        agents.target_building[a] = usize::MAX;
        agents.planned_target_building[a] = usize::MAX;
        agents.transit[a] = TRANSIT_IN_BUILDING;
    }

    households.resolve_household_housing(&mut agents, &mut allocator);

    assert_eq!(households.households[0].home_building_id, usize::MAX);
    assert_eq!(allocator.buildings[0].occupancy, 0);
    assert_eq!(allocator.buildings[1].occupancy, 0);
    assert_eq!(agents.home_building[a0], usize::MAX);
    assert_eq!(agents.home_building[a1], usize::MAX);
}
