// SPDX-License-Identifier: GPL-2.0-only

//! Contraction ordering, shortest-route oracles and deterministic query regressions.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
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
                let (order, rank) = CchGraph::compute_node_order(&graph);
                assert_eq!(order, expected, "side={side}, edited={edited}");
                for (position, node) in order.into_iter().enumerate() {
                    assert_eq!(rank[node as usize], position as u32);
                }
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
        if mode == TransitFlags::FOOT || edge.fwd_lanes > 0 {
            costs[a][b] = costs[a][b].min(cost);
        }
        if mode == TransitFlags::FOOT || edge.bkw_lanes > 0 {
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
                    assert!(
                        mode == TransitFlags::FOOT
                            || if edge.start_node == pair[0] {
                                edge.fwd_lanes > 0
                            } else {
                                edge.bkw_lanes > 0
                            }
                    );
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

fn turn_aware_dijkstra(
    graph: &RegionGraph,
    start: u32,
    end: u32,
    start_edge: usize,
) -> Option<f32> {
    let mut heap = BinaryHeap::from([CchState {
        node: start,
        incoming_edge: start_edge,
        cost: 0.0,
    }]);
    let mut costs = HashMap::from([((start, start_edge), 0.0)]);
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
            for start_edge in graph
                .node_adjacency(start)
                .iter()
                .copied()
                .chain([usize::MAX])
            {
                let expected = turn_aware_dijkstra(&graph, start, end, start_edge);
                let actual = cch.find_path(start, end, start_edge, &graph, TransitFlags::CAR);
                assert_eq!(
                    actual.as_ref().map(|path| path.0),
                    expected,
                    "{start}->{end}, origin edge={start_edge}"
                );
                if let Some((_, _, path)) = actual {
                    assert!(CchGraph::path_has_valid_vehicle_turns(&path, &graph));
                }
            }
        }
    }
}

#[test]
fn vehicle_whitelists_preserve_pedestrian_routes_and_origin_context() {
    let mut graph = grid(4);
    for edge in graph.edges_iter_mut() {
        edge.allowed_types |= TransitFlags::FOOT;
    }
    for node in [1, 5, 6, 10] {
        let incoming = graph.node_adjacency(node)[0];
        let outgoing = *graph.node_adjacency(node).last().unwrap();
        graph.add_lane_connection(node, incoming, 0, outgoing, 0);
    }
    let mut cch = CchGraph::build(&graph);
    for phase in 0..2 {
        if phase == 1 {
            for (id, edge) in graph.edges_iter_mut().enumerate() {
                edge.current_congestion = if id % 3 == 0 { 4.0 } else { 0.0 };
            }
            cch.customize(&graph);
        }
        assert_all_pairs_costs(&cch, &graph, TransitFlags::FOOT);
        for start in 0..16 {
            for end in 0..16 {
                let expected_foot = cch
                    .find_path(start, end, usize::MAX, &graph, TransitFlags::FOOT)
                    .unwrap()
                    .0;
                for &start_edge in graph.node_adjacency(start) {
                    let foot = cch.find_path(start, end, start_edge, &graph, TransitFlags::FOOT);
                    assert_eq!(
                        foot.map(|route| route.0),
                        Some(expected_foot),
                        "walk {start}->{end}, incoming edge={start_edge}, phase={phase}"
                    );
                    let car = cch.find_path(start, end, start_edge, &graph, TransitFlags::CAR);
                    assert_eq!(
                        car.map(|route| route.0),
                        turn_aware_dijkstra(&graph, start, end, start_edge),
                        "car {start}->{end}, incoming edge={start_edge}, phase={phase}"
                    );
                }
            }
        }
    }
}

#[test]
fn query_heap_has_consistent_cost_and_identity_ordering() {
    let values = [
        (0.0, 5, usize::MAX),
        (1.0, 1, 3),
        (1.0, 2, 1),
        (1.0, 2, 2),
        (2.0, 0, 0),
    ];
    for reverse in [false, true] {
        let mut states: Vec<_> = values
            .iter()
            .map(|&(cost, node, incoming_edge)| CchState {
                cost,
                node,
                incoming_edge,
            })
            .collect();
        if reverse {
            states.reverse();
        }
        let mut heap = BinaryHeap::from(states);
        let mut popped = Vec::new();
        while let Some(state) = heap.pop() {
            popped.push((state.cost, state.node, state.incoming_edge));
        }
        assert_eq!(popped, values);
    }
    // Equality must agree with total ordering even for signed zero and NaN bit patterns.
    for a in [0.0, -0.0, f32::INFINITY, f32::NAN] {
        for b in [0.0, -0.0, f32::INFINITY, f32::NAN] {
            let left = CchState {
                cost: a,
                node: 0,
                incoming_edge: usize::MAX,
            };
            let right = CchState { cost: b, ..left };
            assert_eq!(left == right, left.cmp(&right) == Ordering::Equal);
        }
    }
}

#[test]
fn equal_cost_query_paths_repeat_with_stable_inputs() {
    for side in [4, 8] {
        let graph = grid(side);
        let cch = CchGraph::build(&graph);
        for (start, end) in [
            (0, side * side - 1),
            (side - 1, side * (side - 1)),
            (side, side * (side - 1) - 1),
        ] {
            let expected = cch.find_path(start, end, usize::MAX, &graph, TransitFlags::CAR);
            assert!(expected.is_some());
            for _ in 0..128 {
                assert_eq!(
                    cch.find_path(start, end, usize::MAX, &graph, TransitFlags::CAR),
                    expected,
                    "identical graph/query must reproduce route: side={side}, {start}->{end}"
                );
            }
        }
    }

    // Both legal outgoing arms must remain distinct at a restricted meeting node.
    // The backward search discovers both before the forward search reaches it.
    let mut graph = RegionGraph::new();
    for (x, z) in [
        (0.0, 0.0),
        (20.0, -10.0),
        (20.0, 10.0),
        (30.0, 0.0),
        (10.0, 0.0),
    ] {
        graph.add_node(Vector3::new(x, 0.0, z), NodeType::Junction);
    }
    for (start, end, cost) in [
        (0, 4, 10.0),
        (4, 1, 1.0),
        (4, 2, 1.0),
        (1, 3, 1.0),
        (2, 3, 1.0),
    ] {
        let geometry = vec![graph.node(start).pos, graph.node(end).pos];
        graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            base_cost: cost,
            physical_length: geometry[0].distance_to(geometry[1]),
            physical_geometry: geometry.clone(),
            geometry,
            ..Edge::default()
        });
    }
    graph.add_lane_connection(4, 0, 0, 1, 0);
    graph.add_lane_connection(4, 0, 0, 2, 0);
    let cch = CchGraph::build(&graph);
    let expected = cch
        .find_path(0, 3, usize::MAX, &graph, TransitFlags::CAR)
        .unwrap();
    assert_eq!(expected.0, 12.0);
    assert!(CchGraph::path_has_valid_vehicle_turns(&expected.2, &graph));
    for _ in 0..128 {
        assert_eq!(
            cch.find_path(0, 3, usize::MAX, &graph, TransitFlags::CAR)
                .unwrap(),
            expected,
            "equal-cost legal turns must not depend on randomized search-map iteration"
        );
    }
}

fn benchmark_graph(side: u32) -> (RegionGraph, String) {
    let variant = std::env::var("METRUM_CCH_BENCH_GRAPH").unwrap_or_else(|_| "car".into());
    assert!(matches!(
        variant.as_str(),
        "car" | "mixed" | "restricted" | "one_way"
    ));
    let mut graph = grid(side);
    if variant != "car" {
        for edge in graph.edges_iter_mut() {
            edge.allowed_types |= TransitFlags::FOOT;
        }
    }
    if variant == "one_way" {
        for (id, edge) in graph.edges_iter_mut().enumerate() {
            if id % 4 == 0 {
                edge.bkw_lanes = 0;
            }
        }
    }
    if variant == "restricted" {
        let node = side / 2 * side + side / 2;
        let incoming = graph.node_adjacency(node)[0];
        let outgoing = *graph.node_adjacency(node).last().unwrap();
        graph.add_lane_connection(node, incoming, 0, outgoing, 0);
    }
    (graph, variant)
}

#[test]
#[ignore = "matched release CCH query benchmark; excludes graph and hierarchy setup"]
fn benchmark_cch_queries() {
    use std::hint::black_box;
    use std::time::Instant;

    for side in [8, 16, 32] {
        let (graph, variant) = benchmark_graph(side);
        let cch = CchGraph::build(&graph);
        let queries: Vec<_> = (0..16)
            .map(|i| {
                (
                    i * 17 % (side * side),
                    side * side - 1 - i * 31 % (side * side),
                )
            })
            .collect();
        for &(start, end) in &queries {
            let (cost, distance, path) = cch
                .find_path(start, end, usize::MAX, &graph, TransitFlags::CAR)
                .unwrap();
            let expected = turn_aware_dijkstra(&graph, start, end, usize::MAX).unwrap();
            assert_eq!(cost, expected);
            assert_eq!(distance, expected * 15.0);
            assert_eq!(path.len(), (expected / 6.0) as usize + 1);
        }
        let mut samples = Vec::with_capacity(21);
        let mut product = (0.0_f64, 0.0_f64, 0_usize);
        for _ in 0..21 {
            let begin = Instant::now();
            for &(start, end) in &queries {
                let (cost, distance, path) = black_box(cch.find_path(
                    black_box(start),
                    black_box(end),
                    usize::MAX,
                    &graph,
                    TransitFlags::CAR,
                ))
                .unwrap();
                product.0 += f64::from(cost);
                product.1 += f64::from(distance);
                product.2 += path.len();
            }
            samples.push(begin.elapsed().as_secs_f64() * 1e6 / queries.len() as f64);
        }
        samples.sort_by(f64::total_cmp);
        eprintln!(
            "CCH_QUERY_BENCH {}",
            serde_json::json!({
                "variant": variant, "side": side, "nodes": graph.node_count(), "edges": graph.edge_count(),
                "shortcuts": cch.shortcuts.len(), "queries_per_sample": queries.len(),
                "samples": samples.len(), "median_us": samples[10], "p95_us": samples[19],
                "cost_sum": product.0, "distance_sum": product.1, "path_nodes": product.2,
            })
        );
    }
}

#[test]
#[ignore = "matched release CCH construction/storage benchmark; excludes source graph setup"]
fn benchmark_cch_build_storage() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    use std::hint::black_box;
    use std::mem::{size_of, size_of_val};
    use std::time::Instant;

    fn vec_bytes<T>(values: &Vec<T>) -> usize {
        values.capacity() * size_of::<T>()
    }
    fn nested_bytes<T>(values: &Vec<Vec<T>>) -> usize {
        vec_bytes(values) + values.iter().map(vec_bytes).sum::<usize>()
    }

    for side in [8, 16, 32] {
        let (graph, variant) = benchmark_graph(side);
        let warm = CchGraph::build(&graph);
        let retained_bytes = size_of_val(&warm)
            + vec_bytes(&warm.shortcuts)
            + nested_bytes(&warm.fwd_up)
            + nested_bytes(&warm.bwd_up)
            + nested_bytes(&warm.shortcut_alternatives)
            + vec_bytes(&warm.customization_order);
        // Compare every query/customization product; the debug build counter is intentionally
        // excluded. Capacity accounting includes vector payloads/headers, not allocator metadata.
        let mut hash = DefaultHasher::new();
        for shortcut in &warm.shortcuts {
            (
                shortcut.start_node,
                shortcut.target_node,
                shortcut.cost.to_bits(),
                shortcut.dist.to_bits(),
                shortcut.base_edge,
                shortcut.mid_l,
                shortcut.mid_r,
                shortcut.first_edge,
                shortcut.last_edge,
                shortcut.allowed_types,
            )
                .hash(&mut hash);
        }
        warm.fwd_up.hash(&mut hash);
        warm.bwd_up.hash(&mut hash);
        warm.shortcut_alternatives.hash(&mut hash);
        warm.customization_order.hash(&mut hash);
        let fingerprint = hash.finish();
        let shortcuts = warm.shortcuts.len();
        drop(warm);

        let mut build_samples = Vec::with_capacity(21);
        let mut cycle_samples = Vec::with_capacity(21);
        for _ in 0..21 {
            let begin = Instant::now();
            let cch = CchGraph::build(black_box(&graph));
            black_box(&cch);
            let build_us = begin.elapsed().as_secs_f64() * 1e6;
            drop(cch);
            let cycle_us = begin.elapsed().as_secs_f64() * 1e6;
            build_samples.push(build_us);
            cycle_samples.push(cycle_us);
        }
        build_samples.sort_by(f64::total_cmp);
        cycle_samples.sort_by(f64::total_cmp);
        eprintln!(
            "CCH_STORAGE_BENCH {}",
            serde_json::json!({
                "variant": variant, "side": side, "nodes": graph.node_count(), "edges": graph.edge_count(),
                "shortcuts": shortcuts, "fingerprint": format!("{fingerprint:016x}"),
                "retained_bytes": retained_bytes, "samples": build_samples.len(),
                "build_median_us": build_samples[10], "cycle_median_us": cycle_samples[10],
            })
        );
    }
}
