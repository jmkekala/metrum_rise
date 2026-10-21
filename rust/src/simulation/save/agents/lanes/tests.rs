// SPDX-License-Identifier: GPL-2.0-only

//! Save/load regressions for directed sidewalks and junction lane identities.

use super::*;
use crate::simulation::economy::agents::MODE_WALK;
use crate::simulation::network::lanes::LaneType;
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
            start_clip: 5.0,
            end_clip: 5.0,
            geometry: geometry.clone(),
            physical_geometry: geometry,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
    }
    // add_edge publishes a live edge regardless of the supplied deleted flag.
    graph.edge_mut(0).deleted = true;
    graph.rebuild_all_indices();
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
    let i = agents.spawn_border_arrival_agent(usize::MAX, node, 0.0, 0.0);
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
    assert!(
        graph.edge(0).deleted,
        "fixture must contain a deleted edge to exercise compaction"
    );
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
        let i = agents.len() - 1;
        agents.speed[i] = 2.75;
        agents.planned_activity[i] = 2;
        agents.network_replan_failures[i] = 1;
        agents.lane_change_from_lane_id[i] = id as u32;
        agents.lane_change_start_d[i] = 1.5;
        agents.lane_change_length_m[i] = 8.0;
        agents.overtake_blocked_time_s[i] = 0.75;
        agents.overtake_cooldown_s[i] = 1.25;
        agents.job_lock_until_day[i] = 2;
        agents.consecutive_unpaid_days[i] = 1;
        network.lane_system.lanes[id].frontage_delay_penalty_s = id as f32 * 0.125;
        network.lane_system.lanes[id].live_frontage_delay_s = id as f32 * 0.25;
    }
    network.frontage_delay_elapsed_s = 0.375;
    let allocator = BuildingAllocator::new();
    let maps = crate::simulation::save::build_snapshot_maps(&graph, &allocator, &agents).unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(crate::simulation::save::schema::SCHEMA)
        .unwrap();
    let tx = conn.transaction().unwrap();
    crate::simulation::save::network::save_network(&tx, &graph, &maps).unwrap();
    save_agents(&tx, &agents, &graph, &network, &maps).unwrap();
    tx.commit().unwrap();
    let mut restored =
        load_agents(&conn, 0.0).unwrap();
    let mut new_graph = crate::simulation::save::network::load_graph(&conn).unwrap();
    assert_eq!(new_graph.edge_count(), graph.edge_count() - 1);
    assert_eq!(new_graph.node_count(), graph.node_count() - 1);
    let mut new_network = TransitNetwork::new();
    new_network.lane_system.rebuild(&mut new_graph);
    restore_lane_references(
        &conn,
        &mut restored,
        &new_graph,
        &mut new_network,
    )
    .unwrap();
    assert_eq!(
        network.frontage_delay_elapsed_s,
        new_network.frontage_delay_elapsed_s
    );
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
                agents.lane_change_from_lane_id[i] as usize,
                restored.lane_change_from_lane_id[i] as usize,
            ),
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
            assert_eq!(old.frontage_delay_penalty_s, new.frontage_delay_penalty_s);
            assert_eq!(old.live_frontage_delay_s, new.live_frontage_delay_s);
        }
        assert_eq!(
            (
                agents.speed[i],
                agents.planned_activity[i],
                agents.network_replan_failures[i],
                agents.lane_change_start_d[i],
                agents.lane_change_length_m[i],
                agents.overtake_blocked_time_s[i],
                agents.overtake_cooldown_s[i],
                agents.job_lock_until_day[i],
                agents.consecutive_unpaid_days[i]
            ),
            (
                restored.speed[i],
                restored.planned_activity[i],
                restored.network_replan_failures[i],
                restored.lane_change_start_d[i],
                restored.lane_change_length_m[i],
                restored.overtake_blocked_time_s[i],
                restored.overtake_cooldown_s[i],
                restored.job_lock_until_day[i],
                restored.consecutive_unpaid_days[i]
            ),
        );
        assert_eq!(agents.lane_distance[i], restored.lane_distance[i]);
        assert_eq!(
            (agents.pos_x[i], agents.pos_y[i]),
            (restored.pos_x[i], restored.pos_y[i])
        );
    }
}

#[test]
fn snapshot_rejects_walker_on_vehicle_lane_on_write_and_restore() {
    let (graph, mut network) = fixture();
    let id = network.lane_system.edge_lanes[&1]
        .iter()
        .copied()
        .find(|&id| network.lane_system.lanes[id].lane_type == LaneType::Vehicle)
        .unwrap();
    let mut agents = AgentSystem::new();
    let i = add_agent(&mut agents, id, &graph, &network.lane_system);
    let allocator = BuildingAllocator::new();
    let maps = crate::simulation::save::build_snapshot_maps(&graph, &allocator, &agents).unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(crate::simulation::save::schema::SCHEMA)
        .unwrap();
    agents.transit_mode[i] = MODE_WALK;
    {
        let tx = conn.transaction().unwrap();
        let error = save_agents(&tx, &agents, &graph, &network, &maps).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("lane type does not match travel mode")
        );
    }
    agents.transit_mode[i] = MODE_CAR;
    let tx = conn.transaction().unwrap();
    save_agents(&tx, &agents, &graph, &network, &maps).unwrap();
    tx.commit().unwrap();
    conn.execute("UPDATE agents SET transit_mode = 0", [])
        .unwrap();
    let mut loaded = load_agents(&conn, 0.0).unwrap();
    // Snapshot edges are compacted; restore against the corresponding compacted graph.
    let tx = conn.transaction().unwrap();
    crate::simulation::save::network::save_network(&tx, &graph, &maps).unwrap();
    tx.commit().unwrap();
    let mut restored_graph = crate::simulation::save::network::load_graph(&conn).unwrap();
    network.lane_system.rebuild(&mut restored_graph);
    let error = restore_lane_references(
        &conn,
        &mut loaded,
        &restored_graph,
        &mut network,
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("lane type does not match travel mode")
    );
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

#[test]
fn loaded_network_retry_preserves_backoff_without_repairing_to_home() {
    let (graph, network) = fixture();
    let mut allocator = BuildingAllocator::new();
    allocator.buildings.push(crate::simulation::buildings::allocator::indexed_test_building(
        "test:retry_home".into(),
        crate::simulation::zoning::ZoneType::Residential,
        0,
    ));
    let current_node = graph.edge(1).start_node;
    let mut agents = AgentSystem::new();
    let i = agents.spawn_border_arrival_agent(usize::MAX, current_node, 0.0, 0.0);
    agents.transit[i] = TRANSIT_NETWORK;
    agents.transit_mode[i] = MODE_WALK;
    agents.home_building[i] = 0;
    agents.current_building[i] = usize::MAX;
    agents.target_building[i] = usize::MAX;
    agents.current_node[i] = current_node;
    agents.current_edge[i] = 1;
    agents.current_lane_id[i] = usize::MAX;
    agents.current_path[i].clear();
    agents.current_path_index[i] = 1;
    agents.access_flags[i] = 0;
    agents.next_replan_time[i] = 9.5;
    agents.network_replan_failures[i] = 2;

    validate_loaded_planned_lane_ids(&mut agents, network.lane_system.lanes.len());
    assert_eq!(agents.next_replan_time[i], 9.5);
    assert_eq!(agents.network_replan_failures[i], 2);

    validate_loaded_agents(&mut agents, &graph, &allocator).unwrap();
    assert_eq!(agents.current_path_index[i], 1);
    assert_eq!(agents.target_building[i], usize::MAX);
    assert_eq!(agents.next_replan_time[i], 9.5);
    assert_eq!(agents.network_replan_failures[i], 2);
    assert_eq!(agents.transit[i], TRANSIT_NETWORK);
    assert_eq!(agents.current_node[i], current_node);
    assert_eq!(agents.current_edge[i], 1);

    // An immediate replan after demolition also has live state, despite zero retry counters.
    agents.next_replan_time[i] = 0.0;
    agents.network_replan_failures[i] = 0;
    agents.target_building[i] = 0;
    agents.planned_target_building[i] = 0;
    validate_loaded_planned_lane_ids(&mut agents, network.lane_system.lanes.len());
    validate_loaded_agents(&mut agents, &graph, &allocator).unwrap();
    assert_eq!(agents.current_path_index[i], 1);
    assert_eq!(agents.target_building[i], 0);
    assert_eq!(agents.planned_target_building[i], 0);
    assert_eq!(agents.next_replan_time[i], 0.0);
    assert_eq!(agents.network_replan_failures[i], 0);

    // A legacy orphan with neither a destination nor a pending retry still gets home repair.
    agents.target_building[i] = usize::MAX;
    agents.planned_target_building[i] = usize::MAX;
    validate_loaded_agents(&mut agents, &graph, &allocator).unwrap();
    assert_eq!(agents.current_path_index[i], 0);
    assert_eq!(agents.target_building[i], 0);
    assert_eq!(agents.next_replan_time[i], 0.0);

    // A car saved at a node before its first edge replans there and keeps its next plan, so a
    // loaded game continues exactly (`SIM-02-A`).
    agents.transit_mode[i] = MODE_CAR;
    agents.current_edge[i] = usize::MAX;
    agents.planned_target_building[i] = 0;
    validate_loaded_agents(&mut agents, &graph, &allocator).unwrap();
    assert_eq!(agents.transit[i], TRANSIT_NETWORK);
    assert_eq!(agents.target_building[i], 0);
    assert_eq!(agents.planned_target_building[i], 0);
}

#[test]
fn departure_holds_survive_loaded_plan_repair() {
    let (graph, network) = fixture();
    let mut allocator = BuildingAllocator::new();
    for index in 0..2 {
        allocator.buildings.push(crate::simulation::buildings::allocator::indexed_test_building(
            "test:held_trip".into(),
            crate::simulation::zoning::ZoneType::Residential,
            index,
        ));
    }
    let mut agents = AgentSystem::new();
    let i = agents.spawn_housed_agent(0, 0.0, 0.0);
    agents.planned_target_building[i] = 1;
    // A shopper reserved on the hour waits at home for its paced departure (`SIM-02-E`).
    agents.hold_departure(i);
    validate_loaded_planned_lane_ids(&mut agents, network.lane_system.lanes.len());
    validate_loaded_agents(&mut agents, &graph, &allocator).unwrap();
    assert!(agents.departure_held(i), "loading must not release a held trip early");
    assert_eq!(agents.planned_target_building[i], 1);
    assert!(agents.release_departure(i));
    assert!(!agents.departure_held(i));
}
