// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: support.rs
//  script_path: rust/src/simulation/economy/households/tests/support.rs
//  module_name: support
//  version: 0.1.0
//  description: Shared household test fixtures: buildings, networks,
//           economy runtime ids, and replenishment setup. Resolves runtime
//           ids through the real economy catalog so the fixtures track
//           profile changes automatically instead of carrying hard-coded
//           numbers that drift.
//  kind: test
//  spec: none
//  internal_dependencies: [households, definitions]
//  external_dependencies: []
//  features: [test-fixtures, household, economy-runtime-id, replenishment]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! Shared household, building, network, and replenishment fixtures.

use super::*;

pub(super) fn test_economy_runtime_id(zone_type: ZoneType) -> u16 {
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

pub(super) fn make_building(
    center_x: f32,
    zone_type: ZoneType,
    asset_id: &str,
    stock: f32,
) -> Building {
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
        edge_idx: 0,
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
        economy_profile_runtime_id: runtime_id,
        economy_broken: false,
        resource_inventory,
        revenue: 0.0,
        operating_budget: 500.0,
        profit_tax_budget_baseline: 500.0,
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

pub(super) fn make_foot_only_edge(start_node: u32, end_node: u32) -> Edge {
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 2.0,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(0, 0),
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
        frontage_class: Default::default(),
    }
}

pub(super) fn make_road_edge(start_node: u32, end_node: u32, start_x: f32, end_x: f32) -> Edge {
    let length = (end_x - start_x).abs();
    Edge {
        start_node,
        end_node,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
        speed_limit: 50.0,
        base_cost: length.max(1.0),
        physical_length: length.max(1.0),
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![
            Vector3::new(start_x, 0.0, 0.0),
            Vector3::new(end_x, 0.0, 0.0),
        ],
        physical_geometry: vec![
            Vector3::new(start_x, 0.0, 0.0),
            Vector3::new(end_x, 0.0, 0.0),
        ],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access:
            crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        frontage_class: Default::default(),
    }
}

pub(super) fn simple_work_graph() -> (RegionGraph, TransitNetwork) {
    work_graph_to(300.0)
}

pub(super) fn work_graph_to(end_x: f32) -> (RegionGraph, TransitNetwork) {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(end_x, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(make_road_edge(n0, n1, -100.0, end_x));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    (graph, network)
}

pub(super) fn register_test_asset(
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
                frontage_forward: None,
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
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

pub(super) fn register_test_commercial_asset_with_profile(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    profile_id: &str,
) -> String {
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Commercial".to_owned(),
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
                zone_type: Some(ZoneClass::Commercial),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: None,
                worker_capacity: Some(4),
                service_class: None,
                economy_profile: Some(profile_id.to_owned()),
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

pub(super) fn register_test_residential_asset_with_capacity(
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
                flat_size_m2: Some(80.0),
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(ZoneClass::Residential),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity: Some(household_capacity),
                worker_capacity: None,
                service_class: None,
                economy_profile: None,
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

pub(super) fn register_test_utility_asset(
    allocator: &mut BuildingAllocator,
    pack_id: &str,
    asset_id: &str,
    profile_id: &str,
) -> String {
    let service_class = match profile_id {
        "power_plant_basic" => Some("power".to_owned()),
        "water_plant_basic" => Some("water".to_owned()),
        "wastewater_treatment_basic" => Some("waste".to_owned()),
        _ => None,
    };
    allocator.registry.register(
        pack_id,
        AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test Utility".to_owned(),
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
                worker_capacity: Some(4),
                service_class,
                economy_profile: Some(profile_id.to_owned()),
                extractor: None,
                field: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
        },
        String::new(),
    );
    format!("{pack_id}:{asset_id}")
}

pub(super) fn setup_replenishment_world(
    household: Household,
    home_asset: &str,
    store_asset: &str,
    store_stock: f32,
    store_x: f32,
) -> (
    HouseholdSystem,
    BuildingAllocator,
    AgentSystem,
    TransitNetwork,
    RegionGraph,
) {
    let mut households = HouseholdSystem::new();
    households.households.push(household);

    let (graph, network) = work_graph_to(store_x.max(300.0) + 100.0);
    let mut allocator = BuildingAllocator::new();
    let residential_asset =
        register_test_asset(&mut allocator, "test", home_asset, ZoneClass::Residential);
    let commercial_asset =
        register_test_asset(&mut allocator, "test", store_asset, ZoneClass::Commercial);
    allocator.buildings.push(make_building(
        0.0,
        ZoneType::Residential,
        &residential_asset,
        0.0,
    ));
    allocator.buildings.push(make_building(
        store_x,
        ZoneType::Commercial,
        &commercial_asset,
        store_stock,
    ));
    allocator.rebuild_zone_index();
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let shopper = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.household_id[shopper] = 0;
    agents.current_building[shopper] = 0;
    agents.transit[shopper] = TRANSIT_IN_BUILDING;
    agents.activity[shopper] = 0;
    (households, allocator, agents, network, graph)
}

pub(super) fn arrive_agent_at_building(
    agents: &mut AgentSystem,
    agent_idx: usize,
    building_idx: usize,
    activity: u8,
) {
    agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
    agents.current_building[agent_idx] = building_idx;
    agents.target_building[agent_idx] = usize::MAX;
    agents.planned_target_building[agent_idx] = usize::MAX;
    agents.planned_activity[agent_idx] = 0;
    agents.activity[agent_idx] = activity;
}

pub(super) fn make_household(
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
    let household_supply_resource = household_supply_resource_runtime_id(&catalog);
    let daily_supply_cost =
        member_count.max(1) as f32 * consumption_rate * household_supply_unit_price(&catalog);
    let daily_service_cost = member_count.max(1) as f32
        * demand_sink_cash_cost_per_resident_excluding_resource(
            &catalog,
            household_supply_resource,
        );
    let daily_utility_cost =
        member_count.max(1) as f32 * tuning.households.utility_cost_per_member_per_day;
    let daily_essential_cost = daily_supply_cost + daily_service_cost + daily_utility_cost;
    Household {
        home_building_id,
        budget: reserve_days * daily_essential_cost,
        stock: stock_days * member_count.max(1) as f32 * consumption_rate,
        member_count,
        child_count: 0,
        adult_count: member_count,
        elder_count: 0,
        consumption_rate,
        stock_days,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        replenishment_failure_count: 0,
        reserved_store_building_id: usize::MAX,
        reserved_amount: 0.0,
        reserved_total_cost: 0.0,
        shopping_agent_id: usize::MAX,
        shopping_agent_schedule_seed: 0,
        shopping_timeout_hours_remaining: 0,
        replenishment_search_cursor: 0,
        stay_failure_days: 0,
        unhoused_days_elapsed: 0,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    }
}
