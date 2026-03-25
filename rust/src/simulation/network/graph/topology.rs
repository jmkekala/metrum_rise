use godot::prelude::*;
use super::data::TransitGraph;
use super::data::Edge;
use super::data::Node;
use super::super::types::*;
use std::collections::HashMap;

impl TransitGraph {
    pub fn get_valid_node(&self, mut id: u32) -> u32 {
        while let Some(&alias) = self.node_aliases.get(&id) {
            id = alias;
        }
        id
    }

    pub fn get_edge_between_nodes(&self, from: u32, to: u32) -> Option<usize> {
        if let Some(edges) = self.adjacency.get(&from) {
            for &idx in edges {
                let e = &self.edges[idx];
                if !e.deleted && ((e.start_node == from && e.end_node == to) || (e.start_node == to && e.end_node == from)) {
                    return Some(idx);
                }
            }
        }
        None
    }

    pub fn add_node(&mut self, pos: Vector3, node_type: NodeType) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node { pos, node_type, lane_connections: HashMap::new() });
        self.add_node_to_spatial_index(id);
        id
    }

    pub fn find_or_add_node(&mut self, pos: Vector3, threshold: f32, node_type: NodeType) -> u32 {
        let chunk_coords = Self::get_node_chunk_coords(pos);
        
        // Search current and adjacent chunks
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(chunk) = self.spatial_node_grid.get(&(chunk_coords.0 + dx, chunk_coords.1 + dz)) {
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

    pub fn add_edge(&mut self, mut edge: Edge) -> usize {
        edge.deleted = false;
        let id = self.edges.len();
        self.edges.push(edge);
        self.add_to_spatial_index(id);
        
        // Update Adjacency
        let e = &self.edges[id];
        self.adjacency.entry(e.start_node).or_default().push(id);
        self.adjacency.entry(e.end_node).or_default().push(id);
        
        id
    }

    pub fn split_edge(&mut self, edge_idx: usize, split_pos: Vector3) -> (u32, usize) {
        let edge = &self.edges[edge_idx];
        let start_pos = self.nodes[edge.start_node as usize].pos;
        let end_pos = self.nodes[edge.end_node as usize].pos;
        
        if split_pos.distance_to(start_pos) < 0.1 {
             return (edge.start_node, edge_idx);
        }
        if split_pos.distance_to(end_pos) < 0.1 {
             return (edge.end_node, edge_idx);
        }

        let old_edge = self.edges[edge_idx].clone();
        let end_node = old_edge.end_node;
        
        // 1. Find the segment closest to split_pos in logical geometry
        let mut min_dist = f32::MAX;
        let mut split_idx = 0;
        let mut best_closest = split_pos;
        
        for i in 0..old_edge.geometry.len() - 1 {
            let p0 = old_edge.geometry[i];
            let p1 = old_edge.geometry[i+1];
            let segment = p1 - p0;
            let l2 = segment.length_squared();
            if l2 < 1e-6 { continue; }
            
            let t = ((split_pos - p0).dot(segment) / l2).clamp(0.0, 1.0);
            let closest = p0 + segment * t;
            let d = closest.distance_to(split_pos);
            if d < min_dist {
                min_dist = d;
                split_idx = i;
                best_closest = closest;
            }
        }

        // 1.5 Prevent degenerate splits (too close to endpoints)
        let start_pos = self.nodes[old_edge.start_node as usize].pos;
        if best_closest.distance_to(start_pos) < 2.0 {
            return (old_edge.start_node, edge_idx);
        }
        let end_pos = self.nodes[old_edge.end_node as usize].pos;
        if best_closest.distance_to(end_pos) < 2.0 {
            return (old_edge.end_node, edge_idx);
        }

        // 2. Create the new node
        let new_node_id = self.add_node(best_closest, NodeType::Junction);
        
        // 3. Split logical geometry
        let mut first_geom = Vec::new();
        for i in 0..=split_idx {
            first_geom.push(old_edge.geometry[i]);
        }
        if (first_geom.last().unwrap().distance_to(best_closest)) > 0.01 {
            first_geom.push(best_closest);
        }
        
        let mut second_geom = Vec::new();
        second_geom.push(best_closest);
        for i in split_idx+1..old_edge.geometry.len() {
            if i == split_idx + 1 && old_edge.geometry[i].distance_to(best_closest) < 0.01 {
                continue;
            }
            second_geom.push(old_edge.geometry[i]);
        }
        
        // 4. Split physical geometry (Crucial for agents!)
        let mut first_phys = Vec::new();
        let mut second_phys = Vec::new();
        
        if old_edge.physical_geometry.is_empty() {
             // Fallback if no physical geometry exists
             first_phys = first_geom.clone();
             second_phys = second_geom.clone();
        } else {
            // Find where best_closest projects onto physical geometry
            let mut min_phys_dist = f32::MAX;
            let mut phys_split_idx = 0;
            let mut phys_closest = best_closest;
            
            for i in 0..old_edge.physical_geometry.len() - 1 {
                let p0 = old_edge.physical_geometry[i];
                let p1 = old_edge.physical_geometry[i+1];
                let segment = p1 - p0;
                let l2 = segment.length_squared();
                if l2 < 1e-6 { continue; }
                let t = ((best_closest - p0).dot(segment) / l2).clamp(0.0, 1.0);
                let closest = p0 + segment * t;
                let d = closest.distance_to(best_closest);
                if d < min_phys_dist {
                    min_phys_dist = d;
                    phys_split_idx = i;
                    phys_closest = closest;
                }
            }
            
            for i in 0..=phys_split_idx {
                first_phys.push(old_edge.physical_geometry[i]);
            }
            if first_phys.last().unwrap().distance_to(phys_closest) > 0.01 {
                first_phys.push(phys_closest);
            }
            
            second_phys.push(phys_closest);
            for i in phys_split_idx+1..old_edge.physical_geometry.len() {
                if i == phys_split_idx + 1 && old_edge.physical_geometry[i].distance_to(phys_closest) < 0.01 {
                    continue;
                }
                second_phys.push(old_edge.physical_geometry[i]);
            }
        }
        
        // 5. Update existing edge as first half
        let old_end_node = self.edges[edge_idx].end_node;
        self.remove_from_spatial_index(edge_idx); // Remove BEFORE changing geometry
        self.edges[edge_idx].end_node = new_node_id;
        self.edges[edge_idx].geometry = first_geom;
        self.edges[edge_idx].physical_geometry = first_phys;
        self.edges[edge_idx].physical_length = self.calculate_length(&self.edges[edge_idx].physical_geometry);
        
        // 5.5 RE-INDEX for modified edge
        self.add_to_spatial_index(edge_idx);
        
        if let Some(adj) = self.adjacency.get_mut(&old_end_node) {
            adj.retain(|&i| i != edge_idx);
        }
        self.adjacency.entry(new_node_id).or_default().push(edge_idx);
        
        // 6. Create new edge as second half
        let new_edge_id = self.add_edge(Edge {
            start_node: new_node_id,
            end_node,
            geometry: second_geom,
            physical_geometry: second_phys,
            physical_length: 0.0, // calculate below
            ..old_edge
        });
        self.edges[new_edge_id].physical_length = self.calculate_length(&self.edges[new_edge_id].physical_geometry);
        
        // 7. Handle Lane Connections (The "Detour" Fix)
        // A. At new_node_id (the Frontage gateway): allow straight-through turns
        let mut frontage_conns = HashMap::new();
        // Forward: Edge edge_idx -> Edge new_edge_id
        let mut fwd_tgts = Vec::new();
        for lane in 0..old_edge.fwd_lanes {
            fwd_tgts.push((new_edge_id, lane as i8));
        }
        if !fwd_tgts.is_empty() { frontage_conns.insert((edge_idx, 0), fwd_tgts); }
        
        // Backward: Edge new_edge_id -> Edge edge_idx
        let mut bkw_tgts = Vec::new();
        for lane in 0..old_edge.bkw_lanes {
            bkw_tgts.push((edge_idx, -(lane as i8) - 1));
        }
        if !bkw_tgts.is_empty() { frontage_conns.insert((new_edge_id, -1), bkw_tgts); }
        self.nodes[new_node_id as usize].lane_connections = frontage_conns;
        
        // B. At end_node: Remap connections from edge_idx to new_edge_id
        let node_ref = &mut self.nodes[end_node as usize];
        let mut new_node_conns = HashMap::new();
        for (src, tgts) in node_ref.lane_connections.drain() {
            let mut new_src = src;
            if src.0 == edge_idx { new_src.0 = new_edge_id; }
            
            let mut new_tgts = Vec::new();
            for mut t in tgts {
                if t.0 == edge_idx { t.0 = new_edge_id; }
                new_tgts.push(t);
            }
            new_node_conns.insert(new_src, new_tgts);
        }
        node_ref.lane_connections = new_node_conns;

        (new_node_id, new_edge_id)
    }

    pub fn calculate_length(&self, pts: &[Vector3]) -> f32 {
        let mut l = 0.0;
        for i in 0..pts.len().saturating_sub(1) {
            l += pts[i].distance_to(pts[i+1]);
        }
        l
    }

    pub fn remove_node_and_merge_edges(&mut self, node_id: u32) -> Option<(usize, usize)> {
        if node_id as usize >= self.nodes.len() { return None; }
        
        // Find edges connected to this node
        let mut e1_idx = None;
        let mut e2_idx = None;
        
        for (i, edge) in self.edges.iter().enumerate() {
            if edge.deleted { continue; } // Important: Skip already deleted edges
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
                if e1.primary_type != e2.primary_type || e1.width != e2.width { return None; }
                
                // Determine the flow: A -> node_id -> B
                let a = if e1.start_node == node_id { e1.end_node } else { e1.start_node };
                let b = if e2.start_node == node_id { e2.end_node } else { e2.start_node };
                
                (a, node_id, b)
            };

            // Combine geometry
            let mut new_geom = Vec::new();
            let (first_edge_idx, second_edge_idx) = {
                if self.edges[i1].end_node == mid_node { (i1, i2) } else { (i2, i1) }
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
            self.edges[first_edge_idx].physical_length = self.calculate_length(&self.edges[first_edge_idx].physical_geometry);

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
    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 { return; }
        // Ensure we always map to the ultimate valid parent and don't loop
        let keep = self.get_valid_node(id1.min(id2));
        let remove = self.get_valid_node(id1.max(id2));
        if keep == remove { return; }
        
        let new_pos = self.nodes[keep as usize].pos;
        self.node_aliases.insert(remove, keep);
        
        // Merging two network pieces transforms any restrictive node type into a Junction
        self.nodes[keep as usize].node_type = NodeType::Junction;
        self.nodes[keep as usize].lane_connections.clear();

        // Update all edges using the 'remove' node to use 'keep' node instead
        let mut affected_edges = Vec::new();
        for (i, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted { continue; }
            let mut changed = false;
            if edge.start_node == remove { 
                edge.start_node = keep; 
                if !edge.geometry.is_empty() { edge.geometry[0] = new_pos; }
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

    pub fn move_node(&mut self, node_id: u32, new_pos: Vector3) {
        let old_pos = self.nodes[node_id as usize].pos;
        let delta = new_pos - old_pos;
        
        self.remove_node_from_spatial_index(node_id, old_pos);
        self.nodes[node_id as usize].pos = new_pos;
        self.add_node_to_spatial_index(node_id);

        // Pre-remove modified edges from spatial index while they still have old geometry
        for i in 0..self.edges.len() {
            if !self.edges[i].deleted && (self.edges[i].start_node == node_id || self.edges[i].end_node == node_id) {
                self.remove_from_spatial_index(i);
            }
        }

        for edge in &mut self.edges {
            if edge.deleted { continue; }
            if edge.start_node == node_id || edge.end_node == node_id {
                let count = edge.geometry.len();
                if count < 2 { continue; }
                
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
