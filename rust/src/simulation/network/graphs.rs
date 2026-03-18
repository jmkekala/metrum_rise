use godot::prelude::*;
use super::types::*;
use std::collections::HashMap;

#[derive(Clone)]
pub struct Node {
    pub pos: Vector3,
    #[allow(dead_code)]
    pub node_type: NodeType,
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

    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 { return; }
        let (keep, remove) = (id1.min(id2), id1.max(id2));
        for edge in &mut self.edges {
            if edge.start_node == remove { edge.start_node = keep; }
            if edge.end_node == remove { edge.end_node = keep; }
        }
    }

    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() { return 0; }
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();
        fn find(i: usize, parent: &mut Vec<usize>) -> usize {
            if parent[i] == i { return i; }
            parent[i] = find(parent[i], parent);
            parent[i]
        }
        fn unite(i: usize, j: usize, parent: &mut Vec<usize>) {
            let root_i = find(i, parent);
            let root_j = find(j, parent);
            if root_i != root_j { parent[root_i] = root_j; }
        }
        for edge in &self.edges {
            unite(edge.start_node as usize, edge.end_node as usize, &mut parent);
        }
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
        for node in &mut self.nodes {
            let gx = node.pos.x + hw;
            let gz = node.pos.z + hh;
            node.pos.y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
        }
        for edge in &mut self.edges {
            let count = edge.geometry.len();
            if count < 2 { continue; }
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;
            for j in 1..count-1 {
                let gx = edge.geometry[j].x + hw;
                let gz = edge.geometry[j].z + hh;
                edge.geometry[j].y = terrain.get_height_interpolated(gx, gz) * crate::config::HEIGHT_SCALE;
            }
            if count > 2 {
                let mut temp_h = vec![0.0; count];
                let (lambda, mu) = (0.5, -0.53);
                for _ in 0..50 {
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + lambda * laplacian;
                    }
                    for j in 1..count-1 { edge.geometry[j].y = temp_h[j]; }
                    for j in 1..count-1 {
                        let laplacian = 0.5 * (edge.geometry[j-1].y + edge.geometry[j+1].y) - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + mu * laplacian;
                    }
                    for j in 1..count-1 { edge.geometry[j].y = temp_h[j]; }
                }
            }
        }
        self.rebuild_intersection_clips();
    }

    pub fn rebuild_intersection_clips(&mut self) {
        self.junction_polygons.clear();

        for node_idx in 0..self.nodes.len() {
            let node_id = node_idx as u32;
            let center = self.nodes[node_idx].pos;
            let mut edges_at_node = Vec::new();

            for edge in &self.edges {
                if edge.primary_type != TransitType::Road || edge.geometry.len() < 2 { continue; }
                
                if edge.start_node == node_id || edge.end_node == node_id {
                    let is_start = edge.start_node == node_id;
                    let p_next = if is_start { edge.geometry[1] } else { edge.geometry[edge.geometry.len() - 2] };
                    
                    let dir3 = (p_next - center).normalized();
                    let angle = f32::atan2(dir3.z, dir3.x);
                    
                    let hw = edge.width * 0.5;
                    // Auto-heal: Ensure clip is at least road width to prevent zero-area triangles
                    let clip_dist = if is_start { edge.start_clip } else { edge.end_clip }.max(hw * 1.1);
                    
                    let side = Vector3::new(-dir3.z, 0.0, dir3.x).normalized();
                    let cut_center = center + (dir3 * clip_dist);
                    
                    let p_left = cut_center + (side * hw);
                    let p_right = cut_center - (side * hw);

                    edges_at_node.push((angle, p_left, p_right));
                }
            }

            if edges_at_node.len() < 2 { continue; }

            // Sort radially to prevent "Bow-tie" crossing
            edges_at_node.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

            let mut poly_3d = Vec::new();

            for i in 0..edges_at_node.len() {
                let (_, curr_left, curr_right) = edges_at_node[i];
                let next_idx = (i + 1) % edges_at_node.len();
                let (_, next_left, next_right) = edges_at_node[next_idx];

                // Road Slice: Center -> Right -> Left
                self.push_validated_triangle(&mut poly_3d, center, curr_right, curr_left);

                // Gap Slice: Center -> Left -> Next-Right (only if not a straight 180 line)
                if curr_left.distance_to(next_right) > 0.1 {
                    self.push_validated_triangle(&mut poly_3d, center, curr_left, next_right);
                }
            }
            self.junction_polygons.insert(node_id, poly_3d);
        }
    }

    /// Auto-Heal function: Swaps points if winding is wrong (Black Triangle Fix)
    fn push_validated_triangle(&self, poly: &mut Vec<Vector3>, p0: Vector3, mut p1: Vector3, mut p2: Vector3) {
        let normal = (p1 - p0).cross(p2 - p0);
        // If Normal points down, swap vertices to flip it up towards camera
        if normal.y < 0.0 {
            std::mem::swap(&mut p1, &mut p2);
        }
        // Only add if triangle has real surface area
        if normal.length() > 0.0001 {
            poly.push(p0);
            poly.push(p1);
            poly.push(p2);
        }
    }
}

