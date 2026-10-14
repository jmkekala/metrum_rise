// SPDX-License-Identifier: GPL-2.0-only

//! Deterministic nested dissection using exact separators of a planar adjacency graph.

#[cfg(test)]
use crate::simulation::network::graph::RegionGraph;
use std::collections::{HashMap, HashSet};

/// Orders road nodes: balanced disconnected halves before their separating vertices.
/// Construction takes O((N log N + E) log N); it does not approximate route costs.
#[cfg(test)]
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
    let positions: Vec<_> = (0..graph.node_count() as u32)
        .map(|node| {
            let pos = graph.node(node).pos;
            [pos.x, pos.z]
        })
        .collect();
    let order = dissect_points(&positions, &adjacency);
    let mut rank = vec![0; order.len()];
    for (i, &node) in order.iter().enumerate() {
        rank[node as usize] = i as u32;
    }
    (order, rank)
}

/// Orders vertices with planar `positions` and symmetric `adjacency` for contraction:
/// balanced disconnected halves before their separating vertices, recursively.
/// Construction takes O((V log V + A) log V); it does not approximate route costs.
pub(super) fn dissect_points(positions: &[[f32; 2]], adjacency: &[Vec<u32>]) -> Vec<u32> {
    fn dissect(mut nodes: Vec<u32>, positions: &[[f32; 2]], adjacency: &[Vec<u32>]) -> Vec<u32> {
        if nodes.len() <= 8 {
            nodes.sort_unstable_by_key(|&n| (adjacency[n as usize].len(), n));
            return nodes;
        }
        // Median coordinate cuts only choose an ordering. Every crossing edge is covered
        // by the separator; no edge or path is discarded. Both surviving halves have at
        // most ceil(n/2) vertices, even on coincident coordinates or nonplanar road graphs.
        let mut best = None;
        for axis in [0, 1] {
            let coordinate = |n: u32| positions[n as usize][axis];
            nodes
                .sort_unstable_by(|&a, &b| coordinate(a).total_cmp(&coordinate(b)).then(a.cmp(&b)));
            let middle = nodes.len() / 2;
            let left: HashSet<_> = nodes[..middle].iter().copied().collect();
            let right: HashSet<_> = nodes[middle..].iter().copied().collect();
            let boundary = crossing_cover(&nodes, &left, &right, adjacency);
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
        // Flow cuts along four directions: the smallest vertex set separating the first and
        // last quarter of the vertices, kept when smaller (or equally small and better
        // balanced) than the median cover. Every vertex still lands in exactly one part.
        for direction in [[1.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, -1.0]] {
            let projection = |n: u32| {
                positions[n as usize][0] * direction[0] + positions[n as usize][1] * direction[1]
            };
            nodes
                .sort_unstable_by(|&a, &b| projection(a).total_cmp(&projection(b)).then(a.cmp(&b)));
            let limit = best.as_ref().map_or(usize::MAX, |(size, _, _, _)| *size);
            let Some((a, b, separator)) = flow_cut(&nodes, adjacency, limit) else {
                continue;
            };
            let worse = best.as_ref().is_some_and(|(size, a0, b0, _)| {
                (separator.len(), a.len().max(b.len())) >= (*size, a0.len().max(b0.len()))
            });
            if !worse {
                best = Some((separator.len(), a, b, separator));
            }
        }
        let (_, a, b, separator) = best.unwrap();
        // Independent subgraphs may build in parallel; output order never depends on scheduling.
        let (mut a, b) = if nodes.len() >= 256 {
            rayon::join(
                || dissect(a, positions, adjacency),
                || dissect(b, positions, adjacency),
            )
        } else {
            (
                dissect(a, positions, adjacency),
                dissect(b, positions, adjacency),
            )
        };
        a.extend(b);
        a.extend(dissect(separator, positions, adjacency));
        a
    }
    dissect((0..positions.len() as u32).collect(), positions, adjacency)
}

// Smallest vertex set touching every link between the two halves, from a maximum matching of
// the crossing links (König's theorem); never larger than either side's boundary. Augmenting
// paths cost O(C · X) for C crossing vertices and X crossing links. Deterministic: vertices
// are visited in the caller's order and hash sets only answer membership.
fn crossing_cover(
    nodes: &[u32],
    left: &HashSet<u32>,
    right: &HashSet<u32>,
    adjacency: &[Vec<u32>],
) -> HashSet<u32> {
    let crossing: Vec<(u32, Vec<u32>)> = nodes
        .iter()
        .copied()
        .filter(|node| left.contains(node))
        .filter_map(|u| {
            let targets: Vec<_> = adjacency[u as usize]
                .iter()
                .copied()
                .filter(|v| right.contains(v))
                .collect();
            (!targets.is_empty()).then_some((u, targets))
        })
        .collect();
    fn augment(
        i: usize,
        crossing: &[(u32, Vec<u32>)],
        matched_left: &mut [u32],
        matched_right: &mut HashMap<u32, usize>,
        seen: &mut HashSet<u32>,
    ) -> bool {
        for &v in &crossing[i].1 {
            if !seen.insert(v) {
                continue;
            }
            let free = match matched_right.get(&v) {
                None => true,
                Some(&j) => augment(j, crossing, matched_left, matched_right, seen),
            };
            if free {
                matched_right.insert(v, i);
                matched_left[i] = v;
                return true;
            }
        }
        false
    }
    let mut matched_left = vec![u32::MAX; crossing.len()];
    let mut matched_right = HashMap::new();
    let mut seen = HashSet::new();
    for i in 0..crossing.len() {
        seen.clear();
        augment(
            i,
            &crossing,
            &mut matched_left,
            &mut matched_right,
            &mut seen,
        );
    }
    // Alternate from unmatched left vertices: unmatched links rightwards, matched leftwards.
    let mut visited_left: Vec<_> = matched_left.iter().map(|&v| v == u32::MAX).collect();
    let mut visited_right = HashSet::new();
    let mut stack: Vec<_> = (0..crossing.len()).filter(|&i| visited_left[i]).collect();
    while let Some(i) = stack.pop() {
        for &v in &crossing[i].1 {
            if v != matched_left[i]
                && visited_right.insert(v)
                && let Some(&j) = matched_right.get(&v)
                && !visited_left[j]
            {
                visited_left[j] = true;
                stack.push(j);
            }
        }
    }
    crossing
        .iter()
        .zip(&visited_left)
        .filter(|&(_, &visited)| !visited)
        .map(|((u, _), _)| *u)
        .chain(visited_right)
        .collect()
}

// Minimum vertex separator between the first and last quarter of `order`, by unit-capacity
// max flow on split vertices (in → out, capacity 1; links and terminals unbounded). Returns
// the two sides and the separator, or None when the quarters touch or the cut exceeds
// `limit`. Each augmenting search is O(V + A); at most `limit` + 1 run. Deterministic.
fn flow_cut(
    order: &[u32],
    adjacency: &[Vec<u32>],
    limit: usize,
) -> Option<(Vec<u32>, Vec<u32>, Vec<u32>)> {
    const UNBOUNDED: u32 = u32::MAX;
    let n = order.len();
    let quarter = (n / 4).max(1);
    let local: HashMap<u32, usize> = order.iter().enumerate().map(|(i, &v)| (v, i)).collect();
    let (source, sink) = (2 * n, 2 * n + 1);
    let mut heads = vec![Vec::new(); 2 * n + 2];
    let mut targets = Vec::new();
    let mut capacities = Vec::new();
    let mut link = |from: usize, to: usize, capacity: u32| {
        heads[from].push(targets.len());
        targets.push(to);
        capacities.push(capacity);
        heads[to].push(targets.len());
        targets.push(from);
        capacities.push(0);
    };
    for (i, &vertex) in order.iter().enumerate() {
        let terminal = i < quarter || i >= n - quarter;
        link(2 * i, 2 * i + 1, if terminal { UNBOUNDED } else { 1 });
        if i < quarter {
            link(source, 2 * i, UNBOUNDED);
        } else if i >= n - quarter {
            link(2 * i + 1, sink, UNBOUNDED);
        }
        for neighbor in &adjacency[vertex as usize] {
            if let Some(&j) = local.get(neighbor) {
                link(2 * i + 1, 2 * j, UNBOUNDED);
            }
        }
    }
    let mut previous = vec![usize::MAX; 2 * n + 2];
    let mut queue = std::collections::VecDeque::new();
    let mut search = |capacities: &[u32], previous: &mut [usize]| -> Vec<bool> {
        let mut reached = vec![false; 2 * n + 2];
        reached[source] = true;
        queue.clear();
        queue.push_back(source);
        while let Some(at) = queue.pop_front() {
            for &arc in &heads[at] {
                let to = targets[arc];
                if capacities[arc] > 0 && !reached[to] {
                    reached[to] = true;
                    previous[to] = arc;
                    queue.push_back(to);
                }
            }
        }
        reached
    };
    let mut flow = 0;
    loop {
        let reached = search(&capacities, &mut previous);
        if !reached[sink] {
            break;
        }
        // Every finite path crosses a capacity-1 vertex; an unbounded path has no vertex cut.
        let mut at = sink;
        let mut bottleneck = UNBOUNDED;
        while at != source {
            let arc = previous[at];
            bottleneck = bottleneck.min(capacities[arc]);
            at = targets[arc ^ 1];
        }
        if bottleneck == UNBOUNDED || flow >= limit {
            return None;
        }
        let mut at = sink;
        while at != source {
            let arc = previous[at];
            capacities[arc] -= 1;
            capacities[arc ^ 1] += 1;
            at = targets[arc ^ 1];
        }
        flow += 1;
    }
    let reached = search(&capacities, &mut previous);
    let (mut a, mut b, mut separator) = (Vec::new(), Vec::new(), Vec::new());
    for (i, &vertex) in order.iter().enumerate() {
        match (reached[2 * i], reached[2 * i + 1]) {
            (true, false) => separator.push(vertex),
            (_, true) => a.push(vertex),
            _ => b.push(vertex),
        }
    }
    Some((a, b, separator))
}
