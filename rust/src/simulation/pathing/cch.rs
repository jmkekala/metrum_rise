// SPDX-License-Identifier: GPL-2.0-only

//! Customizable Contraction Hierarchy (CCH) pathfinding.
//!
//! Provides fast queries and lower-triangle metric customization for dynamic traffic.
//! Replaces HPA* as the primary routing engine for agents.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{TransitFlags, TransitType};
use crate::traffic_log;
use rayon::prelude::*;
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// A shortcut edge in the contracted graph.
///
/// Each shortcut is either a *direct* shortcut (wrapping a single base edge) or a *compound*
/// shortcut (combining two child shortcuts via a middle node). Storing only the child indices
/// keeps each shortcut at O(1) memory regardless of path length.
#[derive(Clone, Debug)]
pub struct CchShortcut {
    /// The start node of this shortcut.
    pub start_node: u32,
    /// The target node of this shortcut.
    pub target_node: u32,
    /// The current traversal cost (seconds). Updated during customization.
    pub cost: f32,
    /// The physical distance (metres).
    pub dist: f32,
    /// Index of the base edge in `RegionGraph::edges` for *direct* shortcuts.
    /// `usize::MAX` for compound shortcuts.
    pub base_edge: usize,
    /// Index into `CchGraph::shortcuts` of the left (incoming) child shortcut.
    /// `usize::MAX` for direct shortcuts.
    pub mid_l: usize,
    /// Index into `CchGraph::shortcuts` of the right (outgoing) child shortcut.
    /// `usize::MAX` for direct shortcuts.
    pub mid_r: usize,
    /// The first concrete edge of the shortcut (for turn restrictions).
    pub first_edge: usize,
    /// The last concrete edge of the shortcut (for turn restrictions).
    pub last_edge: usize,
    /// Bitmask of permitted transit modes.
    pub allowed_types: u8,
}

/// A pre-computed contraction hierarchy used for routing.
pub struct CchGraph {
    /// Mapping from node ID to its rank in the contraction order.
    pub node_rank: Vec<u32>,
    /// Mapping from rank to the corresponding original node ID.
    pub node_order: Vec<u32>,
    /// Parent pointers in the elimination tree, ordered by contraction rank.
    pub elimination_tree: Vec<Option<u32>>,
    /// All shortcuts formed during contraction.
    pub shortcuts: Vec<CchShortcut>,
    /// Outgoing upward shortcuts for forward search: u -> v where Original u -> v and rank(u) < rank(v).
    /// Stores indices into `shortcuts`.
    pub fwd_up: Vec<Vec<usize>>,
    /// Incoming upward shortcuts for backward search: v -> u where Original u -> v and rank(u) < rank(v).
    /// Used to go from v to u (backwards) where rank(u) > rank(v).
    pub bwd_up: Vec<Vec<usize>>,
    /// Precomputed maximum speed in the network for heuristic calculations.
    pub max_v: f32,
    /// Monotonically incremented each time the graph is rebuilt; used to reset per-build debug counters.
    pub build_generation: u32,
    /// Per-node index of shortcuts whose `start_node` equals the index. Used during contraction.
    shortcuts_by_start: Vec<Vec<usize>>,
    /// Per-node index of shortcuts whose `target_node` equals the index. Used during contraction.
    shortcuts_by_end: Vec<Vec<usize>>,
    // One arc per endpoint/boundary-edge/mode state, with all lower-triangle alternatives.
    // Retaining choices avoids both exponential path expansion and first-seen-path pruning.
    shortcut_alternatives: Vec<Vec<(usize, usize)>>,
    customization_order: Vec<usize>,
}

impl CchGraph {
    /// Returns an empty `CchGraph`.
    pub fn new(n_nodes: usize) -> Self {
        Self {
            node_rank: vec![u32::MAX; n_nodes],
            node_order: Vec::with_capacity(n_nodes),
            elimination_tree: vec![None; n_nodes],
            shortcuts: Vec::new(),
            fwd_up: vec![Vec::new(); n_nodes],
            bwd_up: vec![Vec::new(); n_nodes],
            max_v: 1.0,
            build_generation: 0,
            shortcuts_by_start: vec![Vec::new(); n_nodes],
            shortcuts_by_end: vec![Vec::new(); n_nodes],
            shortcut_alternatives: Vec::new(),
            customization_order: Vec::new(),
        }
    }

    /// Builds a new CCH from the provided road network.
    pub fn build(graph: &RegionGraph) -> Self {
        // Fetch and increment the global build generation so per-build debug counters reset.
        use std::sync::atomic::{AtomicU32, Ordering};
        static BUILD_GEN: AtomicU32 = AtomicU32::new(0);
        let generation = BUILD_GEN.fetch_add(1, Ordering::Relaxed);

        let n = graph.node_count();
        if n == 0 {
            let mut cch = Self::new(0);
            cch.build_generation = generation;
            return cch;
        }

        let mut cch = Self::new(n);
        cch.build_generation = generation;
        cch.compute_node_order(graph);
        cch.contract(graph);
        cch.customize(graph);

        cch
    }

    fn compute_node_order(&mut self, graph: &RegionGraph) {
        let n = graph.node_count();
        let mut adj = vec![HashSet::new(); n];
        for edge in graph.edges().iter().filter(|edge| !edge.deleted) {
            if edge.start_node != edge.end_node {
                adj[edge.start_node as usize].insert(edge.end_node);
                adj[edge.end_node as usize].insert(edge.start_node);
            }
        }

        // A node's exact fill score is choose(degree, 2) - neighbour triangles - degree.
        // Count triangles once, then update only when a fill edge is actually inserted.
        // Sequential elimination is dependent; independent initial counts use Rayon.
        let mut triangles: Vec<u64> = adj
            .par_iter()
            .map(|neighbors| {
                neighbors
                    .iter()
                    .map(|&v| {
                        neighbors
                            .iter()
                            .filter(|&&w| v < w && adj[v as usize].contains(&w))
                            .count() as u64
                    })
                    .sum()
            })
            .collect();
        let mut priorities: Vec<i64> = adj
            .iter()
            .zip(&triangles)
            .map(|(neighbors, &triangles)| fill_importance(neighbors.len(), triangles))
            .collect();
        let mut heap: BinaryHeap<_> = priorities
            .iter()
            .enumerate()
            .map(|(node, &importance)| NodePriority {
                node: node as u32,
                importance,
            })
            .collect();
        let mut neighbors = Vec::new();
        let mut changed = Vec::new();
        let mut marked = vec![false; n];

        while let Some(NodePriority { node, importance }) = heap.pop() {
            let u = node as usize;
            // Priorities can increase OR decrease. Both directions leave obsolete heap entries.
            if self.node_rank[u] != u32::MAX || importance != priorities[u] {
                continue;
            }
            self.node_rank[u] = self.node_order.len() as u32;
            self.node_order.push(node);
            neighbors.clear();
            neighbors.extend(adj[u].iter().copied());
            neighbors.sort_unstable();

            // O(d² + sum(min(deg(v), deg(w))) over newly inserted fill edges), instead of
            // recounting every neighbour's O(deg²) score after every contraction.
            // Scratch buffers are reused; adjacency storage grows only for new fill edges.
            for (i, &v) in neighbors.iter().enumerate() {
                mark_order_score_changed(v, &mut marked, &mut changed);
                for &w in &neighbors[i + 1..] {
                    if adj[v as usize].contains(&w) {
                        continue;
                    }
                    let mut common_count = 0;
                    for &common in adj[v as usize].intersection(&adj[w as usize]) {
                        triangles[common as usize] += 1;
                        common_count += 1;
                        mark_order_score_changed(common, &mut marked, &mut changed);
                    }
                    triangles[v as usize] += common_count;
                    triangles[w as usize] += common_count;
                    adj[v as usize].insert(w);
                    adj[w as usize].insert(v);
                }
            }

            // The surviving neighbours now form a clique. Removing u deletes exactly d-1
            // triangles at each neighbour, including triangles introduced by the fill above.
            for &v in &neighbors {
                adj[v as usize].remove(&node);
                triangles[v as usize] -= neighbors.len().saturating_sub(1) as u64;
            }
            adj[u].clear();
            for v in changed.drain(..) {
                let v_index = v as usize;
                marked[v_index] = false;
                if self.node_rank[v_index] != u32::MAX {
                    continue;
                }
                let real_p = fill_importance(adj[v_index].len(), triangles[v_index]);
                if priorities[v_index] != real_p {
                    priorities[v_index] = real_p;
                    heap.push(NodePriority {
                        node: v,
                        importance: real_p,
                    });
                }
            }
        }
    }

    fn contract(&mut self, graph: &RegionGraph) {
        let n = graph.node_count();
        // Debug: log nodes that have user vehicle connections (whitelist mode).
        if crate::debug::is_traffic_enabled() {
            for i in 0..n {
                let node = graph.node(i as u32);
                let vehicle_conns: Vec<_> = node
                    .lane_connections
                    .iter()
                    .filter(|((_, lane_idx), _)| *lane_idx != 100 && *lane_idx != -100)
                    .map(|((e, l), targets)| format!("(edge={e},lane={l})->{targets:?}"))
                    .collect();
                if !vehicle_conns.is_empty() {
                    eprintln!(
                        "[CCH_BUILD] node={i} has user vehicle connections: {}",
                        vehicle_conns.join(", ")
                    );
                }
            }
        }

        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            if edge.deleted {
                continue;
            }

            if edge.fwd_lanes > 0
                || (edge.primary_type == TransitType::Foot
                    && (edge.allowed_types & TransitFlags::FOOT != 0))
            {
                self.add_direct_shortcut(edge.start_node, edge.end_node, edge_idx, edge);
            }
            if edge.bkw_lanes > 0
                || (edge.primary_type == TransitType::Foot
                    && (edge.allowed_types & TransitFlags::FOOT != 0))
            {
                self.add_direct_shortcut(edge.end_node, edge.start_node, edge_idx, edge);
            }
        }

        let turn_restricted: Vec<bool> = (0..n)
            .into_par_iter()
            .map(|node| Self::node_has_vehicle_whitelist(graph.node(node as u32)))
            .collect();
        let mut compound_arcs: HashMap<(u32, u32, usize, usize, u8), usize> = HashMap::new();
        let mut neighbors_in = Vec::new();
        let mut neighbors_out = Vec::new();
        for rank in 0..n {
            let u = self.node_order[rank];
            let u_rank = self.node_rank[u as usize];

            neighbors_in.clear();
            neighbors_out.clear();

            // Use per-node index instead of scanning all shortcuts — O(degree) not O(S).
            for &idx in &self.shortcuts_by_end[u as usize] {
                let s = &self.shortcuts[idx];
                if self.node_rank[s.start_node as usize] > u_rank {
                    neighbors_in.push(idx);
                }
            }
            for &idx in &self.shortcuts_by_start[u as usize] {
                let s = &self.shortcuts[idx];
                if self.node_rank[s.target_node as usize] > u_rank {
                    neighbors_out.push(idx);
                }
            }

            // These arcs have u as their lower-ranked endpoint. Their children have strictly
            // lower endpoints, even if an alternative was appended after the parent arc.
            self.customization_order.extend_from_slice(&neighbors_in);
            self.customization_order.extend_from_slice(&neighbors_out);
            if crate::debug::is_traffic_enabled()
                && (!neighbors_in.is_empty() || !neighbors_out.is_empty())
            {
                let ni: Vec<_> = neighbors_in
                    .iter()
                    .map(|&i| {
                        let s = &self.shortcuts[i];
                        format!("{}→{}(e{})", s.start_node, s.target_node, s.last_edge)
                    })
                    .collect();
                let no: Vec<_> = neighbors_out
                    .iter()
                    .map(|&i| {
                        let s = &self.shortcuts[i];
                        format!("{}→{}(e{})", s.start_node, s.target_node, s.first_edge)
                    })
                    .collect();
                eprintln!(
                    "[CCH_CONTRACT] contracting node={u} rank={u_rank} neighbors_in=[{}] neighbors_out=[{}]",
                    ni.join(","),
                    no.join(",")
                );
            }

            for &idx_in in &neighbors_in {
                let (s_in_start, s_in_last, s_in_first, s_in_mask) = {
                    let s = &self.shortcuts[idx_in];
                    (s.start_node, s.last_edge, s.first_edge, s.allowed_types)
                };

                for &idx_out in &neighbors_out {
                    let (s_out_target, s_out_first, s_out_last, s_out_mask) = {
                        let s = &self.shortcuts[idx_out];
                        (s.target_node, s.first_edge, s.last_edge, s.allowed_types)
                    };

                    if s_in_start == s_out_target {
                        traffic_log!(
                            "[CCH_CONTRACT] node={u} skip U-turn {s_in_start}↔{s_out_target} (in_edge={s_in_last} out_edge={s_out_first})"
                        );
                        continue;
                    }

                    if !Self::vehicle_turn_allowed(graph.node(u), s_in_last, s_out_first) {
                        traffic_log!(
                            "[CCH_BUILD] blocked shortcut: {s_in_start}→{u}→{s_out_target} (in_edge={s_in_last}, out_edge={s_out_first})"
                        );
                        continue;
                    }

                    traffic_log!(
                        "[CCH_CONTRACT] node={u} create shortcut {s_in_start}→{s_out_target} (in_edge={s_in_last} out_edge={s_out_first})"
                    );
                    let mask = s_in_mask & s_out_mask;
                    if mask == 0 {
                        continue;
                    }
                    // Boundary-edge identity only distinguishes legal continuations at restricted
                    // endpoints. Merge equivalent open-end states, retaining all metric choices.
                    let first_key = if turn_restricted[s_in_start as usize] {
                        s_in_first
                    } else {
                        usize::MAX
                    };
                    let last_key = if turn_restricted[s_out_target as usize] {
                        s_out_last
                    } else {
                        usize::MAX
                    };
                    let key = (s_in_start, s_out_target, first_key, last_key, mask);
                    if let Some(&idx) = compound_arcs.get(&key) {
                        self.shortcut_alternatives[idx].push((idx_in, idx_out));
                        continue;
                    }
                    let idx = self.add_compound_shortcut(
                        s_in_start,
                        s_out_target,
                        idx_in,
                        idx_out,
                        s_in_first,
                        s_out_last,
                        mask,
                    );
                    compound_arcs.insert(key, idx);
                }
            }
        }

        // 3. Populate fwd_up, bwd_up and elimination tree
        for (idx, s) in self.shortcuts.iter().enumerate() {
            let u = s.start_node;
            let v = s.target_node;
            if self.node_rank[u as usize] < self.node_rank[v as usize] {
                self.fwd_up[u as usize].push(idx);
                // Parent in elimination tree is the lowest-rank higher neighbor
                let p = self.elimination_tree[u as usize];
                if p.is_none() || self.node_rank[v as usize] < self.node_rank[p.unwrap() as usize] {
                    self.elimination_tree[u as usize] = Some(v);
                }
            } else {
                self.bwd_up[v as usize].push(idx);
            }
        }
    }

    fn add_direct_shortcut(
        &mut self,
        start: u32,
        end: u32,
        edge_idx: usize,
        edge: &crate::simulation::network::graph::Edge,
    ) {
        let idx = self.shortcuts.len();
        self.shortcuts.push(CchShortcut {
            start_node: start,
            target_node: end,
            cost: 0.0, // Updated in customize
            dist: edge.physical_length,
            base_edge: edge_idx,
            mid_l: usize::MAX,
            mid_r: usize::MAX,
            first_edge: edge_idx,
            last_edge: edge_idx,
            allowed_types: edge.allowed_types,
        });
        self.shortcut_alternatives.push(Vec::new());
        self.shortcuts_by_start[start as usize].push(idx);
        self.shortcuts_by_end[end as usize].push(idx);
    }

    fn add_compound_shortcut(
        &mut self,
        start: u32,
        end: u32,
        l_idx: usize,
        r_idx: usize,
        first: usize,
        last: usize,
        mask: u8,
    ) -> usize {
        let idx = self.shortcuts.len();
        self.shortcuts.push(CchShortcut {
            start_node: start,
            target_node: end,
            cost: 0.0,
            dist: 0.0,
            base_edge: usize::MAX,
            mid_l: l_idx,
            mid_r: r_idx,
            first_edge: first,
            last_edge: last,
            allowed_types: mask,
        });
        self.shortcut_alternatives.push(vec![(l_idx, r_idx)]);
        self.shortcuts_by_start[start as usize].push(idx);
        self.shortcuts_by_end[end as usize].push(idx);
        idx
    }

    /// Expands a shortcut into its sequence of concrete base-edge indices.
    ///
    /// Uses an iterative stack to avoid recursion depth issues on long paths.
    fn collect_base_edges(&self, idx: usize) -> Vec<usize> {
        let mut result = Vec::new();
        let mut stack = vec![idx];
        while let Some(i) = stack.pop() {
            let s = &self.shortcuts[i];
            if s.mid_l == usize::MAX {
                // Direct shortcut — emit the base edge.
                result.push(s.base_edge);
            } else {
                // Compound shortcut — push right then left so left is processed first.
                stack.push(s.mid_r);
                stack.push(s.mid_l);
            }
        }
        result
    }

    /// Re-evaluates all retained lower-triangle alternatives for current dynamic edge costs.
    ///
    /// O(base edges + shortcuts + alternatives), allocation-free. Alternatives are evaluated in
    /// increasing lower-endpoint rank, so every child is finalized before its parent.
    pub fn customize(&mut self, graph: &RegionGraph) {
        self.max_v = graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted)
            .fold(1.0_f32, |speed, edge| speed.max(edge.speed_limit));
        for &idx in &self.customization_order {
            if self.shortcuts[idx].base_edge != usize::MAX {
                let edge = graph.edge(self.shortcuts[idx].base_edge);
                self.shortcuts[idx].cost = edge.base_cost * (1.0 + edge.current_congestion);
                self.shortcuts[idx].dist = edge.physical_length;
            } else {
                let mut best = (f32::INFINITY, usize::MAX, usize::MAX);
                for &(left, right) in &self.shortcut_alternatives[idx] {
                    let cost = self.shortcuts[left].cost + self.shortcuts[right].cost;
                    if cost < best.0 {
                        best = (cost, left, right);
                    }
                }
                let (_, left, right) = best;
                self.shortcuts[idx].cost = best.0;
                if left != usize::MAX {
                    self.shortcuts[idx].dist =
                        self.shortcuts[left].dist + self.shortcuts[right].dist;
                    self.shortcuts[idx].mid_l = left;
                    self.shortcuts[idx].mid_r = right;
                    self.shortcuts[idx].first_edge = self.shortcuts[left].first_edge;
                    self.shortcuts[idx].last_edge = self.shortcuts[right].last_edge;
                }
            }
        }
    }

    /// Finds a path from `start` to `end` using bidirectional upward search.
    pub fn find_path(
        &self,
        start: u32,
        end: u32,
        start_edge: usize,
        graph: &RegionGraph,
        allowed_mask: u8,
    ) -> Option<(f32, f32, Vec<u32>)> {
        // B31: Prevent panic if pathfinding is called with new nodes before CCH rebuild
        if start as usize >= self.fwd_up.len() || end as usize >= self.fwd_up.len() {
            return None;
        }

        if start == end {
            return Some((0.0, 0.0, vec![start]));
        }

        let mut fwd_heap = BinaryHeap::new();
        let mut bwd_heap = BinaryHeap::new();

        let mut fwd_data: HashMap<(u32, usize), (f32, f32, Option<usize>, (u32, usize))> =
            HashMap::new();
        let mut bwd_data: HashMap<(u32, usize), (f32, f32, Option<usize>, (u32, usize))> =
            HashMap::new();

        fwd_data.insert(
            (start, start_edge),
            (0.0, 0.0, None, (u32::MAX, usize::MAX)),
        );
        fwd_heap.push(CchState {
            priority: 0.0,
            cost: 0.0,
            node: start,
            incoming_edge: start_edge,
        });

        bwd_data.insert((end, usize::MAX), (0.0, 0.0, None, (u32::MAX, usize::MAX)));
        bwd_heap.push(CchState {
            priority: 0.0,
            cost: 0.0,
            node: end,
            incoming_edge: usize::MAX,
        });
        let mut min_total_cost = f32::MAX;
        let mut meeting_node = u32::MAX;
        let mut meeting_f_edge = usize::MAX;
        let mut meeting_b_edge = usize::MAX;

        while !fwd_heap.is_empty() || !bwd_heap.is_empty() {
            // Forward expansion
            if let Some(state) = fwd_heap.pop() {
                if state.cost >= min_total_cost {
                    fwd_heap.clear();
                }
                let (cost, node, l_edge) = (state.cost, state.node, state.incoming_edge);
                if cost < min_total_cost
                    && let Some(&(best_cost, _, _, _)) = fwd_data.get(&(node, l_edge))
                {
                    if cost > best_cost {
                        continue;
                    }

                    for (&(b_node, b_out_edge), &(b_cost, _, _, _)) in &bwd_data {
                        if b_node == node {
                            let allowed = self.is_turn_allowed(node, l_edge, b_out_edge, graph);
                            if !allowed {
                                traffic_log!(
                                    "[CCH_REJECT] fwd meeting blocked at node={node} in_edge={l_edge} out_edge={b_out_edge} (query {start}→{end})"
                                );
                            } else if cost + b_cost < min_total_cost {
                                traffic_log!(
                                    "[DEBUG:CCH_ACCEPT] fwd meeting at node={node} in_edge={l_edge} out_edge={b_out_edge} (query {start}→{end})"
                                );
                                min_total_cost = cost + b_cost;
                                meeting_node = node;
                                meeting_f_edge = l_edge;
                                meeting_b_edge = b_out_edge;
                            }
                        }
                    }

                    // Upward expansion
                    for &s_idx in &self.fwd_up[node as usize] {
                        let shortcut = &self.shortcuts[s_idx];
                        if (shortcut.allowed_types & allowed_mask) == 0 {
                            continue;
                        }

                        if self.is_turn_allowed(node, l_edge, shortcut.first_edge, graph) {
                            let next_cost = cost + shortcut.cost;
                            let state_key = (shortcut.target_node, shortcut.last_edge);
                            if next_cost < fwd_data.get(&state_key).map(|d| d.0).unwrap_or(f32::MAX)
                            {
                                let dist = fwd_data.get(&(node, l_edge)).unwrap().1 + shortcut.dist;
                                fwd_data.insert(
                                    state_key,
                                    (next_cost, dist, Some(s_idx), (node, l_edge)),
                                );
                                fwd_heap.push(CchState {
                                    priority: next_cost,
                                    cost: next_cost,
                                    node: shortcut.target_node,
                                    incoming_edge: shortcut.last_edge,
                                });
                            }
                        }
                    }
                }
            }

            // Backward expansion
            if let Some(state) = bwd_heap.pop() {
                if state.cost >= min_total_cost {
                    bwd_heap.clear();
                }
                let (cost, node, outgoing_edge) = (state.cost, state.node, state.incoming_edge);
                if cost < min_total_cost
                    && let Some(&(best_cost, _, _, _)) = bwd_data.get(&(node, outgoing_edge))
                {
                    if cost > best_cost {
                        continue;
                    }

                    // Check for meeting point against fwd_data
                    for (&(f_node, f_in_edge), &(f_cost, _, _, _)) in &fwd_data {
                        if f_node == node
                            && self.is_turn_allowed(node, f_in_edge, outgoing_edge, graph)
                        {
                            if cost + f_cost < min_total_cost {
                                min_total_cost = cost + f_cost;
                                meeting_node = node;
                                meeting_f_edge = f_in_edge;
                                meeting_b_edge = outgoing_edge;
                            }
                        }
                    }

                    // Upward expansion (backwards)
                    for &s_idx in &self.bwd_up[node as usize] {
                        let shortcut = &self.shortcuts[s_idx];
                        if (shortcut.allowed_types & allowed_mask) == 0 {
                            continue;
                        }

                        if self.is_turn_allowed(node, shortcut.last_edge, outgoing_edge, graph) {
                            let next_cost = cost + shortcut.cost;
                            let state_key = (shortcut.start_node, shortcut.first_edge);
                            if next_cost < bwd_data.get(&state_key).map(|d| d.0).unwrap_or(f32::MAX)
                            {
                                let dist =
                                    bwd_data.get(&(node, outgoing_edge)).unwrap().1 + shortcut.dist;
                                bwd_data.insert(
                                    state_key,
                                    (next_cost, dist, Some(s_idx), (node, outgoing_edge)),
                                );
                                bwd_heap.push(CchState {
                                    priority: next_cost,
                                    cost: next_cost,
                                    node: shortcut.start_node,
                                    incoming_edge: shortcut.first_edge,
                                });
                            }
                        }
                    }
                }
            }
        }

        if meeting_node == u32::MAX {
            return None;
        }

        let mut full_edges: Vec<usize> = Vec::new();

        // Forward part: MEETING -> START (reconstruct backwards)
        let mut curr_key = (meeting_node, meeting_f_edge);
        let mut fwd_indices = Vec::new();
        while let Some(&(_, _, s_opt, prev_key)) = fwd_data.get(&curr_key) {
            if let Some(s_idx) = s_opt {
                fwd_indices.push(s_idx);
                curr_key = prev_key;
            } else {
                break;
            }
        }
        fwd_indices.reverse();
        for idx in fwd_indices {
            full_edges.extend(self.collect_base_edges(idx));
        }

        // Backward part: MEETING -> TARGET
        let mut curr_key = (meeting_node, meeting_b_edge);
        while let Some(&(_, _, s_opt, next_key)) = bwd_data.get(&curr_key) {
            if let Some(s_idx) = s_opt {
                full_edges.extend(self.collect_base_edges(s_idx));
                curr_key = next_key;
            } else {
                break;
            }
        }

        if full_edges.is_empty() {
            return None;
        }

        let mut nodes = vec![start];
        let mut curr_n = start;
        for &e_idx in &full_edges {
            let edge = graph.edge(e_idx);
            curr_n = if edge.start_node == curr_n {
                edge.end_node
            } else {
                edge.start_node
            };
            nodes.push(curr_n);
        }

        let f_dist = fwd_data.get(&(meeting_node, meeting_f_edge)).unwrap().1;
        let b_dist = bwd_data.get(&(meeting_node, meeting_b_edge)).unwrap().1;

        // Debug: log the first 20 paths per CCH build generation.
        // build_generation increments each time CchGraph::build() is called, so this
        // counter resets after every CCH rebuild (e.g., after lane connection changes).
        if crate::debug::is_traffic_enabled() {
            use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
            static LAST_GEN: AtomicU32 = AtomicU32::new(u32::MAX);
            static PATH_LOG_COUNT: AtomicU64 = AtomicU64::new(0);
            let prev_gen = LAST_GEN.swap(self.build_generation, Ordering::Relaxed);
            if prev_gen != self.build_generation {
                PATH_LOG_COUNT.store(0, Ordering::Relaxed);
                eprintln!(
                    "[CCH_QUERY] --- new CCH generation {} ---",
                    self.build_generation
                );
            }
            let count = PATH_LOG_COUNT.fetch_add(1, Ordering::Relaxed);
            if count < 20 {
                eprintln!(
                    "[CCH_QUERY] find_path {start}→{end}: nodes={nodes:?} edges={full_edges:?}"
                );
            }
            if count == 20 {
                eprintln!("[CCH_QUERY] (further path logs suppressed for this generation)");
            }
        }

        Some((min_total_cost, f_dist + b_dist, nodes))
    }

    fn is_turn_allowed(
        &self,
        node: u32,
        in_edge: usize,
        out_edge: usize,
        graph: &RegionGraph,
    ) -> bool {
        if in_edge == usize::MAX || out_edge == usize::MAX {
            return true;
        }
        Self::vehicle_turn_allowed(graph.node(node), in_edge, out_edge)
    }

    /// Returns whether a vehicle turn from `in_edge` to `out_edge` at `node_data` is
    /// permitted under the global whitelist model.
    ///
    /// - If the node has **no** user-defined vehicle connections (lane_idx ≠ ±100) the
    ///   junction is fully open and all turns are allowed.
    /// - If **any** user vehicle connection exists anywhere on the node, the entire node
    ///   enters whitelist mode: only explicitly listed turns are permitted; all others
    ///   are blocked regardless of which arm they come from.
    ///
    /// Pedestrian lane entries (lane_idx == ±100) are always ignored so that the
    /// presence of sidewalk connections does not accidentally engage whitelist mode.
    fn vehicle_turn_allowed(
        node_data: &crate::simulation::network::graph::data::Node,
        in_edge: usize,
        out_edge: usize,
    ) -> bool {
        // Global whitelist: if the node has any user vehicle connection, all unspecified
        // turns are blocked. If the node has no user connections, all turns are open.
        if !Self::node_has_vehicle_whitelist(node_data) {
            return true; // open node
        }
        // Whitelist mode: check explicit vehicle turns only.
        for (&(e_from, lane_idx), targets) in &node_data.lane_connections {
            if lane_idx != 100 && lane_idx != -100 && e_from == in_edge {
                if targets.iter().any(|&(e_to, _)| e_to == out_edge) {
                    traffic_log!(
                        "[TURN_ALLOWED] in_edge={in_edge} out_edge={out_edge} -> TRUE (explicit connection)"
                    );
                    return true;
                }
            }
        }
        traffic_log!(
            "[TURN_BLOCKED] in_edge={in_edge} out_edge={out_edge} (whitelist active, no match)"
        );
        false
    }

    fn node_has_vehicle_whitelist(node: &crate::simulation::network::graph::data::Node) -> bool {
        node.lane_connections
            .keys()
            .any(|&(_, lane)| lane != 100 && lane != -100)
    }

    /// Returns `true` if every turn in `path` is permitted under the current turn
    /// restrictions. A path with fewer than 3 nodes has no intermediate junctions and
    /// is always valid. Paths where consecutive edge IDs cannot be resolved are skipped
    /// (treated as open).
    ///
    /// Complexity: O(path.len() × node_degree) — cheap for typical city paths.
    pub fn path_has_valid_turns(path: &[u32], graph: &RegionGraph) -> bool {
        if path.len() < 3 {
            return true;
        }
        for i in 0..path.len() - 2 {
            let a = path[i];
            let jct = path[i + 1];
            let b = path[i + 2];
            let Some(in_e) = graph.get_edge_between_nodes(a, jct) else {
                continue;
            };
            let Some(out_e) = graph.get_edge_between_nodes(jct, b) else {
                continue;
            };
            if !Self::vehicle_turn_allowed(graph.node(jct), in_e, out_e) {
                return false;
            }
        }
        true
    }
}

fn fill_importance(degree: usize, triangles: u64) -> i64 {
    let degree = degree as u64;
    (degree * degree.saturating_sub(1) / 2 - triangles) as i64 - degree as i64
}

fn mark_order_score_changed(node: u32, marked: &mut [bool], changed: &mut Vec<u32>) {
    if !marked[node as usize] {
        marked[node as usize] = true;
        changed.push(node);
    }
}

#[derive(Copy, Clone, PartialEq)]
struct NodePriority {
    node: u32,
    importance: i64,
}

impl Eq for NodePriority {}

impl Ord for NodePriority {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .importance
            .cmp(&self.importance)
            .then_with(|| other.node.cmp(&self.node))
    }
}

impl PartialOrd for NodePriority {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Copy, Clone, PartialEq)]
struct CchState {
    priority: f32,
    cost: f32,
    node: u32,
    incoming_edge: usize,
}

impl Eq for CchState {}

impl Ord for CchState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .priority
            .partial_cmp(&self.priority)
            .unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for CchState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod ordering_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::{Edge, Node, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    use std::collections::HashMap;

    fn setup_test_graph() -> RegionGraph {
        let mut graph = RegionGraph::new();

        let nodes = vec![
            Node {
                pos: Vector3::new(0.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(1200.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 600.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
        ];

        let edge_defaults = Edge {
            start_node: 0,
            end_node: 0,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 20.0,
            base_cost: 30.0,
            physical_length: 600.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: Vec::new(),
            physical_geometry: Vec::new(),
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        };

        let edges = vec![
            Edge {
                start_node: 0,
                end_node: 1,
                physical_length: 600.0,
                ..edge_defaults.clone()
            }, // 0
            Edge {
                start_node: 1,
                end_node: 2,
                physical_length: 600.0,
                ..edge_defaults.clone()
            }, // 1
            Edge {
                start_node: 0,
                end_node: 3,
                physical_length: 600.0,
                base_cost: 10.0,
                ..edge_defaults.clone()
            }, // 2
            Edge {
                start_node: 3,
                end_node: 2,
                physical_length: 600.0,
                base_cost: 10.0,
                ..edge_defaults.clone()
            }, // 3
        ];

        graph.set_nodes_edges_for_test(nodes, edges);
        graph.rebuild_adjacency_list();
        graph
    }

    #[test]
    fn test_cch_pathfinding() {
        let graph = setup_test_graph();
        let cch = CchGraph::build(&graph);
        let path_opt = cch.find_path(0, 2, usize::MAX, &graph, TransitFlags::CAR);
        assert!(path_opt.is_some());
        let (cost, _dist, sequence) = path_opt.unwrap();
        // Base cost of path 0->3->2 is 10 + 10 = 20.
        assert_eq!(cost, 20.0);
        assert_eq!(sequence, vec![0, 3, 2]);
    }
}
