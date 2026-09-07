// SPDX-License-Identifier: GPL-2.0-only

//! Exact full-recount oracle and routing regressions for incremental contraction scores.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{EdgeClass, NodeType};
use godot::prelude::Vector3;
use std::collections::BTreeSet;

fn grid(side: u32) -> RegionGraph {
    let mut graph = RegionGraph::new();
    for z in 0..side {
        for x in 0..side {
            graph.add_node(
                Vector3::new(x as f32 * 90.0, 0.0, z as f32 * 90.0),
                NodeType::Junction,
            );
        }
    }
    for z in 0..side {
        for x in 0..side {
            let a = z * side + x;
            for b in [
                (x + 1 < side).then_some(a + 1),
                (z + 1 < side).then_some(a + side),
            ]
            .into_iter()
            .flatten()
            {
                let geometry = vec![graph.node(a).pos, graph.node(b).pos];
                graph.add_edge(Edge {
                    start_node: a,
                    end_node: b,
                    primary_type: TransitType::Road,
                    allowed_types: TransitFlags::CAR,
                    width: 7.0,
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    speed_limit: 15.0,
                    base_cost: 6.0,
                    physical_length: 90.0,
                    current_congestion: 0.0,
                    start_clip: 0.0,
                    end_clip: 0.0,
                    physical_geometry: geometry.clone(),
                    geometry,
                    class: EdgeClass::Standard,
                    deleted: false,
                    no_building_spawn: false,
                    vehicle_frontage_access:
                        crate::simulation::network::types::VehicleFrontageAccess::BothSides,
                });
            }
        }
    }
    graph.rebuild_adjacency_list();
    graph
}

fn full_recount_order(graph: &RegionGraph) -> Vec<u32> {
    let mut adj = vec![BTreeSet::new(); graph.node_count()];
    for edge in graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.start_node != edge.end_node)
    {
        adj[edge.start_node as usize].insert(edge.end_node);
        adj[edge.end_node as usize].insert(edge.start_node);
    }
    let mut remaining: BTreeSet<u32> = (0..graph.node_count() as u32).collect();
    let mut order = Vec::new();
    while !remaining.is_empty() {
        // Independent deliberately slow oracle: recount every candidate after each contraction.
        let node = *remaining
            .iter()
            .min_by_key(|&&u| {
                let neighbors: Vec<_> = adj[u as usize].iter().copied().collect();
                let mut missing = 0_i64;
                for (i, &v) in neighbors.iter().enumerate() {
                    for w in &neighbors[i + 1..] {
                        missing += i64::from(!adj[v as usize].contains(w));
                    }
                }
                (missing - neighbors.len() as i64, u)
            })
            .unwrap();
        let neighbors: Vec<_> = adj[node as usize].iter().copied().collect();
        for &v in &neighbors {
            adj[v as usize].remove(&node);
            for &w in &neighbors {
                if v != w {
                    adj[v as usize].insert(w);
                }
            }
        }
        remaining.remove(&node);
        order.push(node);
    }
    order
}

#[test]
fn incremental_order_matches_full_recount_after_topology_edits() {
    for side in [1, 2, 4, 8] {
        let mut graph = grid(side);
        graph.add_node(Vector3::new(-1000.0, 0.0, 0.0), NodeType::Junction);
        for edited in [false, true] {
            if edited {
                for edge in (0..graph.edge_count()).step_by(5) {
                    graph.edge_mut(edge).deleted = true;
                }
                graph.rebuild_adjacency_list();
            }
            let expected = full_recount_order(&graph);
            for _ in 0..3 {
                let mut cch = CchGraph::new(graph.node_count());
                cch.compute_node_order(&graph);
                assert_eq!(cch.node_order, expected, "side={side}, edited={edited}");
            }
        }
    }
}

#[test]
fn reordered_grid_routes_match_all_pairs_shortest_costs() {
    for side in [2, 4, 6] {
        let graph = grid(side);
        let cch = CchGraph::build(&graph);
        let edge_cost = graph.edge(0).base_cost;
        for a in 0..side * side {
            for b in 0..side * side {
                let expected_steps = (a % side).abs_diff(b % side) + (a / side).abs_diff(b / side);
                let expected = expected_steps as f32 * edge_cost;
                let (cost, _, path) = cch
                    .find_path(a, b, usize::MAX, &graph, TransitFlags::CAR)
                    .unwrap();
                assert!(
                    (cost - expected).abs() < 0.001,
                    "side={side} {a}->{b}: {cost} != {expected}, {path:?}"
                );
            }
        }
    }
}

fn assert_all_pairs_costs(cch: &CchGraph, graph: &RegionGraph, mode: u8) {
    let n = graph.node_count();
    let mut costs = vec![vec![f32::INFINITY; n]; n];
    for (i, row) in costs.iter_mut().enumerate() {
        row[i] = 0.0;
    }
    for edge in graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && edge.allowed_types & mode != 0)
    {
        let a = edge.start_node as usize;
        let b = edge.end_node as usize;
        let cost = edge.base_cost * (1.0 + edge.current_congestion);
        if edge.fwd_lanes > 0 {
            costs[a][b] = costs[a][b].min(cost);
        }
        if edge.bkw_lanes > 0 {
            costs[b][a] = costs[b][a].min(cost);
        }
    }
    // Floyd-Warshall is intentionally confined to this tiny independent correctness oracle.
    for k in 0..n {
        for a in 0..n {
            for b in 0..n {
                costs[a][b] = costs[a][b].min(costs[a][k] + costs[k][b]);
            }
        }
    }
    for (a, row) in costs.iter().enumerate() {
        for (b, &expected) in row.iter().enumerate() {
            let actual = cch.find_path(a as u32, b as u32, usize::MAX, graph, mode);
            if expected.is_infinite() {
                assert!(actual.is_none(), "unexpected path {a}->{b}");
            } else {
                let (cost, _, path) = actual.expect("reachable pair");
                assert!(
                    (cost - expected).abs() < 0.001,
                    "{a}->{b}, mode={mode}: {cost} != {expected}, {path:?}"
                );
                assert_eq!(path.first(), Some(&(a as u32)));
                assert_eq!(path.last(), Some(&(b as u32)));
                let mut reconstructed_cost = 0.0;
                for pair in path.windows(2) {
                    let edge = graph.edge(graph.get_edge_between_nodes(pair[0], pair[1]).unwrap());
                    assert!(!edge.deleted && edge.allowed_types & mode != 0);
                    assert!(if edge.start_node == pair[0] {
                        edge.fwd_lanes > 0
                    } else {
                        edge.bkw_lanes > 0
                    });
                    reconstructed_cost += edge.base_cost * (1.0 + edge.current_congestion);
                }
                assert!(
                    (reconstructed_cost - cost).abs() < 0.001,
                    "unpacked route must realize its reported cost"
                );
            }
        }
    }
}

#[test]
fn shortcut_alternatives_survive_recustomization_modes_and_one_way_edits() {
    let mut graph = grid(4);
    // Vary modes, lengths' costs and direction. Include a disconnected storage node.
    graph.add_node(Vector3::new(-1000.0, 0.0, 0.0), NodeType::Junction);
    for (id, edge) in graph.edges_iter_mut().enumerate() {
        edge.allowed_types = if id % 3 == 0 {
            TransitFlags::FOOT
        } else {
            TransitFlags::FOOT | TransitFlags::CAR
        };
        edge.base_cost = 2.0 + (id % 7) as f32;
        if id % 4 == 0 {
            edge.bkw_lanes = 0;
        }
        if id % 11 == 0 {
            edge.deleted = true;
        }
    }
    graph.rebuild_adjacency_list();
    let mut cch = CchGraph::build(&graph);
    for phase in 0..3 {
        if phase > 0 {
            for (id, edge) in graph.edges_iter_mut().enumerate() {
                edge.current_congestion = if (id + phase) % 3 == 0 { 8.0 } else { 0.0 };
            }
            cch.customize(&graph);
        }
        assert_all_pairs_costs(&cch, &graph, TransitFlags::CAR);
        assert_all_pairs_costs(&cch, &graph, TransitFlags::FOOT);
    }
}

#[test]
fn cheaper_lower_triangle_survives_a_direct_arc_and_metric_changes() {
    let mut graph = grid(2);
    graph.edge_mut(2).deleted = true;
    graph.edge_mut(3).deleted = true;
    let mut direct = graph.edge(0).clone();
    direct.start_node = 1;
    direct.end_node = 2;
    direct.base_cost = 100.0;
    graph.add_edge(direct);
    graph.rebuild_adjacency_list();
    let mut cch = CchGraph::build(&graph);
    assert_eq!(
        cch.find_path(1, 2, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        12.0
    );
    graph.edge_mut(0).current_congestion = 100.0;
    cch.customize(&graph);
    assert_eq!(
        cch.find_path(1, 2, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        100.0
    );
    graph.edge_mut(0).current_congestion = 0.0;
    cch.customize(&graph);
    assert_eq!(
        cch.find_path(1, 2, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        12.0
    );
}

fn turn_aware_dijkstra(graph: &RegionGraph, start: u32, end: u32) -> Option<f32> {
    let mut heap = BinaryHeap::from([CchState {
        node: start,
        incoming_edge: usize::MAX,
        priority: 0.0,
        cost: 0.0,
    }]);
    let mut costs = HashMap::from([((start, usize::MAX), 0.0)]);
    while let Some(state) = heap.pop() {
        if state.cost > costs[&(state.node, state.incoming_edge)] {
            continue;
        }
        if state.node == end {
            return Some(state.cost);
        }
        for &id in graph.node_adjacency(state.node) {
            let edge = graph.edge(id);
            if edge.deleted || edge.allowed_types & TransitFlags::CAR == 0 {
                continue;
            }
            let next = if edge.start_node == state.node && edge.fwd_lanes > 0 {
                edge.end_node
            } else if edge.end_node == state.node && edge.bkw_lanes > 0 {
                edge.start_node
            } else {
                continue;
            };
            if state.incoming_edge != usize::MAX
                && !CchGraph::vehicle_turn_allowed(graph.node(state.node), state.incoming_edge, id)
            {
                continue;
            }
            let cost = state.cost + edge.base_cost * (1.0 + edge.current_congestion);
            let key = (next, id);
            if cost < costs.get(&key).copied().unwrap_or(f32::INFINITY) {
                costs.insert(key, cost);
                heap.push(CchState {
                    node: next,
                    incoming_edge: id,
                    priority: cost,
                    cost,
                });
            }
        }
    }
    None
}

#[test]
fn merged_open_endpoint_states_preserve_restricted_turn_routes() {
    let mut graph = grid(4);
    let incoming = graph.get_edge_between_nodes(1, 5).unwrap();
    let outgoing = graph.get_edge_between_nodes(5, 6).unwrap();
    graph.add_lane_connection(5, incoming, 0, outgoing, 0);
    let cch = CchGraph::build(&graph);
    for start in 0..16 {
        for end in 0..16 {
            let expected = turn_aware_dijkstra(&graph, start, end);
            let actual = cch.find_path(start, end, usize::MAX, &graph, TransitFlags::CAR);
            assert_eq!(
                actual.as_ref().map(|path| path.0),
                expected,
                "{start}->{end}"
            );
            if let Some((_, _, path)) = actual {
                assert!(CchGraph::path_has_valid_turns(&path, &graph));
            }
        }
    }
}
