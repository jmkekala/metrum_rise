// SPDX-License-Identifier: GPL-2.0-only

//! Adjacency reconstruction and disconnected-component queries.

use super::super::data::RegionGraph;

impl RegionGraph {
    /// Rebuilds the adjacency list from the current set of non-deleted edges.
    pub fn rebuild_adjacency_list(&mut self) {
        self.adjacency.clear();
        self.adjacency.resize(self.nodes.len(), Vec::new());
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            self.adjacency[e.start_node as usize].push(i);
            self.adjacency[e.end_node as usize].push(i);
        }
    }

    /// Deletes an edge from the graph alone: the edge index, its flag and its two endpoint
    /// adjacency lists, which keep their order. `O(endpoint degree + log edges)`; what a road
    /// removal tries before it commits (`ROAD-68` open question 11 (c)).
    pub(crate) fn delete_edge_locally(&mut self, edge_idx: usize) {
        self.remove_from_spatial_index(edge_idx);
        let edge = &mut self.edges[edge_idx];
        edge.deleted = true;
        for node in [edge.start_node, edge.end_node] {
            if let Some(incident) = self.adjacency.get_mut(node as usize) {
                incident.retain(|&incident_edge| incident_edge != edge_idx);
            }
        }
    }

    /// Returns the number of disconnected components (islands) in the network
    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }

        let node_count = self.nodes.len();
        let mut parent = (0..node_count).collect::<Vec<_>>();
        let mut rank = vec![0_u8; node_count];
        let mut active = vec![false; node_count];

        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            let start = edge.start_node as usize;
            let end = edge.end_node as usize;
            active[start] = true;
            active[end] = true;
            union_roots(start, end, &mut parent, &mut rank);
        }

        let mut seen_root = vec![false; node_count];
        let mut island_count = 0;
        for (node_idx, is_active) in active.into_iter().enumerate() {
            if !is_active {
                continue;
            }
            let root = find_root(node_idx, &mut parent);
            if !seen_root[root] {
                seen_root[root] = true;
                island_count += 1;
            }
        }

        island_count
    }
}

fn find_root(mut node: usize, parent: &mut [usize]) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

fn union_roots(left: usize, right: usize, parent: &mut [usize], rank: &mut [u8]) {
    let left_root = find_root(left, parent);
    let right_root = find_root(right, parent);
    if left_root == right_root {
        return;
    }

    match rank[left_root].cmp(&rank[right_root]) {
        std::cmp::Ordering::Less => parent[left_root] = right_root,
        std::cmp::Ordering::Greater => parent[right_root] = left_root,
        std::cmp::Ordering::Equal => {
            parent[right_root] = left_root;
            rank[left_root] = rank[left_root].saturating_add(1);
        }
    }
}
