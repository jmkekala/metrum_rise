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
                    let mut dir = Vector2::new(1.0, 0.0);
                    let mut seg_len = 0.0;
                    for j in 0..edge.geometry.len() - 1 {
                        let d3 = edge.geometry[j+1] - edge.geometry[j];
                        let d2 = Vector2::new(d3.x, d3.z);
                        if d2.length() > 0.1 {
                            dir = d2.normalized();
                            seg_len = d3.length();
                            break;
                        }
                    }
                    let angle = f32::atan2(dir.y, dir.x);
                    connected_edges.push((i, dir, edge.width * 0.5, angle, seg_len));
                } else if edge.end_node == node_id {
                    let mut dir = Vector2::new(-1.0, 0.0);
                    let mut seg_len = 0.0;
                    let lc = edge.geometry.len();
                    for j in (1..lc).rev() {
                        let d3 = edge.geometry[j-1] - edge.geometry[j];
                        let d2 = Vector2::new(d3.x, d3.z);
                        if d2.length() > 0.1 {
                            dir = d2.normalized();
                            seg_len = d3.length();
                            break;
                        }
                    }
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

            let n_center = Vector2::new(node.pos.x, node.pos.z);
            let mut clips: HashMap<usize, f32> = HashMap::new();
            for i in 0..connected_edges.len() {
                let e_core = &connected_edges[i];
                let mut max_c = if connected_edges.len() >= 3 { 1.5 } else { 0.0 };

                for j in 0..connected_edges.len() {
                    if i == j { continue; }
                    let e_other = &connected_edges[j];
                    
                    // Angle between vectors
                    let sweep = (e_core.1.x * e_other.1.y - e_core.1.y * e_other.1.x).atan2(e_core.1.x * e_other.1.x + e_core.1.y * e_other.1.y);
                    let abs_sin = sweep.sin().abs();
                    
                    if abs_sin > 0.01 {
                        let mut req = (e_other.2) / abs_sin; // Distance needed to clear HALF width of the other road
                        if req > 8.0 {
                            // EXTREME ACUTE ANGLE: 
                            // The required clearance is enormous, which destroys the road. 
                            // Instead of a massive clip, we allow the roads to smoothly overlap 
                            // and draw directly to the node center by aggressively dropping the clip to 0.0.
                            req = 0.0;
                        }
                        if req > max_c { max_c = req; }
                    }
                }
                clips.insert(e_core.0, max_c);
            }
            // CRITICAL FIX: Sort edges radially (Counter-Clockwise) based on their outward direction!
            // If they are not radially sorted, connecting edge `i` to `i+1` creates criss-crossing 'bowtie' planes
            // because they might be geometrically opposite to each other based on draw order!
            connected_edges.sort_unstable_by(|a, b| {
                let angle_a = a.1.y.atan2(a.1.x); // Godot X-Z plane mapped to Vector2(x,y)
                let angle_b = b.1.y.atan2(b.1.x);
                angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
            });

            // Symmetrize Parallel Pairs (T-Junctions and Curves)
            if connected_edges.len() >= 3 {
                for i in 0..connected_edges.len() {
                    for j in i+1..connected_edges.len() {
                        let dot = connected_edges[i].1.dot(connected_edges[j].1);
                        if dot < -0.80 { // Relaxed to handle curves (thru-roads)
                            let id1 = connected_edges[i].0;
                            let id2 = connected_edges[j].0;
                            let c1 = *clips.get(&id1).unwrap();
                            let c2 = *clips.get(&id2).unwrap();
                            let c_max = c1.max(c2);
                            clips.insert(id1, c_max);
                            clips.insert(id2, c_max);
                            
                            // SYMMETRIZE TANGENTS: Force them to be perfectly opposite for the Hub mesh
                            let avg_x = (connected_edges[i].1.x - connected_edges[j].1.x) * 0.5;
                            let avg_y = (connected_edges[i].1.y - connected_edges[j].1.y) * 0.5;
                            let v = Vector2::new(avg_x, avg_y).normalized();
                            connected_edges[i].1 = v;
                            connected_edges[j].1 = -v;
                        }
                    }
                }
            }
            
            // Final Safety Clamp and Store
            for (edge_id, clip) in clips.iter_mut() {
                let length = *edge_lengths.get(edge_id).unwrap();
                *clip = clip.min(20.0).min(length * 0.45);
                node_clips.insert((node_id as usize, *edge_id), *clip);
            }

            let mut j_mesh = JunctionMesh {
                vertices: Vec::new(),
                uvs: Vec::new(),
                colors: Vec::new(),
            };
            let ct = Vector3::new(n_center.x, node.pos.y, n_center.y);
            
            let mut add_triangle = |a: Vector3, mut b: Vector3, mut c: Vector3, color: Color| {
                let cross = (b - a).cross(c - a);
                if cross.length() < 1e-4 { return; } 
                let normal = cross.normalized();
                // Godot uses +Y for UP. If normal faces down (-Y), swap vertices to flip it UP.
                if normal.y < -0.01 {
                    std::mem::swap(&mut b, &mut c); 
                }
                
                // UV Mapping for Hub (Bottom-Right quadrant of atlas: 0.5-1.0)
                let uv_map = |p: Vector3| {
                    let rel = (p - ct) * 0.05; // 10m = 0.5 units
                    Vector2::new(0.75 + rel.x, 0.75 + rel.z)
                };

                j_mesh.vertices.push(a); j_mesh.vertices.push(b); j_mesh.vertices.push(c);
                j_mesh.uvs.push(uv_map(a)); j_mesh.uvs.push(uv_map(b)); j_mesh.uvs.push(uv_map(c));
                j_mesh.colors.push(color); j_mesh.colors.push(color); j_mesh.colors.push(color);
            };
            
            let is_turn = connected_edges.len() == 2;

            for i in 0..connected_edges.len() {
                let nxt = (i + 1) % connected_edges.len();
                let e1 = &connected_edges[i];
                let e2 = &connected_edges[nxt];

                // e1 is current road
                let c1 = *clips.get(&e1.0).unwrap();
                let cut1 = n_center + e1.1 * c1;
                let right1 = Vector2::new(-e1.1.y, e1.1.x);
                let left1 = Vector2::new(e1.1.y, -e1.1.x);
                let _p_right1 = cut1 + right1 * e1.2;
                let _p_left1 = cut1 + left1 * e1.2;

                // e2 is next road
                let c2 = *clips.get(&e2.0).unwrap();
                let cut2 = n_center + e2.1 * c2;
                let left2 = Vector2::new(e2.1.y, -e2.1.x);
                let p_left2 = cut2 + left2 * e2.2;
                
                let edge1 = &self.edges[e1.0];
                let edge2 = &self.edges[e2.0];
                let fwd = (edge1.fwd_lanes + edge2.fwd_lanes) as f32 * 0.5 / 10.0;
                let bkw = (edge1.bkw_lanes + edge2.bkw_lanes) as f32 * 0.5 / 10.0;
                
                let _kerb_w = 0.0; // Disabled: Kerbs are managed externally or entirely suppressed for T-junctions
                let _kerb_h = 0.05;
                let asph_w1 = e1.2;
                let asph_w2 = e2.2;
                
                // Align Y level with road.rs `h_offset = 0.15` to cure massive Z-fighting
                let node_y = node.pos.y + 0.15;

                let p_right1_asph = cut1 + right1 * asph_w1;
                let p_left1_asph = cut1 + left1 * asph_w1;
                let _p_left2_asph = cut2 + left2 * asph_w2;

                let pr1_a = Vector3::new(p_right1_asph.x, node_y, p_right1_asph.y);
                let pl1_a = Vector3::new(p_left1_asph.x, node_y, p_left1_asph.y);
                let _pl2_a = Vector3::new(p_left2.x, node_y, p_left2.y);
                let ct_a = Vector3::new(ct.x, node_y, ct.z);

                let total_lanes = (edge1.fwd_lanes + edge1.bkw_lanes) as f32;
                let bkw_l = edge1.bkw_lanes as f32;
                
                // Gap-specific miter point for UV origin
                // For the 'inner' side of the turn, use the inner miter point.
                let e1_r_norm = Vector2::new(-e1.1.y, e1.1.x);
                let e2_l_norm = Vector2::new(e2.1.y, -e2.1.x);
                let l1_p = n_center + e1_r_norm * e1.2;
                let l2_p = n_center + e2_l_norm * e2.2;
                let denom = e1.1.x * e2.1.y - e1.1.y * e2.1.x;
                let pi = if denom.abs() > 0.001 {
                    let t1 = ((l2_p - l1_p).x * e2.1.y - (l2_p - l1_p).y * e2.1.x) / denom;
                    let inter = l1_p + e1.1 * t1;
                    Vector3::new(inter.x, node_y, inter.y)
                } else { ct_a };

                let get_v_uv = |v: Vector3| {
                    if !is_turn { return Vector2::new(0.5, 0.5); } // Neutral UV for 3+ junctions to kill ghost decals
                    let d = (v - pi).length();
                    // Map distance from local miter point. 
                    // If this is the outer gap, pi is the inner corner, so dist 0 = inner edge.
                    let uvx = (1.0 - d / edge1.width).clamp(0.0, 1.0) * total_lanes;
                    Vector2::new(uvx, 0.0)
                };

                let _uv_ct = if is_turn { Vector2::new(bkw_l, 0.0) } else { get_v_uv(ct_a) };
                let _uv_r1 = get_v_uv(pr1_a);
                let _uv_l1 = get_v_uv(pl1_a);

                let fade_color = if is_turn { Color::from_rgba(fwd, bkw, 0.0, 1.0) } else { Color::from_rgba(0.0, 0.0, 0.0, 0.0) };

                // 1. Asphalt Core (Sector to Road End)
                add_triangle(ct_a, pr1_a, pl1_a, fade_color);

                let left2 = Vector2::new(e2.1.y, -e2.1.x);
                let p_left2 = cut2 + left2 * asph_w2;
                let pl2_a = Vector3::new(p_left2.x, node_y, p_left2.y);

                let l1_dir = Vector2::new(-e1.1.x, -e1.1.y);
                let l2_dir = Vector2::new(-e2.1.x, -e2.1.y);
                let denom = l1_dir.x * l2_dir.y - l1_dir.y * l2_dir.x;
                
                let pr1_a_2d = Vector2::new(pr1_a.x, pr1_a.z);
                let pl2_a_2d = Vector2::new(pl2_a.x, pl2_a.z);
                let diff = pl2_a_2d - pr1_a_2d;

                let use_bezier = if denom.abs() > 0.001 {
                    // Prevent sharp hairpin or near-straight >148deg outer sections from attempting mathematically unstable 30m Bezier curves!
                    if e1.1.dot(e2.1) < -0.85 {
                        false
                    } else {
                        let t1 = (diff.x * l2_dir.y - diff.y * l2_dir.x) / denom;
                        let t2 = (diff.x * l1_dir.y - diff.y * l1_dir.x) / denom;
                        // Prevent near-parallel lines from shooting intersection point to infinity
                        let max_dist = diff.length() * 1.5;
                        if t1 > 0.0 && t2 > 0.0 && t1 < max_dist && t2 < max_dist {
                            // Spatial Inversion Guard: Ensures the control point hasn't mathematically crossed 
                            // through the node center onto the opposite side of the road (happens on >180 deg outer curves).
                            let pi_test = pr1_a_2d + l1_dir * t1;
                            let gap_mid = (pr1_a_2d + pl2_a_2d) * 0.5;
                            let center_2d = Vector2::new(ct_a.x, ct_a.z);
                            let center_to_mid = gap_mid - center_2d;
                            let center_to_pi = pi_test - center_2d;
                            center_to_mid.dot(center_to_pi) > 0.01 // Must reside in the same hemisphere
                        } else { false }
                    }
                } else { false };

                let pi_a = if use_bezier {
                    let t1 = (diff.x * l2_dir.y - diff.y * l2_dir.x) / denom;
                    let inter = pr1_a_2d + l1_dir * t1;
                    Vector3::new(inter.x, node_y, inter.y)
                } else { ct_a };

                let steps = 16;
                let mut prev_a_low = pr1_a;

                // 2. Smooth Asphalt Corner
                for j in 1..=steps {
                    let t = j as f32 / steps as f32;
                    let curr_a_low = if use_bezier {
                        let inv_t = 1.0 - t;
                        pr1_a * (inv_t * inv_t) + pi_a * (2.0 * inv_t * t) + pl2_a * (t * t)
                    } else {
                        pr1_a.lerp(pl2_a, t)
                    };

                    if (curr_a_low - prev_a_low).length() > 0.001 {
                        add_triangle(ct_a, prev_a_low, curr_a_low, fade_color);
                    }
                    prev_a_low = curr_a_low;
                }
                
                let current_c1 = *node_clips.get(&(node_id as usize, e1.0)).unwrap_or(&0.0);
                node_clips.insert((node_id as usize, e1.0), current_c1.max(c1.max(0.0)));
                
                let current_c2 = *node_clips.get(&(node_id as usize, e2.0)).unwrap_or(&0.0);
                node_clips.insert((node_id as usize, e2.0), current_c2.max(c2.max(0.0)));
            }

            if j_mesh.vertices.len() >= 3 {
                match verify_intersection_geometry(ct, &j_mesh.vertices) {
                    Ok(_) => {
                        self.junction_polygons.insert(node_id, j_mesh);
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
                    let p0 = edge.geometry[i];
                    let p1 = edge.geometry[i + 1];
                    total_length += Vector2::new(p1.x - p0.x, p1.z - p0.z).length();
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
                        let d = Vector2::new(p1.x - p0.x, p1.z - p0.z).length();
                        if curr + d >= dist {
                            // Defensive guard against catastrophic NaN injection when split_pos falls exactly
                            // onto an existing interpolation marker, causing d=0.0.
                            let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                            resampled.push(p0.lerp(p1, t));
                            found = true;
                            break;
                        }
                        curr += d;
                    }
                    if !found { resampled.push(*edge.geometry.last().unwrap()); }
                }
                
                // Hardware Enforce Y-Altitude Alignment with Intersection Hubs
                // Curve3D geometry heights often detach from the mathematically verified node center over hills.
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
                let start_node_y = self.nodes[edge.start_node as usize].pos.y;
                let end_node_y = self.nodes[edge.end_node as usize].pos.y;
                if !edge.physical_geometry.is_empty() {
                    edge.physical_geometry[0].y = start_node_y;
                    let last_idx = edge.physical_geometry.len() - 1;
                    edge.physical_geometry[last_idx].y = end_node_y;
                }
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
