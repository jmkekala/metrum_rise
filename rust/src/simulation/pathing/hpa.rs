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
}

impl HpaGraph {
    /// Returns an empty `HpaGraph`. Call [`build`](Self::build) to populate it.
    pub fn new() -> Self {
        Self {
            abstract_edges: HashMap::new(),
            chunk_entries: HashMap::new(),
            is_abstract: HashSet::new(),
        }
    }

    /// Constructs the abstract graph from the current road network.
    ///
    /// Nodes that straddle a 512 m chunk boundary become abstract nodes.
    /// Per-chunk Dijkstra computes intra-chunk costs between all abstract entries.
    /// Must be called (and rebuilt) after every structural road-network change.
    pub fn build(graph: &TransitGraph) -> Self {
        let mut hpa = Self::new();
        let n = graph.nodes.len();
        if n == 0 { return hpa; }

        let mut adj: Vec<Vec<(u32, usize, f32)>> = vec![Vec::new(); n];
        for (idx, edge) in graph.edges.iter().enumerate() {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            let can_fwd = edge.fwd_lanes > 0 || edge.primary_type == TransitType::Foot;
            let can_bkw = edge.bkw_lanes > 0 || edge.primary_type == TransitType::Foot;

            if can_fwd {
                adj[edge.start_node as usize].push((edge.end_node, idx, cost));
            }
            if can_bkw {
                adj[edge.end_node as usize].push((edge.start_node, idx, cost));
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
                    
                    for &(neighbor, out_edge, edge_cost) in &adj[node as usize] {
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
        hpa
    }

    /// Finds a path from `start_raw` to `end_raw` and returns `(time_cost, distance_m, node_sequence)`.
    ///
    /// `start_edge` is the edge the agent is currently traversing (`usize::MAX` if at a bare node).
    /// If `pedestrian` is `true`, road edges are heavily penalised and one-way restrictions are ignored.
    ///
    /// **Known bug**: the pre-built abstract graph is ignored; this runs a full concrete A* on every call.
    /// See `docs/project.md` bug B2 for the fix required before v0.01.
    pub fn find_path(&self, start_raw: u32, end_raw: u32, start_edge: usize, graph: &TransitGraph, pedestrian: bool) -> Option<(f32, f32, Vec<u32>)> {
        let start = graph.get_valid_node(start_raw);
        let end = graph.get_valid_node(end_raw);

        if start == end { return Some((0.0, 0.0, vec![start])); }
        let n = graph.nodes.len();
        if start as usize >= n || end as usize >= n { return None; }

        let mut adj: Vec<Vec<(u32, usize, f32)>> = vec![Vec::new(); n];
        for (idx, edge) in graph.edges.iter().enumerate() {
            let mut cost = edge.base_cost * (1.0 + edge.current_congestion);
            if pedestrian && edge.primary_type == crate::simulation::network::types::TransitType::Road {
                cost *= 10.0;
            }
            
            let mut can_fwd = if pedestrian { 
                (edge.allowed_types & 1) != 0 
            } else { 
                (edge.allowed_types & 2) != 0 && edge.fwd_lanes > 0
            };
            
            let mut can_bkw = if pedestrian { 
                (edge.allowed_types & 1) != 0 
            } else { 
                (edge.allowed_types & 2) != 0 && edge.bkw_lanes > 0
            };

            // STRICT RESTRICTION: Cars never allowed on walkways (Foot primary type)
            if !pedestrian && edge.primary_type == crate::simulation::network::types::TransitType::Foot {
                can_fwd = false;
                can_bkw = false;
            }

            if can_fwd { adj[edge.start_node as usize].push((edge.end_node, idx, cost)); }
            if can_bkw { adj[edge.end_node as usize].push((edge.start_node, idx, cost)); }

        }

        let mut h = BinaryHeap::new();
        let mut costs: HashMap<(u32, usize), (f32, f32)> = HashMap::new();
        let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
        let mut final_inc = usize::MAX;
        
        costs.insert((start, start_edge), (0.0, 0.0));
        h.push(State { priority: 0.0, cost: 0.0, dist: 0.0, node: start, incoming_edge: start_edge });
        
        while let Some(State { priority: _, cost, dist, node, incoming_edge }) = h.pop() {
            if node == end { final_inc = incoming_edge; break; }
            if cost > costs.get(&(node, incoming_edge)).unwrap_or(&(f32::MAX, f32::MAX)).0 { continue; }
            
            for &(neighbor, out_edge, edge_cost) in &adj[node as usize] {
                if !pedestrian && incoming_edge != usize::MAX && incoming_edge != out_edge {
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
                    let h_val = graph.nodes[neighbor as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
                    h.push(State { priority: next_cost + h_val, cost: next_cost, dist: next_dist, node: neighbor, incoming_edge: out_edge });
                }
            }
        }
        
        if let Some(&(fc, fd)) = costs.get(&(end, final_inc)) {
            let mut path = Vec::new();
            let mut path_edges = Vec::new();
            let mut curr_state = (end, final_inc);
            while curr_state.0 != start {
                path_edges.push(graph.edges[curr_state.1].primary_type);
                path.push(curr_state.0);
                curr_state = *prev.get(&curr_state).unwrap();
            }
            path.push(start);
            path.reverse();
            path_edges.reverse();
            
            return Some((fc, fd, path));
        } else {
            // Pathfinding failed (Graph mathematically disconnected for this transit type)
            // No need to spam the console here, agents.rs now handles this gracefully.
            return None;
        }
    }
}
