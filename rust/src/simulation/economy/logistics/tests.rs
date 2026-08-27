// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: tests.rs
//  script_path: rust/src/simulation/economy/logistics/tests.rs
//  module_name: tests
//  version: 0.1.0
//  description: Shipment and freight regression tests. The logistics_tick
//           macro exists because an hourly tick needs an agent system, a
//           treasury balance, a network, and a graph threaded through it;
//           spelling that out per case buried the assertion being made.
//  kind: test
//  spec: none
//  internal_dependencies: [logistics, allocator, network]
//  external_dependencies: [godot]
//  features: [shipments, freight, logistics-tick, carrier]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! Logistics unit tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{
    Anchor, AnchorType, BuildingData, BuildingFieldData, MeshPart, PlacementMode, ZoneClass,
};
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, resolve_building_economy_profile_binding,
};
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};

macro_rules! logistics_tick {
    ($shipments:expr, $allocator:expr, $network:expr, $graph:expr, $minute:expr) => {{
        let mut agents = AgentSystem::new();
        let mut treasury_balance = 0.0;
        $shipments.hourly_tick(
            $allocator,
            &mut agents,
            $network,
            $graph,
            $minute,
            &mut treasury_balance,
        )
    }};
}

fn create_profile_input_shipments_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    minute_of_day: u16,
) {
    let mut treasury_balance = 0.0;
    let mut planning = super::planning::FreightPlanningContext::build(shipments, allocator, graph);
    shipments.create_profile_input_shipments(
        allocator,
        network,
        graph,
        minute_of_day,
        &mut planning,
        &mut treasury_balance,
    );
    planning.finish(shipments);
}

fn create_profile_output_exports_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    minute_of_day: u16,
) {
    let mut planning = super::planning::FreightPlanningContext::build(shipments, allocator, graph);
    shipments.create_profile_output_exports(
        allocator,
        network,
        graph,
        minute_of_day,
        &mut planning,
    );
    planning.finish(shipments);
}

fn progress_shipments_for_test(
    shipments: &mut ShipmentSystem,
    allocator: &mut BuildingAllocator,
    agents: &mut AgentSystem,
    network: &TransitNetwork,
    graph: &RegionGraph,
) {
    let mut treasury_balance = 0.0;
    shipments.progress_shipments(allocator, agents, network, graph, &mut treasury_balance)
}

fn mark_carrier_arrived(shipments: &ShipmentSystem, agents: &mut AgentSystem, shipment_idx: usize) {
    let shipment = &shipments.shipments[shipment_idx];
    mark_carrier_at_endpoint(agents, shipment.carrier_agent_id, shipment.destination);
}

fn mark_carrier_returned(
    shipments: &ShipmentSystem,
    agents: &mut AgentSystem,
    shipment_idx: usize,
) {
    let shipment = &shipments.shipments[shipment_idx];
    mark_carrier_at_endpoint(agents, shipment.carrier_agent_id, shipment.source);
}

fn mark_carrier_at_endpoint(
    agents: &mut AgentSystem,
    carrier_idx: usize,
    endpoint: ShipmentEndpoint,
) {
    assert!(carrier_idx < agents.len(), "shipment carrier should exist");
    match endpoint {
        ShipmentEndpoint::Building(building_idx) => {
            agents.transit[carrier_idx] = TRANSIT_IN_BUILDING;
            agents.current_building[carrier_idx] = building_idx;
            agents.current_lane_id[carrier_idx] = usize::MAX;
            agents.current_path[carrier_idx].clear();
        }
        ShipmentEndpoint::OwaBorder(border_node) => {
            agents.current_node[carrier_idx] = border_node;
            agents.current_lane_id[carrier_idx] = usize::MAX;
            agents.current_path[carrier_idx].clear();
        }
    }
}

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
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            flat_size_m2: None,
            placement_mode: PlacementMode::ZonedPrivate,
            zone_type: Some(zone),
            density: Some("low".to_owned()),
            lot_width_cells: 1,
            lot_depth_cells: 1,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity,
            worker_capacity,
            service_class: None,
            economy_profile,
            extractor: None,
            field: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn register_test_city_service_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    service_class: &str,
    economy_profile: &str,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test Service".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            flat_size_m2: None,
            placement_mode: PlacementMode::Explicit,
            zone_type: None,
            density: None,
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: None,
            worker_capacity: None,
            service_class: Some(service_class.to_owned()),
            economy_profile: Some(economy_profile.to_owned()),
            extractor: None,
            field: None,
        }),
        prop: None,
        vehicle: None,
        character: None,
    };
    allocator
        .registry
        .register(pack_id, manifest, String::new());
    format!("{pack_id}:{asset_id}")
}

fn register_test_field_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
) -> String {
    let manifest = AssetManifest {
        asset_id: asset_id.to_owned(),
        display_name: "Test Field".to_owned(),
        asset_set: None,
        tags: vec![],
        thumbnail: None,
        lods: vec![],
        mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
        anchors: vec![Anchor {
            anchor_type: AnchorType::Entrance,
            name: "main".to_owned(),
            position: [0.0, 0.0, 0.5],
            forward: [0.0, 0.0, 1.0],
            width_m: None,
            length_m: None,
            vehicle_class: None,
        }],
        site_surfaces: vec![],
        building: Some(BuildingData {
            flat_size_m2: None,
            placement_mode: PlacementMode::Explicit,
            zone_type: None,
            density: None,
            lot_width_cells: 2,
            lot_depth_cells: 2,
            frontage_forward: None,
            min_zone_width_cells: None,
            min_zone_depth_cells: None,
            level: 1,
            household_capacity: None,
            worker_capacity: None,
            service_class: None,
            economy_profile: Some("grain_farm_basic".to_owned()),
            extractor: None,
            field: Some(BuildingFieldData {
                resource: "grain".to_owned(),
                area_mode: "player_polygon".to_owned(),
            }),
        }),
        prop: None,
        vehicle: None,
        character: None,
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
        support_height_m: 0.0,
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
        service_funding_override: -1.0,
        asset_id: asset_id.to_owned(),
        level: 1,
        construction_total_hours: 0,
        construction_remaining_hours: 0,
        broken: false,
        economy_profile_runtime_id: economy_binding.runtime_id,
        economy_broken: economy_binding.economy_broken,
        resource_inventory,
        revenue: 0.0,
        operating_budget: budget,
        profit_tax_budget_baseline: budget,
        last_day_profit: 0.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        daily_city_funded_input_cost: 0.0,
        daily_household_sales_value: 0.0,
        daily_power_service_units: 0.0,
        daily_power_served_units: 0.0,
        recent_power_service_units: 0.0,
        recent_power_served_units: 0.0,
        recent_household_sales_value: 0.0,
        commercial_activity_floor_scale: 0.0,
        work_area_scale: 1.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    }
}

fn simple_graph_with_border() -> (RegionGraph, TransitNetwork, usize, usize, u32) {
    graph_with_border_to(100.0)
}

fn graph_with_border_to(end_x: f32) -> (RegionGraph, TransitNetwork, usize, usize, u32) {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Border);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(end_x, 0.0, 0.0), NodeType::Junction);
    let e0 = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
        speed_limit: 20.0,
        base_cost: 5.0,
        physical_length: end_x.max(1.0),
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        frontage_class: Default::default(),
    });
    let e1 = graph.add_edge(Edge {
        start_node: n1,
        end_node: n2,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
        speed_limit: 20.0,
        base_cost: 5.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(end_x, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(end_x, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        frontage_class: Default::default(),
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
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    assert_eq!(
        allocator.buildings[1].inventory_units(household_supplies),
        100.0
    );

    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[1].inventory_units(packaged_food) > 0.0);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);
    assert!(allocator.buildings[0].revenue > 0.0);
    assert!(
        shipments
            .shipments
            .iter()
            .all(|shipment| shipment.resource_runtime_id == packaged_food)
    );

    mark_carrier_returned(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Fulfilled);
    assert_eq!(agents.len(), 0);
}

#[test]
fn dispatched_local_shipment_no_longer_reserves_source_inventory() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "dispatched_reservation_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "dispatched_reservation_commercial",
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
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    let queued_source_reservation = shipments
        .build_reservation_views(catalog.resource_count())
        .reserved_outbound_amount(0, packaged_food);
    assert!(queued_source_reservation > 0.0);

    let mut agents = AgentSystem::new();
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    let reservations = shipments.build_reservation_views(catalog.resource_count());
    assert_eq!(reservations.reserved_outbound_amount(0, packaged_food), 0.0);
    assert!(reservations.has_open_inbound(1, packaged_food));
}

#[test]
fn local_supplier_can_serve_far_reachable_destination() {
    let (graph, network, industrial_edge, commercial_edge, _) = graph_with_border_to(6_500.0);
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_logistics_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "far_logistics_commercial",
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
        6_000.0,
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
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
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
    let mut agents = AgentSystem::new();
    let mut treasury_balance = 0.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &network,
        &graph,
        480,
        &mut treasury_balance,
    );

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border_node)
    );

    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
    mark_carrier_returned(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    shipments
        .shipments
        .retain(|shipment| shipment.status.is_open());
    assert!(shipments.shipments.is_empty());
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    assert!(allocator.buildings[0].inventory_units(packaged_food) > 0.0);
}

#[test]
fn city_service_owa_fuel_import_debits_treasury_not_building_budget() {
    let (graph, network, _industrial_edge, service_edge, border_node) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let power_asset = register_test_city_service_asset(
        &mut allocator,
        "test",
        "municipal_power_with_fuel",
        "power",
        "power_plant_basic",
    );
    let service_building = make_building(
        &allocator,
        50.0,
        ZoneType::None,
        service_edge,
        &power_asset,
        0.0,
        0.0,
    );
    allocator.buildings.push(service_building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let coal = catalog
        .resource_runtime_id_for_id("coal")
        .expect("coal resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut treasury_balance = 1_000.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &network,
        &graph,
        480,
        &mut treasury_balance,
    );

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, coal);
    assert_eq!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    assert!(treasury_balance < 1_000.0);
    assert_eq!(allocator.buildings[0].operating_budget, 0.0);
    assert!(allocator.buildings[0].daily_city_funded_input_cost > 0.0);

    mark_carrier_arrived(&shipments, &mut agents, 0);
    shipments.progress_shipments(
        &mut allocator,
        &mut agents,
        &network,
        &graph,
        &mut treasury_balance,
    );
    assert!(allocator.buildings[0].inventory_units(coal) > 0.0);
    assert!(allocator.buildings[0].daily_owa_input_value > 0.0);
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
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert!(matches!(
        shipments.shipments[0].source,
        ShipmentEndpoint::OwaBorder(_)
    ));
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    let truck_load_units = load_runtime_economy_tuning()
        .expect("runtime economy tuning")
        .logistics
        .truck_load_units;
    assert!(shipments.shipments[0].amount >= grocery.min_shipment_units);
    assert_eq!(shipments.shipments[0].amount % truck_load_units, 0.0);
    let grocery_input = grocery.inputs.first().expect("grocery input port");
    assert!(shipments.shipments[0].amount < grocery.inventory_target_units_for(grocery_input));
    assert!(allocator.buildings[0].operating_budget >= 0.0);
    assert!(allocator.buildings[0].operating_budget < 1_500.0);
}

#[test]
fn industrial_processor_requests_input_imports() {
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
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grain = catalog
        .resource_runtime_id_for_id("grain")
        .expect("grain resource");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, grain);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::Building(0)
    );
    assert_eq!(allocator.buildings[0].inventory_units(grain), 0.0);
}

#[test]
fn explicit_field_producer_supplies_processor_input() {
    let (graph, network, industrial_edge, _commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "grain_field_supplier");
    let processor_asset = register_test_asset(
        &mut allocator,
        "test",
        "grain_processor_destination",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        300.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        -40.0,
        ZoneType::Industrial,
        industrial_edge,
        &processor_asset,
        0.0,
        5_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grain = catalog
        .resource_runtime_id_for_id("grain")
        .expect("grain resource");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].resource_runtime_id, grain);
    assert_eq!(shipments.shipments[0].source, ShipmentEndpoint::Building(0));
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::Building(1)
    );
}

#[test]
fn owa_export_buffer_uses_active_explicit_work_area_output() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "scaled_export_field");
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        120.0,
        0.0,
    ));
    allocator.buildings[0].work_area_scale = 0.25;
    allocator.buildings[0].commercial_activity_floor_scale = 1.0;
    allocator.buildings[0].worker_count = 1;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    assert_eq!(shipments.shipments[0].amount, 80.0);
}

#[test]
fn owa_export_buffer_ignores_output_headroom_throttle() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let field_asset = register_test_field_asset(&mut allocator, "test", "headroom_export_field");
    allocator.buildings.push(make_building(
        &allocator,
        -70.0,
        ZoneType::None,
        industrial_edge,
        &field_asset,
        290.0,
        0.0,
    ));
    allocator.buildings[0].work_area_scale = 0.25;
    allocator.buildings[0].commercial_activity_floor_scale = 1.0;
    allocator.buildings[0].worker_count = 1;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(border_node)
    );
    assert_eq!(shipments.shipments[0].amount, 240.0);
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
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    let local_reserved: f32 = shipments
        .shipments
        .iter()
        .filter(|shipment| {
            shipment.source == ShipmentEndpoint::Building(0)
                && shipment.resource_runtime_id == packaged_food
        })
        .map(|shipment| shipment.amount)
        .sum();
    assert!(local_reserved <= 300.0 + f32::EPSILON);
    assert_eq!(
        shipments
            .shipments
            .iter()
            .filter(|shipment| {
                shipment.source == ShipmentEndpoint::Building(0)
                    && shipment.resource_runtime_id == packaged_food
            })
            .count(),
        2
    );
}

#[test]
fn owa_export_holds_affordable_local_input_need_first() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_priority_export_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "local_priority_export_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        200.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(
        shipments.shipments.is_empty(),
        "industrial surplus should be held for affordable local commercial input demand"
    );
}

#[test]
fn unreachable_local_input_need_does_not_hold_owa_export() {
    let (mut graph, mut network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    graph.edge_mut(commercial_edge).allowed_types = TransitFlags::FOOT;
    graph.rebuild_adjacency_list();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);

    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "unreachable_local_priority_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "unreachable_local_priority_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        200.0,
        0.0,
    ));
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    assert!(matches!(
        shipments.shipments[0].destination,
        ShipmentEndpoint::OwaBorder(_)
    ));
}

#[test]
fn zero_sales_commercial_input_target_uses_starter_load_floor() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "scaled_input_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        20_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let grocery = catalog
        .profile_for_id("grocery_basic")
        .expect("grocery starter profile");
    assert_eq!(shipments.shipments.len(), 1);
    assert_eq!(shipments.shipments[0].amount, grocery.min_shipment_units);
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
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 0.0);
    assert!((allocator.buildings[0].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn under_construction_destination_rejects_inbound_delivery() {
    let (graph, network, _industrial_edge, commercial_edge, _border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "construction_delivery_commercial",
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
    allocator.buildings[0].construction_total_hours = 3;
    allocator.buildings[0].construction_remaining_hours = 2;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::OwaBorder(0),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 0.0);
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
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::Building(0),
        destination: ShipmentEndpoint::Building(1),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::InTransit,
        carrier_agent_id: usize::MAX,
        total_cost: 600.0,
        eta_hours: 1,
        queued_hours: 0,
    });

    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(shipments.shipments.is_empty());
    assert_eq!(allocator.buildings[0].inventory_units(packaged_food), 300.0);
    assert_eq!(allocator.buildings[1].inventory_units(packaged_food), 0.0);
    assert!((allocator.buildings[1].operating_budget - 700.0).abs() < f32::EPSILON);
}

#[test]
fn queued_owa_import_expires_and_refunds_destination() {
    let (graph, network, _industrial_edge, commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "queued_expiry_commercial",
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
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    shipments.shipments.push(Shipment {
        id: 0,
        resource_runtime_id: packaged_food,
        amount: 40.0,
        source: ShipmentEndpoint::OwaBorder(border_node),
        destination: ShipmentEndpoint::Building(0),
        carrier_class: CarrierClass::Truck,
        status: ShipmentStatus::Queued,
        carrier_agent_id: usize::MAX,
        total_cost: 240.0,
        eta_hours: 1,
        queued_hours: tuning.logistics.queued_shipment_expiry_hours - 1,
    });

    let mut agents = AgentSystem::new();
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );

    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Expired);
    assert!((allocator.buildings[0].operating_budget - 340.0).abs() < f32::EPSILON);
    assert_eq!(
        allocator.buildings[0].shipment_cooldown_hours,
        tuning.operational_clock.shipment_retry_cooldown_hours
    );
}

#[test]
fn stuck_in_transit_local_delivery_expires_and_restores_source_stock() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stuck_delivery_industrial",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "stuck_delivery_commercial",
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
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    let total_cost = shipments.shipments[0].total_cost;

    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);
    assert!((allocator.buildings[1].operating_budget - (2_000.0 - total_cost)).abs() < 0.01);

    for _ in 0..6 {
        progress_shipments_for_test(
            &mut shipments,
            &mut allocator,
            &mut agents,
            &network,
            &graph,
        );
    }

    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Expired);
    assert_eq!(agents.len(), 0);
    assert!((allocator.buildings[0].inventory_units(packaged_food) - 300.0).abs() < 0.01);
    assert!((allocator.buildings[1].operating_budget - 2_000.0).abs() < 0.01);
}

#[test]
fn destination_removal_restores_dispatched_local_source_stock() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_destination_source",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_destination_target",
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
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(agents.len(), 1);
    assert!(allocator.buildings[0].inventory_units(packaged_food) < 300.0);

    shipments.invalidate_building(1, &mut allocator, &mut agents);

    assert!(shipments.shipments.is_empty());
    assert_eq!(agents.len(), 0);
    assert!((allocator.buildings[0].inventory_units(packaged_food) - 300.0).abs() < 0.01);
    assert_eq!(
        allocator.buildings[0].shipment_cooldown_hours,
        tuning.operational_clock.shipment_retry_cooldown_hours
    );
}

#[test]
fn destination_removal_does_not_restore_already_settled_returning_cargo() {
    let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_returning_source",
        ZoneClass::Industrial,
    );
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "removed_returning_target",
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
        100.0,
        2_000.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    create_profile_input_shipments_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(shipments.shipments[0].status, ShipmentStatus::Returning);
    let source_inventory_after_sale = allocator.buildings[0].inventory_units(packaged_food);
    assert!(source_inventory_after_sale < 300.0);

    shipments.invalidate_building(1, &mut allocator, &mut agents);

    assert!(shipments.shipments.is_empty());
    assert_eq!(agents.len(), 0);
    assert!(
        (allocator.buildings[0].inventory_units(packaged_food) - source_inventory_after_sale).abs()
            < 0.01
    );
}

#[test]
fn unresolved_input_request_escalates_to_terminal_failure() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "terminal_failure_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let mut shipments = ShipmentSystem::new();
    let ticks = usize::from(tuning.logistics.terminal_failure_attempts)
        * (usize::from(
            tuning
                .operational_clock
                .shipment_retry_cooldown_hours
                .max(1),
        ) + 1);
    for _ in 0..ticks {
        logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);
    }

    let failure = shipments
        .request_failures
        .get(&FreightRequestKey {
            destination_building_id: 0,
            resource_runtime_id: packaged_food,
        })
        .expect("request failure");
    assert!(failure.terminal);
    assert_eq!(failure.failures, tuning.logistics.terminal_failure_attempts);
    assert!(shipments.shipments.is_empty());
}

#[test]
fn terminal_input_request_retries_after_budget_recovers() {
    let (graph, network, _industrial_edge, commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let commercial_asset = register_test_asset(
        &mut allocator,
        "test",
        "terminal_retry_commercial",
        ZoneClass::Commercial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        50.0,
        ZoneType::Commercial,
        commercial_edge,
        &commercial_asset,
        0.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    let request_key = FreightRequestKey {
        destination_building_id: 0,
        resource_runtime_id: packaged_food,
    };
    let mut shipments = ShipmentSystem::new();
    let ticks = usize::from(tuning.logistics.terminal_failure_attempts)
        * (usize::from(
            tuning
                .operational_clock
                .shipment_retry_cooldown_hours
                .max(1),
        ) + 1);
    for _ in 0..ticks {
        logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);
    }
    assert!(
        shipments
            .request_failures
            .get(&request_key)
            .is_some_and(|failure| failure.terminal)
    );

    allocator.buildings[0].operating_budget = 5_000.0;
    allocator.buildings[0].shipment_cooldown_hours = 0;
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    assert!(!shipments.shipments.is_empty());
    assert!(!shipments.request_failures.contains_key(&request_key));
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
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let border_imports: Vec<_> = shipments
        .shipments
        .iter()
        .filter(|shipment| shipment.source == ShipmentEndpoint::OwaBorder(border_node))
        .collect();
    assert_eq!(border_imports.len(), 5);
    assert_eq!(
        border_imports
            .iter()
            .filter(|shipment| shipment.status == ShipmentStatus::InTransit)
            .count(),
        4
    );
    assert_eq!(
        border_imports
            .iter()
            .filter(|shipment| shipment.status == ShipmentStatus::Queued)
            .count(),
        1
    );
}

#[test]
fn owa_exports_respect_border_cap_within_pass() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset_with_profile(
        &mut allocator,
        "test",
        "cap_export_industrial",
        ZoneClass::Industrial,
        Some("grain_farm_basic"),
    );
    for idx in 0..5 {
        allocator.buildings.push(make_building(
            &allocator,
            -70.0 + idx as f32 * 5.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            400.0,
            0.0,
        ));
    }
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    logistics_tick!(&mut shipments, &mut allocator, &network, &graph, 480);

    let border_exports: Vec<_> = shipments
        .shipments
        .iter()
        .filter(|shipment| shipment.destination == ShipmentEndpoint::OwaBorder(border_node))
        .collect();
    assert_eq!(border_exports.len(), 5);
    assert_eq!(
        border_exports
            .iter()
            .filter(|shipment| shipment.status == ShipmentStatus::InTransit)
            .count(),
        4
    );
    assert_eq!(
        border_exports
            .iter()
            .filter(|shipment| shipment.status == ShipmentStatus::Queued)
            .count(),
        1
    );
}

#[test]
fn repeated_owa_exports_saturate_export_revenue() {
    let (graph, network, industrial_edge, _commercial_edge, _) = simple_graph_with_border();
    let mut allocator = BuildingAllocator::new();
    let industrial_asset = register_test_asset(
        &mut allocator,
        "test",
        "saturated_export_industrial",
        ZoneClass::Industrial,
    );
    allocator.buildings.push(make_building(
        &allocator,
        -50.0,
        ZoneType::Industrial,
        industrial_edge,
        &industrial_asset,
        500.0,
        0.0,
    ));
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);
    assert_eq!(shipments.shipments.len(), 1);
    let first_unit_revenue = shipments.shipments[0].total_cost / shipments.shipments[0].amount;
    assert!(
        shipments
            .owa_export_saturation_units()
            .iter()
            .all(|units| *units == 0.0)
    );

    let resource_runtime_id = shipments.shipments[0].resource_runtime_id;
    let mut agents = AgentSystem::new();
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    mark_carrier_arrived(&shipments, &mut agents, 0);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert!(
        shipments
            .owa_export_saturation_units()
            .iter()
            .any(|units| *units > 0.0)
    );

    shipments.shipments.clear();
    allocator.buildings[0].shipment_cooldown_hours = 0;
    allocator.buildings[0].add_inventory_units(resource_runtime_id, 500.0);
    create_profile_output_exports_for_test(&mut shipments, &mut allocator, &network, &graph, 480);

    assert_eq!(shipments.shipments.len(), 1);
    let second_unit_revenue = shipments.shipments[0].total_cost / shipments.shipments[0].amount;
    assert!(second_unit_revenue < first_unit_revenue);
}

#[test]
fn owa_export_eta_uses_freight_timing_window() {
    let (graph, network, industrial_edge, _commercial_edge, border_node) =
        simple_graph_with_border();
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
    let packaged_food = catalog
        .resource_runtime_id_for_id("packaged_food")
        .expect("packaged food resource");
    building.add_inventory_units(packaged_food, 500.0);
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    allocator.rebuild_zone_index();

    let mut shipments = ShipmentSystem::new();
    let mut agents = AgentSystem::new();
    let mut treasury_balance = 0.0;
    shipments.hourly_tick(
        &mut allocator,
        &mut agents,
        &network,
        &graph,
        0,
        &mut treasury_balance,
    );

    let export_idx = shipments
        .shipments
        .iter()
        .position(|shipment| shipment.destination == ShipmentEndpoint::OwaBorder(border_node))
        .expect("OWA export shipment");
    assert!(shipments.shipments[export_idx].eta_hours >= 2);

    mark_carrier_arrived(&shipments, &mut agents, export_idx);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(
        shipments.shipments[export_idx].status,
        ShipmentStatus::Returning
    );
    assert!(allocator.buildings[0].revenue > 0.0);

    mark_carrier_returned(&shipments, &mut agents, export_idx);
    progress_shipments_for_test(
        &mut shipments,
        &mut allocator,
        &mut agents,
        &network,
        &graph,
    );
    assert_eq!(
        shipments.shipments[export_idx].status,
        ShipmentStatus::Fulfilled
    );
    assert_eq!(agents.len(), 0);
}
