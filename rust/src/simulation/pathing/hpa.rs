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

        let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        for edge in &graph.edges {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            adj[edge.start_node as usize].push((edge.end_node, cost));
            adj[edge.end_node as usize].push((edge.start_node, cost)); // Bidirectional
            
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
                let mut costs = HashMap::new();
                let mut prev = HashMap::new();
                let mut heap = BinaryHeap::new();
                
                costs.insert(start_node, 0.0);
                heap.push(State { cost: 0.0, node: start_node });
                
                while let Some(State { cost, node }) = heap.pop() {
                    if cost > *costs.get(&node).unwrap_or(&f32::MAX) { continue; }
                    
                    for &(neighbor, edge_cost) in &adj[node as usize] {
                        if graph.get_node_chunk(neighbor) != chunk { continue; }
                        
                        let next_cost = cost + edge_cost;
                        if next_cost < *costs.get(&neighbor).unwrap_or(&f32::MAX) {
                            costs.insert(neighbor, next_cost);
                            prev.insert(neighbor, node);
                            heap.push(State { cost: next_cost, node: neighbor });
                        }
                    }
                }
                
                for &end_node in entries {
                    if start_node != end_node {
                        if let Some(&final_cost) = costs.get(&end_node) {
                            let mut path = Vec::new();
                            let mut curr = end_node;
                            while curr != start_node {
                                path.push(curr);
                                curr = *prev.get(&curr).unwrap();
                            }
                            path.reverse();
                            
                            hpa.abstract_edges.entry(start_node).or_default().push(AbstractEdge {
                                target: end_node,
                                cost: final_cost,
                                inner_path: path
                            });
                        }
                    }
                }
            }
        }
        hpa
    }

    pub fn find_local_path(start: u32, end: u32, chunk: (i32, i32), graph: &TransitGraph, adj: &Vec<Vec<(u32, f32)>>) -> Option<(f32, Vec<u32>)> {
        if start == end { return Some((0.0, vec![])); }
        
        let mut costs = HashMap::new();
        let mut prev = HashMap::new();
        let mut heap = BinaryHeap::new();
        let mut visited = HashSet::new();
        
        costs.insert(start, 0.0);
        heap.push(State { cost: 0.0, node: start });
        
        while let Some(State { cost: _, node }) = heap.pop() {
            if node == end { break; }
            if !visited.insert(node) { continue; }
            let cost = *costs.get(&node).unwrap();
            
            for &(neighbor, edge_cost) in &adj[node as usize] {
                if graph.get_node_chunk(neighbor) != chunk { continue; }
                
                let next_cost = cost + edge_cost;
                if next_cost < *costs.get(&neighbor).unwrap_or(&f32::MAX) {
                    costs.insert(neighbor, next_cost);
                    prev.insert(neighbor, node);
                    
                    // A* heuristic
                    let p1 = graph.nodes[neighbor as usize].pos;
                    let p2 = graph.nodes[end as usize].pos;
                    let heuristic = p1.distance_to(p2) / 100.0;
                    
                    heap.push(State { cost: next_cost + heuristic, node: neighbor });
                }
            }
        }
        
        if let Some(&final_cost) = costs.get(&end) {
            let mut path = Vec::new();
            let mut curr = end;
            while curr != start {
                path.push(curr);
                curr = *prev.get(&curr).unwrap();
            }
            path.reverse();
            Some((final_cost, path))
        } else {
            None
        }
    }

    pub fn find_path(&self, start_raw: u32, end_raw: u32, graph: &TransitGraph) -> Option<Vec<u32>> {
        let start = graph.get_valid_node(start_raw);
        let end = graph.get_valid_node(end_raw);

        if start == end { return Some(vec![start]); }
        let n = graph.nodes.len();
        if start as usize >= n || end as usize >= n { return None; }

        let chunk_s = graph.get_node_chunk(start);
        let chunk_e = graph.get_node_chunk(end);

        let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        for edge in &graph.edges {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            adj[edge.start_node as usize].push((edge.end_node, cost));
            adj[edge.end_node as usize].push((edge.start_node, cost));
        }

        // Direct path logic if in same chunk:
        if chunk_s == chunk_e {
            if let Some((_, path)) = Self::find_local_path(start, end, chunk_s, graph, &adj) {
                return Some(path);
            }
        }

        // Dynamic abstract edges injection
        let mut temp_abstract_edges = self.abstract_edges.clone();
        
        if !self.is_abstract.contains(&start) {
            if let Some(entries) = self.chunk_entries.get(&chunk_s) {
                for &entry in entries {
                    if let Some((cost, path)) = Self::find_local_path(start, entry, chunk_s, graph, &adj) {
                        temp_abstract_edges.entry(start).or_default().push(AbstractEdge {
                            target: entry, cost, inner_path: path
                        });
                    }
                }
            }
        }

        if !self.is_abstract.contains(&end) {
            if let Some(entries) = self.chunk_entries.get(&chunk_e) {
                for &entry in entries {
                    if let Some((cost, path)) = Self::find_local_path(entry, end, chunk_e, graph, &adj) {
                        temp_abstract_edges.entry(entry).or_default().push(AbstractEdge {
                            target: end, cost, inner_path: path
                        });
                    }
                }
            }
        }

        // Hierarchical A* Search on Abstract Graph
        let mut costs = HashMap::new();
        let mut prev_edge = HashMap::new(); 
        let mut heap = BinaryHeap::new();
        let mut visited = HashSet::new();
        
        costs.insert(start, 0.0);
        heap.push(State { cost: 0.0, node: start });
        
        while let Some(State { cost: _, node }) = heap.pop() {
            if node == end { break; }
            if !visited.insert(node) { continue; }
            let cost = *costs.get(&node).unwrap();
            
            if let Some(edges) = temp_abstract_edges.get(&node) {
                for edge in edges {
                    let next_cost = cost + edge.cost;
                    if next_cost < *costs.get(&edge.target).unwrap_or(&f32::MAX) {
                        costs.insert(edge.target, next_cost);
                        prev_edge.insert(edge.target, (node, edge.inner_path.clone()));
                        
                        let p1 = graph.nodes[edge.target as usize].pos;
                        let p2 = graph.nodes[end as usize].pos;
                        let heuristic = p1.distance_to(p2) / 100.0;
                        
                        heap.push(State { cost: next_cost + heuristic, node: edge.target });
                    }
                }
            }
        }

        if costs.contains_key(&end) {
            let mut full_path = Vec::new();
            let mut curr = end;
            
            while curr != start {
                let (prev_node, inner_path) = prev_edge.get(&curr).unwrap();
                let mut rev_inner = inner_path.clone();
                rev_inner.reverse();
                for inner_node in rev_inner {
                    full_path.push(inner_node);
                }
                curr = *prev_node;
            }
            
            full_path.reverse();
            Some(full_path)
        } else {
            let mut h = BinaryHeap::new();
            let mut v = HashSet::new();
            let mut c = HashMap::new();
            let mut p_map = HashMap::new();
            c.insert(start, 0.0);
            h.push(State { cost: 0.0, node: start });
            while let Some(State { cost: _, node }) = h.pop() {
                if node == end { break; }
                if !v.insert(node) { continue; }
                let cc = *c.get(&node).unwrap_or(&0.0);
                for &(neighbor, ec) in &adj[node as usize] {
                    let nc = cc + ec;
                    if nc < *c.get(&neighbor).unwrap_or(&f32::MAX) {
                        c.insert(neighbor, nc);
                        p_map.insert(neighbor, node);
                        let h_val = graph.nodes[neighbor as usize].pos.distance_to(graph.nodes[end as usize].pos) / 100.0;
                        h.push(State { cost: nc + h_val, node: neighbor });
                    }
                }
            }
            
            if c.contains_key(&end) {
                println!("CRITICAL: HPA FAILED BUT UNCONSTRAINED A* FOUND THE PATH! chunk_s: {:?}, chunk_e: {:?}. MATH FLAW IN HPA!", chunk_s, chunk_e);
                let mut full_path = Vec::new();
                let mut curr = end;
                while curr != start {
                    full_path.push(curr);
                    curr = *p_map.get(&curr).unwrap();
                }
                full_path.reverse();
                Some(full_path)
            } else {
                println!("CRITICAL: UNCONSTRAINED A* ALSO FAILED! GRAPH IS LITERALLY DISCONNECTED!");
                None
            }
        }
    }
}
