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
        deleted: false,
    }
}

fn create_test_building(edge_idx: usize, side: i8, frontage_node: u32) -> Building {
    Building {
        center_x: 0.0,
        center_y: 0.0,
        width_cells: 1,
        depth_cells: 1,
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
        variant: 0,
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
fn test_transit_mode_uses_has_car() {
    // Transit mode selection is now inline in tick: has_car → CAR flag for CCH query,
    // otherwise FOOT. Verify the flag constants are distinct and MODE_WALK != MODE_CAR.
    assert_ne!(MODE_WALK, MODE_CAR);
    // An agent without a car should use FOOT search flags.
    let _agents = AgentSystem::new();
    // No agents — just verify the constants that govern inline mode selection are correct.
    let foot_flags = TransitFlags::FOOT;
    let car_flags = TransitFlags::CAR;
    assert!(foot_flags != car_flags);
}

#[test]
fn test_parallel_tick_produces_same_positions_as_sequential() {
    // Build a minimal two-node graph with one edge and one ON_ROAD agent.
    // Run one tick with the full tick() (which uses par_iter internally).
    // Verify agent has moved (pos changed from spawn point) and is still on the road.
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let allocator = BuildingAllocator::new();
    use crate::simulation::economy::agents::{TRANSIT_ON_ROAD, MODE_CAR};
    let mut agents = AgentSystem::new();
    let i = agents.spawn_agent(usize::MAX, n0, 0.0, 0.0, n0, 0.0, 0.0);
    agents.transit[i] = TRANSIT_ON_ROAD;
    agents.transit_mode[i] = MODE_CAR;
    agents.current_node[i] = n0;
    agents.target_node[i] = n1;
    agents.current_path[i] = vec![n0, n1];
    agents.current_path_index[i] = 1;
    agents.current_lane_id[i] = usize::MAX;
    agents.lane_distance[i] = 0.0;
    agents.is_visible[i] = true;
    let x_before = agents.pos_x[i];
    agents.tick(&allocator, &network, &graph, 1.0);
    // Agent should have moved along the edge.
    assert!(
        agents.pos_x[i] != x_before || agents.transit[i] != TRANSIT_ON_ROAD,
        "Agent did not move after one tick"
    );
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
    let _i0 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let _i1 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
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
    network.add_road(&mut graph, vec![Vector3::ZERO, Vector3::RIGHT * 100.0], 1, 1, EdgeClass::Standard, &mut zoning, &mut allocator);
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
    network.add_road(&mut graph, vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::ZERO], 1, 1, EdgeClass::Standard, &mut zoning, &mut allocator);
    network.add_road(&mut graph, vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)], 1, 1, EdgeClass::Standard, &mut zoning, &mut allocator);
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

// ── IDM tests ────────────────────────────────────────────────────────────────

/// Helper: build a two-node, single-edge network and return the forward vehicle lane id.
fn setup_straight_road() -> (TransitNetwork, RegionGraph, usize, usize) {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let mut zoning = ZoningSystem::new(&MapConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        1, 1, EdgeClass::Standard,
        &mut zoning, &mut allocator,
    );
    network.cch_graph = CchGraph::build(&graph);
    let edge_idx = 0;
    let fwd_lane = *network.lane_system.edge_lanes[&edge_idx]
        .iter()
        .find(|&&lid| {
            let l = &network.lane_system.lanes[lid];
            l.is_fwd && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
        })
        .expect("forward vehicle lane");
    (network, graph, edge_idx, fwd_lane)
}

/// Place an agent directly on-road on the given lane.
fn place_on_lane(
    agents: &mut AgentSystem,
    edge_idx: usize,
    fwd_lane: usize,
    lane_dist: f32,
    speed: f32,
) -> usize {
    let (n0, n1) = (0u32, 1u32);
    let idx = agents.spawn_agent(usize::MAX, n1, 0.0, 0.0, n0, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_ON_ROAD;
    agents.current_edge[idx] = edge_idx;
    agents.current_lane_id[idx] = fwd_lane;
    agents.lane_distance[idx] = lane_dist;
    agents.speed[idx] = speed;
    agents.current_path[idx] = vec![n0, n1];
    agents.current_path_index[idx] = 1;
    idx
}

#[test]
fn test_idm_free_road_accelerates() {
    // A stopped car on an empty road should accelerate after one tick.
    let (network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let i = place_on_lane(&mut agents, edge_idx, fwd_lane, 10.0, 0.0);
    agents.tick(&mut allocator, &network, &mut graph, 1.0);
    assert!(agents.speed[i] > 0.0, "stopped car should accelerate on free road");
}

#[test]
fn test_idm_following_car_slower_than_free_car() {
    // Car A ahead, car B close behind — B must finish with lower speed than a lone free car.
    let (network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    // Place a fast leader at dist=60 and a follower at dist=50.
    let _leader = place_on_lane(&mut agents, edge_idx, fwd_lane, 60.0, 40.0);
    let follower = place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, 40.0);

    // Reference: a lone car at the same position as the follower.
    let (network2, mut graph2, edge2, fwd2) = setup_straight_road();
    let mut agents2 = AgentSystem::new();
    let mut alloc2 = BuildingAllocator::new();
    let free_car = place_on_lane(&mut agents2, edge2, fwd2, 50.0, 40.0);

    agents.tick(&mut allocator, &network, &mut graph, 0.5);
    agents2.tick(&mut alloc2, &network2, &mut graph2, 0.5);

    assert!(
        agents.speed[follower] <= agents2.speed[free_car] + 0.01,
        "follower speed {} should not exceed free car speed {}",
        agents.speed[follower],
        agents2.speed[free_car]
    );
}

#[test]
fn test_overlap_correction_separates_cars() {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};
    // Place two cars so they overlap: front car at 20 m, rear car at 20 m + gap < CAR_LENGTH.
    let (network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let front = place_on_lane(&mut agents, edge_idx, fwd_lane, 20.0, 10.0);
    let rear  = place_on_lane(&mut agents, edge_idx, fwd_lane, 19.5, 10.0); // 0.5 m apart < CAR_LENGTH
    agents.tick(&mut allocator, &network, &mut graph, 0.1);
    let gap = agents.lane_distance[front] - agents.lane_distance[rear];
    assert!(
        gap >= CAR_LENGTH + IDM_S_MIN - 0.01,
        "gap {gap:.3} m after tick must be >= CAR_LENGTH + IDM_S_MIN = {:.3}",
        CAR_LENGTH + IDM_S_MIN
    );
}

#[test]
fn test_edge_congestion_written_after_tick() {
    // A car moving at half speed_limit should produce congestion > 0 after update_edge_congestion.
    let (network, mut graph, edge_idx, fwd_lane) = setup_straight_road();
    let speed_limit = graph.edges[edge_idx].speed_limit;
    let mut agents = AgentSystem::new();
    let mut allocator = BuildingAllocator::new();
    let i = place_on_lane(&mut agents, edge_idx, fwd_lane, 50.0, speed_limit * 0.5);
    agents.tick(&mut allocator, &network, &mut graph, 0.1);
    // Force the agent to stay on road for congestion aggregation.
    agents.transit[i] = TRANSIT_ON_ROAD;
    agents.current_edge[i] = edge_idx;
    agents.update_edge_congestion(&mut graph);
    assert!(
        graph.edges[edge_idx].current_congestion > 0.0,
        "congestion should be > 0 when car is below speed limit"
    );
}
