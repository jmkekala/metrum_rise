//! Hierarchical Pathfinding A* (HPA*) — two-phase routing over the road graph.
//!
//! # How it works
//!
//! **Build phase** ([`HpaGraph::build`]): nodes that sit on a 512 m chunk boundary
//! are designated *abstract nodes*. For each chunk, Dijkstra is run from every
//! abstract entry node to every other abstract entry node within that chunk,
//! respecting turn restrictions. The resulting inter-node costs are stored as
//! [`AbstractEdge`]s in [`HpaGraph::abstract_edges`].
//!
//! **Query phase** ([`HpaGraph::find_path`]): **currently broken** — the pre-built
//! abstract graph is ignored and a full A* is run on the concrete graph instead.
//! Fix required before v0.01: route long-distance queries through the abstract graph,
//! falling back to local A* only within the source and destination chunks.
//!
//! The build phase is correct and should not be modified until the query phase is fixed.

use crate::simulation::network::graph::TransitGraph;
use crate::simulation::network::types::TransitType;
use super::astar::State;
use std::collections::{HashMap, BinaryHeap, HashSet};

/// A directed edge in the abstract (chunk-boundary) graph produced by the HPA* build phase.
#[derive(Clone)]
pub struct AbstractEdge {
    /// The abstract node this edge leads to.
    pub target: u32,
    /// Pre-computed traversal cost (seconds) along the intra-chunk shortest path.
    pub cost: f32,
    /// Sequence of concrete node IDs forming the intra-chunk path from source to `target`,
    /// used to expand the abstract route back to a concrete road sequence.
    pub inner_path: Vec<u32>,
}

/// Pre-computed hierarchical graph used to accelerate long-distance pathfinding queries.
///
/// Built once by [`HpaGraph::build`] after every road-network change.
/// Queries are issued via [`HpaGraph::find_path`].
pub struct HpaGraph {
    /// Outgoing abstract edges keyed by source node ID.
    /// Only nodes in [`is_abstract`] appear as keys.
    pub abstract_edges: HashMap<u32, Vec<AbstractEdge>>,
    /// Abstract (chunk-boundary) node IDs present in each 512 m chunk, keyed by `(chunk_x, chunk_z)`.
    pub chunk_entries: HashMap<(i32, i32), Vec<u32>>,
    /// Set of node IDs that lie on a chunk boundary and participate in the abstract graph.
    pub is_abstract: HashSet<u32>,
    /// Cached concrete adjacency list for fast per-chunk A* and long-distance queries.
    /// Index is node ID; value is a list of `(target_node, edge_index, cost)`.
    pub concrete_adj: Vec<Vec<(u32, usize, f32)>>,
    /// Reverse adjacency list for backward searches (goal to boundary).
    pub concrete_rev_adj: Vec<Vec<(u32, usize, f32)>>,
}

impl HpaGraph {
    /// Returns an empty `HpaGraph`. Call [`build`](Self::build) to populate it.
    pub fn new() -> Self {
        Self {
            abstract_edges: HashMap::new(),
            chunk_entries: HashMap::new(),
            is_abstract: HashSet::new(),
            concrete_adj: Vec::new(),
            concrete_rev_adj: Vec::new(),
        }
    }

    /// Constructs the abstract graph from the current road network.
    ///
    /// Nodes that straddle a 512 m chunk boundary become abstract nodes.
    /// Per-chunk Dijkstra computes intra-chunk costs between all abstract entries.
    /// Must be called (and rebuilt) after every structural road-network change.
    pub fn build(graph: &TransitGraph) -> Self {
        let start_time = std::time::Instant::now();
        let mut hpa = HpaGraph::new();
        let n = graph.nodes.len();
        if n == 0 { return hpa; }

        // 1. Build concrete adjacency list (cached for future find_path calls)
        hpa.concrete_adj = vec![Vec::new(); n];
        hpa.concrete_rev_adj = vec![Vec::new(); n];
        for (idx, edge) in graph.edges.iter().enumerate() {
            if edge.deleted { continue; }
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            let can_fwd = edge.fwd_lanes > 0 || edge.primary_type == TransitType::Foot;
            let can_bkw = edge.bkw_lanes > 0 || edge.primary_type == TransitType::Foot;

            if can_fwd {
                hpa.concrete_adj[edge.start_node as usize].push((edge.end_node, idx, cost));
                hpa.concrete_rev_adj[edge.end_node as usize].push((edge.start_node, idx, cost));
            }
            if can_bkw {
                hpa.concrete_adj[edge.end_node as usize].push((edge.start_node, idx, cost));
                hpa.concrete_rev_adj[edge.start_node as usize].push((edge.end_node, idx, cost));
            }
            
            let chunk_a = graph.get_node_chunk(edge.start_node);
            let chunk_b = graph.get_node_chunk(edge.end_node);
            
            if chunk_a != chunk_b {
                hpa.is_abstract.insert(edge.start_node);
                hpa.is_abstract.insert(edge.end_node);
                
                hpa.chunk_entries.entry(chunk_a).or_default().push(edge.start_node);
                hpa.chunk_entries.entry(chunk_b).or_default().push(edge.end_node);
                
                hpa.abstract_edges.entry(edge.start_node).or_default().push(AbstractEdge {
                    target: edge.end_node, cost, inner_path: vec![edge.end_node]
                });
                hpa.abstract_edges.entry(edge.end_node).or_default().push(AbstractEdge {
                    target: edge.start_node, cost, inner_path: vec![edge.start_node]
                });
            }
        }

        for entries in hpa.chunk_entries.values_mut() {
            entries.sort();
            entries.dedup();
        }

        for (&chunk, entries) in &hpa.chunk_entries {
            for &start_node in entries {
                let mut costs: HashMap<(u32, usize), (f32, f32)> = HashMap::new();
                let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
                let mut heap = BinaryHeap::new();
                
                costs.insert((start_node, usize::MAX), (0.0, 0.0));
                heap.push(State { priority: 0.0, cost: 0.0, dist: 0.0, node: start_node, incoming_edge: usize::MAX });
                
                while let Some(State { priority: _, cost, dist, node, incoming_edge }) = heap.pop() {
                    if cost > costs.get(&(node, incoming_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 { continue; }
                    
                    for &(neighbor, out_edge, edge_cost) in &hpa.concrete_adj[node as usize] {
                        if graph.get_node_chunk(neighbor) != chunk { continue; }
                        
                        if incoming_edge != usize::MAX && incoming_edge != out_edge {
                            let mut has_any = false;
                            let mut valid = false;
                            let n_ref = &graph.nodes[node as usize];
                            for (src, tgts) in &n_ref.lane_connections {
                                if src.0 == incoming_edge { 
                                    has_any = true;
                                    for t in tgts {
                                        if t.0 == out_edge { valid = true; break; }
                                    }
                                }
                            }
                            if has_any && !valid { continue; }
                        }
                        
                        let next_cost = cost + edge_cost;
                        let next_dist = dist + graph.edges[out_edge].physical_length;
                        if next_cost < costs.get(&(neighbor, out_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 {
                            costs.insert((neighbor, out_edge), (next_cost, next_dist));
                            prev.insert((neighbor, out_edge), (node, incoming_edge));
                            heap.push(State { priority: next_cost, cost: next_cost, dist: next_dist, node: neighbor, incoming_edge: out_edge });
                        }
                    }
                }
                
                for &end_node in entries {
                    if start_node != end_node {
                        let mut best_inc = usize::MAX;
                        let mut min_c = f32::MAX;
                        for (&(n, inc), &(c, _d)) in &costs {
                            if n == end_node && c < min_c {
                                min_c = c;
                                best_inc = inc;
                            }
                        }
                        
                        if best_inc != usize::MAX {
                            let mut path = Vec::new();
                            let mut curr = (end_node, best_inc);
                            while curr.0 != start_node {
                                path.push(curr.0);
                                curr = *prev.get(&curr).unwrap();
                            }
                            path.reverse();
                            
                            hpa.abstract_edges.entry(start_node).or_default().push(AbstractEdge {
                                target: end_node,
                                cost: min_c,
                                inner_path: path
                            });
                        }
                    }
                }
            }
        }
        // Logging disabled in tests to avoid Godot FFI initialization errors
        #[cfg(not(test))]
        {
            godot::prelude::godot_print!("HPA* Build completed in {:.2}ms ({} abstract nodes, {} abstract edges)", 
                start_time.elapsed().as_secs_f32() * 1000.0,
                hpa.is_abstract.len(),
                hpa.abstract_edges.values().map(|v| v.len()).sum::<usize>()
            );
        }

        hpa
    }

    /// Finds a path from `start_raw` to `end_raw` and returns `(time_cost, distance_m, node_sequence)`.
    ///
    /// `start_edge` is the edge the agent is currently traversing (`usize::MAX` if at a bare node).
    /// If `pedestrian` is `true`, road edges are heavily penalised and one-way restrictions are ignored.
    pub fn find_path(&self, start_raw: u32, end_raw: u32, start_edge: usize, graph: &TransitGraph, pedestrian: bool) -> Option<(f32, f32, Vec<u32>)> {
        let start = graph.get_valid_node(start_raw);
        let end = graph.get_valid_node(end_raw);

        if self.concrete_adj.is_empty() { return None; }
        if start as usize >= graph.nodes.len() || end as usize >= graph.nodes.len() { return None; }
        if start == end { return Some((0.0, 0.0, vec![start])); }
        
        let start_chunk = graph.get_node_chunk(start);
        let end_chunk = graph.get_node_chunk(end);

        // Path optimization: If start/end in same chunk, use local concrete search
        if start_chunk == end_chunk {
            return self.local_concrete_search(start, end, start_edge, graph, pedestrian, Some(start_chunk));
        }

        // --- Phase A: Start local search ---
        let start_borders = if self.is_abstract.contains(&start) {
            let mut results = HashMap::new();
            results.insert(start, (0.0, 0.0, start_edge, Vec::new()));
            results
        } else {
            self.local_boundary_search(start, start_edge, start_chunk, graph, pedestrian, false)
        };
        if start_borders.is_empty() { return None; }

        // --- Phase B: End local search ---
        let end_borders = if self.is_abstract.contains(&end) {
            let mut results = HashMap::new();
            results.insert(end, (0.0, 0.0, usize::MAX, Vec::new()));
            results
        } else {
            self.local_boundary_search(end, usize::MAX, end_chunk, graph, pedestrian, true)
        };
        if end_borders.is_empty() { return None; }

        // Phase C: A* on the abstract graph
        let mut h = BinaryHeap::new();
        let mut costs: HashMap<u32, (f32, f32)> = HashMap::new();
        let mut prev: HashMap<u32, (u32, Vec<u32>)> = HashMap::new();

        for (node, (c, d, _inc, _path)) in &start_borders {
            costs.insert(*node, (*c, *d));
            let h_val = graph.nodes[*node as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
            h.push(State { priority: *c + h_val, cost: *c, dist: *d, node: *node, incoming_edge: usize::MAX });
        }

        let mut best_end_node = u32::MAX;
        let mut min_total_cost = f32::MAX;

        while let Some(State { priority: _, cost, dist, node, incoming_edge: _ }) = h.pop() {
            if let Some((ec, _ed, _ei, _ep)) = end_borders.get(&node) {
                let total_c = cost + ec;
                if total_c < min_total_cost {
                    min_total_cost = total_c;
                    best_end_node = node;
                }
                // We don't break yet because a better path might exist through another abstract node
            }
            
            if cost > costs.get(&node).unwrap_or(&(f32::MAX, f32::MAX)).0 { continue; }

            if let Some(abs_edges) = self.abstract_edges.get(&node) {
                for abs in abs_edges {
                    // Transit type filtering for abstract edges
                    if let Some(edge_idx) = graph.get_edge_between_nodes(node, abs.target) {
                        let edge = &graph.edges[edge_idx];
                        if pedestrian {
                            if (edge.allowed_types & 1) == 0 { continue; }
                        } else {
                            if (edge.allowed_types & 2) == 0 || edge.fwd_lanes == 0 { continue; }
                        }
                    }

                    let next_cost = cost + abs.cost;
                    let next_dist = dist + 0.0; // Abstract edges don't track distance yet (B2 focus on cost/time)
                    
                    if next_cost < costs.get(&abs.target).unwrap_or(&(f32::MAX, f32::MAX)).0 {
                        costs.insert(abs.target, (next_cost, next_dist));
                        prev.insert(abs.target, (node, abs.inner_path.clone()));
                        let h_val = graph.nodes[abs.target as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
                        h.push(State { priority: next_cost + h_val, cost: next_cost, dist: next_dist, node: abs.target, incoming_edge: usize::MAX });
                    }
                }
            }
        }

        if best_end_node != u32::MAX {
            // Reconstruct path (StartConnection + AbstractEdges + EndConnection)
            let mut full_path = Vec::new();
            
            // 1. Start connection (start -> node_where_abstract_search_started)
            // We need to find which 'curr' node in start_borders we ended up using.
            // Walk back from best_end_node via 'prev' to find the start_border node.
            let mut route = Vec::new();
            let mut node = best_end_node;
            let mut loop_cap = 10000;
            while let Some((prev_node, inner_path)) = prev.get(&node) {
                route.push(inner_path.clone());
                node = *prev_node;
                loop_cap -= 1;
                if loop_cap == 0 { panic!("HPA* Path Reconstruction: Infinite loop in abstract path!"); }
            }
            let abstract_start_node = node;
            
            // Start connection: start -> abstract_start_node
            let (_, _, _, mut start_p) = start_borders.get(&abstract_start_node).unwrap().clone();
            start_p.reverse(); // [abstract_start_node, ..., neighbor_of_start]
            full_path.push(start);
            full_path.extend(start_p);
            
            // 2. Abstract edges: abstract_start_node -> best_end_node
            route.reverse(); // [inner_path_start_to_next, ..., inner_path_prev_to_best_end]
            for segment in route {
                // segment is [node_after_prev, ..., curr_node]
                full_path.extend(segment);
            }
            
            // 3. End connection (best_end_node -> end)
            let (_, _, _, mut end_p) = end_borders.get(&best_end_node).unwrap().clone();
            end_p.reverse(); // [best_end_node, ..., neighbor_of_end]
            if !end_p.is_empty() {
                // Skip the first node if it's already there (it's best_end_node)
                if !full_path.is_empty() && end_p[0] == *full_path.last().unwrap() {
                    full_path.extend(&end_p[1..]);
                } else {
                    full_path.extend(end_p);
                }
            }
            
            if full_path.is_empty() || *full_path.last().unwrap() != end {
                full_path.push(end);
            }
            
            full_path.dedup();

            // Calculate exact distance of the final path
            let mut actual_dist = 0.0;
            for i in 0..full_path.len().saturating_sub(1) {
                if let Some(edge_idx) = graph.get_edge_between_nodes(full_path[i], full_path[i+1]) {
                    actual_dist += graph.edges[edge_idx].physical_length;
                }
            }

            return Some((min_total_cost, actual_dist, full_path));
        }

        None
    }

    /// Internal: Run concrete A* over a specific chunk or globally (fallback).
    /// 
    /// Used for short-distance paths or intra-chunk routing.
    fn local_concrete_search(&self, start: u32, end: u32, start_edge: usize, graph: &TransitGraph, pedestrian: bool, chunk: Option<(i32, i32)>) -> Option<(f32, f32, Vec<u32>)> {
        let mut h = BinaryHeap::new();
        let mut costs: HashMap<(u32, usize), (f32, f32)> = HashMap::new();
        let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
        let mut final_inc = usize::MAX;
        
        costs.insert((start, start_edge), (0.0, 0.0));
        h.push(State { priority: 0.0, cost: 0.0, dist: 0.0, node: start, incoming_edge: start_edge });
        
        while let Some(State { priority: _, cost, dist, node, incoming_edge }) = h.pop() {
            if node == end { final_inc = incoming_edge; break; }
            if cost > costs.get(&(node, incoming_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 { continue; }
            
            for &(neighbor, out_edge, mut edge_cost) in &self.concrete_adj[node as usize] {
                let edge = &graph.edges[out_edge];
                
                // Transit type filtering
                if pedestrian {
                    if (edge.allowed_types & 1) == 0 { continue; }
                    if edge.primary_type == crate::simulation::network::types::TransitType::Road {
                        edge_cost *= 10.0;
                    }
                } else {
                    if (edge.allowed_types & 2) == 0 || edge.fwd_lanes == 0 { continue; }
                    // Turn restriction check
                    if incoming_edge != usize::MAX && incoming_edge != out_edge {
                        let mut has_any = false;
                        let mut valid = false;
                        let n_ref = &graph.nodes[node as usize];
                        for (src, tgts) in &n_ref.lane_connections {
                            if src.0 == incoming_edge {
                                has_any = true;
                                for t in tgts {
                                    if t.0 == out_edge { valid = true; break; }
                                }
                            }
                        }
                        if has_any && !valid { continue; }
                    }
                }
                
                let next_cost = cost + edge_cost;
                let next_dist = dist + graph.edges[out_edge].physical_length;
                if next_cost < costs.get(&(neighbor, out_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 {
                    costs.insert((neighbor, out_edge), (next_cost, next_dist));
                    prev.insert((neighbor, out_edge), (node, incoming_edge));
                    let h_val = graph.nodes[neighbor as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
                    h.push(State { priority: next_cost + h_val, cost: next_cost, dist: next_dist, node: neighbor, incoming_edge: out_edge });
                }
            }
        }
        
        if let Some(&(fc, fd)) = costs.get(&(end, final_inc)) {
            let mut path = Vec::new();
            let mut curr_state = (end, final_inc);
            while curr_state.0 != start {
                path.push(curr_state.0);
                curr_state = *prev.get(&curr_state).unwrap();
            }
            path.push(start);
            path.reverse();
            return Some((fc, fd, path));
        }
        None
    }

    /// Internal: Find all abstract nodes reachable within a chunk via local traversal.
    /// 
    /// Returns a map of abstract node IDs to their cost, distance, and the path taken.
    fn local_boundary_search(&self, start: u32, start_edge: usize, chunk: (i32, i32), graph: &TransitGraph, pedestrian: bool, backward: bool) -> HashMap<u32, (f32, f32, usize, Vec<u32>)> {
        let mut h = BinaryHeap::new();
        let mut costs: HashMap<(u32, usize), (f32, f32)> = HashMap::new();
        let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
        
        costs.insert((start, start_edge), (0.0, 0.0));
        h.push(State { priority: 0.0, cost: 0.0, dist: 0.0, node: start, incoming_edge: start_edge });
        
        let mut results: HashMap<u32, (f32, f32, usize, Vec<u32>)> = HashMap::new();
        let adj_ref = if backward { &self.concrete_rev_adj } else { &self.concrete_adj };

        if self.is_abstract.contains(&start) {
            results.insert(start, (0.0, 0.0, start_edge, Vec::new()));
        }

        while let Some(State { priority: _, cost, dist, node, incoming_edge }) = h.pop() {
            if cost > costs.get(&(node, incoming_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 { continue; }
            
            if node != start && self.is_abstract.contains(&node) {
                // Found a boundary node
                if !results.contains_key(&node) || cost < results.get(&node).unwrap().0 {
                    let mut path = Vec::new();
                    let mut curr = (node, incoming_edge);
                    let mut loop_cap = 10000;
                    while curr.0 != start {
                        path.push(curr.0);
                        curr = *prev.get(&curr).expect("HPA* local_boundary_search: Predecessor missing!");
                        loop_cap -= 1;
                        if loop_cap == 0 { panic!("HPA* local_boundary_search: Infinite loop in path reconstruction!"); }
                    }
                    results.insert(node, (cost, dist, incoming_edge, path));
                }
                // Do not stop! Continue search to find other boundaries
            }

            for &(neighbor, out_edge, mut edge_cost) in &adj_ref[node as usize] {
                let edge = &graph.edges[out_edge];
                if graph.get_node_chunk(neighbor) != chunk { continue; }

                // Transit type filtering
                if pedestrian {
                    if (edge.allowed_types & 1) == 0 { continue; }
                    if edge.primary_type == crate::simulation::network::types::TransitType::Road {
                        edge_cost *= 10.0;
                    }
                } else {
                    if (edge.allowed_types & 2) == 0 { continue; }
                    // Note: backward search also checks fwd_lanes vs bkw_lanes implicitly by adj_ref
                    if !backward && edge.fwd_lanes == 0 { continue; }

                    if !backward { // Turn restrictions only apply forward for now in this impl
                        if incoming_edge != usize::MAX && incoming_edge != out_edge {
                            let mut has_any = false;
                            let mut valid = false;
                            let n_ref = &graph.nodes[node as usize];
                            for (src, tgts) in &n_ref.lane_connections {
                                if src.0 == incoming_edge {
                                    has_any = true;
                                    for t in tgts {
                                        if t.0 == out_edge { valid = true; break; }
                                    }
                                }
                            }
                            if has_any && !valid { continue; }
                        }
                    }
                }
                
                let next_cost = cost + edge_cost;
                let next_dist = dist + graph.edges[out_edge].physical_length;
                if next_cost < costs.get(&(neighbor, out_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 {
                    costs.insert((neighbor, out_edge), (next_cost, next_dist));
                    prev.insert((neighbor, out_edge), (node, incoming_edge));
                    h.push(State { priority: next_cost, cost: next_cost, dist: next_dist, node: neighbor, incoming_edge: out_edge });
                }
            }
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::{TransitGraph, Edge};
    use crate::simulation::network::types::{TransitType, NodeType, TransitFlags};
    use godot::prelude::Vector3;

    fn setup_test_graph() -> TransitGraph {
        let mut graph = TransitGraph::new();
        // Chunk boundary at 512m. We place nodes to span chunks.
        // Chunk (0,0): 0,0,0
        graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction); // 0
        // Chunk (1,0): 600,0,0
        graph.add_node(Vector3::new(600.0, 0.0, 0.0), NodeType::Junction); // 1
        // Chunk (1,1): 600,0,600
        graph.add_node(Vector3::new(600.0, 0.0, 600.0), NodeType::Junction); // 2
        // Middle nodes
        graph.add_node(Vector3::new(300.0, 0.0, 0.0), NodeType::Junction); // 3 (Inside Chunk (0,0))
        
        // Edge 0->3 (Chunk (0,0) to Chunk (0,0))
        graph.add_edge(Edge {
            start_node: 0, end_node: 3,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 10.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 20.0, base_cost: 15.0,
            physical_length: 300.0, physical_geometry: vec![Vector3::ZERO, Vector3::new(300.0,0.0,0.0)],
            deleted: false, ..Edge::default_test()
        }); // Edge 0
        
        // Edge 3->1 (Chunk (0,0) to Chunk (1,0)) -> This crosses a boundary!
        graph.add_edge(Edge {
            start_node: 3, end_node: 1,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 10.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 20.0, base_cost: 15.0,
            physical_length: 300.0, physical_geometry: vec![Vector3::new(300.0,0.0,0.0), Vector3::new(600.0,0.0,0.0)],
            deleted: false, ..Edge::default_test()
        }); // Edge 1
        
        // Edge 1->2 (Chunk (1,0) to Chunk (1,1)) -> This crosses a boundary!
        graph.add_edge(Edge {
            start_node: 1, end_node: 2,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 10.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 20.0, base_cost: 30.0,
            physical_length: 600.0, physical_geometry: vec![Vector3::new(600.0,0.0,0.0), Vector3::new(600.0,0.0,600.0)],
            deleted: false, ..Edge::default_test()
        }); // Edge 2
        
        graph
    }

    impl Edge {
        fn default_test() -> Self {
            Self {
                start_node: 0, end_node: 0, primary_type: TransitType::Road, allowed_types: 0,
                width: 0.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 0.0, base_cost: 0.0,
                physical_length: 0.0, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
                geometry: Vec::new(), physical_geometry: Vec::new(), parking_occupied: 0,
                zoning_left: false, zoning_right: false, deleted: false
            }
        }
    }

    #[test]
    fn test_hierarchical_pathbuilding() {
        let graph = setup_test_graph();
        let hpa = HpaGraph::build(&graph);
        
        // Abstract nodes should be 3 and 1 (they flank the chunk boundary 512m)
        // Actually nodes 3 and 1 cross the boundary, so they are abstract.
        assert!(hpa.is_abstract.contains(&3));
        assert!(hpa.is_abstract.contains(&1));
        
        // There should be a path in abstract graph between 3 and 1
        assert!(hpa.abstract_edges.contains_key(&3));
        let edges = hpa.abstract_edges.get(&3).unwrap();
        assert!(edges.iter().any(|e| e.target == 1));
    }

    #[test]
    fn test_find_path_hierarchical() {
        let graph = setup_test_graph();
        let hpa = HpaGraph::build(&graph);
        
        // Path from 0 to 2 (span chunks 0,0 -> 1,0 -> 1,1)
        let path_opt = hpa.find_path(0, 2, usize::MAX, &graph, false);
        assert!(path_opt.is_some());
        let (cost, dist, sequence) = path_opt.unwrap();
        
        assert_eq!(sequence, vec![0, 3, 1, 2]);
        assert!(cost > 0.0);
        assert_eq!(dist, 1200.0);
    }
    
    #[test]
    fn test_pedestrian_path() {
        let mut graph = setup_test_graph();
        // Add a shortcut that is Foot-only
        graph.add_node(Vector3::new(300.0, 0.0, 300.0), NodeType::Junction); // 4
        graph.add_edge(Edge {
            start_node: 0, end_node: 4,
            primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
            width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 4.0, base_cost: 1.0,
            physical_length: 424.0, physical_geometry: vec![Vector3::ZERO, Vector3::new(300.0,0.0,300.0)],
            deleted: false, ..Edge::default_test()
        });
        graph.add_edge(Edge {
            start_node: 4, end_node: 2,
            primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
            width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 4.0, base_cost: 1.0,
            physical_length: 424.0, physical_geometry: vec![Vector3::new(300.0,0.0,300.0), Vector3::new(600.0,0.0,600.0)],
            deleted: false, ..Edge::default_test()
        });
        
        let hpa = HpaGraph::build(&graph);
        
        // Pedestrian should take the shortcut (0->4->2)
        let path_p = hpa.find_path(0, 2, usize::MAX, &graph, true).unwrap();
        assert!(path_p.2.contains(&4));
        
        // Car should NOT take the shortcut (0->3->1->2)
        let path_c = hpa.find_path(0, 2, usize::MAX, &graph, false).unwrap();
        assert!(!path_c.2.contains(&4));
    }
}
