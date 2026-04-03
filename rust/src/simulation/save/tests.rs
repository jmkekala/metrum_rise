use super::*;
use crate::simulation::core::config::MapConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{MODE_CAR, MODE_WALK, TRANSIT_ON_ROAD};
use crate::simulation::economy::demand::DemandSystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::terrain::TerrainSystem;
use crate::simulation::water::WaterSystem;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::AgentSystem;
use godot::prelude::{Vector2, Vector3};
use std::fs;

fn temp_path(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("metrum_rise_{name}_{}_{}.sqlite", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()))
}

#[test]
fn sqlite_round_trip_preserves_authoritative_state() {
    let config = MapConfig::new(100.0, 100.0, 10.0, 10.0);
    let mut time = TimeSystem::new();
    time.speed_multiplier = 2.0; time.time_elapsed = 1.25; time.current_day = 3; time.seconds_per_day = 4.0;
    let mut terrain = TerrainSystem::new(config.zone_grid_width(), config.zone_grid_height());
    terrain.source_data.fill(1.0); terrain.reset_visuals_from_source();
    let mut water = WaterSystem::new(terrain.width, terrain.height);
    water.depth[0] = 2.5; water.velocity[0] = 0.75; water.flux[0] = [1.0, 2.0, 3.0, 4.0]; water.sources.push((1, 2, 0.5));
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-20.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_id = graph.add_edge(Edge {
        start_node: n0, end_node: n1, primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT, class: EdgeClass::Standard,
        width: 7.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 40.0,
        physical_length: 40.0, current_congestion: 0.1, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(-20.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        deleted: false,
    });
    graph.nodes[n0 as usize].lane_connections.insert((edge_id, 0), vec![(edge_id, 0)]);
    let mut zoning = ZoningSystem::new(&config);
    zoning.update_edge_grid_size(edge_id, 40.0);
    zoning.set_zone_range(edge_id, 1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
    let mut pollution = PollutionSystem::new(&config); pollution.grid.data[0] = 3.0;
    let mut noise = NoiseSystem::new(&config); noise.grid.data[0] = 7.0;
    let mut demand = DemandSystem::new(); demand.residential = 12.0; demand.commercial = 8.0; demand.industrial = 4.0;
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(Building {
        center_x: 0.0, center_y: 0.0, width_cells: 3, depth_cells: 3, zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(0.0, 1.0), frontage_t: 0.5, frontage_node: n1, side_offset: 1.0, abandoned_timer: 0,
        edge_idx: edge_id, side: 1, cell_x: 0, cell_y: 0, occupancy: 2, variant: 0,
    });
    allocator.recompute_derived_transforms(&graph, &zoning).expect("transforms");
    world::repaint_building_occupancy(&mut zoning, &allocator).expect("occupancy");
    allocator.rebuild_zone_index();
    let mut agents_sys = AgentSystem::new();
    agents_sys.sim_time = 42.0;
    agents::push_loaded_agent(&mut agents_sys, agents::LoadedAgentRecord {
        home_building: 0, work_building: usize::MAX, current_building: usize::MAX, target_building: 0,
        current_node: n0, target_node: n1, current_edge: edge_id, current_lane_id: 0, lane_distance: 0.0,
        pos_x: -5.0, pos_y: 0.0, is_visible: true, activity: 1, transit: TRANSIT_ON_ROAD, transit_mode: MODE_CAR,
        happiness: 88.0, money: 123.0, journey_start_time: 12.5, has_car: true, vehicle_type: 0,
        current_path_index: 1, current_path: vec![n0, n1], pedestrian_type: 0, walk_phase: 0.0,
    });
    agents::push_loaded_agent(&mut agents_sys, agents::LoadedAgentRecord {
        home_building: 0, work_building: usize::MAX, current_building: usize::MAX, target_building: 0,
        current_node: n1, target_node: n0, current_edge: usize::MAX, current_lane_id: -1, lane_distance: 0.0,
        pos_x: 5.0, pos_y: 0.0, is_visible: true, activity: 0, transit: TRANSIT_ON_ROAD, transit_mode: MODE_WALK,
        happiness: 77.0, money: 55.0, journey_start_time: 6.0, has_car: false, vehicle_type: 0,
        current_path_index: 0, current_path: Vec::new(), pedestrian_type: 0, walk_phase: 0.0,
    });
    let mut network_sys = TransitNetwork::new();
    network_sys.lane_system.rebuild(&mut graph);
    let path = temp_path("round_trip");
    save_to_sqlite(&path, SaveGameView {
        config: &config, time: &time, terrain: &terrain, water: &water, graph: &graph, zoning: &zoning,
        pollution: &pollution, noise: &noise, demand: &demand, allocator: &allocator, agents: &agents_sys, network: &network_sys,
    }).expect("save");
    let loaded = load_from_sqlite(&path).expect("load");
    fs::remove_file(&path).ok();

    assert_eq!(loaded.config.width_m, config.width_m);
    assert_eq!(loaded.time.current_day, time.current_day);
    assert_eq!(loaded.terrain.source_data, terrain.source_data);
    assert_eq!(loaded.water.depth, water.depth);
    assert_eq!(loaded.demand.residential, demand.residential);
    assert_eq!(loaded.pollution.grid.data, pollution.grid.data);
    assert_eq!(loaded.noise.grid.data, noise.grid.data);
    assert_eq!(loaded.graph.edges.len(), 1);
    assert_eq!(loaded.zoning.edge_grids.len(), 1);
    assert_eq!(loaded.allocator.buildings.len(), 1);
    assert_eq!(loaded.agents.len(), 2);
    assert_eq!(loaded.agents.current_path[0], vec![0, 1]);
    assert_eq!(loaded.agents.sim_time, agents_sys.sim_time);
    assert_eq!(loaded.allocator.buildings[0].frontage_node, 1);
}
