// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic nested dissection using exact separators of the road adjacency graph.

use crate::simulation::network::graph::RegionGraph;
use std::collections::HashSet;

/// Orders balanced disconnected halves before their separating vertices.
/// Construction takes O((N log N + E) log N); it does not approximate route costs.
pub(super) fn nested_dissection(graph: &RegionGraph) -> (Vec<u32>, Vec<u32>) {
    let mut adjacency = vec![Vec::new(); graph.node_count()];
    for edge in graph
        .edges()
        .iter()
        .filter(|e| !e.deleted && e.start_node != e.end_node)
    {
        adjacency[edge.start_node as usize].push(edge.end_node);
        adjacency[edge.end_node as usize].push(edge.start_node);
    }
    fn dissect(mut nodes: Vec<u32>, graph: &RegionGraph, adjacency: &[Vec<u32>]) -> Vec<u32> {
        if nodes.len() <= 8 {
            nodes.sort_unstable_by_key(|&n| (adjacency[n as usize].len(), n));
            return nodes;
        }
        // Median coordinate cuts only choose an ordering. Every crossing edge is covered
        // by the separator; no edge or path is discarded. Both surviving halves have at
        // most ceil(n/2) vertices, even on coincident coordinates or nonplanar road graphs.
        let mut best = None;
        for x_axis in [true, false] {
            let coordinate = |n| {
                let pos = graph.node(n).pos;
                if x_axis { pos.x } else { pos.z }
            };
            nodes
                .sort_unstable_by(|&a, &b| coordinate(a).total_cmp(&coordinate(b)).then(a.cmp(&b)));
            let middle = nodes.len() / 2;
            let left: HashSet<_> = nodes[..middle].iter().copied().collect();
            let right: HashSet<_> = nodes[middle..].iter().copied().collect();
            let boundary_left: HashSet<_> = left
                .iter()
                .copied()
                .filter(|&u| adjacency[u as usize].iter().any(|v| right.contains(v)))
                .collect();
            let boundary_right: HashSet<_> = right
                .iter()
                .copied()
                .filter(|&u| adjacency[u as usize].iter().any(|v| left.contains(v)))
                .collect();
            let boundary = if boundary_left.len() <= boundary_right.len() {
                boundary_left
            } else {
                boundary_right
            };
            if best
                .as_ref()
                .is_none_or(|(size, _, _, _)| boundary.len() < *size)
            {
                let mut a = Vec::new();
                let mut b = Vec::new();
                let mut separator = Vec::new();
                for &node in &nodes {
                    if boundary.contains(&node) {
                        separator.push(node);
                    } else if left.contains(&node) {
                        a.push(node);
                    } else {
                        b.push(node);
                    }
                }
                best = Some((boundary.len(), a, b, separator));
            }
        }
        let (_, a, b, separator) = best.unwrap();
        // Independent subgraphs may build in parallel; output order never depends on scheduling.
        let (mut a, b) = if nodes.len() >= 256 {
            rayon::join(
                || dissect(a, graph, adjacency),
                || dissect(b, graph, adjacency),
            )
        } else {
            (dissect(a, graph, adjacency), dissect(b, graph, adjacency))
        };
        a.extend(b);
        a.extend(dissect(separator, graph, adjacency));
        a
    }
    let order = dissect((0..graph.node_count() as u32).collect(), graph, &adjacency);
    let mut rank = vec![0; order.len()];
    for (i, &node) in order.iter().enumerate() {
        rank[node as usize] = i as u32;
    }
    (order, rank)
}
