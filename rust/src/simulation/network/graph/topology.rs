//! Network topology management: node/edge creation, merging, and movement.
//!
//! This module handles the structural mutation of the [`RegionGraph`], ensuring that
//! adjacency lists and spatial indices are kept in sync after every edit.

use super::super::types::*;
use super::data::Edge;
use super::data::Node;
use super::data::RegionGraph;
use godot::prelude::*;
use std::collections::HashMap;

impl RegionGraph {
    /// Returns the canonical node ID for a given ID, following any merge aliases.
    pub fn get_valid_node(&self, mut id: u32) -> u32 {
        while let Some(&alias) = self.node_aliases.get(&id) {
            id = alias;
        }
        id
    }

    /// Returns the index of the edge connecting two nodes, if one exists.
    pub fn get_edge_between_nodes(&self, from: u32, to: u32) -> Option<usize> {
        if (from as usize) < self.adjacency.len() {
            for &idx in &self.adjacency[from as usize] {
                let e = &self.edges[idx];
                if !e.deleted
                    && ((e.start_node == from && e.end_node == to)
                        || (e.start_node == to && e.end_node == from))
                {
                    return Some(idx);
                }
            }
        }
        None
    }

    /// Adds a new node to the graph at the specified position.
    pub fn add_node(&mut self, pos: Vector3, node_type: NodeType) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node {
            pos,
            node_type,
            lane_connections: HashMap::new(),
        });
        self.adjacency.push(Vec::new());
        self.add_node_to_spatial_index(id);
        id
    }

    /// Finds an existing node within `threshold` distance or adds a new one.
    pub fn find_or_add_node(&mut self, pos: Vector3, threshold: f32, node_type: NodeType) -> u32 {
        let chunk_coords = Self::get_node_chunk_coords(pos);

        // Search current and adjacent chunks
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(chunk) = self
                    .spatial_node_grid
                    .get(&(chunk_coords.0 + dx, chunk_coords.1 + dz))
                {
                    for &node_id in chunk {
                        if self.nodes[node_id as usize].pos.distance_to(pos) < threshold {
                            let id = self.get_valid_node(node_id);
                            return id;
                        }
                    }
                }
            }
        }
        self.add_node(pos, node_type)
    }

    /// Adds a new edge to the graph and updates adjacency and spatial indices.
    pub fn add_edge(&mut self, mut edge: Edge) -> usize {
        edge.deleted = false;
        let id = self.edges.len();
        self.edges.push(edge);
        self.add_to_spatial_index(id);

        // Update Adjacency
        let e = &self.edges[id];
        self.adjacency[e.start_node as usize].push(id);
        self.adjacency[e.end_node as usize].push(id);

        id
    }

    /// Calculates the total physical length of a polyline.
    pub fn calculate_length(&self, pts: &[Vector3]) -> f32 {
        let mut l = 0.0;
        for i in 0..pts.len().saturating_sub(1) {
            l += pts[i].distance_to(pts[i + 1]);
        }
        l
    }

    /// Removes a node and merges its two connected edges if they are compatible.
    pub fn remove_node_and_merge_edges(&mut self, node_id: u32) -> Option<(usize, usize)> {
        if node_id as usize >= self.nodes.len() {
            return None;
        }

        // Find edges connected to this node
        let mut e1_idx = None;
        let mut e2_idx = None;

        for (i, edge) in self.edges.iter().enumerate() {
            if edge.deleted {
                continue;
            } // Important: Skip already deleted edges
            if edge.start_node == node_id || edge.end_node == node_id {
                if e1_idx.is_none() {
                    e1_idx = Some(i);
                } else if e2_idx.is_none() {
                    e2_idx = Some(i);
                } else {
                    // More than 2 edges? This node is likely a real intersection now.
                    // DO NOT MERGE.
                    return None;
                }
            }
        }

        if let (Some(i1), Some(i2)) = (e1_idx, e2_idx) {
            // Check if they are compatible for merging
            let (_target_end_node, mid_node, target_start_node) = {
                let e1 = &self.edges[i1];
                let e2 = &self.edges[i2];
                if e1.primary_type != e2.primary_type || e1.width != e2.width {
                    return None;
                }

                // Determine the flow: A -> node_id -> B
                let a = if e1.start_node == node_id {
                    e1.end_node
                } else {
                    e1.start_node
                };
                let b = if e2.start_node == node_id {
                    e2.end_node
                } else {
                    e2.start_node
                };

                (a, node_id, b)
            };

            // Combine geometry
            let mut new_geom = Vec::new();
            let (first_edge_idx, second_edge_idx) = {
                if self.edges[i1].end_node == mid_node {
                    (i1, i2)
                } else {
                    (i2, i1)
                }
            };

            for p in &self.edges[first_edge_idx].geometry {
                new_geom.push(*p);
            }
            // Skip the first point of the second edge as it's the same as the last point of the first
            for i in 1..self.edges[second_edge_idx].geometry.len() {
                new_geom.push(self.edges[second_edge_idx].geometry[i]);
            }

            // Update the first edge to span the whole distance
            self.edges[first_edge_idx].end_node = target_start_node;
            self.edges[first_edge_idx].geometry = new_geom;
            // Also combine physical geometry to keep lengths and rendering stable until next rebuild
            let mut new_phys = self.edges[first_edge_idx].physical_geometry.clone();
            if !self.edges[second_edge_idx].physical_geometry.is_empty() {
                new_phys.extend_from_slice(&self.edges[second_edge_idx].physical_geometry[1..]);
            }
            self.edges[first_edge_idx].physical_geometry = new_phys;
            self.edges[first_edge_idx].physical_length =
                self.calculate_length(&self.edges[first_edge_idx].physical_geometry);

            // Mark the second edge as deleted instead of removing it to keep indices stable
            let to_remove = second_edge_idx;
            self.edges[to_remove].deleted = true;
            self.remove_from_spatial_index(to_remove);

            // Re-index the first edge as its geometry changed
            self.remove_from_spatial_index(first_edge_idx);
            self.add_to_spatial_index(first_edge_idx);

            return Some((first_edge_idx, second_edge_idx));
        }
        None
    }

    /// Merges two nodes into one, updating all edges that use them
    /// Merges two nodes into one, updating all connected edges.
    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 {
            return;
        }
        // Ensure we always map to the ultimate valid parent and don't loop
        let keep = self.get_valid_node(id1.min(id2));
        let remove = self.get_valid_node(id1.max(id2));
        if keep == remove {
            return;
        }

        let new_pos = self.nodes[keep as usize].pos;
        self.node_aliases.insert(remove, keep);

        // Merging two network pieces transforms any restrictive node type into a Junction
        self.nodes[keep as usize].node_type = NodeType::Junction;
        self.nodes[keep as usize].lane_connections.clear();

        // Update all edges using the 'remove' node to use 'keep' node instead
        let mut affected_edges = Vec::new();
        for (i, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted {
                continue;
            }
            let mut changed = false;
            if edge.start_node == remove {
                edge.start_node = keep;
                if !edge.geometry.is_empty() {
                    edge.geometry[0] = new_pos;
                }
                changed = true;
            }
            if edge.end_node == remove {
                edge.end_node = keep;
                if !edge.geometry.is_empty() {
                    let last = edge.geometry.len() - 1;
                    edge.geometry[last] = new_pos;
                }
                changed = true;
            }
            if changed {
                affected_edges.push(i);
            }
        }

        for i in affected_edges {
            self.remove_from_spatial_index(i);
            self.add_to_spatial_index(i);
        }
    }

    /// Moves a node to a new position, smoothly deforming all connected edges.
    pub fn move_node(&mut self, node_id: u32, new_pos: Vector3) {
        let old_pos = self.nodes[node_id as usize].pos;
        let delta = new_pos - old_pos;

        self.remove_node_from_spatial_index(node_id, old_pos);
        self.nodes[node_id as usize].pos = new_pos;
        self.add_node_to_spatial_index(node_id);

        // Pre-remove modified edges from spatial index while they still have old geometry
        for i in 0..self.edges.len() {
            if !self.edges[i].deleted
                && (self.edges[i].start_node == node_id || self.edges[i].end_node == node_id)
            {
                self.remove_from_spatial_index(i);
            }
        }

        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.start_node == node_id || edge.end_node == node_id {
                let count = edge.geometry.len();
                if count < 2 {
                    continue;
                }

                let is_start = edge.start_node == node_id;
                let is_end = edge.end_node == node_id;

                if is_start && is_end {
                    for i in 0..count {
                        edge.geometry[i] += delta;
                    }
                } else if is_start {
                    for i in 0..count {
                        let w = 1.0 - (i as f32 / (count - 1) as f32);
                        let w_smooth = w * w * (3.0 - 2.0 * w); // Smoothstep curve
                        edge.geometry[i] += delta * w_smooth;
                    }
                } else if is_end {
                    for i in 0..count {
                        let w = i as f32 / (count - 1) as f32;
                        let w_smooth = w * w * (3.0 - 2.0 * w); // Smoothstep curve
                        edge.geometry[i] += delta * w_smooth;
                    }
                }
            }
        }

        // Re-index all affected edges
        self.rebuild_intersection_clips();
    }
}
