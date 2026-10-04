// SPDX-License-Identifier: GPL-2.0-only

//! One-to-all CCH costs: an upward search from the start, then one pass down the hierarchy.

use super::{CchGraph, CchState, SearchKey};
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::collections::hash_map::Entry;
use std::collections::{BinaryHeap, HashMap};

/// Costs from one start node to every node, over the hierarchy paths, turn checks and meeting
/// rule of [`CchGraph::find_path`]. A cost equals the query's up to float summation order: the
/// sweep adds arcs outward from the start, the query from both ends toward the meeting node.
/// Assumes the hierarchy was built from `graph`, so every hierarchy edge is in its endpoints'
/// adjacency and the query's meeting check admits every pair of states at a node.
#[derive(Clone, Debug)]
pub struct CchCostsFrom {
    cost: Vec<f32>,
}

impl CchCostsFrom {
    /// The cost to `node`, or `None` where `find_path` finds no route.
    pub fn cost_to(&self, node: u32) -> Option<f32> {
        self.cost
            .get(node as usize)
            .copied()
            .filter(|&cost| cost < f32::MAX)
    }
}

// States `(node, first edge)` of a backward query: it has climbed to `node` and leaves it
// toward its target along that edge. Sorted, so each node's states form one range.
struct DownwardStates {
    states: Vec<SearchKey>,
    // `node_start[n]..node_start[n + 1]` are the states of node `n`.
    node_start: Vec<u32>,
    // Per shortcut, the state a downward shortcut leaves from; u32::MAX for the others.
    arc_source: Vec<u32>,
}

impl DownwardStates {
    fn new(cch: &CchGraph, allowed_mask: u8) -> Self {
        let mut arcs: Vec<(SearchKey, usize)> = cch
            .bwd_up
            .iter()
            .flatten()
            .filter_map(|&idx| {
                let shortcut = &cch.shortcuts[idx];
                (shortcut.allowed_types & allowed_mask != 0)
                    .then_some(((shortcut.start_node, shortcut.first_edge), idx))
            })
            .collect();
        arcs.par_sort_unstable();
        let mut states = Vec::new();
        let mut arc_source = vec![u32::MAX; cch.shortcuts.len()];
        for (state, idx) in arcs {
            if states.last() != Some(&state) {
                states.push(state);
            }
            arc_source[idx] = (states.len() - 1) as u32;
        }
        let mut node_start = vec![0_u32; cch.bwd_up.len() + 1];
        for &(node, _) in &states {
            node_start[node as usize + 1] += 1;
        }
        for node in 0..cch.bwd_up.len() {
            node_start[node + 1] += node_start[node];
        }
        Self {
            states,
            node_start,
            arc_source,
        }
    }
}

impl CchGraph {
    /// Costs from each of `starts`, entered without an arrival edge, to every node.
    ///
    /// Setup sorts the A downward shortcuts once, O(A log A). Each start then runs one upward
    /// search and one top-down pass of O(A × node degree) turn checks; starts run in parallel.
    /// Worth it where many queries share a few start nodes.
    pub fn costs_from_each(
        &self,
        starts: &[u32],
        graph: &RegionGraph,
        allowed_mask: u8,
    ) -> Vec<CchCostsFrom> {
        let layout = DownwardStates::new(self, allowed_mask);
        starts
            .par_iter()
            .map(|&start| self.costs_from(&layout, start, graph, allowed_mask))
            .collect()
    }

    fn costs_from(
        &self,
        layout: &DownwardStates,
        start: u32,
        graph: &RegionGraph,
        allowed_mask: u8,
    ) -> CchCostsFrom {
        let mut cost = vec![f32::INFINITY; self.fwd_up.len()];
        if start as usize >= self.fwd_up.len() {
            return CchCostsFrom { cost };
        }
        let upward = self.upward_costs(start, graph, allowed_mask);
        let mut down = vec![f32::INFINITY; layout.states.len()];
        for &node in self.node_order.iter().rev() {
            let first = upward.partition_point(|&((n, _), ..)| n < node);
            let count = upward[first..].partition_point(|&((n, _), ..)| n == node);
            let meetings = &upward[first..first + count];
            let arcs = &self.bwd_up[node as usize];
            let states = layout.node_start[node as usize] as usize
                ..layout.node_start[node as usize + 1] as usize;

            // A backward query leaving `node` on `edge` meets the upward search here, or
            // continues up a shortcut whose turn onto `edge` is legal.
            let relax_state = |i: usize, down: &mut [f32], loops: bool| {
                let edge = layout.states[i].1;
                let mut best = down[i];
                if !loops {
                    for &((_, in_edge), up_cost) in meetings {
                        if up_cost < best
                            && self.is_turn_allowed(node, in_edge, edge, graph, allowed_mask)
                        {
                            best = up_cost;
                        }
                    }
                }
                for &idx in arcs {
                    let shortcut = &self.shortcuts[idx];
                    if shortcut.allowed_types & allowed_mask == 0
                        || (shortcut.start_node == node) != loops
                    {
                        continue;
                    }
                    let via = down[layout.arc_source[idx] as usize] + shortcut.cost;
                    if via < best
                        && self.is_turn_allowed(node, shortcut.last_edge, edge, graph, allowed_mask)
                    {
                        best = via;
                    }
                }
                let improved = best < down[i];
                down[i] = best;
                improved
            };
            for i in states.clone() {
                relax_state(i, &mut down, false);
            }
            // A loop leaves from one of this node's own states; relax loops to a fixed point.
            if arcs
                .iter()
                .any(|&idx| self.shortcuts[idx].start_node == node)
            {
                while states.clone().fold(false, |changed, i| {
                    relax_state(i, &mut down, true) | changed
                }) {}
            }

            // The query's own target state meets every upward state at the target.
            let mut best = meetings
                .iter()
                .fold(f32::INFINITY, |best, meeting| best.min(meeting.1));
            for &idx in arcs {
                let shortcut = &self.shortcuts[idx];
                if shortcut.allowed_types & allowed_mask != 0 {
                    best = best.min(down[layout.arc_source[idx] as usize] + shortcut.cost);
                }
            }
            cost[node as usize] = best;
        }
        CchCostsFrom { cost }
    }

    // `find_path`'s forward search run to exhaustion: every upward state with its cost, sorted.
    fn upward_costs(
        &self,
        start: u32,
        graph: &RegionGraph,
        allowed_mask: u8,
    ) -> Vec<(SearchKey, f32)> {
        let mut costs: HashMap<SearchKey, f32, foldhash::fast::FixedState> = HashMap::default();
        let mut heap = BinaryHeap::new();
        costs.insert((start, usize::MAX), 0.0);
        heap.push(CchState {
            cost: 0.0,
            node: start,
            incoming_edge: usize::MAX,
        });
        while let Some(CchState {
            cost,
            node,
            incoming_edge,
        }) = heap.pop()
        {
            if cost > costs[&(node, incoming_edge)] {
                continue;
            }
            for &idx in &self.fwd_up[node as usize] {
                let shortcut = &self.shortcuts[idx];
                if shortcut.allowed_types & allowed_mask == 0
                    || !self.is_turn_allowed(
                        node,
                        incoming_edge,
                        shortcut.first_edge,
                        graph,
                        allowed_mask,
                    )
                {
                    continue;
                }
                let next_cost = cost + shortcut.cost;
                if next_cost >= f32::MAX || next_cost.is_nan() {
                    continue;
                }
                match costs.entry((shortcut.target_node, shortcut.last_edge)) {
                    Entry::Vacant(entry) => {
                        entry.insert(next_cost);
                    }
                    Entry::Occupied(mut entry) if next_cost < *entry.get() => {
                        entry.insert(next_cost);
                    }
                    Entry::Occupied(_) => continue,
                }
                heap.push(CchState {
                    cost: next_cost,
                    node: shortcut.target_node,
                    incoming_edge: shortcut.last_edge,
                });
            }
        }
        let mut upward: Vec<_> = costs.into_iter().collect();
        upward.sort_unstable_by_key(|&(state, ..)| state);
        upward
    }
}
