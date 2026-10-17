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

#[test]
fn nested_dissection_is_deterministic_after_topology_edits() {
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
            let expected = CchGraph::compute_node_order(&graph).0;
            assert_eq!(
                expected.iter().copied().collect::<BTreeSet<_>>().len(),
                graph.node_count()
            );
            for _ in 0..3 {
                let (order, rank) = CchGraph::compute_node_order(&graph);
                assert_eq!(order, expected);
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
                    .find_path(a, b, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
                    .unwrap();
                assert!(
                    (cost - expected).abs() < 0.001,
                    "side={side} {a}->{b}: {cost} != {expected}, {path:?}"
                );
            }
        }
    }
}

// Walkers go at walking pace, blind to speed limits (TRAFFIC-05).
fn edge_cost(edge: &Edge, mode: u8) -> f32 {
    if mode == TransitFlags::FOOT {
        crate::simulation::pathing::cost::CostCalculator::walk_cost_s(edge)
    } else {
        edge.base_cost
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
        let cost = edge_cost(edge, mode);
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
            let actual = cch.find_path(a as u32, b as u32, usize::MAX, usize::MAX, graph, mode);
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
                    reconstructed_cost += edge_cost(edge, mode);
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
                edge.base_cost =
                    (2.0 + (id % 7) as f32) * if (id + phase) % 3 == 0 { 9.0 } else { 1.0 };
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
        cch.find_path(1, 2, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        12.0
    );
    let original_cost = graph.edge(0).base_cost;
    graph.edge_mut(0).base_cost = 1000.0;
    cch.customize(&graph);
    assert_eq!(
        cch.find_path(1, 2, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        100.0
    );
    graph.edge_mut(0).base_cost = original_cost;
    cch.customize(&graph);
    assert_eq!(
        cch.find_path(1, 2, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        12.0
    );
}

#[test]
fn rebuilding_with_live_congestion_preserves_free_flow_costs() {
    let mut graph = grid(3);
    let mut original = CchGraph::build(&graph);
    let expected = original.find_path(0, 8, usize::MAX, usize::MAX, &graph, TransitFlags::CAR);
    for edge in graph.edges_iter_mut() {
        edge.current_congestion = 100.0;
    }
    let restored = CchGraph::build(&graph);
    assert_eq!(
        restored.find_path(0, 8, usize::MAX, usize::MAX, &graph, TransitFlags::CAR),
        expected
    );
    original.customize(&graph);
    assert_eq!(
        original.find_path(0, 8, usize::MAX, usize::MAX, &graph, TransitFlags::CAR),
        expected
    );
    assert_costs_from_match_queries(&restored, &graph, TransitFlags::CAR);
}

#[test]
fn positive_self_loop_pays_its_cost_after_each_customization() {
    let mut graph = RegionGraph::new();
    for _ in 0..3 {
        graph.add_node(Vector3::ZERO, NodeType::Junction);
    }
    for (start_node, end_node, base_cost) in [(0, 2, 1.0), (2, 2, 5.0), (2, 1, 1.0)] {
        graph.add_edge(Edge {
            start_node,
            end_node,
            base_cost,
            physical_length: base_cost,
            fwd_lanes: 1,
            bkw_lanes: 1,
            allowed_types: TransitFlags::CAR,
            ..Default::default()
        });
    }
    graph.add_lane_connection(2, 0, 0, 1, 0);
    graph.add_lane_connection(2, 1, 0, 2, 0);
    graph.rebuild_adjacency_list();
    let mut cch = CchGraph::build(&graph);
    for loop_cost in [5.0, 11.0, 0.5] {
        graph.edge_mut(1).base_cost = loop_cost;
        cch.customize(&graph);
        let expected = turn_aware_dijkstra(&graph, 0, 1, usize::MAX).unwrap();
        assert_eq!(expected, 2.0 + loop_cost);
        let (cost, distance, path) = cch
            .find_path(0, 1, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap();
        assert_eq!(path, [0, 2, 2, 1]);
        assert_eq!(distance, 7.0);
        assert_eq!(cost, expected);
        assert_eq!(
            cch.costs_from_each(&[0], &graph, TransitFlags::CAR)[0].cost_to(1),
            Some(expected)
        );
    }
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
                && !CchGraph::vehicle_turn_allowed(graph, state.node, state.incoming_edge, id)
            {
                continue;
            }
            let cost = state.cost + edge.base_cost;
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
            for start_edge in
                graph
                    .node_adjacency(start)
                    .iter()
                    .copied()
                    .chain([usize::MAX, usize::MAX - 1, 0])
            {
                let expected = turn_aware_dijkstra(&graph, start, end, start_edge);
                let actual = cch.find_path(
                    start,
                    end,
                    start_edge,
                    usize::MAX,
                    &graph,
                    TransitFlags::CAR,
                );
                assert_eq!(
                    actual.as_ref().map(|path| path.0),
                    expected,
                    "{start}->{end}, origin edge={start_edge}"
                );
                if let Some((_, _, path)) = actual {
                    assert!(CchGraph::path_has_valid_vehicle_turns(
                        &path,
                        start_edge,
                        usize::MAX,
                        &graph
                    ));
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
                edge.base_cost *= if id % 3 == 0 { 5.0 } else { 1.0 };
            }
            cch.customize(&graph);
        }
        assert_all_pairs_costs(&cch, &graph, TransitFlags::FOOT);
        for start in 0..16 {
            for end in 0..16 {
                let expected_foot = cch
                    .find_path(
                        start,
                        end,
                        usize::MAX,
                        usize::MAX,
                        &graph,
                        TransitFlags::FOOT,
                    )
                    .unwrap()
                    .0;
                for &start_edge in graph.node_adjacency(start) {
                    let foot = cch.find_path(
                        start,
                        end,
                        start_edge,
                        usize::MAX,
                        &graph,
                        TransitFlags::FOOT,
                    );
                    assert_eq!(
                        foot.map(|route| route.0),
                        Some(expected_foot),
                        "walk {start}->{end}, incoming edge={start_edge}, phase={phase}"
                    );
                    let car = cch.find_path(
                        start,
                        end,
                        start_edge,
                        usize::MAX,
                        &graph,
                        TransitFlags::CAR,
                    );
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

fn assert_costs_from_match_queries(cch: &CchGraph, graph: &RegionGraph, mode: u8) {
    let starts: Vec<u32> = (0..graph.node_count() as u32).collect();
    for (&start, costs) in starts.iter().zip(cch.costs_from_each(&starts, graph, mode)) {
        for end in 0..graph.node_count() as u32 {
            let sweep = costs.cost_to(end);
            let point = cch
                .find_path(start, end, usize::MAX, usize::MAX, graph, mode)
                .map(|route| route.0);
            assert_eq!(sweep.is_some(), point.is_some());
            if let (Some(sweep), Some(point)) = (sweep, point) {
                // PHAST sums outward; point queries join independently summed halves.
                assert!(
                    (sweep - point).abs() <= 4.0 * f32::EPSILON * point.abs().max(1.0),
                    "{start}->{end}, mode={mode}: {sweep} vs {point}"
                );
            }
        }
    }
}

#[test]
fn costs_from_each_match_point_queries() {
    // Restricted junctions. Queries meet at the top-ranked node, so a loop there that is the
    // only legal way through it is taken by some routes.
    let mut graph = grid(4);
    for edge in graph.edges_iter_mut() {
        edge.allowed_types |= TransitFlags::FOOT;
    }
    let top = *CchGraph::build(&graph).node_order.last().unwrap();
    let incoming = graph.node_adjacency(top)[0];
    let outgoing = *graph.node_adjacency(top).last().unwrap();
    let mut loop_edge = graph.edge(0).clone();
    loop_edge.start_node = top;
    loop_edge.end_node = top;
    let loop_edge = graph.add_edge(loop_edge);
    graph.add_lane_connection(top, incoming, 0, loop_edge, 0);
    graph.add_lane_connection(top, loop_edge, 0, outgoing, 0);
    for node in [1, 6, 10].into_iter().filter(|&node| node != top) {
        let incoming = graph.node_adjacency(node)[0];
        let outgoing = *graph.node_adjacency(node).last().unwrap();
        graph.add_lane_connection(node, incoming, 0, outgoing, 0);
    }
    graph.rebuild_adjacency_list();
    let cch = CchGraph::build(&graph);
    assert_costs_from_match_queries(&cch, &graph, TransitFlags::CAR);
    assert_costs_from_match_queries(&cch, &graph, TransitFlags::FOOT);

    // Mixed modes, one-way and deleted edges, a disconnected node and recustomized metrics.
    let mut graph = grid(5);
    graph.add_node(Vector3::new(-1000.0, 0.0, 0.0), NodeType::Junction);
    for (id, edge) in graph.edges_iter_mut().enumerate() {
        edge.allowed_types = if id % 3 == 0 {
            TransitFlags::FOOT
        } else {
            TransitFlags::FOOT | TransitFlags::CAR
        };
        edge.base_cost = 2.0 + (id % 7) as f32;
        edge.bkw_lanes = u8::from(id % 4 != 0);
        edge.deleted = id % 11 == 0;
    }
    graph.rebuild_adjacency_list();
    let mut cch = CchGraph::build(&graph);
    for phase in 0..3 {
        if phase > 0 {
            for (id, edge) in graph.edges_iter_mut().enumerate() {
                edge.base_cost =
                    (2.0 + (id % 7) as f32) * if (id + phase) % 3 == 0 { 9.0 } else { 1.0 };
            }
            cch.customize(&graph);
        }
        assert_costs_from_match_queries(&cch, &graph, TransitFlags::CAR);
        assert_costs_from_match_queries(&cch, &graph, TransitFlags::FOOT);
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
            let expected = cch.find_path(
                start,
                end,
                usize::MAX,
                usize::MAX,
                &graph,
                TransitFlags::CAR,
            );
            assert!(expected.is_some());
            for _ in 0..128 {
                assert_eq!(
                    cch.find_path(
                        start,
                        end,
                        usize::MAX,
                        usize::MAX,
                        &graph,
                        TransitFlags::CAR
                    ),
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
        .find_path(0, 3, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
        .unwrap();
    assert_eq!(expected.0, 12.0);
    assert!(CchGraph::path_has_valid_vehicle_turns(
        &expected.2,
        usize::MAX,
        usize::MAX,
        &graph
    ));
    for _ in 0..128 {
        assert_eq!(
            cch.find_path(0, 3, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
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
                .find_path(
                    start,
                    end,
                    usize::MAX,
                    usize::MAX,
                    &graph,
                    TransitFlags::CAR,
                )
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

    for side in [8, 16, 32] {
        let (graph, variant) = benchmark_graph(side);
        let warm = CchGraph::build(&graph);
        let retained_bytes = size_of_val(&warm)
            + vec_bytes(&warm.shortcuts)
            + vec_bytes(&warm.up.offsets)
            + vec_bytes(&warm.up.values)
            + vec_bytes(&warm.parent)
            + vec_bytes(&warm.states.offsets)
            + vec_bytes(&warm.states.values)
            + vec_bytes(&warm.departures.offsets)
            + vec_bytes(&warm.departures.values)
            + vec_bytes(&warm.turns.restricted)
            + vec_bytes(&warm.turns.allowed.offsets)
            + vec_bytes(&warm.turns.allowed.values)
            + vec_bytes(&warm.base_arcs)
            + vec_bytes(&warm.base_metric)
            + vec_bytes(&warm.base_walk)
            + vec_bytes(&warm.triangles)
            + vec_bytes(&warm.node_order)
            + vec_bytes(&warm.profiles)
            + warm
                .profiles
                .iter()
                .map(|p| vec_bytes(&p.costs) + vec_bytes(&p.distances) + vec_bytes(&p.witnesses))
                .sum::<usize>();
        let mut hash = DefaultHasher::new();
        warm.node_order.hash(&mut hash);
        warm.parent.hash(&mut hash);
        for profile in &warm.profiles {
            for cost in &profile.costs {
                cost.to_bits().hash(&mut hash);
            }
        }
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

#[test]
fn owned_path_scratch_does_not_prepare_the_city_for_invalid_or_trivial_queries() {
    let graph = grid(5);
    let cch = CchGraph::build(&graph);
    OWNED_PATH_SCRATCH.with_borrow_mut(|scratch| *scratch = CchPathScratch::default());
    assert!(
        cch.find_path(0, 25, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .is_none()
    );
    assert_eq!(
        cch.find_path(0, 0, usize::MAX, usize::MAX, &graph, TransitFlags::CAR),
        Some((0.0, 0.0, vec![0]))
    );
    OWNED_PATH_SCRATCH.with_borrow_mut(|scratch| {
        assert_eq!(scratch.query().capacities(), [0; 4]);
    });
    assert!(
        cch.find_path(0, 24, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .is_some()
    );
}

#[test]
fn prepared_cost_queries_match_paths_without_growing_search_storage() {
    let mut graph = grid(5);
    let mut cch = CchGraph::build(&graph);
    let mut scratch = CchQueryScratch::default();
    scratch.prepare(&cch, &graph);
    let capacities = CchQueryScratch::capacities;
    let prepared = capacities(&scratch);
    let mut paths = CchPathScratch::default();
    paths.prepare(&cch, &graph);
    let path_capacities = CchPathScratch::capacities;
    let path_prepared = path_capacities(&paths);
    for multiplier in [1.0, 2.0] {
        for index in 0..graph.edge_count() {
            graph.edge_mut(index).base_cost *= multiplier;
        }
        cch.customize(&graph);
        for from in 0..graph.node_count() as u32 {
            for to in 0..graph.node_count() as u32 {
                for mode in [TransitFlags::CAR, TransitFlags::FOOT] {
                    let expected = cch.find_path(from, to, usize::MAX, usize::MAX, &graph, mode);
                    assert_eq!(
                        cch.find_cost(from, to, usize::MAX, usize::MAX, &graph, mode, &mut scratch),
                        expected.as_ref().map(|route| route.0)
                    );
                    assert_eq!(
                        cch.find_path_in(
                            from,
                            to,
                            usize::MAX,
                            usize::MAX,
                            &graph,
                            mode,
                            &mut paths
                        ),
                        expected
                            .as_ref()
                            .map(|route| (route.0, route.1, route.2.as_slice()))
                    );
                    assert_eq!(path_capacities(&paths), path_prepared);
                    assert_eq!(capacities(paths.query()), prepared);
                    assert_eq!(
                        capacities(&scratch),
                        prepared,
                        "prepared searches must retain their storage across distinct endpoint pairs"
                    );
                }
            }
        }
    }
    let mut fork = scratch.clone();
    assert_eq!(capacities(&fork), prepared);
    assert_eq!(
        cch.find_cost(
            0,
            24,
            usize::MAX,
            usize::MAX,
            &graph,
            TransitFlags::CAR,
            &mut fork
        ),
        cch.find_path(0, 24, usize::MAX, usize::MAX, &graph, TransitFlags::CAR)
            .map(|route| route.0)
    );
    assert_eq!(capacities(&fork), prepared);
    // Epoch rollover must clear old labels rather than making the first query's records
    // visible again. Nonincident and invalid incoming edges retain the ordinary origin rule.
    fork.roll_epochs_over();
    for incoming in [usize::MAX, usize::MAX - 1, 0] {
        assert_eq!(
            cch.find_cost(
                24,
                0,
                incoming,
                usize::MAX,
                &graph,
                TransitFlags::CAR,
                &mut fork
            ),
            turn_aware_dijkstra(&graph, 24, 0, incoming)
        );
        assert_eq!(capacities(&fork), prepared);
    }
}

// Commute quotes use the cost-only query; random one-ways, costs and turn whitelists must
// never make it disagree with the materialized path or the turn-aware oracle.
#[test]
fn cost_only_queries_match_paths_on_random_restricted_graphs() {
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = |bound: u64| {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state % bound
    };
    let mut scratch = CchQueryScratch::default();
    let mut paths = CchPathScratch::default();
    for seed in 0..24 {
        let side = 3 + (seed % 3) as u32;
        let mut graph = grid(side);
        for edge in graph.edges_iter_mut() {
            edge.base_cost = 1.0 + next(12) as f32;
            if next(5) == 0 {
                edge.bkw_lanes = 0;
            }
            if next(2) == 0 {
                edge.allowed_types |= TransitFlags::FOOT;
            }
        }
        for node in 0..graph.node_count() as u32 {
            let arms = graph.node_adjacency(node).to_vec();
            if arms.len() < 3 || next(3) != 0 {
                continue;
            }
            let incoming = arms[next(arms.len() as u64) as usize];
            for &outgoing in &arms {
                if outgoing != incoming && next(2) == 0 {
                    graph.add_lane_connection(node, incoming, 0, outgoing, 0);
                }
            }
        }
        let mut cch = CchGraph::build(&graph);
        for phase in 0..2 {
            if phase == 1 {
                for edge in graph.edges_iter_mut() {
                    edge.base_cost = 1.0 + next(12) as f32;
                }
                cch.customize(&graph);
            }
            for from in 0..graph.node_count() as u32 {
                for to in 0..graph.node_count() as u32 {
                    for start_edge in graph
                        .node_adjacency(from)
                        .iter()
                        .copied()
                        .chain([usize::MAX])
                    {
                        for mode in [TransitFlags::CAR, TransitFlags::FOOT] {
                            let cost = cch.find_cost(
                                from,
                                to,
                                start_edge,
                                usize::MAX,
                                &graph,
                                mode,
                                &mut scratch,
                            );
                            let path = cch
                                .find_path_in(
                                    from,
                                    to,
                                    start_edge,
                                    usize::MAX,
                                    &graph,
                                    mode,
                                    &mut paths,
                                )
                                .map(|route| route.0);
                            assert_eq!(
                                cost, path,
                                "seed={seed} phase={phase} {from}->{to} edge={start_edge} mode={mode}"
                            );
                            if mode == TransitFlags::CAR {
                                assert_eq!(
                                    cost,
                                    turn_aware_dijkstra(&graph, from, to, start_edge),
                                    "seed={seed} phase={phase} {from}->{to} edge={start_edge}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn filled_neighbors_are_ancestors_and_form_cliques() {
    for variant in [false, true] {
        let mut graph = grid(8);
        if variant {
            for node in (0..graph.node_count() as u32).step_by(3) {
                let edges = graph.node_adjacency(node);
                graph.add_lane_connection(node, edges[0], 0, *edges.last().unwrap(), 0);
            }
        }
        let cch = CchGraph::build(&graph);
        for low in 0..cch.parent.len() {
            let neighbors = &cch.up[low];
            for (i, &(high, _)) in neighbors.iter().enumerate() {
                assert!(high as usize > low);
                let mut ancestor = cch.parent[low];
                while ancestor != u32::MAX && ancestor != high {
                    ancestor = cch.parent[ancestor as usize];
                }
                assert_eq!(ancestor, high);
                for &(other, _) in &neighbors[i + 1..] {
                    assert!(cch.up[high as usize].iter().any(|&(node, _)| node == other));
                }
            }
        }
    }
}

#[test]
fn restricted_origin_uses_customized_metric_until_next_customization() {
    let mut graph = grid(2);
    for edge in graph.edges_iter_mut() {
        edge.allowed_types |= TransitFlags::FOOT;
    }
    graph.add_lane_connection(0, 0, 0, 1, 0);
    let mut cch = CchGraph::build(&graph);
    let walk_before = cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::FOOT);
    let before = cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::CAR);
    graph.edge_mut(1).base_cost *= 3.0;
    assert_eq!(
        cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::CAR),
        before
    );
    assert_eq!(
        cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::FOOT),
        walk_before
    );
    cch.customize(&graph);
    assert!(
        cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::FOOT)
            .unwrap()
            .0
            > walk_before.unwrap().0
    );
    assert_eq!(
        cch.find_path(0, 2, 0, usize::MAX, &graph, TransitFlags::CAR)
            .unwrap()
            .0,
        before.unwrap().0 * 3.0
    );
}

#[test]
fn restricted_detour_can_return_to_a_junction_with_a_different_arrival_edge() {
    let mut graph = RegionGraph::new();
    for _ in 0..4 {
        graph.add_node(Vector3::ZERO, NodeType::Junction);
    }
    for (start_node, end_node, base_cost) in [(0, 1, 1.0), (1, 2, 2.0), (1, 3, 4.0)] {
        graph.add_edge(Edge {
            start_node,
            end_node,
            base_cost,
            physical_length: base_cost,
            fwd_lanes: 1,
            bkw_lanes: 1,
            allowed_types: TransitFlags::CAR,
            ..Default::default()
        });
    }
    graph.add_lane_connection(1, 0, 0, 1, 0);
    graph.add_lane_connection(1, 1, 0, 2, 0);
    graph.rebuild_adjacency_list();
    let cch = CchGraph::build(&graph);
    assert_eq!(turn_aware_dijkstra(&graph, 0, 3, usize::MAX), Some(9.0));
    assert_eq!(
        cch.find_path(0, 3, usize::MAX, usize::MAX, &graph, TransitFlags::CAR),
        Some((9.0, 9.0, vec![0, 1, 2, 1, 3]))
    );
    assert_eq!(
        cch.costs_from_each(&[0], &graph, TransitFlags::CAR)[0].cost_to(3),
        Some(9.0)
    );
}
