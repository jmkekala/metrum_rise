//! Logic for rebuilding graph metadata (adjacency, clips, and terrain synchronization).

use super::super::types::*;
use super::data::RegionGraph;
use std::collections::{HashMap, HashSet};

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

    /// Removes all edges marked as `deleted` and remaps graph-local edge references.
    ///
    /// This is a low-level canonicalization helper for tests and future persistence code.
    /// Live gameplay/editor code keeps soft-deleted edge slots in place and skips them.
    /// Returns a mapping from [Old Edge Index] -> [New Edge Index].
    pub fn compact_edges(&mut self) -> HashMap<usize, usize> {
        let mut old_to_new = HashMap::new();
        let mut new_edges = Vec::new();

        for (old_idx, edge) in self.edges.iter().enumerate() {
            if !edge.deleted {
                let new_idx = new_edges.len();
                old_to_new.insert(old_idx, new_idx);
                new_edges.push(edge.clone());
            }
        }

        // If no edges were deleted, we're already compacted.
        if new_edges.len() == self.edges.len() {
            return HashMap::new();
        }

        self.edges = new_edges;

        // 1. Rebuild Adjacency List (Fastest way to update indices)
        self.rebuild_adjacency_list();

        // 2. Rebuild Spatial Index
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }

        // 3. Update Lane Connection rules inside each Node
        for node in &mut self.nodes {
            let mut new_lane_conns = HashMap::new();
            for (src, targets) in node.lane_connections.drain() {
                // If the source edge still exists, remap it
                if let Some(&new_src_idx) = old_to_new.get(&src.0) {
                    let mut new_targets = Vec::new();
                    for mut tgt in targets {
                        // If the target edge still exists, remap it
                        if let Some(&new_tgt_idx) = old_to_new.get(&tgt.0) {
                            tgt.0 = new_tgt_idx;
                            new_targets.push(tgt);
                        }
                    }
                    if !new_targets.is_empty() {
                        new_lane_conns.insert((new_src_idx, src.1), new_targets);
                    }
                }
            }
            node.lane_connections = new_lane_conns;
        }

        old_to_new
    }

    /// Returns the number of disconnected components (islands) in the network
    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }

        // Disjoint Set Union (DSU)
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();

        fn find(i: usize, parent: &mut Vec<usize>) -> usize {
            if parent[i] == i {
                return i;
            }
            parent[i] = find(parent[i], parent);
            parent[i]
        }

        fn unite(i: usize, j: usize, parent: &mut Vec<usize>) {
            let root_i = find(i, parent);
            let root_j = find(j, parent);
            if root_i != root_j {
                parent[root_i] = root_j;
            }
        }

        // Unite nodes connected by edges
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            unite(
                edge.start_node as usize,
                edge.end_node as usize,
                &mut parent,
            );
        }

        // Count unique roots (only for nodes that are part of an edge to avoid counting "floating" preview nodes)
        let mut active_nodes = std::collections::HashSet::new();
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            active_nodes.insert(edge.start_node as usize);
            active_nodes.insert(edge.end_node as usize);
        }

        let mut roots = std::collections::HashSet::new();
        for &node_idx in &active_nodes {
            roots.insert(find(node_idx, &mut parent));
        }

        roots.len()
    }

    /// Synchronizes all road nodes and intermediate geometries to the terrain heightmap.
    ///
    /// Applies Laplacian smoothing to road grades to ensure smooth vertical transitions.
    pub fn sync_to_terrain(&mut self, terrain: &crate::simulation::terrain::TerrainSystem) {
        let hw = (terrain.width as f32 - 1.0) * 0.5;
        let hh = (terrain.height as f32 - 1.0) * 0.5;

        // 0. Pre-calculate which nodes are snappable (Standard only)
        let mut node_snappable = vec![true; self.nodes.len()];
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                node_snappable[edge.start_node as usize] = false;
                node_snappable[edge.end_node as usize] = false;
            }
        }

        // 1. Sync Nodes Only if snappable
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !node_snappable[i] {
                continue;
            }
            let gx = node.pos.x + hw;
            let gz = node.pos.z + hh;
            node.pos.y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
        }

        // 2. Re-interpolate Edge Geometry (Smooth Grades)
        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                continue;
            }

            let count = edge.geometry.len();
            if count < 2 {
                continue;
            }

            // Snap endpoints to nodes
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;

            // HARMONIC CONFORMANCE (Laplacian Smoothing)
            // 1. Re-sample raw terrain for all intermediate points so road follows new hills
            for j in 1..count - 1 {
                let gx = edge.geometry[j].x + hw;
                let gz = edge.geometry[j].z + hh;
                edge.geometry[j].y =
                    terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
            }

            // 2. Taubin Smoothing to iron out bumps without volume shrinkage
            let iters = 50;
            if count > 2 {
                let mut temp_h = vec![0.0; count];
                let lambda = 0.5;
                let mu = -0.53;
                for _ in 0..iters {
                    // Positive Pass (Shrink/Smooth)
                    for j in 1..count - 1 {
                        let laplacian = 0.5 * (edge.geometry[j - 1].y + edge.geometry[j + 1].y)
                            - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + lambda * laplacian;
                    }
                    for j in 1..count - 1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                    // Negative Pass (Inflate/Restore Volume)
                    for j in 1..count - 1 {
                        let laplacian = 0.5 * (edge.geometry[j - 1].y + edge.geometry[j + 1].y)
                            - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + mu * laplacian;
                    }
                    for j in 1..count - 1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                }
            }
        }
        self.rebuild_intersection_clips();
    }

    /// Recalculates the start/end clipping distances for all edges meeting at junctions.
    ///
    /// This prevents road geometry from overlapping in the center of an intersection
    /// and ensures space for the junction mesh.
    pub fn rebuild_intersection_clips(&mut self) {
        let mut connection_counts = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            *connection_counts
                .entry(self.get_valid_node(edge.start_node))
                .or_insert(0) += 1;
            *connection_counts
                .entry(self.get_valid_node(edge.end_node))
                .or_insert(0) += 1;
        }

        // Calculate maximum road width and width difference for junction detection
        let mut node_max_width = HashMap::new();
        let mut node_min_width = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let w = edge.width;

            let s_node = self.get_valid_node(edge.start_node);
            let s_max = node_max_width.entry(s_node).or_insert(0.0);
            if w > *s_max {
                *s_max = w;
            }
            let s_min = node_min_width.entry(s_node).or_insert(f32::MAX);
            if w < *s_min {
                *s_min = w;
            }

            let e_node = self.get_valid_node(edge.end_node);
            let e_max = node_max_width.entry(e_node).or_insert(0.0);
            if w > *e_max {
                *e_max = w;
            }
            let e_min = node_min_width.entry(e_node).or_insert(f32::MAX);
            if w < *e_min {
                *e_min = w;
            }
        }

        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();

        for (_edge_id, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            // Dynamic clipping: Ensure the junction covers at least the width of the road
            let s_valid = valid_node_ids[edge.start_node as usize];
            let e_valid = valid_node_ids[edge.end_node as usize];

            let s_conn = *connection_counts.get(&s_valid).unwrap_or(&0);
            let e_conn = *connection_counts.get(&e_valid).unwrap_or(&0);

            let s_max_w = *node_max_width.get(&s_valid).unwrap_or(&0.0);
            let s_min_w = *node_min_width.get(&s_valid).unwrap_or(&0.0);
            let s_different = (s_max_w - s_min_w).abs() > 0.1;

            let e_max_w = *node_max_width.get(&e_valid).unwrap_or(&0.0);
            let e_min_w = *node_min_width.get(&e_valid).unwrap_or(&0.0);
            let e_different = (e_max_w - e_min_w).abs() > 0.1;

            let s_clip = (s_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;
            let e_clip = (e_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;

            let s_node_type = self.nodes[s_valid as usize].node_type;
            let e_node_type = self.nodes[e_valid as usize].node_type;

            edge.start_clip = if (s_conn >= 3 || s_different) && s_node_type == NodeType::Junction {
                s_clip
            } else {
                0.0
            };

            edge.end_clip = if (e_conn >= 3 || e_different) && e_node_type == NodeType::Junction {
                e_clip
            } else {
                0.0
            };

            // Keep physical_geometry in sync with geometry (which may have updated Y values
            // from terrain height re-interpolation). The renderer trims it via start_clip/end_clip.
            edge.physical_geometry = edge.geometry.clone();
        }

        // Re-index all roads after a massive batch clip rebuild (e.g. after terrain sync)
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
        self.rebuild_adjacency_list();
    }

    /// Rebuilds intersection clips for only the edges incident to `affected_nodes`.
    ///
    /// Equivalent to [`rebuild_intersection_clips`] but scoped to the nodes touched
    /// by a single road placement. Avoids the full O(E) resample pass and full
    /// R-tree / adjacency rebuild — only the O(K) incident edges are updated.
    ///
    /// Use this from the `AddRoad` handler. Use the full rebuild for bulk operations
    /// such as save-load restore or terrain sync.
    pub fn rebuild_intersection_clips_for_nodes(&mut self, affected_nodes: &HashSet<u32>) {
        if affected_nodes.is_empty() {
            return;
        }

        // Pre-compute valid node IDs so we can use them during the mutable edge pass.
        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();

        // Pass 1 (read-only): scan every road edge once to build per-node stats,
        // but only populate entries for nodes in the affected set.
        let mut connection_counts: HashMap<u32, usize> = HashMap::new();
        let mut node_max_width: HashMap<u32, f32> = HashMap::new();
        let mut node_min_width: HashMap<u32, f32> = HashMap::new();

        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let s = valid_node_ids[edge.start_node as usize];
            let e = valid_node_ids[edge.end_node as usize];
            let w = edge.width;
            for &node in &[s, e] {
                if affected_nodes.contains(&node) {
                    *connection_counts.entry(node).or_insert(0) += 1;
                    let max_w = node_max_width.entry(node).or_insert(0.0_f32);
                    if w > *max_w {
                        *max_w = w;
                    }
                    let min_w = node_min_width.entry(node).or_insert(f32::MAX);
                    if w < *min_w {
                        *min_w = w;
                    }
                }
            }
        }

        // Pass 2 (mutable): resample only edges that touch an affected node.
        let mut reindex_ids: Vec<usize> = Vec::new();
        for (edge_idx, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let s = valid_node_ids[edge.start_node as usize];
            let e = valid_node_ids[edge.end_node as usize];

            if !affected_nodes.contains(&s) && !affected_nodes.contains(&e) {
                continue;
            }

            let s_conn = *connection_counts.get(&s).unwrap_or(&0);
            let e_conn = *connection_counts.get(&e).unwrap_or(&0);

            let s_max_w = *node_max_width.get(&s).unwrap_or(&0.0);
            let s_min_w = *node_min_width.get(&s).unwrap_or(&0.0);
            let e_max_w = *node_max_width.get(&e).unwrap_or(&0.0);
            let e_min_w = *node_min_width.get(&e).unwrap_or(&0.0);
            let s_different = (s_max_w - s_min_w).abs() > 0.1;
            let e_different = (e_max_w - e_min_w).abs() > 0.1;

            let s_node_type = self.nodes[s as usize].node_type;
            let e_node_type = self.nodes[e as usize].node_type;

            let s_clip = (s_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;
            let e_clip = (e_max_w * 0.5 + crate::config::SIDEWALK_WIDTH) * 1.2;

            edge.start_clip = if (s_conn >= 3 || s_different) && s_node_type == NodeType::Junction {
                s_clip
            } else {
                0.0
            };
            edge.end_clip = if (e_conn >= 3 || e_different) && e_node_type == NodeType::Junction {
                e_clip
            } else {
                0.0
            };

            edge.physical_geometry = edge.geometry.clone();

            reindex_ids.push(edge_idx);
        }

        // Update the spatial R-tree only for the affected edges (not full rebuild).
        for edge_idx in reindex_ids {
            self.remove_from_spatial_index(edge_idx);
            self.add_to_spatial_index(edge_idx);
        }
        // Adjacency is unchanged — no rebuild needed.
    }
}
