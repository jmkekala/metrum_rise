// SPDX-License-Identifier: GPL-2.0-only

//! Exact arrival-state expansion, undirected fill and metric profile construction.

use super::*;
use std::collections::{BTreeSet, HashMap};

impl CchGraph {
    /// Builds turn states, nested dissection, the filled topology and its elimination forest.
    pub fn build(graph: &RegionGraph) -> Self {
        // Turn states, dissection and customization are parallel; edits and loads on Godot's
        // main thread build here outside any pool (CODE-17).
        crate::thread_pools::install(|| Self::build_in_pool(graph))
    }

    fn build_in_pool(graph: &RegionGraph) -> Self {
        use std::sync::atomic::{AtomicU32, Ordering};
        static BUILD_GEN: AtomicU32 = AtomicU32::new(0);
        let mut result = Self::new(graph.node_count());
        result.build_generation = BUILD_GEN.fetch_add(1, Ordering::Relaxed);
        result.turns = VehicleTurns::build(graph);
        // States are created in node order, then relabelled by their own contraction order.
        let mut states = vec![Vec::new(); graph.node_count()];
        let mut incoming = Vec::new();
        let mut arrival = HashMap::new();
        for node in 0..graph.node_count() as u32 {
            if result.turns.restricted[node as usize] {
                let mut edges = graph.node_adjacency(node).to_vec();
                edges.sort_unstable();
                edges.dedup();
                for edge in edges.into_iter().filter(|&e| !graph.edge(e).deleted) {
                    let rank = incoming.len() as u32;
                    states[node as usize].push(rank);
                    incoming.push(edge);
                    arrival.insert((node, edge), rank);
                }
            }
            if states[node as usize].is_empty() {
                states[node as usize].push(incoming.len() as u32);
                incoming.push(usize::MAX);
            }
        }
        let mut departures = vec![Vec::new(); graph.node_count()];
        for (edge_id, edge) in graph.edges().iter().enumerate().filter(|(_, e)| !e.deleted) {
            for (from, to, forward) in [
                (edge.start_node, edge.end_node, true),
                (edge.end_node, edge.start_node, false),
            ] {
                let flags = edge.traversal_flags(forward);
                if flags == 0 {
                    continue;
                }
                let target = if result.turns.restricted[to as usize] {
                    arrival[&(to, edge_id)]
                } else {
                    states[to as usize][0]
                };
                departures[from as usize].push(Departure {
                    target,
                    edge: edge_id,
                    flags,
                });
            }
        }
        let rank = Self::order_states(graph, &result.turns, &states, &incoming, &departures);
        let mut ranked = vec![usize::MAX; incoming.len()];
        for (state, &edge) in incoming.iter().enumerate() {
            ranked[rank[state] as usize] = edge;
        }
        incoming = ranked;
        for state in states.iter_mut().flatten() {
            *state = rank[*state as usize];
        }
        for departure in departures.iter_mut().flatten() {
            departure.target = rank[departure.target as usize];
        }
        // Road nodes by their highest-ranked state, for diagnostics and tests.
        let mut order: Vec<_> = (0..graph.node_count() as u32).collect();
        order.sort_unstable_by_key(|&node| states[node as usize].iter().max().copied());
        let mut neighbors = vec![BTreeSet::new(); incoming.len()];
        let mut arc_ids = HashMap::new();
        fn ensure_arc(
            from: u32,
            to: u32,
            arcs: &mut Vec<CchShortcut>,
            ids: &mut HashMap<(u32, u32), usize>,
            neighbors: &mut [BTreeSet<u32>],
        ) -> usize {
            let (low, high) = (from.min(to), from.max(to));
            let id = *ids.entry((low, high)).or_insert_with(|| {
                let id = arcs.len();
                arcs.push(CchShortcut { low, high });
                neighbors[low as usize].insert(high);
                id
            });
            2 * id + usize::from(from > to)
        }
        for node in 0..graph.node_count() {
            for &from in &states[node] {
                for departure in &departures[node] {
                    // A same-state loop cannot change future legality and has nonnegative cost.
                    // A loop that changes the incoming edge remains an ordinary transition.
                    if from == departure.target {
                        continue;
                    }
                    let vehicle_turn = incoming[from as usize] == usize::MAX
                        || result.turns.permits(
                            node as u32,
                            incoming[from as usize],
                            departure.edge,
                        );
                    if !vehicle_turn && departure.flags & TransitFlags::FOOT == 0 {
                        continue;
                    }
                    let arc = ensure_arc(
                        from,
                        departure.target,
                        &mut result.shortcuts,
                        &mut arc_ids,
                        &mut neighbors,
                    );
                    result.base_arcs.push(BaseArc {
                        arc,
                        edge: departure.edge,
                        flags: departure.flags,
                        vehicle_turn,
                    });
                }
            }
        }
        let mut up = Vec::with_capacity(incoming.len());
        // Contraction is dependent. Work is O(sum d_up² log d_up), with exactly one
        // triangle per higher-neighbor pair and one retained filled edge per endpoint pair.
        for low in 0..incoming.len() {
            let higher: Vec<_> = neighbors[low].iter().copied().collect();
            for (i, &v) in higher.iter().enumerate() {
                for &w in &higher[i + 1..] {
                    let vw =
                        ensure_arc(v, w, &mut result.shortcuts, &mut arc_ids, &mut neighbors) / 2;
                    result.triangles.push([
                        arc_ids[&(low as u32, v)],
                        arc_ids[&(low as u32, w)],
                        vw,
                    ]);
                }
            }
            result
                .parent
                .push(higher.first().copied().unwrap_or(u32::MAX));
            up.push(
                higher
                    .into_iter()
                    .map(|high| (high, arc_ids[&(low as u32, high)]))
                    .collect(),
            );
            neighbors[low].clear();
        }
        result.up = NodeSlices::from_rows(up);
        result.states = NodeSlices::from_rows(states);
        result.arrival_edges = incoming;
        result.departures = NodeSlices::from_rows(departures);
        result.node_order = order;
        // Cache only semantically distinct masks: identical enabled directed road types and
        // identical turn rules share a metric. Combined masks mean each edge may use any
        // selected type, while only the exact FOOT mask bypasses vehicle turn restrictions and
        // walks at walking pace.
        let mut types: Vec<_> = result.departures.values.iter().map(|d| d.flags).collect();
        types.sort_unstable();
        types.dedup();
        let mut signatures: Vec<Vec<bool>> = Vec::new();
        for mask in 0..=255_u8 {
            let mut signature: Vec<_> = types.iter().map(|flags| flags & mask != 0).collect();
            // The exact FOOT mask is priced at walking pace, so it never shares a metric.
            signature.push(mask == TransitFlags::FOOT);
            let profile = if let Some(id) = signatures.iter().position(|s| *s == signature) {
                id
            } else {
                let id = signatures.len();
                signatures.push(signature);
                result.profiles.push(Metric {
                    mask,
                    costs: vec![f32::INFINITY; result.shortcuts.len() * 2],
                    distances: vec![0.0; result.shortcuts.len() * 2],
                    witnesses: vec![Witness::default(); result.shortcuts.len() * 2],
                });
                id
            };
            result.profile_for_mask[mask as usize] = profile as u8;
        }
        result.shortcuts.shrink_to_fit();
        result.base_arcs.shrink_to_fit();
        result.triangles.shrink_to_fit();
        result.base_metric.resize(graph.edge_count(), (0.0, 0.0));
        result.base_walk.resize(graph.edge_count(), 0.0);
        result.customize(graph);
        result
    }

    /// Returns the road-node nested-dissection order and ranks.
    #[cfg(test)]
    pub(super) fn compute_node_order(graph: &RegionGraph) -> (Vec<u32>, Vec<u32>) {
        order::nested_dissection(graph)
    }

    /// Contraction rank of every arrival state, by nested dissection of the state transition
    /// graph. A restricted node's states sit just inside the node along their arrival edges, so
    /// separators may cut between one junction's states instead of taking all of them.
    /// O((S log S + T) log S) for S states and T transitions; deterministic.
    fn order_states(
        graph: &RegionGraph,
        turns: &VehicleTurns,
        states: &[Vec<u32>],
        incoming: &[usize],
        departures: &[Vec<Departure>],
    ) -> Vec<u32> {
        // Offset in metres, bounded by a quarter of the edge so states stay on their own side.
        const STATE_OFFSET_M: f32 = 0.5;
        let mut positions = vec![[0.0; 2]; incoming.len()];
        let mut adjacency = vec![Vec::new(); incoming.len()];
        for (node, node_states) in states.iter().enumerate() {
            let pos = graph.node(node as u32).pos;
            for &state in node_states {
                let mut point = [pos.x, pos.z];
                let edge = incoming[state as usize];
                if edge != usize::MAX {
                    let edge = graph.edge(edge);
                    let other = if edge.start_node as usize == node {
                        edge.end_node
                    } else {
                        edge.start_node
                    };
                    let towards = graph.node(other).pos - pos;
                    let length = (towards.x * towards.x + towards.z * towards.z).sqrt();
                    if length > 0.0 {
                        let step = STATE_OFFSET_M.min(length * 0.25) / length;
                        point = [pos.x + towards.x * step, pos.z + towards.z * step];
                    }
                }
                positions[state as usize] = point;
                // The same transitions the hierarchy keeps as base arcs.
                for departure in &departures[node] {
                    let from = incoming[state as usize];
                    let vehicle_turn =
                        from == usize::MAX || turns.permits(node as u32, from, departure.edge);
                    if state != departure.target
                        && (vehicle_turn || departure.flags & TransitFlags::FOOT != 0)
                    {
                        adjacency[state as usize].push(departure.target);
                        adjacency[departure.target as usize].push(state);
                    }
                }
            }
        }
        for neighbors in &mut adjacency {
            neighbors.sort_unstable();
            neighbors.dedup();
        }
        let order = order::dissect_points(&positions, &adjacency);
        let mut rank = vec![0; order.len()];
        for (position, &state) in order.iter().enumerate() {
            rank[state as usize] = position as u32;
        }
        rank
    }
}
