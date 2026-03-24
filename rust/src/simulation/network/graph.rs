//! Core road-network data structures: [`Node`], [`Edge`], and [`TransitGraph`].
//!
//! The graph is stored as two parallel flat `Vec`s (`nodes`, `edges`) plus several
//! acceleration structures. Node and edge IDs are indices into those vecs.
//!
//! # Spatial indexing
//!
//! `TransitGraph::spatial_edge_grid` maps 512 m chunks to the edge indices whose AABB
//! overlaps that chunk. Use [`TransitGraph::get_edges_near_point`] for all spatial queries —
//! never scan the full `edges` vec.
//!
//! # Soft deletion
//!
//! Edges are never physically removed from the `edges` vec. When an edge is "deleted"
//! `edge.deleted` is set to `true`. All O(E) scans must skip deleted edges. This causes
//! degradation over long sessions (bug B15 in `docs/project.md`).

use godot::prelude::*;
use super::types::*;

use std::collections::HashMap;

/// A junction or endpoint in the road graph.
#[derive(Clone)]
pub struct Node {
    /// World-space 3-D position (metres). Y component reflects terrain height.
    pub pos: Vector3,
    #[allow(dead_code)]
    /// Classification of the node (regular junction, cul-de-sac end, highway border, etc.).
    pub node_type: NodeType,
    /// Turn restriction table for vehicles at this junction.
    ///
    /// Key `(from_edge, from_lane)` → list of `(to_edge, to_lane)` pairs that are permitted.
    /// If the key is absent, all turns from that edge/lane are allowed (open junction).
    /// Pedestrians bypass this table entirely.
    pub lane_connections: HashMap<(usize, i8), Vec<(usize, i8)>>,
}

/// A directed road segment connecting two [`Node`]s.
#[allow(dead_code)]
#[derive(Clone)]
pub struct Edge {
    /// Index of the start node in [`TransitGraph::nodes`].
    pub start_node: u32,
    /// Index of the end node in [`TransitGraph::nodes`].
    pub end_node: u32,
    /// The dominant transit mode this edge was built for (Road, Foot, etc.).
    pub primary_type: TransitType,
    /// Bitmask of permitted transit modes. Bit 0 = Foot, Bit 1 = Road/Car.
    pub allowed_types: u8,
    /// Total road width in metres (asphalt + sidewalks).
    pub width: f32,
    /// Number of forward (start→end) vehicle lanes.
    pub fwd_lanes: u8,
    /// Number of backward (end→start) vehicle lanes.
    pub bkw_lanes: u8,
    /// Design speed in m/s used for pathfinding cost calculation.
    pub speed_limit: f32,
    /// Pre-computed traversal cost (seconds) at `speed_limit` with slope penalty applied.
    /// Updated by [`crate::simulation::pathing::cost::CostCalculator::calculate_costs`].
    pub base_cost: f32,
    /// Arc length of `physical_geometry` in metres.
    pub physical_length: f32,
    /// Dynamic congestion multiplier in `[0, ∞)`. `0.0` = free-flow. Applied on top of `base_cost`.
    pub current_congestion: f32,
    /// Fraction of `geometry` clipped from the start end at junctions (for junction mesh rendering).
    pub start_clip: f32,
    /// Fraction of `geometry` clipped from the end at junctions (for junction mesh rendering).
    pub end_clip: f32,
    /// Unclipped polyline control points (may extend into junction areas), used for zoning placement.
    pub geometry: Vec<Vector3>,
    /// Clipped polyline used for actual road mesh rendering and agent movement.
    pub physical_geometry: Vec<Vector3>,
    /// Whether the left side of this edge (relative to travel direction) is enabled for zoning.
    pub zoning_left: bool,
    /// Whether the right side of this edge is enabled for zoning.
    pub zoning_right: bool,
    /// Soft-deletion flag. `true` = edge is logically removed but still occupies its index.
    /// All O(E) scans must skip edges where `deleted == true`.
    pub deleted: bool,
}


/// Pre-computed mesh data for a road junction polygon, passed to Godot for rendering.
#[derive(Clone)]
pub struct JunctionMesh {
    /// Vertex positions of the junction polygon in world space.
    pub vertices: Vec<Vector3>,
    /// UV texture coordinates, parallel to `vertices`.
    pub uvs: Vec<Vector2>,
    /// Vertex colours used for road marking overlays, parallel to `vertices`.
    pub colors: Vec<Color>,
}

/// The complete road network: nodes, edges, and all acceleration structures.
///
/// This is the central data structure of the simulation. All pathfinding, zoning,
/// agent movement, and building placement operate on this graph.
#[derive(Clone)]
pub struct TransitGraph {
    /// All road nodes (junctions and endpoints). Indexed by node ID (`u32`).
    pub nodes: Vec<Node>,
    /// All road edges (segments). Indexed by edge ID (`usize`). Includes soft-deleted entries.
    pub edges: Vec<Edge>,
    /// Pre-computed junction mesh polygons keyed by the central node ID.
    pub junction_polygons: HashMap<u32, JunctionMesh>,
    /// Node alias map for the union-find structure used during node merging.
    /// Maps a node ID to its canonical representative after `unite_nodes`.
    pub node_aliases: HashMap<u32, u32>,
    /// Spatial acceleration structure: 512 m grid chunks → edge indices whose AABB overlaps the chunk.
    /// Query via [`get_edges_near_point`](Self::get_edges_near_point); do not access directly.
    pub spatial_edge_grid: HashMap<(i32, i32), Vec<usize>>,
    /// Adjacency list: node ID → list of outgoing edge indices. Rebuilt after every road edit.
    pub adjacency: HashMap<u32, Vec<usize>>,
    /// Spatial acceleration structure for nodes: 16 m grid chunks → node IDs.
    pub spatial_node_grid: HashMap<(i32, i32), Vec<u32>>,
}

impl TransitGraph {
    pub const CHUNK_SIZE: f32 = 512.0;
    pub const NODE_CHUNK_SIZE: f32 = 16.0;

    pub fn get_chunk_coords(pos: godot::prelude::Vector3) -> (i32, i32) {
        ((pos.x / Self::CHUNK_SIZE).floor() as i32, (pos.z / Self::CHUNK_SIZE).floor() as i32)
    }

    pub fn get_node_chunk_coords(pos: godot::prelude::Vector3) -> (i32, i32) {
        ((pos.x / Self::NODE_CHUNK_SIZE).floor() as i32, (pos.z / Self::NODE_CHUNK_SIZE).floor() as i32)
    }

    pub fn get_node_chunk(&self, node_id: u32) -> (i32, i32) {
        Self::get_chunk_coords(self.nodes[node_id as usize].pos)
    }

    pub fn add_to_spatial_index(&mut self, edge_idx: usize) {
        let edge = &self.edges[edge_idx];
        if edge.deleted { return; }
        
        // Find all chunks touched by this edge's AABB
        let mut min_x = f32::MAX; let mut max_x = f32::MIN;
        let mut min_z = f32::MAX; let mut max_z = f32::MIN;
        
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x); max_x = max_x.max(p.x);
            min_z = min_z.min(p.z); max_z = max_z.max(p.z);
        }
        
        let min_c = Self::get_chunk_coords(godot::prelude::Vector3::new(min_x, 0.0, min_z));
        let max_c = Self::get_chunk_coords(godot::prelude::Vector3::new(max_x, 0.0, max_z));
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                let chunk = self.spatial_edge_grid.entry((cx, cz)).or_default();
                if !chunk.contains(&edge_idx) {
                    chunk.push(edge_idx);
                }
            }
        }
    }

    pub fn remove_from_spatial_index(&mut self, edge_idx: usize) {
        let chunks = self.get_edge_chunks(edge_idx);
        for coords in chunks {
            if let Some(chunk) = self.spatial_edge_grid.get_mut(&coords) {
                chunk.retain(|&idx| idx != edge_idx);
            }
        }
    }

    pub fn add_node_to_spatial_index(&mut self, node_id: u32) {
        let pos = self.nodes[node_id as usize].pos;
        let chunk_coords = Self::get_node_chunk_coords(pos);
        let chunk = self.spatial_node_grid.entry(chunk_coords).or_default();
        if !chunk.contains(&node_id) {
            chunk.push(node_id);
        }
    }

    pub fn remove_node_from_spatial_index(&mut self, node_id: u32, pos: Vector3) {
        let chunk_coords = Self::get_node_chunk_coords(pos);
        if let Some(chunk) = self.spatial_node_grid.get_mut(&chunk_coords) {
            chunk.retain(|&id| id != node_id);
        }
    }

    pub fn get_edges_near_point(&self, pos: godot::prelude::Vector3, radius: f32) -> Vec<usize> {
        let min = godot::prelude::Vector3::new(pos.x - radius, 0.0, pos.z - radius);
        let max = godot::prelude::Vector3::new(pos.x + radius, 0.0, pos.z + radius);
        self.get_edges_near_aabb(min, max)
    }

    pub fn get_edges_near_aabb(&self, min: godot::prelude::Vector3, max: godot::prelude::Vector3) -> Vec<usize> {
        let mut result = Vec::new();
        let min_c = Self::get_chunk_coords(min);
        let max_c = Self::get_chunk_coords(max);
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                if let Some(chunk) = self.spatial_edge_grid.get(&(cx, cz)) {
                    for &idx in chunk {
                        if !result.contains(&idx) {
                            result.push(idx);
                        }
                    }
                }
            }
        }
        result
    }

    /// Returns the coordinates of all chunks (512m) that an edge's AABB overlaps.
    pub fn get_edge_chunks(&self, edge_idx: usize) -> Vec<(i32, i32)> {
        let mut result = Vec::new();
        if edge_idx >= self.edges.len() { return result; }
        let edge = &self.edges[edge_idx];
        if edge.deleted { return result; }
        
        let mut min_x = f32::MAX; let mut max_x = f32::MIN;
        let mut min_z = f32::MAX; let mut max_z = f32::MIN;
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x); max_x = max_x.max(p.x);
            min_z = min_z.min(p.z); max_z = max_z.max(p.z);
        }
        
        let min_c = Self::get_chunk_coords(godot::prelude::Vector3::new(min_x, 0.0, min_z));
        let max_c = Self::get_chunk_coords(godot::prelude::Vector3::new(max_x, 0.0, max_z));
        
        for cx in min_c.0..=max_c.0 {
            for cz in min_c.1..=max_c.1 {
                result.push((cx, cz));
            }
        }
        result
    }

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

    pub fn rebuild_adjacency_list(&mut self) {
        self.adjacency.clear();
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted { continue; }
            self.adjacency.entry(e.start_node).or_default().push(i);
            self.adjacency.entry(e.end_node).or_default().push(i);
        }
    }

    /// Removes all edges marked as `deleted` and remaps all internal indices.
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
        self.spatial_edge_grid.clear();
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

    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            junction_polygons: std::collections::HashMap::new(),
            node_aliases: std::collections::HashMap::new(),
            spatial_edge_grid: HashMap::new(),
            adjacency: HashMap::new(),
            spatial_node_grid: HashMap::new(),
        }
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
                            if node_type == NodeType::Junction && self.nodes[id as usize].node_type == NodeType::Frontage {
                                self.nodes[id as usize].node_type = NodeType::Junction;
                                self.nodes[id as usize].lane_connections.clear();
                            }
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
        if self.nodes[keep as usize].node_type == NodeType::Frontage {
            self.nodes[keep as usize].node_type = NodeType::Junction;
            self.nodes[keep as usize].lane_connections.clear();
        }

        // (Cul-de-sac logic removed)
        
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
        
        // Note: We don't remove the node from the Vec to keep indices stable
        // The DSU in get_island_count will naturally see them as united now
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
        
        // Re-index all affected edges (rebuild_intersection_clips will also clear everything, but we do it for consistency if that changes)
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
            if edge.deleted { continue; }
            unite(edge.start_node as usize, edge.end_node as usize, &mut parent);
        }
        
        // Count unique roots (only for nodes that are part of an edge to avoid counting "floating" preview nodes)
        let mut active_nodes = std::collections::HashSet::new();
        for edge in &self.edges {
            if edge.deleted { continue; }
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
            if edge.deleted { continue; }
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
            *connection_counts.entry(self.get_valid_node(edge.start_node)).or_insert(0) += 1;
            *connection_counts.entry(self.get_valid_node(edge.end_node)).or_insert(0) += 1;
        }

        // Calculate maximum road width and width difference for junction detection
        let mut node_max_width = HashMap::new();
        let mut node_min_width = HashMap::new();
        for edge in &self.edges {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }
            let w = edge.width;
            
            let s_node = self.get_valid_node(edge.start_node);
            let s_max = node_max_width.entry(s_node).or_insert(0.0);
            if w > *s_max { *s_max = w; }
            let s_min = node_min_width.entry(s_node).or_insert(f32::MAX);
            if w < *s_min { *s_min = w; }

            let e_node = self.get_valid_node(edge.end_node);
            let e_max = node_max_width.entry(e_node).or_insert(0.0);
            if w > *e_max { *e_max = w; }
            let e_min = node_min_width.entry(e_node).or_insert(f32::MAX);
            if w < *e_min { *e_min = w; }
        }


        let valid_node_ids: Vec<u32> = (0..self.nodes.len()).map(|i| self.get_valid_node(i as u32)).collect();

        self.junction_polygons.clear(); // Removing procedural intersection meshes

        for (_edge_id, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road { continue; }

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

            let s_node = &self.nodes[s_valid as usize];
            let e_node = &self.nodes[e_valid as usize];

            edge.start_clip = if (s_conn >= 3 || s_different) && s_node.node_type == NodeType::Junction { s_clip } 
                              else { 0.0 };

            edge.end_clip = if (e_conn >= 3 || e_different) && e_node.node_type == NodeType::Junction { e_clip } 
                            else { 0.0 };

            let count = edge.geometry.len();
            if count >= 2 {
                let mut total_length = 0.0;
                for i in 0..count - 1 {
                    let p0 = edge.geometry[i];
                    let p1 = edge.geometry[i + 1];
                    total_length += (p1 - p0).length();
                }
                
                let num_segments = f32::max(1.0, f32::ceil(total_length / 2.0)) as usize;
                let mut resampled = Vec::new();
                
                for i in 0..=num_segments {
                    let dist = (i as f32 / num_segments as f32) * total_length;
                    let mut curr = 0.0;
                    let mut found = false;
                    for j in 0..count - 1 {
                        let p0 = edge.geometry[j];
                        let p1 = edge.geometry[j + 1];
                        let d = (p1 - p0).length();
                        if curr + d >= dist || (i == num_segments && j == count - 2) {
                            let t = if d > 1e-5 { (dist - curr) / d } else { 0.0 };
                            resampled.push(p0.lerp(p1, t.clamp(0.0, 1.0)));
                            found = true;
                            break;
                        }
                        curr += d;
                    }
                    if !found && !edge.geometry.is_empty() {
                         resampled.push(edge.geometry[count - 1]);
                    }
                }
                if !resampled.is_empty() {
                    let start_node_y = self.nodes[edge.start_node as usize].pos.y;
                    let end_node_y = self.nodes[edge.end_node as usize].pos.y;
                    resampled[0].y = start_node_y;
                    let last_idx = resampled.len() - 1;
                    resampled[last_idx].y = end_node_y;
                }
                edge.physical_geometry = resampled;
                edge.physical_length = total_length;
            } else {
                edge.physical_geometry = edge.geometry.clone();
                edge.physical_length = 0.0;
            }
        }

        // Re-index all roads after a massive batch clip rebuild (e.g. after terrain sync)
        self.spatial_edge_grid.clear();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
        self.rebuild_adjacency_list();
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
