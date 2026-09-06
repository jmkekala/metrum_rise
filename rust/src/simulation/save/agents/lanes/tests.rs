// SPDX-License-Identifier: GPL-2.0-only

//! Save/load regressions for directed sidewalks and junction lane identities.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::lanes::geometry::agent_lane_position;
use crate::simulation::network::types::{
    EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
};
use godot::prelude::Vector3;
use std::collections::HashSet;

fn fixture() -> (RegionGraph, TransitNetwork) {
    let mut graph = RegionGraph::new();
    let points = [
        Vector3::new(-200.0, 0.0, 0.0),
        Vector3::new(-100.0, 0.0, 0.0),
        Vector3::ZERO,
        Vector3::new(0.0, 0.0, 100.0),
    ];
    let nodes = points.map(|p| graph.add_node(p, NodeType::Junction));
    for i in 0..3 {
        let geometry = vec![points[i], points[i + 1]];
        graph.add_edge(Edge {
            start_node: nodes[i],
            end_node: nodes[i + 1],
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 10.0,
            base_cost: 10.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 5.0,
            end_clip: 5.0,
            geometry: geometry.clone(),
            physical_geometry: geometry,
            deleted: i == 0,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
    }
    let mut network = TransitNetwork::new();
    network.lane_system.rebuild(&mut graph);
    (graph, network)
}

fn add_agent(
    agents: &mut AgentSystem,
    lane_id: usize,
    graph: &RegionGraph,
    lanes: &LaneSystem,
) -> usize {
    let lane = &lanes.lanes[lane_id];
    let node = if lane.edge_id == usize::MAX {
        lane.node_id as u32
    } else {
        let edge = graph.edge(lane.edge_id);
        if lane.is_fwd {
            edge.start_node
        } else {
            edge.end_node
        }
    };
    let i = agents.spawn_border_arrival_agent(usize::MAX, node, 0.0, 0.0, node, 0.0, 0.0);
    agents.transit_mode[i] = if lane.lane_type == LaneType::Foot {
        MODE_WALK
    } else {
        MODE_CAR
    };
    agents.transit[i] = if lane.edge_id == usize::MAX {
        TRANSIT_INTERSECTION
    } else {
        TRANSIT_NETWORK
    };
    agents.current_edge[i] = lane.edge_id;
    agents.current_lane_id[i] = lane_id;
    agents.lane_distance[i] = lane.length * 0.35;
    let pos = agent_lane_position(
        lane,
        agents.lane_distance[i],
        (agents.transit_mode[i] == MODE_WALK).then_some(i),
    )
    .unwrap();
    agents.pos_x[i] = pos.x;
    agents.pos_y[i] = pos.z;
    let plan_lane = if lane.edge_id == usize::MAX {
        lane.next_lanes[0]
    } else {
        lane_id
    };
    let planned = &lanes.lanes[plan_lane];
    let edge = graph.edge(planned.edge_id);
    agents.access_flags[i] = ACCESS_PLAN_VALID;
    agents.planned_attach_lane_id[i] = plan_lane as u32;
    agents.planned_detach_lane_id[i] = plan_lane as u32;
    agents.planned_attach_node[i] = if planned.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    };
    agents.planned_detach_node[i] = if planned.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    };
    i
}

#[test]
fn snapshot_remaps_all_lane_roles_after_incremental_rebuild_and_edge_compaction() {
    let (mut graph, mut network) = fixture();
    network
        .lane_system
        .rebuild_edges_incremental(&mut graph, &HashSet::from([1]));
    let mut ids = network
        .lane_system
        .edge_lanes
        .values()
        .chain(network.lane_system.node_lanes.values())
        .flatten()
        .copied()
        .collect::<Vec<_>>();
    ids.sort_unstable();
    let mut agents = AgentSystem::new();
    for &id in &ids {
        add_agent(&mut agents, id, &graph, &network.lane_system);
    }
    let allocator = BuildingAllocator::new();
    let maps = crate::simulation::save::build_snapshot_maps(&graph, &allocator, &agents).unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(crate::simulation::save::schema::SCHEMA)
        .unwrap();
    let tx = conn.transaction().unwrap();
    crate::simulation::save::network::save_network(&tx, &graph, &maps).unwrap();
    save_agents(&tx, &agents, &graph, &network, &maps).unwrap();
    tx.commit().unwrap();
    let mut restored = load_agents(&conn, 0.0).unwrap();
    let mut new_graph = crate::simulation::save::network::load_graph(&conn).unwrap();
    let mut new_network = TransitNetwork::new();
    new_network.lane_system.rebuild(&mut new_graph);
    restore_lane_references(
        &conn,
        58,
        &mut restored,
        &new_graph,
        &new_network,
        &allocator,
    )
    .unwrap();
    validate_loaded_agents(&mut restored, &new_graph, &allocator).unwrap();
    assert!(
        ids.iter()
            .zip(&restored.current_lane_id)
            .any(|(a, b)| a != b)
    );
    for i in 0..agents.len() {
        for (old_id, new_id) in [
            (agents.current_lane_id[i], restored.current_lane_id[i]),
            (
                agents.planned_attach_lane_id[i] as usize,
                restored.planned_attach_lane_id[i] as usize,
            ),
            (
                agents.planned_detach_lane_id[i] as usize,
                restored.planned_detach_lane_id[i] as usize,
            ),
        ] {
            let old = &network.lane_system.lanes[old_id];
            let new = &new_network.lane_system.lanes[new_id];
            assert_eq!(old.geometry, new.geometry);
            assert_eq!(old.is_fwd, new.is_fwd);
            assert_eq!(old.lane_type, new.lane_type);
        }
        assert_eq!(agents.lane_distance[i], restored.lane_distance[i]);
        assert_eq!(
            (agents.pos_x[i], agents.pos_y[i]),
            (restored.pos_x[i], restored.pos_y[i])
        );
    }
}

#[test]
fn v57_restores_both_sidewalk_directions_and_backward_vehicle_sentinel() {
    let (graph, network) = fixture();
    let lanes = &network.lane_system;
    let mut agents = AgentSystem::new();
    let ids = &lanes.edge_lanes[&1];
    for &id in ids {
        let i = add_agent(&mut agents, id, &graph, lanes);
        agents.current_lane_id[i] = lanes.lanes[id].lane_idx as usize;
        // A final frontage leg has already consumed every graph path node.
        agents.current_path[i] = vec![agents.current_node[i]];
        agents.current_path_index[i] = 1;
    }
    restore_v57_lane_references(&mut agents, &graph, &network, &BuildingAllocator::new()).unwrap();
    validate_loaded_agents(&mut agents, &graph, &BuildingAllocator::new()).unwrap();
    assert_eq!(agents.current_lane_id, *ids);
    assert!(agents.current_path_index.iter().all(|&idx| idx == 1));
}

#[test]
fn v57_restores_junction_from_saved_route_and_pose_instead_of_lane_zero() {
    let (graph, network) = fixture();
    let lanes = &network.lane_system;
    let mut agents = AgentSystem::new();
    let incoming = lanes.edge_lanes[&1]
        .iter()
        .copied()
        .find(|&id| lanes.lanes[id].lane_type == LaneType::Foot && lanes.lanes[id].is_fwd)
        .unwrap();
    let connector = lanes.lanes[incoming]
        .next_lanes
        .iter()
        .copied()
        .find(|&id| lanes.lanes[lanes.lanes[id].next_lanes[0]].edge_id == 2)
        .unwrap();
    let i = add_agent(&mut agents, connector, &graph, lanes);
    agents.current_lane_id[i] = 0;
    agents.current_path[i] = vec![1, 2, 3];
    agents.current_path_index[i] = 2;
    restore_v57_lane_references(&mut agents, &graph, &network, &BuildingAllocator::new()).unwrap();
    let restored = &lanes.lanes[agents.current_lane_id[i]];
    assert_eq!(restored.lane_type, LaneType::Foot);
    assert_eq!(restored.node_id, 2);
    assert_eq!(lanes.lanes[restored.next_lanes[0]].edge_id, 2);
    assert!(saved_pose_matches(
        &agents,
        i,
        restored,
        agents.lane_distance[i]
    ));
}

#[test]
fn loaded_network_and_ingress_plans_preserve_detach_without_origin_attach() {
    let (graph, network) = fixture();
    let mut agents = AgentSystem::new();
    let lane_id = network.lane_system.edge_lanes[&1][2];
    for transit in [TRANSIT_NETWORK, TRANSIT_ACCESS_INGRESS] {
        let i = add_agent(&mut agents, lane_id, &graph, &network.lane_system);
        agents.transit[i] = transit;
        agents.planned_attach_lane_id[i] = u32::MAX;
        agents.planned_attach_node[i] = u32::MAX;
        if transit == TRANSIT_ACCESS_INGRESS {
            agents.current_lane_id[i] = usize::MAX;
            agents.current_edge[i] = usize::MAX;
        }
    }
    validate_loaded_planned_lane_ids(&mut agents, network.lane_system.lanes.len());
    validate_loaded_agents(&mut agents, &graph, &BuildingAllocator::new()).unwrap();
    assert!(
        agents
            .access_flags
            .iter()
            .all(|&flags| flags == ACCESS_PLAN_VALID)
    );
    assert!(
        agents
            .planned_detach_lane_id
            .iter()
            .all(|&id| id == lane_id as u32)
    );
}
