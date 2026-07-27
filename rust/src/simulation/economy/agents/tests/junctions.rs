//! Vehicle and pedestrian junction transition tests.

use super::support::*;
use super::*;
use crate::simulation::network::lanes::LaneType;

#[test]
fn test_junction_entry_uses_turn_speed_for_remaining_tick() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let north_node = 3_u32;
    let west_edge = 0_usize;
    let west_lane = fwd_vehicle_lanes(&network, west_edge)[0];
    let west_lane_len = network.lane_system.lanes[west_lane].length;

    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, north_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_CAR;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = west_lane;
    agents.lane_distance[idx] = (west_lane_len - 5.0).max(0.0);
    agents.speed[idx] = 14.0;
    agents.current_path[idx] = vec![west_node, center_node, north_node];
    agents.current_path_index[idx] = 1;

    agents.tick(&mut allocator, &mut network, &mut graph, 10.0, 0, 0);

    let lane_id = agents.current_lane_id[idx];
    assert!(lane_id < network.lane_system.lanes.len());
    assert_eq!(
        agents.transit[idx],
        TRANSIT_INTERSECTION,
        "post-tick lane={} edge={} d={:.3} len={:.3} next={:?}",
        lane_id,
        network.lane_system.lanes[lane_id].edge_id,
        agents.lane_distance[idx],
        network.lane_system.lanes[lane_id].length,
        network.lane_system.lanes[lane_id].next_lanes,
    );
    assert_eq!(network.lane_system.lanes[lane_id].edge_id, usize::MAX);
    assert!(
        agents.lane_distance[idx] < network.lane_system.lanes[lane_id].length,
        "agent should remain inside the junction connector for this tick"
    );
}

#[test]
fn test_vehicle_pass_through_split_continues_on_road_lane() {
    let (mut network, mut graph, fwd_lanes) = build_two_edge_road(1, 1);
    let west_node = 0_u32;
    let center_node = 1_u32;
    let east_node = 2_u32;
    let west_edge = 0_usize;
    let east_edge = 1_usize;
    let west_lane = fwd_lanes[0];
    let east_lane = fwd_vehicle_lanes(&network, east_edge)[0];
    assert!(
        network.lane_system.lanes[west_lane]
            .next_lanes
            .contains(&east_lane),
        "test setup should expose a direct pass-through lane link"
    );

    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, east_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_CAR;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = west_lane;
    agents.lane_distance[idx] = network.lane_system.lanes[west_lane].length - 0.2;
    agents.speed[idx] = 14.0;
    agents.current_path[idx] = vec![west_node, center_node, east_node];
    agents.current_path_index[idx] = 1;

    agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert_eq!(agents.transit[idx], TRANSIT_NETWORK);
    assert_eq!(agents.current_edge[idx], east_edge);
    assert_eq!(agents.current_lane_id[idx], east_lane);
    assert!(
        agents.lane_distance[idx] > 0.0,
        "remaining movement should continue onto the next physical lane"
    );
}

#[test]
fn test_walking_junction_entry_does_not_skip_connector_with_large_tick() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let north_node = 3_u32;
    let west_edge = 0_usize;
    let north_edge = 2_usize;
    let west_lane = fwd_foot_lane_to_edge(&network, west_edge, north_edge);
    let west_lane_len = network.lane_system.lanes[west_lane].length;

    let mut allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, north_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_WALK;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = west_lane;
    agents.lane_distance[idx] = (west_lane_len - 5.0).max(0.0);
    agents.speed[idx] = 0.0;
    agents.current_path[idx] = vec![west_node, center_node, north_node];
    agents.current_path_index[idx] = 1;

    agents.tick(&mut allocator, &mut network, &mut graph, 10.0, 0, 0);

    let lane_id = agents.current_lane_id[idx];
    assert!(lane_id < network.lane_system.lanes.len());
    assert_eq!(
        agents.transit[idx],
        TRANSIT_INTERSECTION,
        "post-tick lane={} edge={} d={:.3} len={:.3} next={:?}",
        lane_id,
        network.lane_system.lanes[lane_id].edge_id,
        agents.lane_distance[idx],
        network.lane_system.lanes[lane_id].length,
        network.lane_system.lanes[lane_id].next_lanes,
    );
    assert_eq!(network.lane_system.lanes[lane_id].edge_id, usize::MAX);
    assert!(
        agents.lane_distance[idx] < network.lane_system.lanes[lane_id].length,
        "walking agent should remain inside the junction connector for this tick"
    );
}

#[test]
fn test_walking_straight_through_junction_uses_composed_crosswalk_connector() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let east_node = 2_u32;
    let west_edge = 0_usize;
    let east_edge = 1_usize;

    for west_lane in fwd_foot_lanes(&network, west_edge) {
        let west_lane_len = network.lane_system.lanes[west_lane].length;
        let allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();
        let idx =
            agents.spawn_border_arrival_agent(usize::MAX, east_node, 0.0, 0.0, west_node, 0.0, 0.0);
        agents.transit[idx] = TRANSIT_NETWORK;
        agents.transit_mode[idx] = MODE_WALK;
        agents.current_node[idx] = west_node;
        agents.current_edge[idx] = west_edge;
        agents.current_lane_id[idx] = west_lane;
        agents.lane_distance[idx] = (west_lane_len - 5.0).max(0.0);
        agents.speed[idx] = 0.0;
        agents.current_path[idx] = vec![west_node, center_node, east_node];
        agents.current_path_index[idx] = 1;

        agents.tick(&allocator, &mut network, &mut graph, 10.0, 0, 0);

        let connector_id = agents.current_lane_id[idx];
        assert_eq!(agents.transit[idx], TRANSIT_INTERSECTION);
        let connector = &network.lane_system.lanes[connector_id];
        assert_eq!(connector.edge_id, usize::MAX);
        assert!(
            connector.geometry.len() >= 4,
            "straight-through walking must follow composed corner/crosswalk geometry"
        );
        let target_lane_id = connector.next_lanes[0];
        assert_eq!(
            network.lane_system.lanes[target_lane_id].edge_id, east_edge,
            "both incoming sidewalk sides must have a continuous route to the opposite arm"
        );
        assert!(
            agents.lane_distance[idx] < connector.length,
            "walker must remain visibly on the junction connector for this tick"
        );
    }
}

#[test]
fn test_walking_final_junction_reaches_exact_destination_sidewalk() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let west_edge = 0_usize;
    let north_edge = 2_usize;
    let inbound_lane = fwd_foot_lanes(&network, west_edge)[0];
    let inbound_len = network.lane_system.lanes[inbound_lane].length;
    let destination_lanes = network.lane_system.edge_lanes[&north_edge]
        .iter()
        .copied()
        .filter(|&lane_id| {
            let lane = &network.lane_system.lanes[lane_id];
            lane.lane_type == LaneType::Foot && !lane.is_fwd
        })
        .collect::<Vec<_>>();
    assert_eq!(destination_lanes.len(), 2);

    let mut exact_connectors = destination_lanes
        .iter()
        .map(|&destination_lane| {
            let connector_id = network.lane_system.lanes[inbound_lane]
                .next_lanes
                .iter()
                .copied()
                .find(|&connector_id| {
                    network.lane_system.lanes[connector_id]
                        .next_lanes
                        .first()
                        .copied()
                        == Some(destination_lane)
                })
                .unwrap_or_else(|| {
                    panic!(
                        "incoming sidewalk {inbound_lane} must reach exact destination sidewalk {destination_lane}"
                    )
                });
            (
                network.lane_system.lanes[connector_id].length,
                destination_lane,
            )
        })
        .collect::<Vec<_>>();
    exact_connectors.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let detach_lane = exact_connectors
        .last()
        .expect("two destination sidewalk connectors")
        .1;

    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(north_edge, -1));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: north_edge,
        side: -1,
        entrance_s_m: 5.0,
        foot_lane_bkw: detach_lane,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_WALK;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = inbound_lane;
    agents.lane_distance[idx] = (inbound_len - 0.01).max(0.0);
    agents.target_building[idx] = 0;
    agents.current_path[idx] = vec![west_node, center_node];
    agents.current_path_index[idx] = 1;
    agents.access_flags[idx] = ACCESS_PLAN_VALID;
    agents.planned_detach_node[idx] = center_node;
    agents.planned_detach_lane_id[idx] = detach_lane as u32;
    agents.planned_detach_lane_d[idx] = 5.0;

    agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, 0);

    let connector_id = agents.current_lane_id[idx];
    assert_eq!(agents.transit[idx], TRANSIT_INTERSECTION);
    assert_eq!(
        network.lane_system.lanes[connector_id]
            .next_lanes
            .first()
            .copied(),
        Some(detach_lane),
        "final-node movement must enter the exact destination-sidewalk connector"
    );
}

#[test]
fn test_walking_zero_hop_reverses_on_stationary_sidewalk_connector() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let west_edge = 0_usize;
    let inbound_lane = fwd_foot_lanes(&network, west_edge)[0];
    let inbound = &network.lane_system.lanes[inbound_lane];
    let detach_lane = network.lane_system.edge_lanes[&west_edge]
        .iter()
        .copied()
        .find(|&lane_id| {
            let lane = &network.lane_system.lanes[lane_id];
            lane.lane_type == LaneType::Foot && !lane.is_fwd && lane.lane_idx == inbound.lane_idx
        })
        .expect("reverse lane on the same sidewalk");
    let inbound_len = inbound.length;

    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(west_edge, -1));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: west_edge,
        side: -1,
        entrance_s_m: 5.0,
        foot_lane_fwd: inbound_lane,
        foot_lane_bkw: detach_lane,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_WALK;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = inbound_lane;
    agents.lane_distance[idx] = (inbound_len - 0.01).max(0.0);
    agents.target_building[idx] = 0;
    agents.current_path[idx].clear();
    agents.current_path_index[idx] = 0;
    agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
    agents.planned_detach_node[idx] = center_node;
    agents.planned_detach_lane_id[idx] = detach_lane as u32;
    agents.planned_detach_lane_d[idx] = 5.0;

    agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, 0);

    let connector_id = agents.current_lane_id[idx];
    assert_eq!(agents.transit[idx], TRANSIT_INTERSECTION);
    let connector = &network.lane_system.lanes[connector_id];
    assert_eq!(connector.length, 0.0);
    assert_eq!(connector.geometry.first(), connector.geometry.last());
    assert_eq!(connector.next_lanes, vec![detach_lane]);
}

#[test]
fn test_walking_missing_crosswalk_route_waits_at_junction_mouth() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    let center_node = 0_u32;
    for edge_id in 0..4 {
        graph.set_crosswalk_override(center_node, edge_id, false);
    }
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let east_node = 2_u32;
    let west_edge = 0_usize;
    let east_edge = 1_usize;
    let west_lane = fwd_foot_lanes(&network, west_edge)[0];
    let west_lane_len = network.lane_system.lanes[west_lane].length;
    assert!(
        !network.lane_system.lanes[west_lane]
            .next_lanes
            .iter()
            .any(|&connector_id| {
                let connector = &network.lane_system.lanes[connector_id];
                connector.next_lanes.first().is_some_and(|&target_lane_id| {
                    network.lane_system.lanes[target_lane_id].edge_id == east_edge
                })
            }),
        "test setup must leave the requested road arm unreachable"
    );

    let allocator = BuildingAllocator::new();
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, east_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_WALK;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = west_lane;
    agents.lane_distance[idx] = (west_lane_len - 0.01).max(0.0);
    agents.current_path[idx] = vec![west_node, center_node, east_node];
    agents.current_path_index[idx] = 1;

    agents.tick(&allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert_eq!(agents.transit[idx], TRANSIT_NETWORK);
    assert_eq!(agents.current_lane_id[idx], west_lane);
    assert_eq!(agents.lane_distance[idx], west_lane_len);
    assert_eq!(
        agents.current_path[idx],
        vec![west_node, center_node, east_node]
    );
    assert_eq!(agents.current_path_index[idx], 1);
}

#[test]
fn test_zero_hop_access_uses_junction_connector() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let west_edge = 0_usize;
    let north_edge = 2_usize;
    let inbound_lane = fwd_vehicle_lanes(&network, west_edge)[0];
    let detach_lane = bkw_vehicle_lanes(&network, north_edge)[0];
    let inbound_len = network.lane_system.lanes[inbound_lane].length;
    let inbound = &network.lane_system.lanes[inbound_lane];
    let inbound_edge = graph.edge(inbound.edge_id);
    let inbound_terminal = if inbound.is_fwd {
        inbound_edge.end_node
    } else {
        inbound_edge.start_node
    };
    assert_eq!(
        inbound_terminal, center_node,
        "test setup must use a lane ending at the center junction"
    );
    let detach = &network.lane_system.lanes[detach_lane];
    let detach_edge = graph.edge(detach.edge_id);
    let detach_origin = if detach.is_fwd {
        detach_edge.start_node
    } else {
        detach_edge.end_node
    };
    assert_eq!(
        detach_origin, center_node,
        "test setup must use a detach lane starting at the center junction"
    );
    assert!(
        inbound.next_lanes.iter().any(|&connector_id| {
            let connector = &network.lane_system.lanes[connector_id];
            connector.edge_id == usize::MAX
                && connector.next_lanes.first().copied() == Some(detach_lane)
        }),
        "test setup must have a connector from inbound lane to detach lane"
    );

    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(north_edge, -1));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: north_edge,
        side: -1,
        entrance_s_m: 5.0,
        door_pos: Vector2::new(0.0, -105.0),
        curb_pos: Vector2::new(0.0, -100.0),
        car_lane_bkw: detach_lane,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let idx =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[idx] = TRANSIT_NETWORK;
    agents.transit_mode[idx] = MODE_CAR;
    agents.current_node[idx] = west_node;
    agents.current_edge[idx] = west_edge;
    agents.current_lane_id[idx] = inbound_lane;
    agents.lane_distance[idx] = (inbound_len - 0.01).max(0.0);
    agents.speed[idx] = 14.0;
    agents.target_building[idx] = 0;
    agents.current_path[idx].clear();
    agents.current_path_index[idx] = 0;
    agents.access_flags[idx] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
    agents.planned_detach_node[idx] = center_node;
    agents.planned_detach_lane_id[idx] = detach_lane as u32;
    agents.planned_detach_lane_d[idx] = 5.0;

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    let lane_id = agents.current_lane_id[idx];
    assert!(lane_id < network.lane_system.lanes.len());
    assert_eq!(
        agents.transit[idx],
        TRANSIT_INTERSECTION,
        "post-tick lane={} edge={} d={:.3} len={:.3} next={:?}",
        lane_id,
        network.lane_system.lanes[lane_id].edge_id,
        agents.lane_distance[idx],
        network.lane_system.lanes[lane_id].length,
        network.lane_system.lanes[lane_id].next_lanes,
    );
    assert_eq!(network.lane_system.lanes[lane_id].edge_id, usize::MAX);
    assert_eq!(
        network.lane_system.lanes[lane_id]
            .next_lanes
            .first()
            .copied(),
        Some(detach_lane),
        "zero-hop access should traverse the connector to the planned detach lane"
    );
}

#[test]
fn test_zero_hop_access_wait_keeps_path_index_stable() {
    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let west_edge = 0_usize;
    let north_edge = 2_usize;
    let inbound_lane = fwd_vehicle_lanes(&network, west_edge)[0];
    let detach_lane = bkw_vehicle_lanes(&network, north_edge)[0];
    let inbound_len = network.lane_system.lanes[inbound_lane].length;
    let connector_lane = network.lane_system.lanes[inbound_lane]
        .next_lanes
        .iter()
        .copied()
        .find(|&connector_id| {
            let connector = &network.lane_system.lanes[connector_id];
            connector.edge_id == usize::MAX
                && connector.next_lanes.first().copied() == Some(detach_lane)
        })
        .expect("test setup must have a connector from inbound lane to detach lane");

    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(north_edge, -1));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: north_edge,
        side: -1,
        entrance_s_m: 5.0,
        door_pos: Vector2::new(0.0, -105.0),
        curb_pos: Vector2::new(0.0, -100.0),
        car_lane_bkw: detach_lane,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let blocker =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[blocker] = TRANSIT_INTERSECTION;
    agents.transit_mode[blocker] = MODE_CAR;
    agents.current_node[blocker] = center_node;
    agents.current_edge[blocker] = usize::MAX;
    agents.current_lane_id[blocker] = connector_lane;
    agents.lane_distance[blocker] = 1.0;
    agents.speed[blocker] = 0.0;

    let waiter =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[waiter] = TRANSIT_NETWORK;
    agents.transit_mode[waiter] = MODE_CAR;
    agents.current_node[waiter] = west_node;
    agents.current_edge[waiter] = west_edge;
    agents.current_lane_id[waiter] = inbound_lane;
    agents.lane_distance[waiter] = (inbound_len - 0.01).max(0.0);
    agents.speed[waiter] = 14.0;
    agents.target_building[waiter] = 0;
    agents.current_path[waiter].clear();
    agents.current_path_index[waiter] = 0;
    agents.access_flags[waiter] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
    agents.planned_detach_node[waiter] = center_node;
    agents.planned_detach_lane_id[waiter] = detach_lane as u32;
    agents.planned_detach_lane_d[waiter] = 5.0;

    for _ in 0..2 {
        agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);
        assert_eq!(agents.transit[waiter], TRANSIT_NETWORK);
        assert_eq!(agents.current_lane_id[waiter], inbound_lane);
        assert_eq!(
            agents.current_path_index[waiter], 0,
            "zero-hop access wait must not drift past an empty node path"
        );
    }
}

#[test]
fn test_zero_hop_access_enters_spaced_occupied_connector() {
    use crate::config::{CAR_LENGTH, IDM_S_MIN};

    let (mut network, mut graph, _) = build_4way_junction(1, 1);
    graph.rebuild_intersection_clips();
    network.lane_system.rebuild(&mut graph);
    network.cch_graph = CchGraph::build(&graph);

    let west_node = 1_u32;
    let center_node = 0_u32;
    let west_edge = 0_usize;
    let north_edge = 2_usize;
    let inbound_lane = fwd_vehicle_lanes(&network, west_edge)[0];
    let detach_lane = bkw_vehicle_lanes(&network, north_edge)[0];
    let inbound_len = network.lane_system.lanes[inbound_lane].length;
    let connector_lane = network.lane_system.lanes[inbound_lane]
        .next_lanes
        .iter()
        .copied()
        .find(|&connector_id| {
            let connector = &network.lane_system.lanes[connector_id];
            connector.edge_id == usize::MAX
                && connector.next_lanes.first().copied() == Some(detach_lane)
        })
        .expect("test setup must have a connector from inbound lane to detach lane");

    let mut allocator = BuildingAllocator::new();
    allocator
        .buildings
        .push(create_test_building(north_edge, -1));
    allocator.entrances.push(BuildingEntrance {
        edge_idx: north_edge,
        side: -1,
        entrance_s_m: 5.0,
        door_pos: Vector2::new(0.0, -105.0),
        curb_pos: Vector2::new(0.0, -100.0),
        car_lane_bkw: detach_lane,
        ..BuildingEntrance::default()
    });
    let mut agents = AgentSystem::new();
    let blocker =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[blocker] = TRANSIT_INTERSECTION;
    agents.transit_mode[blocker] = MODE_CAR;
    agents.current_node[blocker] = center_node;
    agents.current_edge[blocker] = usize::MAX;
    agents.current_lane_id[blocker] = connector_lane;
    agents.lane_distance[blocker] = CAR_LENGTH + IDM_S_MIN + 0.5;
    agents.speed[blocker] = 0.0;

    let follower =
        agents.spawn_border_arrival_agent(usize::MAX, center_node, 0.0, 0.0, west_node, 0.0, 0.0);
    agents.transit[follower] = TRANSIT_NETWORK;
    agents.transit_mode[follower] = MODE_CAR;
    agents.current_node[follower] = west_node;
    agents.current_edge[follower] = west_edge;
    agents.current_lane_id[follower] = inbound_lane;
    agents.lane_distance[follower] = (inbound_len - 0.01).max(0.0);
    agents.speed[follower] = 14.0;
    agents.target_building[follower] = 0;
    agents.current_path[follower].clear();
    agents.current_path_index[follower] = 0;
    agents.access_flags[follower] = ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH;
    agents.planned_detach_node[follower] = center_node;
    agents.planned_detach_lane_id[follower] = detach_lane as u32;
    agents.planned_detach_lane_d[follower] = 5.0;

    agents.tick(&mut allocator, &mut network, &mut graph, 0.1, 0, 0);

    assert_eq!(agents.transit[follower], TRANSIT_INTERSECTION);
    assert_eq!(agents.current_lane_id[follower], connector_lane);
    assert_eq!(agents.transit[blocker], TRANSIT_INTERSECTION);
    assert_eq!(agents.current_lane_id[blocker], connector_lane);
    let gap = (agents.lane_distance[blocker] - agents.lane_distance[follower]).abs();
    assert!(
        gap >= CAR_LENGTH + IDM_S_MIN - 0.01,
        "connector follower should enter only with safe spacing; gap {gap:.3}"
    );
}
