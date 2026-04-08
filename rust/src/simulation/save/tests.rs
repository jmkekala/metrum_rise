use super::*;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::{ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK, TRANSIT_NETWORK};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::economy::households::{Household, HouseholdSystem, REPLENISHMENT_STABLE};
use crate::simulation::economy::logistics::{
    CARRIER_TRUCK, RESOURCE_HOUSEHOLD_SUPPLIES, SHIPMENT_IN_TRANSIT, SHIPMENT_SOURCE_OWA, Shipment,
    ShipmentSystem,
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

#[test]
fn sqlite_round_trip_preserves_authoritative_state() {
    let config = MapConfig::new(100.0, 100.0, 10.0, 10.0);
    let mut time = TimeSystem::new();
    time.speed_multiplier = 2.0;
    time.time_elapsed = 1.25;
    time.current_day = 3;
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
    zoning.set_zone_rect(-20.0, -15.0, 20.0, 15.0, ZoneType::Residential);
    let mut pollution = PollutionSystem::new(&config);
    pollution.grid.data[0] = 3.0;
    let mut noise = NoiseSystem::new(&config);
    noise.grid.data[0] = 7.0;
    let mut demand = DemandSystem::new();
    demand.residential = 12.0;
    demand.commercial = 8.0;
    demand.industrial = 4.0;
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(Building {
        center_x: 0.0,
        center_y: 0.0,
        width_cells: 3,
        depth_cells: 3,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0),
        frontage_t: 0.5,
        side_offset: 1.0,
        abandoned_timer: 0,
        edge_idx: edge_id,
        side: 1,
        cell_x: 0,
        cell_y: 0,
        occupancy: 2,
        worker_count: 0,
        asset_id: String::new(),
        level: 1,
        broken: false,
        stock: 0.0,
        revenue: 0.0,
        operating_budget: 500.0,
        utility_service_available: false,
        shipment_cooldown_days: 0,
    });
    allocator
        .recompute_derived_transforms(&graph, &zoning)
        .expect("transforms");
    world::repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");
    allocator.rebuild_zone_index();
    allocator.founding_bootstrap_consumed = true;
    let mut households = HouseholdSystem::new();
    households.households.push(Household {
        home_building_id: 0,
        budget: 178.0,
        stock: 3.0,
        member_count: 2,
        consumption_rate: 1.0,
        stock_days: 1.5,
        replenishment_state: REPLENISHMENT_STABLE,
        cooldown_days: 0,
        reserved_store_building_id: 0,
        reserved_amount: 2.5,
        reserved_total_cost: 15.0,
        pickup_eta_days: 1,
    });
    let mut logistics = ShipmentSystem::new();
    logistics.shipments.push(Shipment {
        resource_type: RESOURCE_HOUSEHOLD_SUPPLIES,
        amount: 80.0,
        source_kind: SHIPMENT_SOURCE_OWA,
        source_building_id: usize::MAX,
        source_border_node: 0,
        destination_building_id: 0,
        carrier_class: CARRIER_TRUCK,
        status: SHIPMENT_IN_TRANSIT,
        total_cost: 640.0,
        eta_days: 1,
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
        },
    )
    .expect("save");
    let loaded = load_from_sqlite(&path, &allocator.registry).expect("load");
    fs::remove_file(&path).ok();

    assert_eq!(loaded.config.width_m, config.width_m);
    assert_eq!(loaded.time.current_day, time.current_day);
    assert_eq!(loaded.terrain.source_data, terrain.source_data);
    assert_eq!(loaded.water.depth, water.depth);
    assert_eq!(loaded.demand.residential, demand.residential);
    assert_eq!(loaded.pollution.grid.data, pollution.grid.data);
    assert_eq!(loaded.noise.grid.data, noise.grid.data);
    assert_eq!(loaded.graph.edge_count(), 1);
    assert_eq!(
        loaded.graph.edge(0).vehicle_frontage_access,
        VehicleFrontageAccess::BothSides
    );
    assert_eq!(
        loaded.zoning.get_zone_world(0.0, 0.0),
        ZoneType::Residential
    );
    assert_eq!(loaded.allocator.buildings.len(), 1);
    assert!(loaded.allocator.founding_bootstrap_consumed);
    assert_eq!(loaded.households.households.len(), 1);
    assert_eq!(
        loaded.households.households[0].reserved_store_building_id,
        0
    );
    assert_eq!(loaded.households.households[0].reserved_amount, 2.5);
    assert_eq!(loaded.households.households[0].reserved_total_cost, 15.0);
    assert_eq!(loaded.households.households[0].pickup_eta_days, 1);
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
