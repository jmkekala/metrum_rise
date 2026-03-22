use godot::prelude::*;
use super::types::*;

use std::collections::HashMap;

#[derive(Clone)]
pub struct Node {
    pub pos: Vector3,
    #[allow(dead_code)]
    pub node_type: NodeType,
    
    // Traffic Lane Manager Constraints: (From Edge, From Lane) -> List of (To Edge, To Lane)
    pub lane_connections: HashMap<(usize, i8), Vec<(usize, i8)>>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct Edge {
    pub start_node: u32,
    pub end_node: u32,
    pub primary_type: TransitType,
    pub allowed_types: u8,
    pub width: f32,
    pub fwd_lanes: u8,
    pub bkw_lanes: u8,
    pub speed_limit: f32,
    pub base_cost: f32,
    pub physical_length: f32,
    pub current_congestion: f32,
    pub start_clip: f32, 
    pub end_clip: f32,
    pub geometry: Vec<Vector3>, 
    pub physical_geometry: Vec<Vector3>, 
    pub parking_occupied: u32,
    pub zoning_left: bool,
    pub zoning_right: bool,
    pub deleted: bool,
}

impl Edge {
    pub fn get_parking_capacity(&self) -> u32 {
        if self.physical_geometry.len() < 2 { return 0; }
        let mut length = 0.0;
        for i in 0..self.physical_geometry.len()-1 {
            length += self.physical_geometry[i].distance_to(self.physical_geometry[i+1]);
        }
        // 6 meters per car, two sides (left and right), regardless of lanes
        ((length / 6.0) as u32) * 2
    }
}

#[derive(Clone)]
pub struct JunctionMesh {
    pub vertices: Vec<Vector3>,
    pub uvs: Vec<Vector2>,
    pub colors: Vec<Color>,
}

#[derive(Clone)]
pub struct TransitGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub junction_polygons: HashMap<u32, JunctionMesh>,
    pub node_aliases: HashMap<u32, u32>,
}

impl TransitGraph {
    pub const CHUNK_SIZE: f32 = 512.0;

    pub fn get_chunk_coords(pos: Vector3) -> (i32, i32) {
        ((pos.x / Self::CHUNK_SIZE).floor() as i32, (pos.z / Self::CHUNK_SIZE).floor() as i32)
    }

    pub fn get_node_chunk(&self, node_id: u32) -> (i32, i32) {
        Self::get_chunk_coords(self.nodes[node_id as usize].pos)
    }

    pub fn get_valid_node(&self, mut id: u32) -> u32 {
        while let Some(&alias) = self.node_aliases.get(&id) {
            id = alias;
        }
        id
    }

    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            junction_polygons: std::collections::HashMap::new(),
            node_aliases: std::collections::HashMap::new(),
        }
    }

    pub fn add_node(&mut self, pos: Vector3, node_type: NodeType) -> u32 {
        let id = self.nodes.len() as u32;
        self.nodes.push(Node { pos, node_type, lane_connections: HashMap::new() });
        id
    }

    pub fn find_or_add_node(&mut self, pos: Vector3, radius: f32, node_type: NodeType) -> u32 {
        let mut found_id = None;
        for (i, node) in self.nodes.iter().enumerate() {
            if node.pos.distance_to(pos) < radius {
                found_id = Some(self.get_valid_node(i as u32));
                break;
            }
        }
        
        if let Some(id) = found_id {
            // Upgrading a Frontage node to a Junction clears its strict driveway lane rules,
            // allowing cars to legally turn onto the newly built intersecting road!
            if node_type == NodeType::Junction && self.nodes[id as usize].node_type == NodeType::Frontage {
                self.nodes[id as usize].node_type = NodeType::Junction;
                self.nodes[id as usize].lane_connections.clear();
            }
            return id;
        }
        self.add_node(pos, node_type)
    }

    pub fn add_edge(&mut self, mut edge: Edge) -> usize {
        edge.deleted = false;
        let id = self.edges.len();
        self.edges.push(edge);
        id
    }

    pub fn split_edge(&mut self, edge_id: usize, split_pos: Vector3) -> (u32, usize) {
        let old_edge = self.edges[edge_id].clone();
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
            return (old_edge.start_node, edge_id);
        }
        let end_pos = self.nodes[old_edge.end_node as usize].pos;
        if best_closest.distance_to(end_pos) < 2.0 {
            return (old_edge.end_node, edge_id);
        }

        // 2. Create the new frontage node
        let new_node_id = self.add_node(best_closest, NodeType::Frontage);
        
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
        self.edges[edge_id].end_node = new_node_id;
        self.edges[edge_id].geometry = first_geom;
        self.edges[edge_id].physical_geometry = first_phys;
        self.edges[edge_id].physical_length = self.calculate_length(&self.edges[edge_id].physical_geometry);
        
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
        // Forward: Edge edge_id -> Edge new_edge_id
        let mut fwd_tgts = Vec::new();
        for lane in 0..old_edge.fwd_lanes {
            fwd_tgts.push((new_edge_id, lane as i8));
        }
        if !fwd_tgts.is_empty() { frontage_conns.insert((edge_id, 0), fwd_tgts); }
        
        // Backward: Edge new_edge_id -> Edge edge_id
        let mut bkw_tgts = Vec::new();
        for lane in 0..old_edge.bkw_lanes {
            bkw_tgts.push((edge_id, -(lane as i8) - 1));
        }
        if !bkw_tgts.is_empty() { frontage_conns.insert((new_edge_id, -1), bkw_tgts); }
        self.nodes[new_node_id as usize].lane_connections = frontage_conns;
        
        // B. At end_node: Remap connections from edge_id to new_edge_id
        let node_ref = &mut self.nodes[end_node as usize];
        let mut new_node_conns = HashMap::new();
        for (src, tgts) in node_ref.lane_connections.drain() {
            let mut new_src = src;
            if src.0 == edge_id { new_src.0 = new_edge_id; }
            
            let mut new_tgts = Vec::new();
            for mut t in tgts {
                if t.0 == edge_id { t.0 = new_edge_id; }
                new_tgts.push(t);
            }
            new_node_conns.insert(new_src, new_tgts);
        }
        node_ref.lane_connections = new_node_conns;
        
        // C. At start_node: Remap connections to edge_id are still valid, 
        // as edge_id now logically ends at the frontage node. 
        // Wait! If there were connections FROM Node 0 TO Node 1 via Edge 0.
        // Now those connections should lead to Node F. This is already true by updating the edge's end_node.

        (new_node_id, new_edge_id)
    }

    fn calculate_length(&self, pts: &[Vector3]) -> f32 {
        let mut l = 0.0;
        for i in 0..pts.len().saturating_sub(1) {
            l += pts[i].distance_to(pts[i+1]);
        }
        l
    }

    pub fn remove_node_and_merge_edges(&mut self, node_id: u32) {
        if node_id as usize >= self.nodes.len() { return; }
        
        // Find edges connected to this node
        let mut e1_idx = None;
        let mut e2_idx = None;
        
        for (i, edge) in self.edges.iter().enumerate() {
            if edge.start_node == node_id || edge.end_node == node_id {
                if e1_idx.is_none() {
                    e1_idx = Some(i);
                } else if e2_idx.is_none() {
                    e2_idx = Some(i);
                } else {
                    // More than 2 edges? This node is likely a real intersection now.
                    // DO NOT MERGE.
                    return;
                }
            }
        }
        
        if let (Some(i1), Some(i2)) = (e1_idx, e2_idx) {
            // Check if they are compatible for merging
            let (_target_end_node, mid_node, target_start_node) = {
                let e1 = &self.edges[i1];
                let e2 = &self.edges[i2];
                if e1.primary_type != e2.primary_type || e1.width != e2.width { return; }
                
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

            // Remove the second edge
            let to_remove = second_edge_idx;
            self.edges.remove(to_remove);
        }
    }

    /// Merges two nodes into one, updating all edges that use them
    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 { return; }
        // Ensure we always map to the ultimate valid parent and don't loop
        let keep = self.get_valid_node(id1.min(id2));
        let remove = self.get_valid_node(id1.max(id2));
        if keep == remove { return; }
        
        self.node_aliases.insert(remove, keep);
        
        // Merging two network pieces transforms any restrictive node type into a Junction
        if self.nodes[keep as usize].node_type == NodeType::Frontage {
            self.nodes[keep as usize].node_type = NodeType::Junction;
            self.nodes[keep as usize].lane_connections.clear();
        }
        
        // Update all edges using the 'remove' node to use 'keep' node instead
        for edge in &mut self.edges {
            if edge.start_node == remove { edge.start_node = keep; }
            if edge.end_node == remove { edge.end_node = keep; }
        }
        
        // Note: We don't remove the node from the Vec to keep indices stable
        // The DSU in get_island_count will naturally see them as united now
    }

    pub fn move_node(&mut self, node_id: u32, new_pos: Vector3) {
        let old_pos = self.nodes[node_id as usize].pos;
        let delta = new_pos - old_pos;
        self.nodes[node_id as usize].pos = new_pos;

        for edge in &mut self.edges {
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
        
        self.rebuild_intersection_clips();
    }

    /// Returns the number of disconnected components (islands) in the network
    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() { return 0; }
        
        // Disjoint Set Union (DSU)
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();
        
        fn find(i: usize, parent: &mut Vec<usize>) -> usize {
            if parent[i] == i { return i; }
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
            unite(edge.start_node as usize, edge.end_node as usize, &mut parent);
        }
        
        // Count unique roots (only for nodes that are part of an edge to avoid counting "floating" preview nodes)
        let mut active_nodes = std::collections::HashSet::new();
        for edge in &self.edges {
            active_nodes.insert(edge.start_node as usize);
            active_nodes.insert(edge.end_node as usize);
        }
        
        let mut roots = std::collections::HashSet::new();
        for &node_idx in &active_nodes {
            roots.insert(find(node_idx, &mut parent));
        }
        
        roots.len()
    }

    pub fn sync_to_terrain(&mut self, terrain: &crate::simulation::terrain::TerrainSystem) {
        let hw = (terrain.width as f32 - 1.0) * 0.5;
        let hh = (terrain.height as f32 - 1.0) * 0.5;
        
        // 1. Sync Nodes Only
        for node in &mut self.nodes {
            let gx = node.pos.x + hw;
            let gz = node.pos.z + hh;
            node.pos.y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
        }
        
        // 2. Re-interpolate Edge Geometry (Smooth Grades)
        for edge in &mut self.edges {
            let count = edge.geometry.len();
            if count < 2 { continue; }
            
            // Snap endpoints to nodes
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;
            
            // HARMONIC CONFORMANCE (Laplacian Smoothing)
            // 1. Re-sample raw terrain for all intermediate points so road follows new hills
            for j in 1..count-1 {
                let gx = edge.geometry[j].x + hw;
                let gz = edge.geometry[j].z + hh;
                edge.geometry[j].y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
            }

            // 2. Taubin Smoothing to iron out bumps without volume shrinkage
            let iters = 50;
            if count > 2 {
                let mut temp_h = vec![0.0; count];
                let lambda = 0.5;
                let mu = -0.53;
                for _ in 0..iters {
                    // Positive Pass (Shrink/Smooth)
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + lambda * laplacian;
                    }
                    for j in 1..count-1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                    // Negative Pass (Inflate/Restore Volume)
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + mu * laplacian;
                    }
                    for j in 1..count-1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                }
            }
        }
        self.rebuild_intersection_clips();
    }

    pub fn rebuild_intersection_clips(&mut self) {
        let mut connection_counts = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }
            *connection_counts.entry(edge.start_node).or_insert(0) += 1;
            *connection_counts.entry(edge.end_node).or_insert(0) += 1;
        }

        // Calculate maximum road width connected to each node for dynamic junction clipping
        let mut node_max_width = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }
            let w = edge.width;
            let s_max = node_max_width.entry(edge.start_node).or_insert(0.0);
            if w > *s_max { *s_max = w; }
            let e_max = node_max_width.entry(edge.end_node).or_insert(0.0);
            if w > *e_max { *e_max = w; }
        }

        self.junction_polygons.clear(); // Removing procedural intersection meshes

        for (_edge_id, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }

            // Dynamic clipping: Ensure the junction covers at least the width of the road
            let s_conn = *connection_counts.get(&edge.start_node).unwrap_or(&0);
            let e_conn = *connection_counts.get(&edge.end_node).unwrap_or(&0);
            
            let s_max_w = *node_max_width.get(&edge.start_node).unwrap_or(&0.0);
            let e_max_w = *node_max_width.get(&edge.end_node).unwrap_or(&0.0);

            let s_clip = s_max_w * 0.7; // 70% of max width provides enough clear space for junctions
            let e_clip = e_max_w * 0.7;

            let s_node = &self.nodes[edge.start_node as usize];
            let e_node = &self.nodes[edge.end_node as usize];

            edge.start_clip = if s_conn > 1 && s_node.node_type == NodeType::Junction { s_clip } else { 0.0 };
            edge.end_clip = if e_conn > 1 && e_node.node_type == NodeType::Junction { e_clip } else { 0.0 };

            let count = edge.geometry.len();
            if count >= 2 {
                let mut total_length = 0.0;
                for i in 0..count - 1 {
                    let p0 = edge.geometry[i];
                    let p1 = edge.geometry[i + 1];
                    total_length += (p1 - p0).length();
                }
                
                let valid_len = (total_length - edge.end_clip) - edge.start_clip;
                if valid_len <= 0.1 {
                    edge.physical_geometry.clear();
                    continue;
                }
                let num_segments = f32::max(1.0, f32::ceil(valid_len / 2.0)) as usize;
                let mut resampled = Vec::new();
                
                for i in 0..=num_segments {
                    let dist = edge.start_clip + (i as f32 / num_segments as f32) * valid_len;
                    let mut curr = 0.0;
                    let mut found = false;
                    for j in 0..count - 1 {
                        let p0 = edge.geometry[j];
                        let p1 = edge.geometry[j + 1];
                        let d = (p1 - p0).length();
                        if curr + d >= dist {
                            let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                            resampled.push(p0.lerp(p1, t));
                            found = true;
                            break;
                        }
                        curr += d;
                    }
                    if !found { resampled.push(*edge.geometry.last().unwrap()); }
                }
                
                // Keep node-end height alignment
                let start_node_y = self.nodes[edge.start_node as usize].pos.y;
                let end_node_y = self.nodes[edge.end_node as usize].pos.y;
                if !resampled.is_empty() {
                    resampled[0].y = start_node_y;
                    let last_idx = resampled.len() - 1;
                    resampled[last_idx].y = end_node_y;
                }
                
                edge.physical_geometry = resampled;
            } else {
                edge.physical_geometry = edge.geometry.clone();
            }
        }
    }
}

/// Validates that the intersection mesh is mathematically sound.
/// Returns Ok(()) if the mesh is valid, or an error describing the failure.
pub fn verify_intersection_geometry(_center: Vector3, triangles: &[Vector3]) -> Result<(), String> {
    // 1. Check for Triangle Completeness
    if triangles.len() % 3 != 0 {
        return Err("Malformed mesh: Vertex count is not a multiple of 3.".into());
    }

    for i in (0..triangles.len()).step_by(3) {
        let p0 = triangles[i];   // Center
        let p1 = triangles[i+1]; // Right Corner
        let p2 = triangles[i+2]; // Left Corner

        // 2. Calculate the Normal using the Cross Product
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let normal = edge1.cross(edge2);

        // 3. Winding Order Check
        // If Y is negative, the triangle is upside down in Godot's coordinate system (assuming clock-wise + Y-up front-facing).
        if normal.y < -0.1 {
            return Err(format!(
                "Inverted Winding: Triangle {} is facing downward. Current rule requires upward winding.",
                i / 3
            ));
        }

        // 4. Degenerate Triangle Check
        if normal.length() < 0.0001 {
            continue;
        }
    }

    Ok(())
}
