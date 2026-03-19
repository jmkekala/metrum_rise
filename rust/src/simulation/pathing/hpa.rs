use crate::simulation::network::graph::TransitGraph;
use super::astar::State;
use std::collections::{HashMap, BinaryHeap, HashSet};

#[derive(Clone)]
pub struct AbstractEdge {
    pub target: u32,
    pub cost: f32,
    pub inner_path: Vec<u32>,
}

pub struct HpaGraph {
    pub abstract_edges: HashMap<u32, Vec<AbstractEdge>>,
    pub chunk_entries: HashMap<(i32, i32), Vec<u32>>,
    pub is_abstract: HashSet<u32>,
}

impl HpaGraph {
    pub fn new() -> Self {
        Self {
            abstract_edges: HashMap::new(),
            chunk_entries: HashMap::new(),
            is_abstract: HashSet::new(),
        }
    }

    pub fn build(graph: &TransitGraph) -> Self {
        let mut hpa = Self::new();
        let n = graph.nodes.len();
        if n == 0 { return hpa; }

        let mut adj: Vec<Vec<(u32, usize, f32)>> = vec![Vec::new(); n];
        for (idx, edge) in graph.edges.iter().enumerate() {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            adj[edge.start_node as usize].push((edge.end_node, idx, cost));
            adj[edge.end_node as usize].push((edge.start_node, idx, cost)); // Bidirectional
            
            let chunk_a = graph.get_node_chunk(edge.start_node);
            let chunk_b = graph.get_node_chunk(edge.end_node);
            
            if chunk_a != chunk_b {
                // Both nodes become abstract entries for their respective chunks
                hpa.is_abstract.insert(edge.start_node);
                hpa.is_abstract.insert(edge.end_node);
                
                hpa.chunk_entries.entry(chunk_a).or_default().push(edge.start_node);
                hpa.chunk_entries.entry(chunk_b).or_default().push(edge.end_node);
                
                // The inter-chunk edge itself
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

        // Intra-chunk paths
        for (&chunk, entries) in &hpa.chunk_entries {
            for &start_node in entries {
                let mut costs: HashMap<(u32, usize), f32> = HashMap::new();
                let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
                let mut heap = BinaryHeap::new();
                
                costs.insert((start_node, usize::MAX), 0.0);
                heap.push(State { cost: 0.0, node: start_node, incoming_edge: usize::MAX });
                
                while let Some(State { cost, node, incoming_edge }) = heap.pop() {
                    if cost > *costs.get(&(node, incoming_edge)).unwrap_or(&f32::MAX) { continue; }
                    
                    for &(neighbor, out_edge, edge_cost) in &adj[node as usize] {
                        if graph.get_node_chunk(neighbor) != chunk { continue; }
                        
                        // Traffic Lane Manager EVAL
                        if incoming_edge != usize::MAX {
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
                        if next_cost < *costs.get(&(neighbor, out_edge)).unwrap_or(&f32::MAX) {
                            costs.insert((neighbor, out_edge), next_cost);
                            prev.insert((neighbor, out_edge), (node, incoming_edge));
                            heap.push(State { cost: next_cost, node: neighbor, incoming_edge: out_edge });
                        }
                    }
                }
                
                for &end_node in entries {
                    if start_node != end_node {
                        let mut best_inc = usize::MAX;
                        let mut min_c = f32::MAX;
                        for (&(n, inc), &c) in &costs {
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

    pub fn find_local_path(start: u32, end: u32, chunk: (i32, i32), graph: &TransitGraph, adj: &Vec<Vec<(u32, usize, f32)>>) -> Option<(f32, Vec<u32>)> {
        if start == end { return Some((0.0, vec![])); }
        
        let mut costs: HashMap<(u32, usize), f32> = HashMap::new();
        let mut prev: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
        let mut heap = BinaryHeap::new();
        let mut visited: HashSet<(u32, usize)> = HashSet::new();
        
        costs.insert((start, usize::MAX), 0.0);
        heap.push(State { cost: 0.0, node: start, incoming_edge: usize::MAX });
        
        let mut final_inc = usize::MAX;
        
        while let Some(State { cost: _, node, incoming_edge }) = heap.pop() {
            if node == end { final_inc = incoming_edge; break; }
            if !visited.insert((node, incoming_edge)) { continue; }
            let cost = *costs.get(&(node, incoming_edge)).unwrap();
            
            for &(neighbor, out_edge, edge_cost) in &adj[node as usize] {
                if graph.get_node_chunk(neighbor) != chunk { continue; }
                
                // Traffic Lane Manager EVAL
                if incoming_edge != usize::MAX {
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
                if next_cost < *costs.get(&(neighbor, out_edge)).unwrap_or(&f32::MAX) {
                    costs.insert((neighbor, out_edge), next_cost);
                    prev.insert((neighbor, out_edge), (node, incoming_edge));
                    
                    let p1 = graph.nodes[neighbor as usize].pos;
                    let p2 = graph.nodes[end as usize].pos;
                    let heuristic = p1.distance_to(p2) / 100.0;
                    
                    heap.push(State { cost: next_cost + heuristic, node: neighbor, incoming_edge: out_edge });
                }
            }
        }
        
        if let Some(&fc) = costs.get(&(end, final_inc)) {
            let mut path = Vec::new();
            let mut curr = (end, final_inc);
            while curr.0 != start {
                path.push(curr.0);
                curr = *prev.get(&curr).unwrap();
            }
            path.reverse();
            Some((fc, path))
        } else {
            None
        }
    }

    pub fn find_path(&self, start_raw: u32, end_raw: u32, start_edge: usize, graph: &TransitGraph) -> Option<Vec<u32>> {
        let start = graph.get_valid_node(start_raw);
        let end = graph.get_valid_node(end_raw);

        if start == end { return Some(vec![start]); }
        let n = graph.nodes.len();
        if start as usize >= n || end as usize >= n { return None; }

        let mut adj: Vec<Vec<(u32, usize, f32)>> = vec![Vec::new(); n];
        for (idx, edge) in graph.edges.iter().enumerate() {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            adj[edge.start_node as usize].push((edge.end_node, idx, cost));
            adj[edge.end_node as usize].push((edge.start_node, idx, cost));
        }

        let mut h = BinaryHeap::new();
        let mut v: HashSet<(u32, usize)> = HashSet::new();
        let mut c: HashMap<(u32, usize), f32> = HashMap::new();
        let mut p_map: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
        let mut final_inc = usize::MAX;
        
        c.insert((start, start_edge), 0.0);
        h.push(State { cost: 0.0, node: start, incoming_edge: start_edge });
        
        while let Some(State { cost: _, node, incoming_edge }) = h.pop() {
            if node == end { final_inc = incoming_edge; break; }
            if !v.insert((node, incoming_edge)) { continue; }
            let cc = *c.get(&(node, incoming_edge)).unwrap_or(&0.0);
            
            for &(neighbor, out_edge, ec) in &adj[node as usize] {
                // Traffic Lane Manager EVAL
                if incoming_edge != usize::MAX {
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
                
                let nc = cc + ec;
                if nc < *c.get(&(neighbor, out_edge)).unwrap_or(&f32::MAX) {
                    c.insert((neighbor, out_edge), nc);
                    p_map.insert((neighbor, out_edge), (node, incoming_edge));
                    let h_val = graph.nodes[neighbor as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
                    h.push(State { cost: nc + h_val, node: neighbor, incoming_edge: out_edge });
                }
            }
        }
        
        if c.contains_key(&(end, final_inc)) {
            let mut full_path = Vec::new();
            let mut curr = (end, final_inc);
            while curr.0 != start {
                full_path.push(curr.0);
                curr = *p_map.get(&curr).unwrap();
            }
            full_path.reverse();
            Some(full_path)
        } else {
            None
        }
    }
}
