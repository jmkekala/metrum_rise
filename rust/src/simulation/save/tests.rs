use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{BuildingData, LodEntry, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::{ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK, TRANSIT_NETWORK};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::{Household, HouseholdSystem, REPLENISHMENT_STABLE};
use crate::simulation::economy::logistics::{
    CARRIER_TRUCK, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_OWA, Shipment, ShipmentSystem,
};
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use godot::prelude::{Vector2, Vector3};
use rusqlite::Connection;
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "metrum_rise_{name}_{}_{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
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
            anchors: vec![],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: 3,
                lot_depth_cells: 3,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: None,
                preview_scale: None,
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

fn paint_zone_rect(zoning: &mut ZoningSystem, x0: f32, z0: f32, x1: f32, z1: f32, zone: ZoneType) {
    let runtime_id = zoning
        .profiles
        .default_runtime_id_for_zone_type(zone)
        .expect("tests should only paint baseline zones");
    zoning.set_zone_profile_rect(x0, z0, x1, z1, runtime_id);
}

fn zone_at_world(zoning: &ZoningSystem, x: f32, z: f32) -> ZoneType {
    zoning
        .profiles
        .zone_type_for_runtime_id(zoning.get_zone_profile_runtime_id_world(x, z))
}

#[test]
fn sqlite_round_trip_preserves_authoritative_state() {
    let config = WorldConfig::new(100.0, 100.0, 10.0, 10.0);
    let mut time = TimeSystem::new();
    time.speed_multiplier = 2.0;
    time.time_elapsed = 1.25;
    time.day_index = 3;
    time.minute_of_day = 480;
    time.seconds_per_day = 4.0;
    let mut terrain = TerrainSystem::new(config.zone_grid_width(), config.zone_grid_height());
    terrain.source_data.fill(1.0);
    terrain.reset_visuals_from_source();
    let mut water = WaterSystem::new(terrain.width, terrain.height);
    water.depth[0] = 2.5;
    water.velocity[0] = 0.75;
    water.flux[0] = [1.0, 2.0, 3.0, 4.0];
    water.sources.push((1, 2, 0.5));
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_id = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 40.0,
        physical_length: 40.0,
        current_congestion: 0.1,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        deleted: false,
        no_building_spawn: false,
        vehicle_frontage_access: VehicleFrontageAccess::BothSides,
    });
    graph.add_lane_connection(n0, edge_id, 0, edge_id, 0);
    let mut zoning = ZoningSystem::new(&config);
    paint_zone_rect(&mut zoning, -20.0, -15.0, 20.0, 15.0, ZoneType::Residential);
    let mut pollution = PollutionSystem::new(&config);
    pollution.grid.data[0] = 3.0;
    let mut noise = NoiseSystem::new(&config);
    noise.grid.data[0] = 7.0;
    let mut demand = DemandSystem::new();
    demand.residential = 0.12;
    demand.commercial = 0.08;
    demand.industrial = 0.04;
    demand.households_to_admit_today = 2;
    demand.admission_action_credit = 1.25;
    demand.removal_action_credit = 0.50;
    let mut allocator = BuildingAllocator::new();
    let residential_asset = register_test_asset(
        &mut allocator,
        "test",
        "save_residential",
        ZoneClass::Residential,
    );
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_profile_runtime_id: zoning
            .profiles
            .default_runtime_id_for_zone_type(ZoneType::Residential)
            .expect("residential runtime id"),
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        budget_distress: false,
        is_deserted: false,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 2,
        worker_count: 0,
        asset_id: residential_asset,
        level: 1,
        broken: false,
        economy_profile_runtime_id: 0,
        economy_broken: false,
        resource_inventory: {
            let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
            let staple_food = catalog
                .resource_runtime_id_for_id("staple_food")
                .expect("staple food resource");
            let mut inventory = vec![0.0; catalog.resource_count()];
            inventory[staple_food as usize - 1] = 42.0;
            inventory
        },
        revenue: 0.0,
        operating_budget: 500.0,
        shipment_cooldown_hours: 0,
        daily_owa_input_value: 0.0,
        daily_local_input_value: 0.0,
        pending_redevelopment: false,
        rezone_grace_days_remaining: 0,
    });
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .expect("transforms");
    world::repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");
    allocator.rebuild_zone_index();
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 178.0,
        stock: 3.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 1.5,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_hours: 0,
        reserved_store_building_id: 0,
        reserved_amount: 2.5,
        reserved_total_cost: 15.0,
        pickup_eta_hours: 1,
        stay_failure_days: 1,
        replenishment_offset_hours: 0,
        unemployment_days_elapsed: 0,
    });
    let mut logistics = ShipmentSystem::new();
    let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
    let household_supplies = catalog
        .resource_runtime_id_for_id("household_supplies")
        .expect("household supplies resource");
    logistics.shipments.push(Shipment {
        resource_runtime_id: household_supplies,
        amount: 80.0,
        source_kind: SHIPMENT_SOURCE_OWA,
        source_building_id: usize::MAX,
        source_border_node: 0,
        destination_building_id: 0,
        carrier_class: CARRIER_TRUCK,
        status: SHIPMENT_IN_TRANSIT,
        total_cost: 640.0,
        eta_hours: 1,
    });
    let mut network_sys = TransitNetwork::new();
    network_sys.lane_system.rebuild(&mut graph);
    let planned_lane_id = network_sys.lane_system.edge_lanes[&edge_id][0] as u32;
    let mut agents_sys = AgentSystem::new();
    agents_sys.sim_time = 42.0;
    agents::push_loaded_agent(
        &mut agents_sys,
        agents::LoadedAgentRecord {
            home_building: 0,
            household_id: 0,
            work_building: usize::MAX,
            current_building: usize::MAX,
            target_building: 0,
            current_node: n0,
            planned_attach_node: n0,
            planned_detach_node: n1,
            planned_attach_lane_id: planned_lane_id,
            planned_detach_lane_id: planned_lane_id,
            planned_attach_lane_d: 3.5,
            planned_detach_lane_d: 7.5,
            access_flags: ACCESS_PLAN_VALID,
            next_replan_time: 9.5,
            current_edge: edge_id,
            current_lane_id: 0,
            lane_distance: 0.0,
            pos_x: -5.0,
            pos_y: 0.0,
            activity: 1,
            transit: TRANSIT_NETWORK,
            transit_mode: MODE_CAR,
            happiness: 88.0,
            money: 123.0,
            journey_start_time: 12.5,
            schedule_seed: 1,
            cached_commute_minutes: 12,
            next_commute_refresh_time: 24.0,
            has_car: true,
            vehicle_type: 0,
            current_path_index: 1,
            current_path: vec![n0, n1],
            pedestrian_type: 0,
            walk_phase: 0.0,
        },
    );
    agents::push_loaded_agent(
        &mut agents_sys,
        agents::LoadedAgentRecord {
            home_building: 0,
            household_id: 0,
            work_building: usize::MAX,
            current_building: usize::MAX,
            target_building: 0,
            current_node: n1,
            planned_attach_node: u32::MAX,
            planned_detach_node: u32::MAX,
            planned_attach_lane_id: u32::MAX,
            planned_detach_lane_id: u32::MAX,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: 0.0,
            access_flags: 0,
            next_replan_time: 0.0,
            current_edge: usize::MAX,
            current_lane_id: -1,
            lane_distance: 0.0,
            pos_x: 5.0,
            pos_y: 0.0,
            activity: 0,
            transit: TRANSIT_NETWORK,
            transit_mode: MODE_WALK,
            happiness: 77.0,
            money: 55.0,
            journey_start_time: 6.0,
            schedule_seed: 2,
            cached_commute_minutes: 8,
            next_commute_refresh_time: 18.0,
            has_car: false,
            vehicle_type: 0,
            current_path_index: 0,
            current_path: Vec::new(),
            pedestrian_type: 0,
            walk_phase: 0.0,
        },
    );
    let path = temp_path("round_trip");
    save_to_sqlite(
        &path,
        SaveGameView {
            config: &config,
            time: &time,
            terrain: &terrain,
            water: &water,
            graph: &graph,
            zoning: &zoning,
            pollution: &pollution,
            noise: &noise,
            demand: &demand,
            allocator: &allocator,
            households: &households,
            logistics: &logistics,
            agents: &agents_sys,
            network: &network_sys,
            treasury: &CityTreasury::new(0.0),
        },
    )
    .expect("save");
    let loaded = load_from_sqlite(&path, &allocator.registry).expect("load");
    fs::remove_file(&path).ok();

    assert_eq!(loaded.config.width_m, config.width_m);
    assert_eq!(loaded.config.height_m, config.height_m);
    assert_eq!(loaded.config.terrain_chunk_m, config.terrain_chunk_m);
    assert_eq!(
        loaded.config.terrain_base_elevation_m,
        config.terrain_base_elevation_m
    );
    assert_eq!(loaded.time.day_index, time.day_index);
    assert_eq!(loaded.time.minute_of_day, time.minute_of_day);
    assert_eq!(loaded.terrain.source_data, terrain.source_data);
    assert_eq!(loaded.water.depth, water.depth);
    assert_eq!(loaded.demand.residential, demand.residential);
    assert_eq!(loaded.demand.commercial, demand.commercial);
    assert_eq!(loaded.demand.industrial, demand.industrial);
    assert_eq!(
        loaded.demand.households_to_admit_today,
        demand.households_to_admit_today
    );
    assert_eq!(
        loaded.demand.admission_action_credit,
        demand.admission_action_credit
    );
    assert_eq!(
        loaded.demand.removal_action_credit,
        demand.removal_action_credit
    );
    assert_eq!(loaded.pollution.grid.data, pollution.grid.data);
    assert_eq!(loaded.noise.grid.data, noise.grid.data);
    assert_eq!(loaded.graph.edge_count(), 1);
    assert_eq!(
        loaded.graph.edge(0).vehicle_frontage_access,
        VehicleFrontageAccess::BothSides
    );
    assert_eq!(
        zone_at_world(&loaded.zoning, 0.0, 0.0),
        ZoneType::Residential
    );
    assert_eq!(loaded.allocator.buildings.len(), 1);
    assert_eq!(loaded.households.households.len(), 1);
    assert_eq!(
        loaded.households.households[0].reserved_store_building_id,
        0
    );
    assert_eq!(loaded.households.households[0].reserved_amount, 2.5);
    assert_eq!(loaded.households.households[0].reserved_total_cost, 15.0);
    assert_eq!(loaded.households.households[0].pickup_eta_hours, 1);
    assert_eq!(loaded.households.households[0].stay_failure_days, 1);
    assert_eq!(loaded.agents.len(), 2);
    assert_eq!(loaded.agents.current_path[0], vec![0, 1]);
    assert_eq!(loaded.agents.planned_attach_node[0], 0);
    assert_eq!(loaded.agents.planned_detach_node[0], 1);
    assert_eq!(loaded.agents.planned_attach_lane_id[0], planned_lane_id);
    assert_eq!(loaded.agents.planned_detach_lane_id[0], planned_lane_id);
    assert_eq!(loaded.agents.planned_attach_lane_d[0], 3.5);
    assert_eq!(loaded.agents.planned_detach_lane_d[0], 7.5);
    assert_eq!(loaded.agents.access_flags[0], ACCESS_PLAN_VALID);
    assert_eq!(loaded.agents.next_replan_time[0], 9.5);
    assert_eq!(loaded.agents.sim_time, agents_sys.sim_time);
    assert_eq!(loaded.allocator.buildings[0].frontage_t, 0.5);
    let staple_food = catalog
        .resource_runtime_id_for_id("staple_food")
        .expect("staple food resource");
    assert_eq!(
        loaded.allocator.buildings[0].inventory_units(staple_food),
        42.0
    );
    assert_eq!(loaded.logistics.shipments.len(), 1);
    assert_eq!(loaded.logistics.shipments[0].destination_building_id, 0);
}

#[test]
fn load_graph_migrates_missing_vehicle_frontage_access_column_to_bothsides() {
    let conn = Connection::open_in_memory().expect("in-memory sqlite");
    conn.execute_batch(
        r#"
        CREATE TABLE network_nodes(
            node_id INTEGER PRIMARY KEY,
            x REAL NOT NULL,
            y REAL NOT NULL,
            z REAL NOT NULL,
            node_type INTEGER NOT NULL
        );
        CREATE TABLE network_edges(
            edge_id INTEGER PRIMARY KEY,
            start_node INTEGER NOT NULL,
            end_node INTEGER NOT NULL,
            primary_type INTEGER NOT NULL,
            allowed_types INTEGER NOT NULL,
            class INTEGER NOT NULL,
            width REAL NOT NULL,
            fwd_lanes INTEGER NOT NULL,
            bkw_lanes INTEGER NOT NULL,
            speed_limit REAL NOT NULL,
            base_cost REAL NOT NULL,
            physical_length REAL NOT NULL,
            current_congestion REAL NOT NULL,
            start_clip REAL NOT NULL,
            end_clip REAL NOT NULL
        );
        CREATE TABLE network_edge_geometry(
            edge_id INTEGER NOT NULL,
            point_index INTEGER NOT NULL,
            x REAL NOT NULL,
            y REAL NOT NULL,
            z REAL NOT NULL,
            physical INTEGER NOT NULL,
            PRIMARY KEY(edge_id, physical, point_index)
        );
        CREATE TABLE lane_connections(
            node_id INTEGER NOT NULL,
            from_edge INTEGER NOT NULL,
            from_lane INTEGER NOT NULL,
            to_edge INTEGER NOT NULL,
            to_lane INTEGER NOT NULL
        );
        "#,
    )
    .expect("legacy schema");

    conn.execute(
        "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (0, 0.0, 0.0, 0.0, 1)",
        [],
    )
    .expect("node 0");
    conn.execute(
        "INSERT INTO network_nodes(node_id, x, y, z, node_type) VALUES (1, 10.0, 0.0, 0.0, 1)",
        [],
    )
    .expect("node 1");
    conn.execute(
        "INSERT INTO network_edges(edge_id, start_node, end_node, primary_type, allowed_types, class, width, fwd_lanes, bkw_lanes, speed_limit, base_cost, physical_length, current_congestion, start_clip, end_clip)
         VALUES (0, 0, 1, 1, 3, 0, 7.0, 1, 1, 50.0, 10.0, 10.0, 0.0, 0.0, 0.0)",
        [],
    )
    .expect("edge");
    conn.execute(
        "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (0, 0, 0.0, 0.0, 0.0, 0)",
        [],
    )
    .expect("geom 0");
    conn.execute(
        "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (0, 1, 10.0, 0.0, 0.0, 0)",
        [],
    )
    .expect("geom 1");
    conn.execute(
        "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (0, 0, 0.0, 0.0, 0.0, 1)",
        [],
    )
    .expect("phys geom 0");
    conn.execute(
        "INSERT INTO network_edge_geometry(edge_id, point_index, x, y, z, physical) VALUES (0, 1, 10.0, 0.0, 0.0, 1)",
        [],
    )
    .expect("phys geom 1");

    let graph = network::load_graph(&conn).expect("migrated graph");
    assert_eq!(graph.edge_count(), 1);
    assert_eq!(
        graph.edge(0).vehicle_frontage_access,
        VehicleFrontageAccess::BothSides
    );
    assert!(!graph.edge(0).no_building_spawn);
}
