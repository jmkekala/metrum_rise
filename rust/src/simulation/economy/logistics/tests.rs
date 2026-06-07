//! Logistics unit tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, resolve_building_economy_profile_binding,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};

fn register_test_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
) -> String {
    let economy_profile = match zone {
        ZoneClass::Commercial => Some("grocery_basic"),
        ZoneClass::Industrial => Some("food_processor_basic"),
        _ => None,
    };
    register_test_asset_with_profile(allocator, pack_id, asset_id, zone, economy_profile)
}

fn register_test_asset_with_profile(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    zone: ZoneClass,
    economy_profile: Option<&str>,
) -> String {
    let (household_capacity, worker_capacity) = match zone {
        ZoneClass::Residential => (Some(6), None),
        ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
        ZoneClass::Mixed => (Some(4), Some(2)),
    };
    let economy_profile = economy_profile.map(str::to_owned);
    let manifest = AssetManifest {
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
            flat_size_m2: None,
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile,
            preview_scale: Some(1.0),
        }),
        prop: None,
        vehicle: None,
        character: None,
        pivot_offset: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn make_building(
    allocator: &BuildingAllocator,
    center_x: f32,
    zone_type: ZoneType,
    edge_idx: usize,
    asset_id: &str,
    stock: f32,
    budget: f32,
) -> Building {
    let economy_binding = resolve_building_economy_profile_binding(&allocator.registry, asset_id);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let mut resource_inventory = vec![0.0; catalog.resource_count()];
    if stock > 0.0
        && let Some(profile) = catalog.profile_by_runtime_id(economy_binding.runtime_id)
        && let Some(output_port) = profile.outputs.first()
    {
        resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
    }
    Building {
        center_x,
        center_y: 10.0,
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
        edge_idx,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
        worker_count: 0,
        asset_id: asset_id.to_owned(),
        level: 1,
        broken: false,
        economy_profile_runtime_id: economy_binding.runtime_id,
        economy_broken: economy_binding.economy_broken,
        resource_inventory,
        revenue: 0.0,
        operating_budget: budget,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

fn simple_graph_with_border() -> (RegionGraph, TransitNetwork, usize, usize, u32) {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Border);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let e0 = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 20.0,
        base_cost: 5.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
    });
    let e1 = graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 20.0,
        base_cost: 5.0,
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
    });
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
    (graph, network, e0, e1, n0)
}

#[test]
fn local_supplier_creates_and_delivers_shipment() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "logistics_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "logistics_commercial",
        ZoneClass::Commercial,
    );
    let supplier = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        100.0,
        2_000.0,
    );
    allocator.buildings.push(supplier);
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_LOCAL);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        100.0
    );

    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert!(allocator.buildings[1].inventory_units(staple_food) > 0.0);
    assert!(allocator.buildings[0].inventory_units(staple_food) < 300.0);
    assert!(allocator.buildings[0].revenue > 0.0);
    assert!(
        shipments
            .shipments
            .iter()
            .all(|shipment| shipment.resource_runtime_id == staple_food)
    );
}

#[test]
fn owa_border_fallback_creates_import_shipment() {
    let (graph, network, _industrial_edge, commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_commercial",
        ZoneClass::Commercial,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        50.0,
        5_000.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
    assert_eq!(shipments.shipments[0].source_border_node, border_node);

    shipments.hourly_tick(&mut allocator, &network, &graph, 480);
    assert!(shipments.shipments.is_empty());
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    assert!(allocator.buildings[0].inventory_units(staple_food) > 0.0);
}

#[test]
fn owa_border_fallback_scales_import_to_affordable_amount() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_affordable_commercial",
        ZoneClass::Commercial,
    );
    let destination = make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        50.0,
        1_500.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    assert!(shipments.shipments[0].amount >= grocery.min_shipment_units);
    let grocery_input = grocery.inputs.first().expect("grocery input port");
    assert!(shipments.shipments[0].amount < grocery.inventory_target_units_for(grocery_input));
    assert!(allocator.buildings[0].operating_budget <= 0.001);
}

#[test]
fn inputless_industrial_profile_does_not_request_input_imports() {
    let (graph, network, industrial_edge, _commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "owa_industrial_inputs",
        ZoneClass::Industrial,
    );
    let destination = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        0.0,
        5_000.0,
    );
    allocator.buildings.push(destination);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    assert_eq!(allocator.buildings[0].inventory_units(staple_food), 0.0);
}

#[test]
fn local_supplier_reservations_prevent_same_pass_overpromise() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "reservation_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "reservation_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        40.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        60.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    let local_reserved: f32 = shipments
        .shipments
        .iter()
        .filter(|shipment| {
            shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && shipment.source_building_id == 0
                && shipment.resource_runtime_id == staple_food
        })
        .map(|shipment| shipment.amount)
        .sum();
    assert!(local_reserved <= 300.0 + f32::EPSILON);
    assert_eq!(
        shipments
            .shipments
            .iter()
            .filter(|shipment| {
                shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                    && shipment.source_building_id == 0
                    && shipment.resource_runtime_id == staple_food
            })
            .count(),
        1
    );
}

#[test]
fn deserted_destination_rejects_inbound_delivery() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_delivery_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        100.0,
    ));
    allocator.buildings[0].is_deserted = true;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        resource_runtime_id: staple_food,
        amount: 40.0,
        source_kind: SHIPMENT_SOURCE_OWA,
        source_building_id: usize::MAX,
        source_border_node: 0,
        destination_building_id: 0,
        carrier_class: CARRIER_TRUCK,
        status: SHIPMENT_IN_TRANSIT,
        total_cost: 600.0,
        eta_hours: 1,
    });

    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(staple_food), 0.0);
    assert!((allocator.buildings[0].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn deserted_local_source_releases_reserved_delivery() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_source_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "deserted_source_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        100.0,
    ));
    allocator.buildings[0].is_deserted = true;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        resource_runtime_id: staple_food,
        amount: 40.0,
        source_kind: SHIPMENT_SOURCE_LOCAL,
        source_building_id: 0,
        source_border_node: u32::MAX,
        destination_building_id: 1,
        carrier_class: CARRIER_TRUCK,
        status: SHIPMENT_IN_TRANSIT,
        total_cost: 600.0,
        eta_hours: 1,
    });

    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(staple_food), 300.0);
    assert_eq!(allocator.buildings[1].inventory_units(staple_food), 0.0);
    assert!((allocator.buildings[1].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn owa_imports_respect_border_cap_within_pass() {
    let (graph, network, _industrial_edge, commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "cap_import_commercial",
        ZoneClass::Commercial,
    );
    for idx in 0..5 {
        allocator.buildings.push(make_building(
            &allocator,
            30.0 + idx as f32 * 5.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            0.0,
            20_000.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert_eq!(
        shipments
            .shipments
            .iter()
            .filter(|shipment| {
                shipment.source_kind == SHIPMENT_SOURCE_OWA
                    && shipment.source_border_node == border_node
            })
            .count(),
        4
    );
}

#[test]
fn owa_exports_respect_border_cap_within_pass() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "cap_export_industrial",
        ZoneClass::Industrial,
    );
    for idx in 0..5 {
        allocator.buildings.push(make_building(
            &allocator,
            -70.0 + idx as f32 * 5.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            300.0,
            0.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 480);

    assert_eq!(
        shipments
            .shipments
            .iter()
            .filter(|shipment| {
                shipment.destination_building_id == SHIPMENT_DEST_OWA
                    && shipment.source_border_node == border_node
            })
            .count(),
        4
    );
}

#[test]
fn owa_export_eta_uses_freight_timing_window() {
    let (graph, network, industrial_edge, _commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_store_asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "timed_export_industrial",
        ZoneClass::Industrial,
        Some("grocery_basic"),
    );
    let mut building = make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_store_asset,
        500.0,
        20_000.0,
    );
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    building.add_inventory_units(staple_food, 500.0);
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    shipments.hourly_tick(&mut allocator, &network, &graph, 0);

    let export = shipments
        .shipments
        .iter()
        .find(|shipment| shipment.destination_building_id == SHIPMENT_DEST_OWA)
        .expect("OWA export shipment");
    assert!(export.eta_hours >= 2);
}
