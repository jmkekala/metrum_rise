use crate::simulation::network::graph::TransitGraph;
use std::collections::{BinaryHeap, HashMap};
use std::cmp::Ordering;

#[derive(Copy, Clone, PartialEq)]
struct State {
    cost: f32,
    node: u32,
}

impl Eq for State {}

impl Ord for State {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering so BinaryHeap acts as a min-heap
        other.cost.partial_cmp(&self.cost).unwrap_or(Ordering::Equal)
    }
}

impl PartialOrd for State {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// The RoutingTable calculates the "Next Hop" for every single node pair in the entire city.
/// By storing this in a global table, millions of agents can query their pathing extremely fast
/// in O(1) time without doing any A* or graph traversal during the tick!
pub struct RoutingTable {
    // A 1D array representing an NxN matrix where N is the number of nodes.
    // Index: start_node * N + end_node
    // Value: The specific `next_node` ID to travel to. u32::MAX means no path.
    pub next_hop: Vec<u32>,
    pub node_count: usize,
}

impl RoutingTable {
    pub fn new() -> Self {
        Self {
            next_hop: Vec::new(),
            node_count: 0,
        }
    }

    /// Rebuilds the massive routing table. Runs N parallel Dijkstra algorithms.
    pub fn build(graph: &TransitGraph) -> Self {
        let n = graph.nodes.len();
        if n == 0 {
            return Self::new();
        }

        let mut next_hop = vec![u32::MAX; n * n];
        
        // Adjacency list
        let mut adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); n];
        for edge in &graph.edges {
            let cost = edge.base_cost * (1.0 + edge.current_congestion);
            adj[edge.start_node as usize].push((edge.end_node, cost));
            adj[edge.end_node as usize].push((edge.start_node, cost)); // Bidirectional
        }

        // For every destination node in the graph, we run a Dijkstra outward to find the shortest path TO it.
        // This is exactly like FlowField but we do it for every node and save the immediate gradient step.
        for dest_node in 0..n {
            let mut costs = vec![f32::MAX; n];
            let mut heap = BinaryHeap::new();

            costs[dest_node] = 0.0;
            heap.push(State { cost: 0.0, node: dest_node as u32 });

            while let Some(State { cost, node }) = heap.pop() {
                if cost > costs[node as usize] {
                    continue;
                }

                for &(neighbor_node, edge_cost) in &adj[node as usize] {
                    let next_cost = cost + edge_cost;
                    if next_cost < costs[neighbor_node as usize] {
                        costs[neighbor_node as usize] = next_cost;
                        heap.push(State { cost: next_cost, node: neighbor_node });
                        
                        // IF we are travelling TO dest_node FROM neighbor_node, 
                        // the optimal step is to go through `node`.
                        let matrix_idx = (neighbor_node as usize) * n + dest_node;
                        next_hop[matrix_idx] = node;
                    }
                }
            }
        }

        Self {
            next_hop,
            node_count: n,
        }
    }

    /// O(1) Query: I am at A, I want to go to B. What is my next immediate jump?
    pub fn get_next_node(&self, current: u32, destination: u32) -> Option<u32> {
        if current >= self.node_count as u32 || destination >= self.node_count as u32 {
            return None;
        }
        if current == destination {
            return Some(destination);
        }
        
        let idx = (current as usize) * self.node_count + (destination as usize);
        let nxt = self.next_hop[idx];
        if nxt == u32::MAX {
            None
        } else {
            Some(nxt)
        }
    }
}
