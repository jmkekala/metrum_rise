use super::*;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::network::TransitNetwork;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::config::MapConfig;
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::pathing::cch::CchGraph;
use godot::prelude::{Vector2, Vector3};

fn create_test_edge(n0: u32, n1: u32) -> Edge {
    Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        class: EdgeClass::Standard,
        width: 7.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 1.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        zoning_left: true,
        zoning_right: true,
        deleted: false,
    }
}

fn create_test_building(edge_idx: usize, side: i8, frontage_node: u32) -> Building {
    Building {
        center_x: 0.0,
        center_y: 0.0,
        width: 10,
        depth: 10,
        zone_type: ZoneType::Residential,
        facing_dir: Vector2::new(1.0, 0.0),
        frontage_t: 0.5,
        frontage_node,
        side_offset: 5.0,
        abandoned_timer: 0,
        edge_idx,
        side,
        cell_x: 0,
        cell_y: 0,
        occupancy: 0,
    }
}

#[test]
fn test_agent_departure_sidewalk_selection() {
    // Two nodes and one edge: n0 --(edge 0)--> n1.
    // Building fronts edge 0, frontage_t=0.5 → frontage_node = n1.
    // Agent departs from building (pos = 0,0) toward n1, then walks n1→n0 on road.
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    // frontage_t = 0.5 → end_node = n1
    allocator.buildings.push(create_test_building(edge_idx, 1, n1));
    let mut agents = AgentSystem::new();
    agents.spawn_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
    let a_id = 0;
    agents.transit[a_id] = TRANSIT_DEPARTING;
    agents.transit_mode[a_id] = MODE_WALK;
    agents.current_node[a_id] = n1;
    agents.current_building[a_id] = 0;
    agents.target_building[a_id] = 0;
    // Path: n1 → n0 (the agent departs from frontage_node=n1 and walks to n0)
    agents.current_path[a_id] = vec![n1, n0];
    agents.current_path_index[a_id] = 1;
    // Tick until ON_ROAD or max iterations
    for _ in 0..500 {
        agents.tick(&mut allocator, &network, &mut graph, 0.1);
        if agents.transit[a_id] == TRANSIT_ON_ROAD {
            break;
        }
    }
    assert_eq!(agents.transit[a_id], TRANSIT_ON_ROAD);
    // One extra tick so the ON_ROAD branch can initialise the lane.
    agents.tick(&mut allocator, &network, &mut graph, 0.1);
    let lane_id = agents.current_lane_id[a_id];
    assert!(lane_id != usize::MAX, "Expected a valid lane after reaching the road");
    let lane = &network.lane_system.lanes[lane_id];
    assert_eq!(lane.lane_type, crate::simulation::network::lanes::LaneType::Foot);
}

#[test]
fn test_agent_departure_car_selection() {
    // Two nodes and one edge: n0 --(edge 0)--> n1.
    // Building fronts edge 0, frontage_t=0.5 → frontage_node = n1.
    // Agent departs from building toward n1, then drives n1→n0 on road.
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    // frontage_t = 0.5 → end_node = n1
    allocator.buildings.push(create_test_building(edge_idx, 1, n1));
    let mut agents = AgentSystem::new();
    agents.spawn_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
    let a_id = 0;
    agents.transit[a_id] = TRANSIT_DEPARTING;
    agents.transit_mode[a_id] = MODE_CAR;
    agents.current_node[a_id] = n1;
    agents.current_building[a_id] = 0;
    agents.target_building[a_id] = 0;
    // Path: n1 → n0 (the agent departs from frontage_node=n1 and drives to n0)
    agents.current_path[a_id] = vec![n1, n0];
    agents.current_path_index[a_id] = 1;
    // Tick until ON_ROAD or max iterations
    for _ in 0..500 {
        agents.tick(&mut allocator, &network, &mut graph, 0.1);
        if agents.transit[a_id] == TRANSIT_ON_ROAD {
            break;
        }
    }
    assert_eq!(agents.transit[a_id], TRANSIT_ON_ROAD);
    // One extra tick so the ON_ROAD branch can initialise the lane.
    agents.tick(&mut allocator, &network, &mut graph, 0.1);
    let lane_id = agents.current_lane_id[a_id];
    assert!(lane_id != usize::MAX, "Expected a valid lane after reaching the road");
    let lane = &network.lane_system.lanes[lane_id];
    assert_eq!(lane.lane_type, crate::simulation::network::lanes::LaneType::Vehicle);
}

#[test]
fn test_car_avoids_walkway() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let n2 = g.add_node(Vector3::new(5.0, 0.0, 10.0), NodeType::Junction);
    g.add_edge(Edge {
        start_node: n0, end_node: n1, primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
        width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 5.0, base_cost: 10.0, physical_length: 10.0,
        ..create_test_edge(n0, n1)
    });
    g.add_edge(create_test_edge(n0, n2));
    g.add_edge(create_test_edge(n2, n1));
    let cch = CchGraph::build(&g);
    let (_, _, p) = cch.find_path(n0, n1, usize::MAX, &g, TransitFlags::CAR).expect("Car should find a path");
    assert_eq!(p.len(), 3);
}

#[test]
fn test_pedestrian_prefers_walkway() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let n2 = g.add_node(Vector3::new(5.0, 0.0, 1.0), NodeType::Junction);
    g.add_edge(Edge {
        base_cost: 2.0, ..create_test_edge(n0, n1)
    });
    g.add_edge(Edge {
        primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT, base_cost: 0.5, ..create_test_edge(n0, n2)
    });
    g.add_edge(Edge {
        primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT, base_cost: 0.5, ..create_test_edge(n2, n1)
    });
    let cch = CchGraph::build(&g);
    let (_, _, p) = cch.find_path(n0, n1, usize::MAX, &g, TransitFlags::FOOT).unwrap();
    assert_eq!(p.len(), 3);
}

#[test]
fn test_car_only_from_home_persistence() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n_far = g.add_node(Vector3::new(1000.0, 0.0, 0.0), NodeType::Junction);
    let mut agents = AgentSystem::new();
    let i = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    agents.home_building[i] = 0;
    agents.current_building[i] = 1;
    agents.has_car[i] = true;
    let cch = CchGraph::build(&g);
    let (driving, _path) = agents.decide_transit_mode(i, n_far, &g, &cch);
    assert_eq!(driving, MODE_WALK);
}

#[test]
fn test_agent_fsm_lifecycle() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(create_test_edge(n0, n1));
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(create_test_building(0, 1, n1));
    allocator.buildings.push(create_test_building(0, 1, n1));
    allocator.buildings[1].zone_type = ZoneType::Industrial;
    allocator.rebuild_zone_index();
    let mut agents = AgentSystem::new();
    for _ in 0..10 {
        let i = agents.spawn_agent(0, n0, 0.0, 0.0, n0, 5.0, 10.0);
        agents.home_building[i] = 0;
        agents.work_building[i] = 1;
        agents.current_building[i] = 0; // Start inside home building
        agents.transit[i] = 0;
    }
    let mut transitioned = false;
    for _ in 0..1000 {
        agents.tick(&mut allocator, &network, &mut g, 1.0);
        if agents.transit.iter().any(|&t| t != 0) { transitioned = true; break; }
    }
    assert!(transitioned);
}

#[test]
fn test_vehicle_type_persistence() {
    let mut agents = AgentSystem::new();
    let i0 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let i1 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let i2 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let type2 = agents.vehicle_type[i2];
    let mut allocator = BuildingAllocator::new();
    agents.kill_agent(1, &mut allocator);
    assert_eq!(agents.len(), 2);
    assert_eq!(agents.vehicle_type[1], type2);
}

#[test]
fn test_border_spawn_movement() {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Border);
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(&mut graph, vec![Vector3::ZERO, Vector3::RIGHT * 100.0], 1, 1, false, false, EdgeClass::Standard, &mut zoning, &mut allocator);
    network.cch_graph = CchGraph::build(&graph);
    let mut agents = AgentSystem::new();
    allocator.buildings.push(create_test_building(0, 1, n1));
    let agent_idx = agents.spawn_agent(0, n0, 0.0, 0.0, n1, 100.0, 0.0);
    agents.tick(&mut allocator, &network, &mut graph, 1.0);
    assert!(agents.pos_x[agent_idx] < 100.0);
}

#[test]
fn test_pedestrian_crosses_junction() {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(&mut graph, vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::ZERO], 1, 1, false, false, EdgeClass::Standard, &mut zoning, &mut allocator);
    network.add_road(&mut graph, vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)], 1, 1, false, false, EdgeClass::Standard, &mut zoning, &mut allocator);
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    allocator.buildings.push(create_test_building(0, 1, n1));
    allocator.buildings.push(create_test_building(1, -1, n2));
    allocator.buildings[1].zone_type = ZoneType::Commercial;
    let mut agents = AgentSystem::new();
    let i = agents.spawn_agent(0, n2, 0.0, 0.0, n0, -50.0, 10.0);
    agents.target_building[i] = 1;
    agents.transit[i] = TRANSIT_DEPARTING;
    for _ in 0..5000 {
        agents.tick(&mut allocator, &network, &mut graph, 0.1);
        if agents.transit[i] == 0 { break; }
    }
    assert!(agents.transit[i] == 0);
}
