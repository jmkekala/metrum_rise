use godot::prelude::*;
use super::types::*;

use std::collections::HashMap;

#[derive(Clone)]
pub struct Node {
    pub pos: Vector3,
    #[allow(dead_code)]
    pub node_type: NodeType,
    
    // TM:PE Lane Constraints: (From Edge, From Lane) -> List of (To Edge, To Lane)
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
}

impl TransitGraph {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            junction_polygons: std::collections::HashMap::new(),
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
        let (keep, remove) = (id1.min(id2), id1.max(id2));
        
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
                    let dir = Vector2::new(d3.x, d3.z).normalized();
                    let angle = f32::atan2(dir.y, dir.x);
                    connected_edges.push((i, dir, edge.width * 0.5, angle));
                } else if edge.end_node == node_id {
                    let lc = edge.geometry.len();
                    let d3 = edge.geometry[lc-2] - edge.geometry[lc-1];
                    let dir = Vector2::new(d3.x, d3.z).normalized();
                    let angle = f32::atan2(dir.y, dir.x);
                    connected_edges.push((i, dir, edge.width * 0.5, angle));
                }
            }

            if connected_edges.len() < 2 {
                for &(edge_id, _, _, _) in &connected_edges {
                    node_clips.insert((node_id as usize, edge_id), 0.0);
                }
                continue;
            }
            
            connected_edges.sort_by(|a, b| a.3.partial_cmp(&b.3).unwrap());

            let mut pool = Vec::new();
            let mut clips: HashMap<usize, f32> = HashMap::new();
            for &(edge_id, _, _, _) in &connected_edges {
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
                    
                    if inter.distance_to(n_center) < 40.0 {
                        pool.push(inter);
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

            for &(edge_id, dir, hw, _) in &connected_edges {
                let c = *clips.get(&edge_id).unwrap();
                let cut_center = n_center + dir * c;
                let right_norm = Vector2::new(-dir.y, dir.x);
                let left_norm = Vector2::new(dir.y, -dir.x);
                pool.push(cut_center + left_norm * hw);
                pool.push(cut_center + right_norm * hw);
                node_clips.insert((node_id as usize, edge_id), c.max(0.0));
            }

            if pool.len() >= 3 {
                pool.sort_by(|a, b| {
                    if a.x == b.x {
                        a.y.partial_cmp(&b.y).unwrap()
                    } else {
                        a.x.partial_cmp(&b.x).unwrap()
                    }
                });
                
                let cross = |o: Vector2, a: Vector2, b: Vector2| -> f32 {
                    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
                };

                let mut lower = Vec::new();
                for &p in &pool {
                    while lower.len() >= 2 && cross(lower[lower.len()-2], *lower.last().unwrap(), p) <= 0.0 {
                        lower.pop();
                    }
                    lower.push(p);
                }

                let mut upper = Vec::new();
                for &p in pool.iter().rev() {
                    while upper.len() >= 2 && cross(upper[upper.len()-2], *upper.last().unwrap(), p) <= 0.0 {
                        upper.pop();
                    }
                    upper.push(p);
                }

                lower.pop();
                upper.pop();
                lower.extend(upper);
                
                let mut poly_3d = Vec::new();
                for p in lower {
                    poly_3d.push(Vector3::new(p.x, node.pos.y, p.y));
                }
                
                if poly_3d.len() >= 3 {
                    self.junction_polygons.insert(node_id, poly_3d);
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
                let num_segments = f32::max(1.0, f32::ceil(valid_len / 10.0)) as usize;
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
