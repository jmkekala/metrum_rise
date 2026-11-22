// SPDX-License-Identifier: GPL-3.0-or-later

//! Metric customization regressions: the free-flow profiles' exact bits and the car-congestion
//! profile (`TRAFFIC-17`).

use super::*;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector3;

// A grid with varied costs, one-way roads, mixed walking roads and, with `whitelist`, one
// whitelisted junction.
fn varied_grid(side: u32, whitelist: bool) -> RegionGraph {
    let mut graph = RegionGraph::new();
    for node in 0..side * side {
        graph.add_node(
            Vector3::new(
                (node % side) as f32 * 80.0,
                0.0,
                (node / side) as f32 * 80.0,
            ),
            NodeType::Junction,
        );
    }
    let mut id = 0_u32;
    for node in 0..side * side {
        for target in [
            ((node % side) < side - 1).then_some(node + 1),
            (node < side * (side - 1)).then_some(node + side),
        ]
        .into_iter()
        .flatten()
        {
            graph.add_edge(Edge {
                start_node: node,
                end_node: target,
                base_cost: 5.0 + (id % 11) as f32 * 0.37,
                physical_length: 80.0 + (id % 5) as f32,
                allowed_types: if id.is_multiple_of(3) {
                    TransitFlags::CAR | TransitFlags::FOOT
                } else {
                    TransitFlags::CAR
                },
                sidewalks: crate::simulation::network::graph::SidewalkSides::BOTH,
                fwd_lanes: 1,
                bkw_lanes: u8::from(!id.is_multiple_of(5)),
                ..Default::default()
            });
            id += 1;
        }
    }
    if whitelist {
        let node = side / 2 * side + side / 2;
        let incoming = graph.node_adjacency(node)[0];
        let outgoing = *graph.node_adjacency(node).last().unwrap();
        graph.add_lane_connection(node, incoming, 0, outgoing, 0);
    }
    graph.rebuild_adjacency_list();
    graph
}

// FNV-1a over every profile's cost, distance and witness, in profile and arc order.
fn profile_fingerprint(cch: &CchGraph) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut add = |value: u64| {
        for byte in value.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for metric in &cch.profiles {
        add(u64::from(metric.mask));
        for arc in 0..metric.costs.len() {
            let witness = metric.witnesses[arc];
            add(u64::from(metric.costs[arc].to_bits()));
            add(u64::from(metric.distances[arc].to_bits()));
            add(witness.left as u64);
            add(witness.right as u64);
            add(witness.edge as u64);
        }
    }
    hash
}

#[test]
fn free_flow_profiles_keep_their_bits() {
    let fingerprints: Vec<_> = [6, 11, 17]
        .into_iter()
        .map(|side| profile_fingerprint(&CchGraph::build(&varied_grid(side, true))))
        .collect();
    // Recorded on `dev` 46aea5b1, before the shared customization (`TRAFFIC-17` round T1), as
    // [0xc8cf762c8a2180e8, 0x679ba1e8f2582fec, 0x5aa319f9c843addd]. Recorded again when the
    // walking speed became 4 m/s (`TRAFFIC-41`): hashed profile by profile at both speeds, only
    // the `FOOT` profile's bits differ on each grid, whose costs are lengths over that speed.
    assert_eq!(
        fingerprints,
        [0x112e690d75d0d533, 0xcd9b80ec01ae38ad, 0xacd2eeb0ffec29ba]
    );
}

// Directed weights: each edge's free-flow cost plus `delay(edge, forward)`.
fn weights(graph: &RegionGraph, delay: impl Fn(usize, bool) -> f32) -> Vec<f32> {
    let mut weights = Vec::with_capacity(graph.edge_count() * 2);
    for (id, edge) in graph.edges().iter().enumerate() {
        weights.push(edge.base_cost + delay(id, true));
        weights.push(edge.base_cost + delay(id, false));
    }
    weights
}

fn adopt(cch: &mut CchGraph, weights: Vec<f32>) {
    let metric = cch
        .congestion_customizer()
        .unwrap()
        .customize(weights, None);
    assert!(cch.adopt_congestion(metric).is_none());
}

type Route = Option<(u32, u32, Vec<u32>)>;

// Cost bits, distance bits and nodes of a spread of queries, from free and restricted origins.
fn routes(cch: &CchGraph, graph: &RegionGraph, mask: u8, route: RouteMetric) -> Vec<Route> {
    let mut scratch = CchPathScratch::default();
    let nodes = graph.node_count() as u32;
    let mut out = Vec::new();
    for start in (0..nodes).step_by(3) {
        for end in (1..nodes).step_by(5) {
            for incoming in [usize::MAX, graph.node_adjacency(start)[0]] {
                out.push(
                    cch.find_path_in(
                        start,
                        end,
                        incoming,
                        usize::MAX,
                        graph,
                        mask,
                        route,
                        &mut scratch,
                        None,
                    )
                    .map(|(cost, distance, nodes)| {
                        (cost.to_bits(), distance.to_bits(), nodes.to_vec())
                    }),
                );
            }
        }
    }
    out
}

#[test]
fn zero_delay_congestion_is_free_flow_bit_for_bit() {
    let graph = varied_grid(11, true);
    let mut cch = CchGraph::build(&graph);
    let car = routes(&cch, &graph, TransitFlags::CAR, RouteMetric::FreeFlow);
    let foot = routes(&cch, &graph, TransitFlags::FOOT, RouteMetric::FreeFlow);
    // Before any adoption a congested query is a free-flow one.
    assert_eq!(
        routes(&cch, &graph, TransitFlags::CAR, RouteMetric::Congested),
        car
    );
    let (generation, metric) = (cch.build_generation, cch.metric_revision);
    adopt(&mut cch, weights(&graph, |_, _| 0.0));
    let free_car = &cch.profiles[cch.profile_for_mask[TransitFlags::CAR as usize] as usize];
    let congested = &cch.congested.as_ref().unwrap().metric;
    assert_eq!(congested.mask, free_car.mask);
    assert!((0..free_car.costs.len()).all(|arc| {
        let (a, b) = (free_car.witnesses[arc], congested.witnesses[arc]);
        free_car.costs[arc].to_bits() == congested.costs[arc].to_bits()
            && free_car.distances[arc].to_bits() == congested.distances[arc].to_bits()
            && (a.left, a.right, a.edge) == (b.left, b.right, b.edge)
    }));
    assert_eq!(
        routes(&cch, &graph, TransitFlags::CAR, RouteMetric::Congested),
        car
    );
    // Walkers never read the car metric; adoption moves no free-flow revision.
    assert_eq!(
        routes(&cch, &graph, TransitFlags::FOOT, RouteMetric::Congested),
        foot
    );
    assert_eq!(
        (cch.build_generation, cch.metric_revision),
        (generation, metric)
    );
    assert_eq!(cch.congestion_revision, 1);
}

#[test]
fn congested_routes_equal_a_hierarchy_built_on_the_delayed_costs() {
    let graph = varied_grid(11, true);
    let delay = |edge: usize| (edge % 7) as f32 * 1.3;
    let mut cch = CchGraph::build(&graph);
    let free = routes(&cch, &graph, TransitFlags::CAR, RouteMetric::FreeFlow);
    adopt(&mut cch, weights(&graph, |edge, _| delay(edge)));
    let mut delayed = graph.clone();
    for (id, edge) in delayed.edges_iter_mut().enumerate() {
        edge.base_cost += delay(id);
    }
    let oracle = CchGraph::build(&delayed);
    let congested = routes(&cch, &graph, TransitFlags::CAR, RouteMetric::Congested);
    assert_eq!(
        congested,
        routes(&oracle, &delayed, TransitFlags::CAR, RouteMetric::FreeFlow)
    );
    assert_ne!(congested, free, "the delays must move some route");
    assert_eq!(
        routes(&cch, &graph, TransitFlags::CAR, RouteMetric::FreeFlow),
        free
    );
}

#[test]
fn a_delay_counts_only_in_its_direction() {
    // No whitelist: every shortest route from a free origin is a simple path, so a node
    // Dijkstra over the directed weights is the oracle.
    let graph = varied_grid(9, false);
    let mut cch = CchGraph::build(&graph);
    let weights = weights(&graph, |edge, forward| {
        if forward {
            (edge % 7) as f32 * 1.3
        } else {
            (edge % 5) as f32 * 4.1
        }
    });
    adopt(&mut cch, weights.clone());
    let nodes = graph.node_count();
    let mut scratch = CchPathScratch::default();
    for start in 0..nodes as u32 {
        let mut best = vec![f64::INFINITY; nodes];
        let mut done = vec![false; nodes];
        best[start as usize] = 0.0;
        while let Some(at) = (0..nodes)
            .filter(|&n| !done[n] && best[n].is_finite())
            .min_by(|&a, &b| best[a].total_cmp(&best[b]))
        {
            done[at] = true;
            for &id in graph.node_adjacency(at as u32) {
                let edge = graph.edge(id);
                let forward = edge.start_node as usize == at;
                if edge.traversal_flags(forward) & TransitFlags::CAR == 0 {
                    continue;
                }
                let next = if forward {
                    edge.end_node
                } else {
                    edge.start_node
                } as usize;
                let cost = best[at] + f64::from(weights[2 * id + usize::from(!forward)]);
                best[next] = best[next].min(cost);
            }
        }
        for end in 0..nodes as u32 {
            let found = cch.find_path_in(
                start,
                end,
                usize::MAX,
                usize::MAX,
                &graph,
                TransitFlags::CAR,
                RouteMetric::Congested,
                &mut scratch,
                None,
            );
            match found {
                Some((cost, _, path)) => {
                    let expected = best[end as usize];
                    assert!((f64::from(cost) - expected).abs() <= 1e-3 * expected.max(1.0));
                    // The returned nodes price at the same directed weights.
                    let walked: f64 = path
                        .windows(2)
                        .map(|pair| {
                            let id = graph.get_edge_between_nodes(pair[0], pair[1]).unwrap();
                            let forward = graph.edge(id).start_node == pair[0];
                            f64::from(weights[2 * id + usize::from(!forward)])
                        })
                        .sum();
                    assert!((walked - expected).abs() <= 1e-3 * expected.max(1.0));
                }
                None => assert!(best[end as usize].is_infinite()),
            }
        }
    }
}

#[test]
fn retained_sweeps_of_one_metric_never_answer_the_other() {
    let graph = varied_grid(11, true);
    let mut cch = CchGraph::build(&graph);
    adopt(&mut cch, weights(&graph, |edge, _| (edge % 3) as f32 * 7.0));
    let mut shared = CchPathScratch::default();
    for end in [7, 40, 99, 120] {
        for route in [
            RouteMetric::FreeFlow,
            RouteMetric::Congested,
            RouteMetric::FreeFlow,
        ] {
            let query = |scratch: &mut CchPathScratch| {
                cch.find_path_in(
                    3,
                    end,
                    usize::MAX,
                    usize::MAX,
                    &graph,
                    TransitFlags::CAR,
                    route,
                    scratch,
                    None,
                )
                .map(|(cost, distance, nodes)| (cost.to_bits(), distance.to_bits(), nodes.to_vec()))
            };
            assert_eq!(query(&mut shared), query(&mut CchPathScratch::default()));
        }
    }
}

#[test]
fn a_rebuild_supersedes_a_metric_customized_before_it() {
    let graph = varied_grid(6, true);
    let mut cch = CchGraph::build(&graph);
    let customizer = cch.congestion_customizer().unwrap();
    let stale = customizer.customize(weights(&graph, |_, _| 1.0), None);
    cch = CchGraph::build(&graph);
    assert!(cch.adopt_congestion(stale).is_some());
    assert!(cch.congested.is_none());
    assert_eq!(cch.congestion_revision, 0);
}

#[test]
fn customizing_into_a_displaced_metric_reuses_its_storage() {
    let graph = varied_grid(11, true);
    let mut cch = CchGraph::build(&graph);
    let customizer = cch.congestion_customizer().unwrap();
    adopt(&mut cch, weights(&graph, |_, _| 0.0));
    let next = customizer.customize(weights(&graph, |edge, _| edge as f32), None);
    let displaced = cch.adopt_congestion(next).unwrap();
    let storage = (
        displaced.metric.costs.as_ptr(),
        displaced.metric.witnesses.as_ptr(),
    );
    let snapshot = displaced.weights().to_vec();
    let again = customizer.customize(snapshot, Some(displaced));
    assert_eq!(
        (again.metric.costs.as_ptr(), again.metric.witnesses.as_ptr()),
        storage
    );
    // The same weights give the same bits wherever they are customized.
    let fresh = customizer.customize(weights(&graph, |_, _| 0.0), None);
    assert!(
        fresh
            .metric
            .costs
            .iter()
            .zip(&again.metric.costs)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn economy_queries_ignore_an_adopted_congestion_metric() {
    // Cost quotes, one-to-all sweeps and bucket joins (freight, commutes, border arrivals)
    // read the free-flow metric whatever congestion metric is adopted.
    let graph = varied_grid(11, true);
    let mut cch = CchGraph::build(&graph);
    let nodes = graph.node_count() as u32;
    let targets: Vec<_> = (0..nodes)
        .step_by(4)
        .map(|node| (node, usize::MAX))
        .collect();
    let quotes = |cch: &CchGraph| {
        let mut scratch = CchQueryScratch::default();
        let mut quotes = Vec::new();
        for start in (0..nodes).step_by(3) {
            for end in (1..nodes).step_by(5) {
                quotes.push(
                    cch.find_cost(
                        start,
                        end,
                        usize::MAX,
                        usize::MAX,
                        &graph,
                        TransitFlags::CAR,
                        &mut scratch,
                    )
                    .map(f32::to_bits),
                );
            }
        }
        for sweep in cch.costs_from_each(&[0, 17, 60], &graph, TransitFlags::CAR) {
            quotes.extend((0..nodes).map(|node| sweep.cost_to(node).map(f32::to_bits)));
        }
        let buckets = CchBuckets::build(cch, &targets, TransitFlags::CAR);
        let mut source = CchBucketSource::default();
        for start in (0..nodes).step_by(7) {
            buckets.query_source(cch, start, usize::MAX, &mut source);
            quotes.extend((0..targets.len()).map(|t| buckets.cost(t, &source).map(f32::to_bits)));
        }
        quotes
    };
    let before = quotes(&cch);
    let revisions = (cch.build_generation, cch.metric_revision);
    adopt(
        &mut cch,
        weights(&graph, |edge, forward| {
            (edge % 4) as f32 * if forward { 9.0 } else { 3.0 }
        }),
    );
    assert_eq!(quotes(&cch), before);
    assert_eq!((cch.build_generation, cch.metric_revision), revisions);
}
