// SPDX-License-Identifier: GPL-2.0-only

//! Heap-free topological queries over unions of elimination-tree ancestor chains.

use super::*;

pub(super) mod buckets;
#[cfg(test)]
pub(super) mod restricted;

#[derive(Clone, Copy, Debug)]
struct Label {
    epoch: u64,
    cost: f32,
    distance: f32,
    arc: usize,
    previous: u32,
    seed_edge: usize,
}
impl Default for Label {
    fn default() -> Self {
        Self {
            epoch: 0,
            cost: f32::INFINITY,
            distance: 0.0,
            arc: usize::MAX,
            previous: u32::MAX,
            seed_edge: usize::MAX,
        }
    }
}
#[derive(Debug, Default)]
struct SearchData {
    epoch: u64,
    records: Vec<Label>,
    order: Vec<u32>,
}
impl SearchData {
    fn clear(&mut self) {
        self.order.clear();
        self.epoch = self.epoch.wrapping_add(1);
        if self.epoch == 0 {
            self.records.fill(Label::default());
            self.epoch = 1;
        }
    }
    fn ancestors(&mut self, mut node: u32, parents: &[u32]) {
        while node != u32::MAX && self.records[node as usize].epoch != self.epoch {
            self.records[node as usize] = Label {
                epoch: self.epoch,
                ..Label::default()
            };
            self.order.push(node);
            node = parents[node as usize];
        }
    }
    fn seed(&mut self, node: u32, cost: f32, distance: f32, edge: usize, parents: &[u32]) {
        self.ancestors(node, parents);
        if cost < self.records[node as usize].cost {
            self.records[node as usize] = Label {
                epoch: self.epoch,
                cost,
                distance,
                seed_edge: edge,
                ..Label::default()
            };
        }
    }
    fn prepare(&mut self, size: usize) {
        self.records.resize(size, Label::default());
        self.order.clear();
        self.order.reserve(size);
    }
    fn empty_like(&self) -> Self {
        let mut result = Self::default();
        result.prepare(self.records.len());
        result
    }
}

/// Reusable dense labels and ancestor lists; prepared queries allocate no search storage.
#[derive(Debug, Default)]
pub(crate) struct CchQueryScratch {
    revision: Option<(u32, usize)>,
    fwd_data: SearchData,
    bwd_data: SearchData,
    // The two sweeps depend on only one endpoint each. Retain their exact labels and
    // witnesses until that endpoint, turn context, mode or frozen metric changes.
    fwd_key: Option<(u64, u8, u32, usize)>,
    bwd_key: Option<(u64, u8, u32)>,
}
impl Clone for CchQueryScratch {
    fn clone(&self) -> Self {
        Self {
            revision: self.revision,
            fwd_data: self.fwd_data.empty_like(),
            bwd_data: self.bwd_data.empty_like(),
            fwd_key: None,
            bwd_key: None,
        }
    }
}
impl CchQueryScratch {
    /// Allocates O(arrival states) worker storage only when the hierarchy changes.
    pub(crate) fn prepare(&mut self, cch: &CchGraph, _graph: &RegionGraph) {
        let revision = (cch.build_generation, cch.parent.len());
        if self.revision == Some(revision) {
            return;
        }
        self.fwd_data.prepare(cch.parent.len());
        self.bwd_data.prepare(cch.parent.len());
        self.fwd_key = None;
        self.bwd_key = None;
        self.revision = Some(revision);
    }
}

/// Query and reconstruction buffers for borrowed path results.
#[derive(Debug, Default)]
pub(crate) struct CchPathScratch {
    query: CchQueryScratch,
    edges: Vec<usize>,
    shortcuts: Vec<usize>,
    nodes: Vec<u32>,
}
impl CchPathScratch {
    /// Borrows the last reconstructed road-node path.
    pub(crate) fn nodes(&self) -> &[u32] {
        &self.nodes
    }
    /// Reuses the search buffers without changing the last reconstructed path.
    pub(crate) fn query(&mut self) -> &mut CchQueryScratch {
        &mut self.query
    }
    /// Reserves O(states + road edges) search and unpacking storage per topology.
    pub(crate) fn prepare(&mut self, cch: &CchGraph, graph: &RegionGraph) {
        self.query.prepare(cch, graph);
        let bound = 2 * graph.edge_count() + 2 * graph.node_count() + 1;
        for values in [&mut self.edges, &mut self.shortcuts] {
            if values.capacity() < bound {
                values.reserve(bound - values.len());
            }
        }
        if self.nodes.capacity() < bound + 1 {
            self.nodes.reserve(bound + 1 - self.nodes.len());
        }
    }
}

impl CchGraph {
    fn seed_source(&self, start: u32, incoming: usize, mask: u8, data: &mut SearchData) {
        if !self.turns.restricted[start as usize] {
            data.seed(
                self.states[start as usize][0],
                0.0,
                0.0,
                usize::MAX,
                &self.parent,
            );
        } else {
            for departure in &self.departures[start as usize] {
                if departure.flags & mask != 0
                    && self.is_turn_allowed(start, incoming, departure.edge, mask)
                {
                    let (cost, distance) = self.base_metric[departure.edge];
                    data.seed(
                        departure.target,
                        cost,
                        distance,
                        departure.edge,
                        &self.parent,
                    );
                }
            }
        }
    }

    fn sweep_up(&self, metric: &Metric, data: &mut SearchData, reverse: bool) {
        // Each higher filled neighbor is an ancestor: the higher neighbors form a clique,
        // so induction along the smallest higher neighbor proves ancestor containment.
        // Rank order is a topological order. No settled state can receive a later update.
        data.order.sort_unstable();
        for &low in &data.order {
            let label = data.records[low as usize];
            if label.cost >= f32::MAX {
                continue;
            }
            for &(high, arc) in &self.up[low as usize] {
                let directed = arc * 2 + usize::from(reverse);
                let cost = label.cost + metric.costs[directed];
                let target = &mut data.records[high as usize];
                debug_assert_eq!(target.epoch, data.epoch);
                if cost < target.cost {
                    *target = Label {
                        epoch: data.epoch,
                        cost,
                        distance: label.distance + metric.distances[directed],
                        arc: directed,
                        previous: low,
                        seed_edge: usize::MAX,
                    };
                }
            }
        }
    }

    fn search(
        &self,
        start: u32,
        end: u32,
        incoming: usize,
        graph: &RegionGraph,
        mask: u8,
        scratch: &mut CchQueryScratch,
    ) -> Option<(f32, u32)> {
        crate::debug::work::add(crate::debug::work::Work::CchPointSearches, 1);
        if self.parent.is_empty() {
            return None;
        }
        scratch.prepare(self, graph);
        let fwd_key = Some((
            self.metric_revision,
            mask,
            start,
            self.source_context(start, incoming, mask),
        ));
        let bwd_key = Some((self.metric_revision, mask, end));
        let forward = &mut scratch.fwd_data;
        let backward = &mut scratch.bwd_data;
        let metric = &self.profiles[self.profile_for_mask[mask as usize] as usize];
        if scratch.fwd_key != fwd_key {
            forward.clear();
            self.seed_source(start, incoming, mask, forward);
            self.sweep_up(metric, forward, false);
            scratch.fwd_key = fwd_key;
        }
        if scratch.bwd_key != bwd_key {
            backward.clear();
            for &state in &self.states[end as usize] {
                backward.seed(state, 0.0, 0.0, usize::MAX, &self.parent);
            }
            self.sweep_up(metric, backward, true);
            scratch.bwd_key = bwd_key;
        }
        let mut best = (f32::INFINITY, u32::MAX);
        for &state in &forward.order {
            let b = backward.records[state as usize];
            if b.epoch != backward.epoch {
                continue;
            }
            let cost = forward.records[state as usize].cost + b.cost;
            if cost < best.0 {
                best = (cost, state);
            }
        }
        (best.0 < f32::MAX).then_some(best)
    }

    /// Finds a deterministic shortest route, preserving the incoming-edge turn context.
    /// Cost uses f32 hierarchy sums; physical distance follows the selected path witnesses.
    pub fn find_path(
        &self,
        start: u32,
        end: u32,
        incoming: usize,
        graph: &RegionGraph,
        mask: u8,
    ) -> Option<(f32, f32, Vec<u32>)> {
        OWNED_PATH_SCRATCH.with_borrow_mut(|scratch| {
            self.find_path_in(start, end, incoming, graph, mask, scratch)
                .map(|(cost, distance, nodes)| (cost, distance, nodes.to_vec()))
        })
    }

    /// Canonical incoming-edge context for retaining an exact query's costs and witness.
    /// Pedestrian queries ignore vehicle turns; unrestricted origins ignore incoming edges.
    /// Restricted vehicle origins retain the original edge, including nonincident sentinels.
    pub(crate) fn source_context(&self, start: u32, incoming: usize, mask: u8) -> usize {
        if mask == TransitFlags::FOOT
            || !self
                .turns
                .restricted
                .get(start as usize)
                .copied()
                .unwrap_or(false)
        {
            usize::MAX
        } else {
            incoming
        }
    }

    /// Quotes a route in O(H log H + A_H), for H ancestors and their A_H upward arcs.
    /// The source and target may each seed multiple arrival states; storage is reused.
    pub(crate) fn find_cost(
        &self,
        start: u32,
        end: u32,
        incoming: usize,
        graph: &RegionGraph,
        mask: u8,
        scratch: &mut CchQueryScratch,
    ) -> Option<f32> {
        if start as usize >= self.states.len() || end as usize >= self.states.len() {
            return None;
        }
        if start == end {
            return Some(0.0);
        }
        self.search(start, end, incoming, graph, mask, scratch)
            .map(|(cost, _)| cost)
    }

    /// Materializes the same selected route into prepared caller-owned storage.
    pub(crate) fn find_path_in<'s>(
        &self,
        start: u32,
        end: u32,
        incoming: usize,
        graph: &RegionGraph,
        mask: u8,
        scratch: &'s mut CchPathScratch,
    ) -> Option<(f32, f32, &'s [u32])> {
        if start as usize >= self.states.len() || end as usize >= self.states.len() {
            return None;
        }
        scratch.edges.clear();
        scratch.shortcuts.clear();
        scratch.nodes.clear();
        if start == end {
            scratch.nodes.push(start);
            return Some((0.0, 0.0, &scratch.nodes));
        }
        scratch.prepare(self, graph);
        let (cost, meeting) = self.search(start, end, incoming, graph, mask, &mut scratch.query)?;
        let metric = &self.profiles[self.profile_for_mask[mask as usize] as usize];
        let forward = &scratch.query.fwd_data.records;
        let backward = &scratch.query.bwd_data.records;
        let distance = forward[meeting as usize].distance + backward[meeting as usize].distance;
        let mut state = meeting;
        while forward[state as usize].arc != usize::MAX {
            scratch.shortcuts.push(forward[state as usize].arc);
            state = forward[state as usize].previous;
        }
        if forward[state as usize].seed_edge != usize::MAX {
            scratch.edges.push(forward[state as usize].seed_edge);
        }
        Self::unpack(metric, &mut scratch.shortcuts, &mut scratch.edges);
        state = meeting;
        while backward[state as usize].arc != usize::MAX {
            scratch.shortcuts.push(backward[state as usize].arc);
            state = backward[state as usize].previous;
        }
        scratch.shortcuts.reverse();
        Self::unpack(metric, &mut scratch.shortcuts, &mut scratch.edges);
        let mut node = start;
        scratch.nodes.push(node);
        for &edge in &scratch.edges {
            let edge = graph.edge(edge);
            node = if edge.start_node == node {
                edge.end_node
            } else {
                edge.start_node
            };
            scratch.nodes.push(node);
        }
        debug_assert_eq!(node, end);
        Some((cost, distance, &scratch.nodes))
    }

    /// Implements the full descending PHAST sweep over the same customized turn states.
    pub(super) fn all_costs(&self, start: u32, _graph: &RegionGraph, mask: u8) -> Vec<f32> {
        let mut nodes = vec![f32::INFINITY; self.states.len()];
        if start as usize >= nodes.len() {
            return nodes;
        }
        nodes[start as usize] = 0.0;
        if self.parent.is_empty() {
            return nodes;
        }
        let mut data = SearchData::default();
        data.prepare(self.parent.len());
        data.clear();
        self.seed_source(start, usize::MAX, mask, &mut data);
        let metric = &self.profiles[self.profile_for_mask[mask as usize] as usize];
        self.sweep_up(metric, &mut data, false);
        let mut costs = vec![f32::INFINITY; self.parent.len()];
        for &state in &data.order {
            costs[state as usize] = data.records[state as usize].cost;
        }
        for low in (0..costs.len()).rev() {
            for &(high, arc) in &self.up[low] {
                costs[low] = costs[low].min(costs[high as usize] + metric.costs[arc * 2 + 1]);
            }
        }
        for (node, states) in self.states.iter().enumerate() {
            for &state in states {
                nodes[node] = nodes[node].min(costs[state as usize]);
            }
        }
        nodes
    }

    fn unpack(metric: &Metric, stack: &mut Vec<usize>, edges: &mut Vec<usize>) {
        while let Some(arc) = stack.pop() {
            let witness = metric.witnesses[arc];
            if witness.edge != usize::MAX {
                edges.push(witness.edge);
            } else {
                stack.push(witness.right);
                stack.push(witness.left);
            }
        }
    }
}

#[cfg(test)]
impl CchQueryScratch {
    /// Reports every allocated search buffer for the no-growth regression.
    pub(super) fn capacities(&self) -> [usize; 4] {
        [
            self.fwd_data.order.capacity(),
            self.bwd_data.order.capacity(),
            self.fwd_data.records.capacity(),
            self.bwd_data.records.capacity(),
        ]
    }
    /// Forces the next query through epoch rollover.
    pub(super) fn roll_epochs_over(&mut self) {
        self.fwd_key = None;
        self.bwd_key = None;
        self.fwd_data.epoch = u64::MAX;
        self.bwd_data.epoch = u64::MAX;
    }
}
#[cfg(test)]
impl CchPathScratch {
    /// Exposes the actual concrete witness to the independent route validation test.
    pub(super) fn selected_edges(&self) -> &[usize] {
        &self.edges
    }
    /// Reports each path buffer for the no-growth regression.
    pub(super) fn capacities(&self) -> [usize; 3] {
        [
            self.edges.capacity(),
            self.shortcuts.capacity(),
            self.nodes.capacity(),
        ]
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;
    use crate::simulation::network::{graph::Edge, types::NodeType};
    use godot::prelude::Vector3;

    fn grid() -> RegionGraph {
        let mut graph = RegionGraph::new();
        for node in 0..289 {
            graph.add_node(
                Vector3::new((node % 17) as f32, 0.0, (node / 17) as f32),
                NodeType::Junction,
            );
        }
        for node in 0..289 {
            for target in [
                ((node % 17) < 16).then_some(node + 1),
                (node < 272).then_some(node + 17),
            ]
            .into_iter()
            .flatten()
            {
                graph.add_edge(Edge {
                    start_node: node,
                    end_node: target,
                    base_cost: 1.0 + (node % 7) as f32 / 13.0,
                    physical_length: 1.0,
                    allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                    fwd_lanes: 1,
                    bkw_lanes: 1,
                    ..Default::default()
                });
            }
        }
        graph.rebuild_adjacency_list();
        graph
    }

    #[test]
    fn endpoint_sweeps_retain_only_matching_inputs_and_survive_clone_and_rollover() {
        let mut graph = grid();
        let mut cch = CchGraph::build(&graph);
        let mut scratch = CchPathScratch::default();
        scratch.prepare(&cch, &graph);
        let capacity = scratch.query.capacities();
        for phase in 0..4 {
            match phase {
                1 => {
                    graph.edge_mut(0).base_cost *= 3.0;
                    cch.customize(&graph);
                }
                2 => {
                    scratch.query = scratch.query.clone();
                }
                3 => {
                    scratch.query.roll_epochs_over();
                }
                _ => {}
            }
            for (start, end, mode, retained) in [
                (0, 288, TransitFlags::CAR, [false, false]),
                (0, 287, TransitFlags::CAR, [true, false]),
                (1, 287, TransitFlags::CAR, [false, true]),
                (1, 287, TransitFlags::CAR, [true, true]),
                (1, 287, TransitFlags::FOOT, [false, false]),
                (0, 288, TransitFlags::CAR, [false, false]),
            ] {
                let before = [scratch.query.fwd_data.epoch, scratch.query.bwd_data.epoch];
                let actual = cch
                    .find_path_in(start, end, usize::MAX, &graph, mode, &mut scratch)
                    .map(|(cost, distance, nodes)| {
                        (cost.to_bits(), distance.to_bits(), nodes.to_vec())
                    });
                let mut fresh = CchPathScratch::default();
                let expected = cch
                    .find_path_in(start, end, usize::MAX, &graph, mode, &mut fresh)
                    .map(|(cost, distance, nodes)| {
                        (cost.to_bits(), distance.to_bits(), nodes.to_vec())
                    });
                assert_eq!(actual, expected);
                assert_eq!(scratch.selected_edges(), fresh.selected_edges());
                let after = [scratch.query.fwd_data.epoch, scratch.query.bwd_data.epoch];
                assert_eq!(
                    std::array::from_fn::<_, 2, _>(|side| before[side] == after[side]),
                    retained
                );
                assert_eq!(scratch.query.capacities(), capacity);
            }
        }
        let rebuilt = CchGraph::build(&graph);
        scratch.query.prepare(&rebuilt, &graph);
        assert!(scratch.query.fwd_key.is_none() && scratch.query.bwd_key.is_none());
    }

    #[test]
    #[ignore = "manual paired release timing of exact endpoint sweep retention"]
    fn benchmark_cch_endpoint_sweeps() {
        use std::{hint::black_box, time::Instant};
        let graph = grid();
        let cch = CchGraph::build(&graph);
        let mut scratch = CchQueryScratch::default();
        scratch.prepare(&cch, &graph);
        for group in [1, 2, 16] {
            let queries: Vec<_> = (0..4_096)
                .map(|index| {
                    (
                        ((index / group * 13) % 289) as u32,
                        ((index * 37 + 1) % 289) as u32,
                    )
                })
                .collect();
            let expected: Vec<_> = queries
                .iter()
                .map(|&(start, end)| {
                    scratch.fwd_key = None;
                    scratch.bwd_key = None;
                    cch.find_cost(
                        start,
                        end,
                        usize::MAX,
                        &graph,
                        TransitFlags::CAR,
                        &mut scratch,
                    )
                    .map(f32::to_bits)
                })
                .collect();
            for pair in 0..11 {
                for retained in if pair % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let begin = Instant::now();
                    for (&(start, end), &expected) in queries.iter().zip(&expected) {
                        if !retained {
                            scratch.fwd_key = None;
                            scratch.bwd_key = None;
                        }
                        let actual = cch.find_cost(
                            start,
                            end,
                            usize::MAX,
                            &graph,
                            TransitFlags::CAR,
                            &mut scratch,
                        );
                        assert_eq!(black_box(actual.map(f32::to_bits)), expected);
                    }
                    eprintln!(
                        "CCH_ENDPOINT {}",
                        serde_json::json!({ "group": group,
                        "pair": pair, "retained": retained,
                        "query_ns": begin.elapsed().as_secs_f64() * 1e9 / queries.len() as f64 })
                    );
                }
            }
        }
    }
}
