// SPDX-License-Identifier: GPL-2.0-only

//! Independent f64 Dijkstra and concrete-edge witnesses for turn-state CCH queries.

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector3;
use rand::{Rng, SeedableRng, rngs::StdRng};

fn oracle(graph: &RegionGraph, start: u32, end: u32, incoming: usize, mask: u8) -> Option<f64> {
    exit_oracle(graph, start, end, incoming, usize::MAX, mask)
}

fn exit_oracle(
    graph: &RegionGraph,
    start: u32,
    end: u32,
    incoming: usize,
    outgoing: usize,
    mask: u8,
) -> Option<f64> {
    // Deliberately slow O(E²) Dijkstra on original directed-road arrival states. Neither
    // hierarchy arcs, the elimination order, cached turn tables nor f32 shortcut sums enter it.
    // An arrival at `end` counts only if a vehicle may turn from it onto `outgoing`.
    let root = graph.edge_count() * 2;
    let mut costs = vec![f64::INFINITY; root + 1];
    let mut settled = vec![false; root + 1];
    costs[root] = 0.0;
    loop {
        let next = costs
            .iter()
            .enumerate()
            .filter(|&(i, c)| !settled[i] && c.is_finite())
            .min_by(|a, b| a.1.total_cmp(b.1).then(a.0.cmp(&b.0)));
        let (state, &cost) = next?;
        let (node, in_edge) = if state == root {
            (start, incoming)
        } else {
            let edge = graph.edge(state / 2);
            (
                if state % 2 == 0 {
                    edge.start_node
                } else {
                    edge.end_node
                },
                state / 2,
            )
        };
        if node == end
            && (mask == TransitFlags::FOOT
                || in_edge == usize::MAX
                || outgoing == usize::MAX
                || CchGraph::vehicle_turn_allowed(graph, end, in_edge, outgoing))
        {
            return Some(cost);
        }
        settled[state] = true;
        for &id in graph.node_adjacency(node) {
            let edge = graph.edge(id);
            if edge.deleted {
                continue;
            }
            if mask != TransitFlags::FOOT
                && in_edge != usize::MAX
                && !CchGraph::vehicle_turn_allowed(graph, node, in_edge, id)
            {
                continue;
            }
            for (from, to, lanes) in [
                (edge.start_node, edge.end_node, edge.fwd_lanes),
                (edge.end_node, edge.start_node, edge.bkw_lanes),
            ] {
                let flags = if lanes > 0 {
                    edge.allowed_types
                } else {
                    edge.allowed_types & TransitFlags::FOOT
                };
                if from != node || flags & mask == 0 {
                    continue;
                }
                let target = id * 2 + usize::from(to != edge.start_node);
                costs[target] = costs[target].min(cost + f64::from(edge.base_cost));
            }
        }
    }
}

// Same random graph family as the multimode test below, including whitelists, deleted edges,
// self-loops and one-way roads.
fn random_graph(rng: &mut StdRng) -> RegionGraph {
    let mut graph = RegionGraph::new();
    for node in 0..9 {
        graph.add_node(
            Vector3::new((node % 3) as f32, 0.0, (node / 3) as f32),
            NodeType::Junction,
        );
    }
    for _ in 0..23 {
        graph.add_edge(Edge {
            start_node: rng.gen_range(0..9),
            end_node: rng.gen_range(0..9),
            base_cost: rng.gen_range(0..100) as f32 / 7.0,
            physical_length: rng.gen_range(1..100) as f32 / 3.0,
            allowed_types: rng.gen_range(1..8),
            fwd_lanes: rng.gen_range(0..2),
            bkw_lanes: rng.gen_range(0..2),
            deleted: rng.gen_ratio(1, 12),
            ..Default::default()
        });
    }
    graph.rebuild_adjacency_list();
    for node in 0..9 {
        if rng.gen_ratio(1, 2) {
            let edges = graph.node_adjacency(node).to_vec();
            for &from in &edges {
                for &to in &edges {
                    if rng.gen_ratio(1, 4) {
                        graph.add_lane_connection(node, from, 0, to, 0);
                    }
                }
            }
        }
    }
    graph
}

#[test]
fn exit_constrained_routes_match_dijkstra_in_point_and_bucket_queries() {
    // NET-07: the edge left on after the destination constrains the final turn exactly as
    // the lane builder does (no U-turn away from a dead end; whitelists otherwise).
    let mut rng = StdRng::seed_from_u64(0x4e45543037);
    for seed in 0..24 {
        let graph = random_graph(&mut rng);
        let cch = CchGraph::build(&graph);
        let mut scratch = CchPathScratch::default();
        for mask in [
            TransitFlags::CAR,
            TransitFlags::FOOT | TransitFlags::CAR,
            TransitFlags::FOOT,
        ] {
            let mut targets = Vec::new();
            for end in 0..9 {
                targets.push((end, usize::MAX));
                targets.extend(graph.node_adjacency(end).iter().map(|&edge| (end, edge)));
            }
            let buckets = CchBuckets::build(&cch, &targets, mask);
            let mut source = CchBucketSource::default();
            let mut batch_path = CchPathScratch::default();
            for start in 0..9 {
                let arms = graph.node_adjacency(start).to_vec();
                for incoming in [usize::MAX].into_iter().chain(arms) {
                    buckets.query_source(&cch, start, incoming, &mut source);
                    for &(end, outgoing) in &targets {
                        let expected = exit_oracle(&graph, start, end, incoming, outgoing, mask);
                        let actual = cch
                            .find_path_in(
                                start,
                                end,
                                incoming,
                                outgoing,
                                &graph,
                                mask,
                                &mut scratch,
                            )
                            .map(|(cost, _, nodes)| (cost, nodes.to_vec()));
                        let context = format!(
                            "seed={seed} mask={mask} {start}->{end} in={incoming} out={outgoing}"
                        );
                        assert_eq!(actual.is_some(), expected.is_some(), "{context}");
                        let Some((cost, nodes)) = actual else {
                            continue;
                        };
                        let expected = expected.unwrap();
                        assert!(
                            (f64::from(cost) - expected).abs() <= 2e-5 * expected.max(1.0),
                            "{context}: {cost} != {expected}"
                        );
                        // Every turn of the witness is legal, including both trip ends.
                        let mut previous = incoming;
                        let mut node = start;
                        for &edge in scratch.selected_edges() {
                            assert!(
                                mask == TransitFlags::FOOT
                                    || previous == usize::MAX
                                    || CchGraph::vehicle_turn_allowed(&graph, node, previous, edge),
                                "{context}: illegal turn at {node}"
                            );
                            let value = graph.edge(edge);
                            node = if value.start_node == node {
                                value.end_node
                            } else {
                                value.start_node
                            };
                            previous = edge;
                        }
                        assert_eq!(node, end, "{context}");
                        assert!(
                            mask == TransitFlags::FOOT
                                || previous == usize::MAX
                                || outgoing == usize::MAX
                                || CchGraph::vehicle_turn_allowed(&graph, end, previous, outgoing),
                            "{context}: illegal exit turn"
                        );
                        let index = buckets.target_index(&cch, end, outgoing).unwrap();
                        let batch = buckets
                            .path_in(index, &cch, &graph, &source, &mut batch_path)
                            .map(|(cost, nodes)| (cost.to_bits(), nodes.to_vec()));
                        assert_eq!(batch, Some((cost.to_bits(), nodes)), "{context}: bucket");
                    }
                }
            }
        }
    }
}

#[test]
fn fractional_multimode_routes_match_independent_dijkstra_and_edge_witnesses() {
    let mut rng = StdRng::seed_from_u64(0x4e45543033);
    let modes = [
        0,
        TransitFlags::FOOT,
        TransitFlags::CAR,
        TransitFlags::RAIL,
        TransitFlags::FOOT | TransitFlags::CAR,
        TransitFlags::CAR | TransitFlags::RAIL,
        31,
    ];
    for seed in 0..16 {
        let mut graph = random_graph(&mut rng);
        let mut cch = CchGraph::build(&graph);
        let mut cost_bounds = CchCarCostBounds::default();
        let mut scratch = CchPathScratch::default();
        scratch.prepare(&cch, &graph);
        for phase in 0..2 {
            if phase == 1 {
                for edge in graph.edges_iter_mut() {
                    edge.base_cost *= 1.317;
                }
                cch.customize(&graph);
            }
            cost_bounds.prepare(&cch, &graph);
            for mask in modes {
                let target_nodes: Vec<_> = (0..9).map(|node| (node, usize::MAX)).collect();
                let targets = CchBuckets::build(&cch, &target_nodes, mask);
                let mut source = CchBucketSource::default();
                source.prepare(&targets, &cch);
                let capacity = source.capacities();
                let mut batch_path = CchPathScratch::default();
                batch_path.prepare(&cch, &graph);
                let all = cch.costs_from_each(&(0..9).collect::<Vec<_>>(), &graph, mask);
                let restricted = RestrictedTargets::build(&cch, &[0, 2, 5, 8]);
                let mut restricted_scratch = RestrictedScratch::default();
                restricted.prepare(&cch, &mut restricted_scratch);
                let restricted_capacity = restricted_scratch.capacities();
                let mut blocked = RestrictedBatchScratch::default();
                restricted.prepare_batch(&cch, &mut blocked);
                let blocked_capacity = blocked.capacities();
                let starts: Vec<_> = (0..9)
                    .map(|node| {
                        (
                            node,
                            match node % 3 {
                                0 => usize::MAX,
                                1 => graph
                                    .node_adjacency(node)
                                    .first()
                                    .copied()
                                    .unwrap_or(usize::MAX - 1),
                                _ => usize::MAX - 1,
                            },
                        )
                    })
                    .collect();
                for starts in starts.chunks(RESTRICTED_BATCH_LANES) {
                    let costs = restricted.query_batch(&cch, starts, mask, &mut blocked);
                    for (lane, &(start, incoming)) in starts.iter().enumerate() {
                        for (index, &cost) in restricted
                            .query(&cch, start, incoming, mask, &mut restricted_scratch)
                            .iter()
                            .enumerate()
                        {
                            assert_eq!(
                                costs[index][lane].to_bits(),
                                cost.to_bits(),
                                "blocked/scalar PHAST mismatch"
                            );
                        }
                    }
                    assert_eq!(blocked.capacities(), blocked_capacity);
                }
                for start in 0..9 {
                    for (&target, &cost) in restricted.targets().iter().zip(restricted.query(
                        &cch,
                        start,
                        usize::MAX,
                        mask,
                        &mut restricted_scratch,
                    )) {
                        assert_eq!(
                            cost.to_bits(),
                            all[start as usize]
                                .cost_to(target)
                                .unwrap_or(f32::INFINITY)
                                .to_bits(),
                            "restricted/full PHAST mismatch"
                        );
                    }
                    for end in 0..9 {
                        let arrival = graph
                            .node_adjacency(start)
                            .first()
                            .copied()
                            .unwrap_or(usize::MAX - 1);
                        for incoming in [usize::MAX, arrival, usize::MAX - 1] {
                            let expected = oracle(&graph, start, end, incoming, mask);
                            if let Ok(index) = restricted.targets().binary_search(&end) {
                                let cost = restricted.query(
                                    &cch,
                                    start,
                                    incoming,
                                    mask,
                                    &mut restricted_scratch,
                                )[index];
                                assert_eq!(restricted_scratch.capacities(), restricted_capacity);
                                assert_eq!(cost.is_finite(), expected.is_some());
                                if let Some(expected) = expected {
                                    assert!(
                                        (f64::from(cost) - expected).abs()
                                            <= 2e-5 * expected.max(1.0)
                                    );
                                }
                            }
                            let actual = cch
                                .find_path_in(
                                    start,
                                    end,
                                    incoming,
                                    usize::MAX,
                                    &graph,
                                    mask,
                                    &mut scratch,
                                )
                                .map(|(c, d, _)| (c, d));
                            let mut fresh = CchPathScratch::default();
                            let uncached = cch
                                .find_path_in(
                                    start,
                                    end,
                                    incoming,
                                    usize::MAX,
                                    &graph,
                                    mask,
                                    &mut fresh,
                                )
                                .map(|(cost, distance, nodes)| {
                                    (cost.to_bits(), distance.to_bits(), nodes)
                                });
                            assert_eq!(
                                uncached,
                                actual.map(|(cost, distance)| (
                                    cost.to_bits(),
                                    distance.to_bits(),
                                    scratch.nodes()
                                )),
                                "retained endpoint labels changed costs or witnesses"
                            );
                            assert_eq!(fresh.selected_edges(), scratch.selected_edges());
                            if mask == TransitFlags::CAR
                                && let Some((cost, _)) = actual
                            {
                                assert!(
                                    cost_bounds.cost(start, end, &cch).unwrap() <= cost,
                                    "integer potential exceeded rounded car query"
                                );
                            }
                            targets.query_source(&cch, start, incoming, &mut source);
                            let batch = targets.path_in(
                                end as usize,
                                &cch,
                                &graph,
                                &source,
                                &mut batch_path,
                            );
                            assert_eq!(
                                batch,
                                actual.map(|(cost, _)| (cost, scratch.nodes())),
                                "bucket witness differs"
                            );
                            let normalized = cch
                                .find_path_in(
                                    start,
                                    end,
                                    cch.source_context(start, incoming, mask),
                                    usize::MAX,
                                    &graph,
                                    mask,
                                    &mut batch_path,
                                )
                                .map(|(cost, distance, path)| {
                                    (cost.to_bits(), distance.to_bits(), path)
                                });
                            assert_eq!(
                                normalized,
                                actual.map(|(cost, distance)| (
                                    cost.to_bits(),
                                    distance.to_bits(),
                                    scratch.nodes()
                                )),
                                "canonical source context changed the exact query"
                            );
                            assert_eq!(source.capacities(), capacity);
                            assert_eq!(
                                actual.is_some(),
                                expected.is_some(),
                                "seed={seed}, phase={phase}, {start}->{end}, incoming={incoming}, mask={mask}"
                            );
                            if let (Some((cost, distance)), Some(expected)) = (actual, expected) {
                                let tolerance = 2e-5 * expected.max(1.0);
                                assert!(
                                    (f64::from(cost) - expected).abs() <= tolerance,
                                    "seed={seed}, phase={phase}, {start}->{end}, incoming={incoming}, mask={mask}: {cost} != {expected}"
                                );
                                if incoming == usize::MAX {
                                    assert!(
                                        (f64::from(all[start as usize].cost_to(end).unwrap())
                                            - expected)
                                            .abs()
                                            <= tolerance
                                    );
                                }
                                let mut node = start;
                                let mut previous = incoming;
                                let mut sum_cost = 0.0_f64;
                                let mut sum_distance = 0.0_f64;
                                for &id in scratch.selected_edges() {
                                    let edge = graph.edge(id);
                                    assert!(!edge.deleted);
                                    assert!(
                                        mask == TransitFlags::FOOT
                                            || previous == usize::MAX
                                            || CchGraph::vehicle_turn_allowed(
                                                &graph, node, previous, id
                                            )
                                    );
                                    let forward = edge.start_node == node;
                                    assert!(forward || edge.end_node == node);
                                    let legal = edge.traversal_flags(forward) & mask != 0
                                        || (edge.start_node == edge.end_node
                                            && edge.traversal_flags(!forward) & mask != 0);
                                    assert!(legal);
                                    node = if forward {
                                        edge.end_node
                                    } else {
                                        edge.start_node
                                    };
                                    previous = id;
                                    sum_cost += f64::from(edge.base_cost);
                                    sum_distance += f64::from(edge.physical_length);
                                }
                                assert_eq!(node, end);
                                assert!((sum_cost - f64::from(cost)).abs() <= tolerance);
                                assert!(
                                    (sum_distance - f64::from(distance)).abs()
                                        <= 2e-5 * sum_distance.max(1.0)
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
