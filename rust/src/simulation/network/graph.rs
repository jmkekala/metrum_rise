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
    pub current_congestion: f32,
    pub start_clip: f32, 
    pub end_clip: f32,
    pub geometry: Vec<Vector3>, 
    pub physical_geometry: Vec<Vector3>, 
}

#[derive(Clone)]
pub struct TransitGraph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub junction_polygons: std::collections::HashMap<u32, Vec<Vector3>>,
    pub node_aliases: std::collections::HashMap<u32, u32>,
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
        for (i, node) in self.nodes.iter().enumerate() {
            if node.pos.distance_to(pos) < radius {
                return i as u32;
            }
        }
        self.add_node(pos, node_type)
    }

    pub fn add_edge(&mut self, edge: Edge) -> usize {
        let id = self.edges.len();
        self.edges.push(edge);
        id
    }

    /// Merges two nodes into one, updating all edges that use them
    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 { return; }
        // Ensure we always map to the ultimate valid parent and don't loop
        let keep = self.get_valid_node(id1.min(id2));
        let remove = self.get_valid_node(id1.max(id2));
        if keep == remove { return; }
        
        self.node_aliases.insert(remove, keep);
        
        // Update all edges using the 'remove' node to use 'keep' node instead
        for edge in &mut self.edges {
            if edge.start_node == remove { edge.start_node = keep; }
            if edge.end_node == remove { edge.end_node = keep; }
        }
        
        // Note: We don't remove the node from the Vec to keep indices stable
        // The DSU in get_island_count will naturally see them as united now
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
        let mut edge_lengths = HashMap::new();
        for (i, edge) in self.edges.iter().enumerate() {
            let mut total = 0.0;
            if edge.geometry.len() >= 2 {
                for j in 0..edge.geometry.len() - 1 {
                    total += (edge.geometry[j + 1] - edge.geometry[j]).length();
                }
            }
            edge_lengths.insert(i, total);
        }

        let mut node_clips: HashMap<(usize, usize), f32> = HashMap::new();
        self.junction_polygons.clear();

        for (node_id_usize, node) in self.nodes.iter().enumerate() {
            let node_id = node_id_usize as u32;
            let mut connected_edges = Vec::new();

            for (i, edge) in self.edges.iter().enumerate() {
                if edge.primary_type != TransitType::Road { continue; }
                if edge.geometry.len() < 2 { continue; }
                if edge.start_node == node_id {
                    let d3 = edge.geometry[1] - edge.geometry[0];
                    let seg_len = d3.length();
                    let dir = Vector2::new(d3.x, d3.z).normalized();
                    let angle = f32::atan2(dir.y, dir.x);
                    connected_edges.push((i, dir, edge.width * 0.5, angle, seg_len));
                } else if edge.end_node == node_id {
                    let lc = edge.geometry.len();
                    let d3 = edge.geometry[lc-2] - edge.geometry[lc-1];
                    let seg_len = d3.length();
                    let dir = Vector2::new(d3.x, d3.z).normalized();
                    let angle = f32::atan2(dir.y, dir.x);
                    connected_edges.push((i, dir, edge.width * 0.5, angle, seg_len));
                }
            }

            if connected_edges.len() < 2 {
                for &(edge_id, _, _, _, _) in &connected_edges {
                    node_clips.insert((node_id as usize, edge_id), 0.0);
                }
                continue;
            }
            
            connected_edges.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());

            let mut clips: HashMap<usize, f32> = HashMap::new();
            for &(edge_id, _, _, _, _) in &connected_edges {
                clips.insert(edge_id, 0.0);
            }

            let n_center = Vector2::new(node.pos.x, node.pos.z);
            for i in 0..connected_edges.len() {
                let nxt = (i + 1) % connected_edges.len();
                let e1 = &connected_edges[i];
                let e2 = &connected_edges[nxt];

                let e1_r_norm = Vector2::new(-e1.1.y, e1.1.x);
                let l1_p = n_center + e1_r_norm * e1.2;
                let l1_dir = e1.1;

                let e2_l_norm = Vector2::new(e2.1.y, -e2.1.x);
                let l2_p = n_center + e2_l_norm * e2.2;
                let l2_dir = e2.1;

                let denom = l1_dir.x * l2_dir.y - l1_dir.y * l2_dir.x;
                if denom.abs() > 0.001 {
                    let diff = l2_p - l1_p;
                    let t1 = (diff.x * l2_dir.y - diff.y * l2_dir.x) / denom;
                    let inter = l1_p + l1_dir * t1;
                    
                    if inter.distance_to(n_center) < 150.0 {
                        let p1_proj = (inter - n_center).dot(l1_dir);
                        let p2_proj = (inter - n_center).dot(l2_dir);
                        
                        if p1_proj > 0.0 {
                            let c1 = clips.get(&e1.0).unwrap().max(p1_proj);
                            clips.insert(e1.0, c1);
                        }
                        if p2_proj > 0.0 {
                            let c2 = clips.get(&e2.0).unwrap().max(p2_proj);
                            clips.insert(e2.0, c2);
                        }
                    }
                }
            }

            let mut poly_3d = Vec::new();
            let ct = Vector3::new(n_center.x, node.pos.y, n_center.y);
            
            let mut add_triangle = |mut a: Vector3, mut b: Vector3, mut c: Vector3| {
                let normal = (b - a).cross(c - a);
                if normal.length() < 0.0001 { return; } // Avoid degenerate zero-area geometry
                if normal.y > 0.0 {
                    std::mem::swap(&mut b, &mut c); // Force CW Upward facing
                }
                poly_3d.push(a); poly_3d.push(b); poly_3d.push(c);
            };

            for i in 0..connected_edges.len() {
                let nxt = (i + 1) % connected_edges.len();
                let e1 = &connected_edges[i];
                let e2 = &connected_edges[nxt];

                // e1 is current road
                let c1 = *clips.get(&e1.0).unwrap();
                let cut1 = n_center + e1.1 * c1;
                let right1 = Vector2::new(-e1.1.y, e1.1.x);
                let left1 = Vector2::new(e1.1.y, -e1.1.x);
                let p_right1 = cut1 + right1 * e1.2;
                let p_left1 = cut1 + left1 * e1.2;

                let pr1 = Vector3::new(p_right1.x, node.pos.y, p_right1.y);
                let pl1 = Vector3::new(p_left1.x, node.pos.y, p_left1.y);

                // e2 is next road
                let c2 = *clips.get(&e2.0).unwrap();
                let cut2 = n_center + e2.1 * c2;
                let left2 = Vector2::new(e2.1.y, -e2.1.x);
                let p_left2 = cut2 + left2 * e2.2;
                let pl2 = Vector3::new(p_left2.x, node.pos.y, p_left2.y);
                
                // Triangle 1: Fill the "Road End"
                add_triangle(ct, pr1, pl1);

                // Triangle 2: Corner Gap
                add_triangle(ct, pl2, pr1);
                
                node_clips.insert((node_id as usize, e1.0), c1.max(0.0));
            }

            if poly_3d.len() >= 3 {
                match verify_intersection_geometry(ct, &poly_3d) {
                    Ok(_) => {
                        self.junction_polygons.insert(node_id, poly_3d);
                    }
                    Err(e) => {
                        println!("Intersection Validation Error at Node {}: {}", node_id, e);
                    }
                }
            }
        }

        for (edge_id, edge) in self.edges.iter_mut().enumerate() {
            edge.start_clip = *node_clips.get(&(edge.start_node as usize, edge_id)).unwrap_or(&0.0_f32);
            edge.end_clip = *node_clips.get(&(edge.end_node as usize, edge_id)).unwrap_or(&0.0_f32);
            
            let count = edge.geometry.len();
            if count >= 2 {
                let mut total_length = 0.0;
                for i in 0..count - 1 {
                    total_length += (edge.geometry[i + 1] - edge.geometry[i]).length();
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
                            let t = (dist - curr) / d;
                            resampled.push(p0.lerp(p1, t));
                            found = true;
                            break;
                        }
                        curr += d;
                    }
                    if !found { resampled.push(*edge.geometry.last().unwrap()); }
                }
                edge.physical_geometry = resampled;
                println!("Edge {} clipped: start {}, end {}", edge_id, edge.start_clip, edge.end_clip);
            } else {
                edge.physical_geometry = edge.geometry.clone();
            }
        }
        println!("Rebuild intersection clips: Total Junction Polygons: {}", self.junction_polygons.len());
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

        // 3. Winding Order Check (The "Black Hole" Fix)
        // If Y is positive, the triangle is upside down in Godot's coordinate system.
        if normal.y >= 0.0 {
            return Err(format!(
                "Inverted Winding: Triangle {} is facing downward. Winding order is incorrect.",
                i / 3
            ));
        }

        // 4. Degenerate Triangle Check (The "Zero-Area" Fix)
        // If the normal's length is near zero, the points are in a straight line.
        if normal.length() < 0.0001 {
            return Err(format!(
                "Degenerate Geometry: Triangle {} has zero area (collinear points).",
                i / 3
            ));
        }
    }

    Ok(())
}
