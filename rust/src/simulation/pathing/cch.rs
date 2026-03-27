//! Customizable Contraction Hierarchy (CCH) pathfinding.
//!
//! Provides fast queries and O(E) metric customization for dynamic traffic.
//! Replaces HPA* as the primary routing engine for agents.

use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{TransitFlags, TransitType};
macro_rules! godot_print {
    ($($arg:tt)*) => {
        #[cfg(not(test))]
        godot::prelude::godot_print!($($arg)*);
        #[cfg(test)]
        println!($($arg)*);
    }
}
use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};

/// A shortcut edge in the contracted graph.
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
    /// Sequence of concrete edge indices forming the path.
    pub inner_edges: Vec<usize>,
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
    /// Parent pointers in the elimination tree. O(E) customization follows this tree.
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
        }
    }

    /// Builds a new CCH from the provided road network.
    pub fn build(graph: &RegionGraph) -> Self {
        let n = graph.nodes.len();
        if n == 0 {
            return Self::new(0);
        }

        // Ensure graph is compacted and adjacency is fresh as per 31c requirements.
        // Note: compact_edges() should be called by the caller (TransitNetwork)
        // prior to this call to avoid mutating the graph here.

        let mut cch = Self::new(n);
        cch.compute_node_order(graph);
        cch.contract(graph);
        cch.customize(graph);

        cch
    }

    fn compute_node_order(&mut self, graph: &RegionGraph) {
        let n = graph.nodes.len();
        let mut adj: Vec<HashSet<u32>> = vec![HashSet::new(); n];
        for edge in &graph.edges {
            if edge.deleted {
                continue;
            }
            adj[edge.start_node as usize].insert(edge.end_node);
            adj[edge.end_node as usize].insert(edge.start_node);
        }

        let mut heap = BinaryHeap::new();
        let mut priorities = vec![0; n];
        for i in 0..n {
            let p = self.calculate_importance(i as u32, &adj);
            priorities[i] = p;
            heap.push(NodePriority {
                node: i as u32,
                importance: p,
            });
        }

        let mut rank = 0;
        let mut contracted = vec![false; n];

        while let Some(NodePriority { node, importance }) = heap.pop() {
            let u = node as usize;
            if contracted[u] {
                continue;
            }
            if importance > priorities[u] {
                let real_p = self.calculate_importance(node, &adj);
                priorities[u] = real_p;
                heap.push(NodePriority {
                    node,
                    importance: real_p,
                });
                continue;
            }

            contracted[u] = true;
            self.node_rank[u] = rank;
            self.node_order.push(node);
            rank += 1;

            let neighbors: Vec<u32> = adj[u].iter().cloned().collect();
            for &v in &neighbors {
                if contracted[v as usize] {
                    continue;
                }

                for &w in &neighbors {
                    if v == w || contracted[w as usize] {
                        continue;
                    }
                    adj[v as usize].insert(w);
                    adj[w as usize].insert(v);
                }

                adj[v as usize].remove(&node);
                let real_p = self.calculate_importance(v, &adj);
                priorities[v as usize] = real_p;
                heap.push(NodePriority {
                    node: v,
                    importance: real_p,
                });
            }
        }
    }

    fn calculate_importance(&self, u: u32, adj: &[HashSet<u32>]) -> i32 {
        let neighbors = &adj[u as usize];
        let mut shortcuts_needed = 0;
        let neighborhood: Vec<u32> = neighbors.iter().cloned().collect();
        for i in 0..neighborhood.len() {
            for j in i + 1..neighborhood.len() {
                let v = neighborhood[i];
                let w = neighborhood[j];
                if !adj[v as usize].contains(&w) {
                    shortcuts_needed += 1;
                }
            }
        }
        // Heuristic: shortcuts added - edges removed.
        shortcuts_needed as i32 - neighbors.len() as i32
    }

    fn contract(&mut self, graph: &RegionGraph) {
        let n = graph.nodes.len();

        // 1. Initial shortcuts from base edges
        for (edge_idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            }

            // Forward edge (start -> end)
            if edge.fwd_lanes > 0
                || (edge.primary_type == TransitType::Foot
                    && (edge.allowed_types & TransitFlags::FOOT != 0))
            {
                self.add_direct_shortcut(edge.start_node, edge.end_node, edge_idx, edge, true);
            }
            // Backward edge (end -> start)
            if edge.bkw_lanes > 0
                || (edge.primary_type == TransitType::Foot
                    && (edge.allowed_types & TransitFlags::FOOT != 0))
            {
                self.add_direct_shortcut(edge.end_node, edge.start_node, edge_idx, edge, false);
            }
        }

        // 2. Triangle insertion following node rank
        for rank in 0..n {
            let u = self.node_order[rank];

            // Find neighbors in hierarchy (rank > u)
            // Incoming to u: v -> u where rank(v) > rank(u). But also we need v -> u where rank(v) is anything
            // for the contraction phase. Wait, CCH contraction: for every pair of neighbors.

            // Collect all current shortcuts connected to u
            let mut neighbors_in = Vec::new(); // Shortcuts S -> u
            let mut neighbors_out = Vec::new(); // Shortcuts u -> T

            for (idx, s) in self.shortcuts.iter().enumerate() {
                if s.target_node == u {
                    neighbors_in.push(idx);
                }
                if s.start_node == u {
                    neighbors_out.push(idx);
                }
            }

            for &idx_in in &neighbors_in {
                let (s_in_start, s_in_last, s_in_inner, s_in_first, s_in_mask) = {
                    let s = &self.shortcuts[idx_in];
                    (
                        s.start_node,
                        s.last_edge,
                        s.inner_edges.clone(),
                        s.first_edge,
                        s.allowed_types,
                    )
                };

                for &idx_out in &neighbors_out {
                    let (s_out_target, s_out_first, s_out_inner, s_out_last, s_out_mask) = {
                        let s = &self.shortcuts[idx_out];
                        (
                            s.target_node,
                            s.first_edge,
                            s.inner_edges.clone(),
                            s.last_edge,
                            s.allowed_types,
                        )
                    };

                    if s_in_start == s_out_target {
                        continue;
                    }

                    // Check turn restrictions at u
                    if !graph.nodes[u as usize].lane_connections.is_empty() {
                        let conn = &graph.nodes[u as usize].lane_connections;
                        let mut allowed = false;
                        for (&(e_from, _), targets) in conn {
                            if e_from == s_in_last {
                                if targets.iter().any(|&(e_to, _)| e_to == s_out_first) {
                                    allowed = true;
                                    break;
                                }
                            }
                        }
                        if !allowed {
                            continue;
                        }
                    }

                    // Form shortcut v -> w
                    let mut inner = s_in_inner.clone();
                    inner.extend(&s_out_inner);

                    self.add_shortcut(
                        s_in_start,
                        s_out_target,
                        inner,
                        s_in_first,
                        s_out_last,
                        s_in_mask & s_out_mask,
                    );
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
        _is_fwd: bool,
    ) {
        self.shortcuts.push(CchShortcut {
            start_node: start,
            target_node: end,
            cost: 0.0, // Updated in customize
            dist: edge.physical_length,
            inner_edges: vec![edge_idx],
            first_edge: edge_idx,
            last_edge: edge_idx,
            allowed_types: edge.allowed_types,
        });
    }

    fn add_shortcut(
        &mut self,
        start: u32,
        end: u32,
        inner: Vec<usize>,
        first: usize,
        last: usize,
        mask: u8,
    ) {
        self.shortcuts.push(CchShortcut {
            start_node: start,
            target_node: end,
            cost: 0.0,
            dist: 0.0,
            inner_edges: inner,
            first_edge: first,
            last_edge: last,
            allowed_types: mask,
        });
    }

    /// Updates shortcut costs based on current dynamic edge costs.
    pub fn customize(&mut self, graph: &RegionGraph) {
        let mut max_speed = 1.0_f32;
        for edge in &graph.edges {
            if !edge.deleted {
                max_speed = max_speed.max(edge.speed_limit);
            }
        }
        self.max_v = max_speed;

        // 1. Update all shortcut costs in one pass
        for idx in 0..self.shortcuts.len() {
            self.update_shortcut_cost(idx, graph);
        }

        // 2. Prune fwd_up and bwd_up following elimination tree order
        for rank in 0..graph.nodes.len() {
            let u = self.node_order[rank] as usize;

            // Deduplicate fwd_up[u]: best shortcut per (target, first_edge, last_edge)
            let mut best_fwd: HashMap<(u32, usize, usize), (usize, f32)> = HashMap::new();
            for &idx in &self.fwd_up[u] {
                let s = &self.shortcuts[idx];
                let key = (s.target_node, s.first_edge, s.last_edge);
                if s.cost < best_fwd.get(&key).map(|&(_, c)| c).unwrap_or(f32::MAX) {
                    best_fwd.insert(key, (idx, s.cost));
                }
            }
            self.fwd_up[u] = best_fwd.values().map(|(idx, _)| *idx).collect();

            // Deduplicate bwd_up[u]: best shortcut per (start, first_edge, last_edge)
            let mut best_bwd: HashMap<(u32, usize, usize), (usize, f32)> = HashMap::new();
            for &idx in &self.bwd_up[u] {
                let s = &self.shortcuts[idx];
                let key = (s.start_node, s.first_edge, s.last_edge);
                if s.cost < best_bwd.get(&key).map(|&(_, c)| c).unwrap_or(f32::MAX) {
                    best_bwd.insert(key, (idx, s.cost));
                }
            }
            self.bwd_up[u] = best_bwd.values().map(|(idx, _)| *idx).collect();
        }
    }

    fn update_shortcut_cost(&mut self, idx: usize, graph: &RegionGraph) {
        let shortcut = &mut self.shortcuts[idx];
        let mut total_cost = 0.0;
        let mut total_dist = 0.0;
        for &e_idx in &shortcut.inner_edges {
            let edge = &graph.edges[e_idx];
            total_cost += edge.base_cost * (1.0 + edge.current_congestion);
            total_dist += edge.physical_length;
        }
        shortcut.cost = total_cost;
        shortcut.dist = total_dist;
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
                    break;
                }
                let (cost, node, l_edge) = (state.cost, state.node, state.incoming_edge);
                if let Some(&(best_cost, _, _, _)) = fwd_data.get(&(node, l_edge)) {
                    if cost > best_cost {
                        continue;
                    }

                    for (&(b_node, b_out_edge), &(b_cost, _, _, _)) in &bwd_data {
                        if b_node == node && self.is_turn_allowed(node, l_edge, b_out_edge, graph) {
                            if cost + b_cost < min_total_cost {
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
                    break;
                }
                let (cost, node, outgoing_edge) = (state.cost, state.node, state.incoming_edge);
                if let Some(&(best_cost, _, _, _)) = bwd_data.get(&(node, outgoing_edge)) {
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
            full_edges.extend(&self.shortcuts[idx].inner_edges);
        }

        // Backward part: MEETING -> TARGET
        let mut curr_key = (meeting_node, meeting_b_edge);
        while let Some(&(_, _, s_opt, next_key)) = bwd_data.get(&curr_key) {
            if let Some(s_idx) = s_opt {
                full_edges.extend(&self.shortcuts[s_idx].inner_edges);
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
            let edge = &graph.edges[e_idx];
            curr_n = if edge.start_node == curr_n {
                edge.end_node
            } else {
                edge.start_node
            };
            nodes.push(curr_n);
        }

        let f_dist = fwd_data.get(&(meeting_node, meeting_f_edge)).unwrap().1;
        let b_dist = bwd_data.get(&(meeting_node, meeting_b_edge)).unwrap().1;

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
        let node_data = &graph.nodes[node as usize];
        if node_data.lane_connections.is_empty() {
            return true;
        }

        // Check if any lane from in_edge can turn to any lane in out_edge
        for (&(e_from, _), targets) in &node_data.lane_connections {
            if e_from == in_edge {
                if targets.iter().any(|&(e_to, _)| e_to == out_edge) {
                    return true;
                }
            }
        }
        false
    }

    fn find_first_from_last(
        &self,
        u: u32,
        e_in: usize,
        v: u32,
        e_out: usize,
        graph: &RegionGraph,
    ) -> usize {
        // Find which shortcut starts at u, ends at v, and ends with e_out, and follows e_in
        for &idx in &self.fwd_up[u as usize] {
            let s = &self.shortcuts[idx];
            if s.target_node == v && s.last_edge == e_out {
                if self.is_turn_allowed(u, e_in, s.first_edge, graph) {
                    return s.first_edge;
                }
            }
        }
        e_out // Fallback
    }

    fn find_last_from_first(
        &self,
        u: u32,
        e_in: usize,
        v: u32,
        e_out: usize,
        graph: &RegionGraph,
    ) -> usize {
        for &idx in &self.fwd_up[u as usize] {
            let s = &self.shortcuts[idx];
            if s.target_node == v && s.first_edge == e_in {
                if self.is_turn_allowed(v, s.last_edge, e_out, graph) {
                    return s.last_edge;
                }
            }
        }
        e_in
    }
}

#[derive(Copy, Clone, PartialEq)]
struct NodePriority {
    node: u32,
    importance: i32,
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
mod tests {
    use super::*;
    use crate::simulation::network::graph::{Edge, Node, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    use std::collections::HashMap;

    fn setup_test_graph() -> RegionGraph {
        let mut graph = RegionGraph::new();
        graph.nodes = vec![
            Node {
                pos: Vector3::new(0.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
            },
            Node {
                pos: Vector3::new(1200.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 600.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
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
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        };

        graph.edges = vec![
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
