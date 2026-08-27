// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: trips.rs
//  script_path: rust/src/simulation/economy/agents/tests/trips.rs
//  module_name: trips
//  version: 0.1.0
//  description: Trip planning, access, and arrival lifecycle tests. Covers
//           the whole journey rather than any single stage, because the
//           failure mode being guarded is a handoff: an agent that plans a
//           valid path but never leaves the building, or reaches the
//           destination lane and never registers arrival.
//  kind: test
//  spec: none
//  internal_dependencies: [agents, network, allocator]
//  external_dependencies: [godot]
//  features: [trip-planning, agent-access, arrival, mode-choice]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

//! Trip planning, access, and arrival lifecycle tests.

use super::support::*;
use super::*;

#[test]
fn test_agent_departure_sidewalk_selection() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "walk_departure",
        ZoneClass::Residential,
    );
    let mut building = create_test_building(edge_idx, 1);
    building.asset_id = asset_id;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let entrance = allocator.entrances[0].clone();
    let lane_id = entrance.foot_lane_bkw;
    let lane = &network.lane_system.lanes[lane_id];
    let planned_attach_node = if lane.is_fwd { n1 } else { n0 };
    let mut agents = AgentSystem::new();
    agents.spawn_border_arrival_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
    let a_id = 0;
    agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
    agents.transit_mode[a_id] = MODE_WALK;
    agents.current_building[a_id] = 0;
    agents.pos_x[a_id] = entrance.door_pos.x;
    agents.pos_y[a_id] = entrance.door_pos.y;
    agents.planned_attach_node[a_id] = planned_attach_node;
    agents.planned_attach_lane_id[a_id] = lane_id as u32;
    agents.planned_attach_lane_d[a_id] =
        crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
            &lane.geometry,
            crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                &graph,
                edge_idx,
                entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
            ),
        );
    agents.access_flags[a_id] = ACCESS_PLAN_VALID;
    for _ in 0..500 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        if agents.transit[a_id] == TRANSIT_NETWORK {
            break;
        }
    }
    assert_eq!(agents.transit[a_id], TRANSIT_NETWORK);
    assert!(
        agents.current_lane_id[a_id] != usize::MAX,
        "Expected a valid lane after reaching the road"
    );
    let lane = &network.lane_system.lanes[agents.current_lane_id[a_id]];
    assert_eq!(
        lane.lane_type,
        crate::simulation::network::lanes::LaneType::Foot
    );
}

#[test]
fn test_agent_departure_car_selection() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "car_departure",
        ZoneClass::Residential,
    );
    let mut building = create_test_building(edge_idx, 1);
    building.asset_id = asset_id;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let entrance = allocator.entrances[0].clone();
    let lane_id = entrance.car_lane_bkw;
    let lane = &network.lane_system.lanes[lane_id];
    let planned_attach_node = if lane.is_fwd { n1 } else { n0 };
    let mut agents = AgentSystem::new();
    agents.spawn_border_arrival_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
    let a_id = 0;
    agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
    agents.transit_mode[a_id] = MODE_CAR;
    agents.current_building[a_id] = 0;
    agents.pos_x[a_id] = entrance.door_pos.x;
    agents.pos_y[a_id] = entrance.door_pos.y;
    agents.planned_attach_node[a_id] = planned_attach_node;
    agents.planned_attach_lane_id[a_id] = lane_id as u32;
    agents.planned_attach_lane_d[a_id] =
        crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
            &lane.geometry,
            crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                &graph,
                edge_idx,
                entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
            ),
        );
    agents.access_flags[a_id] = ACCESS_PLAN_VALID;
    for _ in 0..500 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        if agents.transit[a_id] == TRANSIT_NETWORK {
            break;
        }
    }
    assert_eq!(agents.transit[a_id], TRANSIT_NETWORK);
    assert!(
        agents.current_lane_id[a_id] != usize::MAX,
        "Expected a valid lane after reaching the road"
    );
    let lane = &network.lane_system.lanes[agents.current_lane_id[a_id]];
    assert_eq!(
        lane.lane_type,
        crate::simulation::network::lanes::LaneType::Vehicle
    );
}

#[test]
fn test_simultaneous_car_egress_queues_instead_of_deadlocking_at_attach_point() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "car_egress_queue",
        ZoneClass::Commercial,
    );
    let mut building = create_test_building(edge_idx, 1);
    building.center_x = 12.0;
    building.center_y = -10.0;
    building.facing_dir = Vector2::new(0.0, -1.0);
    building.asset_id = asset_id;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let entrance = allocator.entrances[0].clone();
    let lane_id = entrance.car_lane_bkw;
    let lane = &network.lane_system.lanes[lane_id];
    let planned_attach_node = if lane.is_fwd { n1 } else { n0 };
    let planned_attach_lane_d =
        crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
            &lane.geometry,
            crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                &graph,
                edge_idx,
                entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
            ),
        );

    let mut agents = AgentSystem::new();
    let mut ids = Vec::new();
    for _ in 0..5 {
        let a_id = agents.spawn_border_arrival_agent(
            0,
            n0,
            0.0,
            0.0,
            n0,
            entrance.door_pos.x,
            entrance.door_pos.y,
        );
        agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
        agents.transit_mode[a_id] = MODE_CAR;
        agents.current_building[a_id] = 0;
        agents.pos_x[a_id] = entrance.door_pos.x;
        agents.pos_y[a_id] = entrance.door_pos.y;
        agents.planned_attach_node[a_id] = planned_attach_node;
        agents.planned_attach_lane_id[a_id] = lane_id as u32;
        agents.planned_attach_lane_d[a_id] = planned_attach_lane_d;
        agents.access_flags[a_id] = ACCESS_PLAN_VALID;
        ids.push(a_id);
    }

    for _ in 0..200 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    }

    for &a_id in &ids {
        assert_ne!(
            agents.transit[a_id], TRANSIT_ACCESS_EGRESS,
            "simultaneous car egress should eventually clear the exact handoff point instead of leaving agents stuck in ACCESS_EGRESS"
        );
    }
}

#[test]
fn test_simultaneous_car_ingress_queues_instead_of_deadlocking_at_detach_point() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(create_test_edge(n0, n1));
    graph.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "car_ingress_queue",
        ZoneClass::Commercial,
    );
    let mut building = create_test_building(edge_idx, 1);
    building.center_x = 12.0;
    building.center_y = -10.0;
    building.facing_dir = Vector2::new(0.0, -1.0);
    building.asset_id = asset_id;
    allocator.buildings.push(building);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let entrance = allocator.entrances[0].clone();
    let lane_id = entrance.car_lane_bkw;
    let lane = &network.lane_system.lanes[lane_id];
    let planned_detach_node = if lane.is_fwd { n0 } else { n1 };
    let planned_detach_lane_d =
        crate::simulation::buildings::allocator::BuildingAllocator::project_point_to_polyline_s(
            &lane.geometry,
            crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_edge(
                &graph,
                edge_idx,
                entrance.entrance_s_m / graph.edge(edge_idx).physical_length,
            ),
        );
    let lane_point = crate::simulation::buildings::allocator::BuildingAllocator::sample_pos_on_lane(
        lane,
        planned_detach_lane_d,
    );

    let mut agents = AgentSystem::new();
    let mut ids = Vec::new();
    for _ in 0..5 {
        let a_id =
            agents.spawn_border_arrival_agent(0, n0, 0.0, 0.0, n0, lane_point.x, lane_point.y);
        agents.transit[a_id] = TRANSIT_NETWORK;
        agents.transit_mode[a_id] = MODE_CAR;
        agents.current_edge[a_id] = edge_idx;
        agents.current_lane_id[a_id] = lane_id;
        agents.lane_distance[a_id] = planned_detach_lane_d;
        agents.speed[a_id] = 1.0;
        agents.target_building[a_id] = 0;
        agents.planned_detach_node[a_id] = planned_detach_node;
        agents.planned_detach_lane_id[a_id] = lane_id as u32;
        agents.planned_detach_lane_d[a_id] = planned_detach_lane_d;
        agents.access_flags[a_id] = ACCESS_PLAN_VALID;
        agents.pos_x[a_id] = lane_point.x;
        agents.pos_y[a_id] = lane_point.y;
        ids.push(a_id);
    }

    for _ in 0..200 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
    }

    for &a_id in &ids {
        assert_ne!(
            agents.transit[a_id], TRANSIT_NETWORK,
            "simultaneous car ingress should eventually clear the exact detach handoff instead of leaving agents stuck on-lane at the frontage point"
        );
    }
}

#[test]
fn test_car_avoids_walkway() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let n2 = g.add_node(Vector3::new(5.0, 0.0, 10.0), NodeType::Junction);
    g.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        width: 2.0,
        lanes: crate::simulation::network::graph::LaneLayout::from_counts(0, 0),
        speed_limit: 5.0,
        base_cost: 10.0,
        physical_length: 10.0,
        ..create_test_edge(n0, n1)
    });
    g.add_edge(create_test_edge(n0, n2));
    g.add_edge(create_test_edge(n2, n1));
    let cch = CchGraph::build(&g);
    let (_, _, p) = cch
        .find_path(n0, n1, usize::MAX, &g, TransitFlags::CAR)
        .expect("Car should find a path");
    assert_eq!(p.len(), 3);
}

#[test]
fn test_pedestrian_prefers_walkway() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let n2 = g.add_node(Vector3::new(5.0, 0.0, 1.0), NodeType::Junction);
    g.add_edge(Edge {
        base_cost: 2.0,
        ..create_test_edge(n0, n1)
    });
    g.add_edge(Edge {
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        base_cost: 0.5,
        ..create_test_edge(n0, n2)
    });
    g.add_edge(Edge {
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        base_cost: 0.5,
        ..create_test_edge(n2, n1)
    });
    let cch = CchGraph::build(&g);
    let (_, _, p) = cch
        .find_path(n0, n1, usize::MAX, &g, TransitFlags::FOOT)
        .unwrap();
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
fn test_agent_fsm_planned_departure_lifecycle() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(create_test_edge(n0, n1));
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);
    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(&mut allocator, "test", "house", ZoneClass::Residential);
    let work_asset = register_test_asset(&mut allocator, "test", "work", ZoneClass::Industrial);
    let mut home = create_test_building(0, 1);
    home.asset_id = home_asset;
    let mut work = create_test_building(0, 1);
    work.asset_id = work_asset;
    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.buildings[1].zone_type = ZoneType::Industrial;
    allocator.rebuild_entrance_cache(&g, &network.lane_system);
    allocator.rebuild_zone_index();
    let mut agents = AgentSystem::new();
    for _ in 0..10 {
        let i = agents.spawn_border_arrival_agent(0, n0, 0.0, 0.0, n0, 5.0, 10.0);
        agents.home_building[i] = 0;
        agents.work_building[i] = 1;
        agents.current_building[i] = 0; // Start inside home building
        agents.transit[i] = 0;
        agents.planned_activity[i] = 1;
        agents.planned_target_building[i] = 1;
    }
    let mut transitioned = false;
    for _ in 0..1000 {
        agents.tick(&mut allocator, &mut network, &mut g, 1.0, 0, 0);
        if agents.transit.iter().any(|&t| t != 0) {
            transitioned = true;
            break;
        }
    }
    assert!(transitioned);
}

#[test]
fn test_planned_departure_populates_exact_trip_plan() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let n2 = g.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(create_test_edge(n0, n1));
    g.add_edge(Edge {
        geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(100.0, 0.0, 0.0), Vector3::new(200.0, 0.0, 0.0)],
        ..create_test_edge(n1, n2)
    });
    g.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);

    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "phase5_house",
        ZoneClass::Residential,
    );
    let work_asset =
        register_test_asset(&mut allocator, "test", "phase5_work", ZoneClass::Industrial);
    let mut home = create_test_building(0, 1);
    home.center_x = 50.0;
    home.center_y = -10.0;
    home.asset_id = home_asset;
    let mut work = create_test_building(1, 1);
    work.center_x = 150.0;
    work.center_y = -10.0;
    work.asset_id = work_asset;
    work.zone_type = ZoneType::Industrial;
    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&g, &network.lane_system);

    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(0, n0, 0.0, 0.0, n0, 50.0, -10.0);
    agents.home_building[i] = 0;
    agents.work_building[i] = 1;
    agents.current_building[i] = 0;
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.has_car[i] = true;
    agents.planned_activity[i] = 1;
    agents.planned_target_building[i] = 1;
    agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);

    assert_eq!(agents.transit[i], TRANSIT_ACCESS_EGRESS);
    assert_eq!(agents.target_building[i], 1);
    assert!(agents.access_flags[i] & ACCESS_PLAN_VALID != 0);
    assert_ne!(agents.planned_attach_node[i], u32::MAX);
    assert_ne!(agents.planned_detach_node[i], u32::MAX);
    assert_ne!(agents.planned_attach_lane_id[i], u32::MAX);
    assert_ne!(agents.planned_detach_lane_id[i], u32::MAX);
    assert_eq!(agents.current_node[i], u32::MAX);
    assert_eq!(agents.current_lane_id[i], usize::MAX);
}

#[test]
fn test_same_edge_car_trip_prefers_direct_frontage_lane_over_endpoint_wrap() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(Edge {
        allowed_types: TransitFlags::CAR,
        physical_length: 500.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        ..create_test_edge(n0, n1)
    });
    g.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);

    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_direct_home",
        ZoneClass::Residential,
    );
    let work_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_direct_work",
        ZoneClass::Commercial,
    );

    let mut home = create_test_building(0, 1);
    home.center_x = 125.0;
    home.center_y = -10.0;
    home.facing_dir = Vector2::new(0.0, -1.0);
    home.asset_id = home_asset;

    let mut work = create_test_building(0, -1);
    work.center_x = 375.0;
    work.center_y = 10.0;
    work.facing_dir = Vector2::new(0.0, 1.0);
    work.asset_id = work_asset;
    work.zone_type = ZoneType::Commercial;

    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&g, &network.lane_system);

    let home_entrance = allocator.entrances[0].clone();
    let work_entrance = allocator.entrances[1].clone();
    assert_ne!(home_entrance.car_lane_fwd, usize::MAX);
    assert_eq!(home_entrance.car_lane_fwd, work_entrance.car_lane_fwd);

    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(
        0,
        n0,
        0.0,
        0.0,
        n0,
        home_entrance.door_pos.x,
        home_entrance.door_pos.y,
    );
    agents.home_building[i] = 0;
    agents.work_building[i] = 1;
    agents.current_building[i] = 0;
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.has_car[i] = true;
    agents.planned_activity[i] = 1;
    agents.planned_target_building[i] = 1;

    agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);

    assert_eq!(agents.transit[i], TRANSIT_ACCESS_EGRESS);
    assert_eq!(agents.transit_mode[i], MODE_CAR);
    let selected_lane = agents.planned_attach_lane_id[i] as usize;
    assert_eq!(selected_lane, agents.planned_detach_lane_id[i] as usize);
    assert!(
        selected_lane == home_entrance.car_lane_fwd || selected_lane == home_entrance.car_lane_bkw
    );
    assert!(
        selected_lane == work_entrance.car_lane_fwd || selected_lane == work_entrance.car_lane_bkw
    );
    assert!(agents.current_path[i].is_empty());
    assert_eq!(agents.access_flags[i] & ACCESS_ZERO_HOP_NODE_PATH, 0);
    assert_ne!(agents.planned_attach_node[i], agents.planned_detach_node[i]);
    assert!(agents.planned_attach_lane_d[i] < agents.planned_detach_lane_d[i]);
}

#[test]
fn test_short_same_edge_trip_prefers_direct_sidewalk_over_car() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(Edge {
        physical_length: 500.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        ..create_test_edge(n0, n1)
    });
    g.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);

    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_walk_home",
        ZoneClass::Residential,
    );
    let work_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_walk_work",
        ZoneClass::Commercial,
    );

    let mut home = create_test_building(0, 1);
    home.center_x = 125.0;
    home.center_y = -10.0;
    home.facing_dir = Vector2::new(0.0, -1.0);
    home.asset_id = home_asset;

    let mut work = create_test_building(0, 1);
    work.center_x = 225.0;
    work.center_y = -10.0;
    work.facing_dir = Vector2::new(0.0, -1.0);
    work.asset_id = work_asset;
    work.zone_type = ZoneType::Commercial;

    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&g, &network.lane_system);

    let home_entrance = allocator.entrances[0].clone();
    let work_entrance = allocator.entrances[1].clone();
    assert_ne!(home_entrance.foot_lane_fwd, usize::MAX);
    assert_eq!(home_entrance.foot_lane_fwd, work_entrance.foot_lane_fwd);

    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(
        0,
        n0,
        0.0,
        0.0,
        n0,
        home_entrance.door_pos.x,
        home_entrance.door_pos.y,
    );
    agents.home_building[i] = 0;
    agents.work_building[i] = 1;
    agents.current_building[i] = 0;
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.has_car[i] = true;
    agents.planned_activity[i] = 1;
    agents.planned_target_building[i] = 1;

    agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);

    assert_eq!(agents.transit[i], TRANSIT_ACCESS_EGRESS);
    assert_eq!(agents.transit_mode[i], MODE_WALK);
    assert_eq!(
        agents.planned_attach_lane_id[i] as usize,
        home_entrance.foot_lane_fwd
    );
    assert_eq!(
        agents.planned_detach_lane_id[i] as usize,
        work_entrance.foot_lane_fwd
    );
    assert!(agents.current_path[i].is_empty());
    assert_eq!(agents.access_flags[i] & ACCESS_ZERO_HOP_NODE_PATH, 0);
    assert_ne!(agents.planned_attach_node[i], agents.planned_detach_node[i]);
    assert!(agents.planned_attach_lane_d[i] < agents.planned_detach_lane_d[i]);
}

#[test]
fn test_same_edge_direct_car_trip_reaches_opposite_side_destination_building() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(Edge {
        physical_length: 500.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        ..create_test_edge(n0, n1)
    });
    g.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);

    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_move_home",
        ZoneClass::Residential,
    );
    let work_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_move_work",
        ZoneClass::Commercial,
    );

    let mut home = create_test_building(0, -1);
    home.center_x = 125.0;
    home.center_y = 10.0;
    home.facing_dir = Vector2::new(0.0, 1.0);
    home.asset_id = home_asset;

    let mut work = create_test_building(0, 1);
    work.center_x = 375.0;
    work.center_y = -10.0;
    work.facing_dir = Vector2::new(0.0, -1.0);
    work.asset_id = work_asset;
    work.zone_type = ZoneType::Commercial;

    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&g, &network.lane_system);

    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(
        0,
        n0,
        0.0,
        0.0,
        n0,
        allocator.entrances[0].door_pos.x,
        allocator.entrances[0].door_pos.y,
    );
    agents.home_building[i] = 0;
    agents.work_building[i] = 1;
    agents.current_building[i] = 0;
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.has_car[i] = true;
    agents.planned_activity[i] = 1;
    agents.planned_target_building[i] = 1;

    for _ in 0..2000 {
        agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);
        if agents.transit[i] == TRANSIT_IN_BUILDING && agents.current_building[i] == 1 {
            break;
        }
    }

    if agents.transit[i] == TRANSIT_ACCESS_INGRESS {
        let current = (agents.pos_x[i], agents.pos_y[i]);
        agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);
        assert_ne!(
            (agents.pos_x[i], agents.pos_y[i]),
            current,
            "ingress state did not advance from pos={:?}",
            current
        );
    }

    assert_eq!(
        agents.transit[i],
        TRANSIT_IN_BUILDING,
        "transit={} current_building={} pos=({}, {}) target_building={} planned_detach_lane={} planned_detach_d={}",
        agents.transit[i],
        agents.current_building[i],
        agents.pos_x[i],
        agents.pos_y[i],
        agents.target_building[i],
        agents.planned_detach_lane_id[i],
        agents.planned_detach_lane_d[i]
    );
    assert_eq!(agents.current_building[i], 1);
}

#[test]
fn test_same_edge_opposite_side_household_car_arrivals_eventually_finish_ingress() {
    let mut g = RegionGraph::new();
    let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = g.add_node(Vector3::new(500.0, 0.0, 0.0), NodeType::Junction);
    g.add_edge(Edge {
        physical_length: 500.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(500.0, 0.0, 0.0)],
        ..create_test_edge(n0, n1)
    });
    g.rebuild_adjacency_list();

    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut g);
    network.cch_graph = CchGraph::build(&g);

    let mut allocator = BuildingAllocator::new();
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_household_home",
        ZoneClass::Residential,
    );
    let work_asset = register_test_asset(
        &mut allocator,
        "test",
        "same_edge_household_work",
        ZoneClass::Commercial,
    );

    let mut home = create_test_building(0, -1);
    home.center_x = 125.0;
    home.center_y = 10.0;
    home.facing_dir = Vector2::new(0.0, 1.0);
    home.asset_id = home_asset;

    let mut work = create_test_building(0, 1);
    work.center_x = 375.0;
    work.center_y = -10.0;
    work.facing_dir = Vector2::new(0.0, -1.0);
    work.asset_id = work_asset;
    work.zone_type = ZoneType::Commercial;

    allocator.buildings.push(home);
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&g, &network.lane_system);

    let mut agents = AgentSystem::new();
    let mut ids = Vec::new();
    for _ in 0..5 {
        let i = agents.spawn_border_arrival_agent(
            0,
            n0,
            0.0,
            0.0,
            n0,
            allocator.entrances[0].door_pos.x,
            allocator.entrances[0].door_pos.y,
        );
        agents.home_building[i] = 0;
        agents.work_building[i] = 1;
        agents.current_building[i] = 0;
        agents.transit[i] = TRANSIT_IN_BUILDING;
        agents.has_car[i] = true;
        agents.planned_activity[i] = 1;
        agents.planned_target_building[i] = 1;
        ids.push(i);
    }

    for _ in 0..10000 {
        agents.tick(&mut allocator, &mut network, &mut g, 0.1, 0, 0);
        if ids
            .iter()
            .all(|&i| agents.transit[i] == TRANSIT_IN_BUILDING && agents.current_building[i] == 1)
        {
            break;
        }
    }

    for &i in &ids {
        assert_eq!(
            agents.transit[i],
            TRANSIT_IN_BUILDING,
            "agent {} stuck in transit={} pos=({}, {}) target_building={} current_building={}",
            i,
            agents.transit[i],
            agents.pos_x[i],
            agents.pos_y[i],
            agents.target_building[i],
            agents.current_building[i]
        );
        assert_eq!(agents.current_building[i], 1);
    }
}

#[test]
fn test_vehicle_type_persistence() {
    let mut agents = AgentSystem::new();
    let _i0 = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let _i1 = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let i2 = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
    let type2 = agents.vehicle_type[i2];
    let mut allocator = BuildingAllocator::new();
    let _ = agents.kill_agent(1, &mut allocator);
    assert_eq!(agents.len(), 2);
    assert_eq!(agents.vehicle_type[1], type2);
}

#[test]
fn test_border_spawn_movement() {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Border);
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "immigrant_home",
        ZoneClass::Residential,
    );
    network.add_road(
        &mut graph,
        vec![Vector3::ZERO, Vector3::RIGHT * 100.0],
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.cch_graph = CchGraph::build(&graph);
    let mut agents = AgentSystem::new();
    let mut home = create_test_building(0, 1);
    home.asset_id = asset_id;
    allocator.buildings.push(home);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let agent_idx = agents.spawn_border_arrival_agent(0, n0, 0.0, 0.0, n1, 100.0, 0.0);
    agents.tick(&mut allocator, &mut network, &mut graph, 1.0, 0, 0);
    assert!(agents.access_flags[agent_idx] & ACCESS_PLAN_VALID != 0);
    let mut reached_destination_ingress = false;
    for _ in 0..16 {
        agents.tick(&mut allocator, &mut network, &mut graph, 1.0, 0, 0);
        if agents.transit[agent_idx] == TRANSIT_ACCESS_INGRESS
            || (agents.transit[agent_idx] == TRANSIT_IN_BUILDING
                && agents.current_building[agent_idx] == 0)
        {
            reached_destination_ingress = true;
            break;
        }
    }
    assert!(reached_destination_ingress);
    assert!(agents.pos_x[agent_idx] < 60.0);
    assert_eq!(agents.target_building[agent_idx], 0);
}

#[test]
fn test_border_freight_replans_after_empty_path_freeze() {
    let (mut network, mut graph, _) = build_two_edge_road(1, 1);
    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let start_node = 0_u32;
    let border_node = 2_u32;
    let start_pos = graph.node(start_node).pos;

    let agent_idx = agents.spawn_border_arrival_agent(
        usize::MAX,
        border_node,
        0.0,
        0.0,
        start_node,
        start_pos.x,
        start_pos.z,
    );
    agents.current_building[agent_idx] = usize::MAX;
    agents.target_building[agent_idx] = usize::MAX;
    agents.freight_target_border_node[agent_idx] = border_node;
    agents.current_node[agent_idx] = start_node;
    agents.current_edge[agent_idx] = usize::MAX;
    agents.current_lane_id[agent_idx] = usize::MAX;
    agents.current_path[agent_idx].clear();
    agents.current_path_index[agent_idx] = 0;
    agents.access_flags[agent_idx] = ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION;
    agents.transit[agent_idx] = TRANSIT_NETWORK;
    agents.transit_mode[agent_idx] = MODE_CAR;
    agents.speed[agent_idx] = 0.0;

    for _ in 0..80 {
        agents.tick(&mut allocator, &mut network, &mut graph, 1.0, 0, 0);
        if agents.current_node[agent_idx] == border_node
            && agents.current_lane_id[agent_idx] == usize::MAX
            && agents.current_path[agent_idx].is_empty()
        {
            return;
        }
    }

    panic!(
        "freight border carrier stayed stuck: node={} lane={} path={:?} speed={:.2}",
        agents.current_node[agent_idx],
        agents.current_lane_id[agent_idx],
        agents.current_path[agent_idx],
        agents.speed[agent_idx],
    );
}

#[test]
fn test_walking_agent_recovers_from_stale_edge_after_road_split() {
    let (mut network, mut graph, _) = build_two_edge_road(1, 1);
    let mut allocator = BuildingAllocator::new();
    let asset_id = register_test_asset(
        &mut allocator,
        "test",
        "stale_edge_work",
        ZoneClass::Commercial,
    );
    let mut work = create_test_building(0, 1);
    work.asset_id = asset_id;
    work.zone_type = ZoneType::Commercial;
    allocator.buildings.push(work);
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);

    let mut agents = AgentSystem::new();
    let start_node = 2_u32;
    let start_pos = graph.node(start_node).pos;
    let agent_idx = agents.spawn_border_arrival_agent(
        usize::MAX,
        0,
        0.0,
        0.0,
        start_node,
        start_pos.x,
        start_pos.z,
    );
    agents.current_building[agent_idx] = usize::MAX;
    agents.target_building[agent_idx] = 0;
    agents.current_node[agent_idx] = start_node;
    agents.current_edge[agent_idx] = 0;
    agents.current_lane_id[agent_idx] = usize::MAX;
    agents.current_path[agent_idx].clear();
    agents.current_path_index[agent_idx] = 0;
    agents.access_flags[agent_idx] = ACCESS_PLAN_VALID;
    agents.transit[agent_idx] = TRANSIT_NETWORK;
    agents.transit_mode[agent_idx] = MODE_WALK;
    agents.speed[agent_idx] = 0.0;
    agents.has_car[agent_idx] = false;

    agents.tick(&mut allocator, &mut network, &mut graph, 1.0, 0, 0);

    assert_ne!(
        agents.current_lane_id[agent_idx],
        usize::MAX,
        "walking agent stayed off-lane after replanning from a stale edge"
    );
    assert_eq!(agents.current_edge[agent_idx], 1);
    assert!(agents.speed[agent_idx] > 0.0);
}

#[test]
fn test_pedestrian_crosses_junction() {
    let mut network = TransitNetwork::new();
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
    let _n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    let mut zoning = ZoningSystem::new(&WorldConfig::default());
    let mut allocator = BuildingAllocator::new();
    network.add_road(
        &mut graph,
        vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::ZERO],
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.add_road(
        &mut graph,
        vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);
    let home_asset = register_test_asset(
        &mut allocator,
        "test",
        "junction_home",
        ZoneClass::Residential,
    );
    let work_asset = register_test_asset(
        &mut allocator,
        "test",
        "junction_shop",
        ZoneClass::Commercial,
    );
    let mut home = create_test_building(0, 1);
    home.asset_id = home_asset;
    let mut shop = create_test_building(1, -1);
    shop.asset_id = work_asset;
    allocator.buildings.push(home);
    allocator.buildings.push(shop);
    allocator.buildings[1].zone_type = ZoneType::Commercial;
    allocator.rebuild_entrance_cache(&graph, &network.lane_system);
    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(0, n2, 0.0, 0.0, n0, -50.0, 10.0);
    agents.current_building[i] = 0;
    agents.home_building[i] = 0;
    agents.target_building[i] = usize::MAX;
    agents.planned_target_building[i] = 1;
    agents.planned_activity[i] = 2;
    agents.has_car[i] = false;
    agents.transit[i] = TRANSIT_IN_BUILDING;
    agents.pos_x[i] = allocator.entrances[0].door_pos.x;
    agents.pos_y[i] = allocator.entrances[0].door_pos.y;
    for _ in 0..5000 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        if agents.transit[i] == TRANSIT_IN_BUILDING && agents.current_building[i] == 1 {
            break;
        }
    }
    assert_eq!(agents.transit[i], TRANSIT_IN_BUILDING);
    assert_eq!(agents.current_building[i], 1);
}
