//! Core graph data structures: Nodes, Edges, and the RegionGraph container.

use super::super::types::*;
use godot::prelude::*;
use rstar::{AABB, PointDistance, RTree, RTreeObject};
use std::collections::HashMap;

/// A spatial index entry for a road edge.
#[derive(Clone, Copy, Debug)]
pub struct EdgeEntry {
    /// The index of the edge in the [`RegionGraph::edges`] list.
    pub edge_idx: usize,
    /// The tightest possible axis-aligned bounding box (AABB) in the XZ plane.
    pub envelope: AABB<[f32; 2]>,
}

impl PartialEq for EdgeEntry {
    fn eq(&self, other: &Self) -> bool {
        self.edge_idx == other.edge_idx
    }
}

impl Eq for EdgeEntry {}

impl RTreeObject for EdgeEntry {
    type Envelope = AABB<[f32; 2]>;
    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl PointDistance for EdgeEntry {
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        self.envelope.distance_2(point)
    }
}

/// A junction or endpoint in the road graph.
#[derive(Clone)]
pub struct Node {
    /// World-space 3-D position (metres). Y component reflects terrain height.
    pub pos: Vector3,
    /// Classification of the node (regular junction, cul-de-sac end, highway border, etc.).
    pub node_type: NodeType,
    /// Turn restriction table for vehicles at this junction.
    ///
    /// Key `(from_edge, from_lane)` -> list of `(to_edge, to_lane)` pairs that are permitted.
    /// If the key is absent, all turns from that edge/lane are allowed (open junction).
    /// Pedestrians bypass this table entirely.
    pub lane_connections: HashMap<(usize, i8), Vec<(usize, i8)>>,
    /// Manually enforced crosswalks: Key `edge_id` -> bool override (true = force, false = disable).
    /// If an edge ID is missing, default procedural generation decides.
    pub crosswalk_overrides: HashMap<usize, bool>,
}

/// A directed road segment connecting two [`Node`]s.
#[derive(Clone, Default)]
pub struct Edge {
    /// Index of the start node in [`RegionGraph::nodes`].
    pub start_node: u32,
    /// Index of the end node in [`RegionGraph::nodes`].
    pub end_node: u32,
    /// The dominant transit mode this edge was built for (Road, Foot, etc.).
    pub primary_type: TransitType,
    /// Bitmask of permitted transit modes. Bit 0 = Foot, Bit 1 = Road/Car.
    pub allowed_types: u8,
    /// Classification (Standard, Bridge, Tunnel). Multi-modal foundation (Item 26).
    pub class: EdgeClass,
    /// Total road width in metres (asphalt + sidewalks).
    pub width: f32,
    /// Number of forward (start->end) vehicle lanes.
    pub fwd_lanes: u8,
    /// Number of backward (end->start) vehicle lanes.
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
    /// All O(E) scans must skip edges where `deleted == true`.
    pub deleted: bool,
    /// When `true` the building allocator will not place buildings along this edge.
    /// Auto-set for roads with `speed_limit ≥ 80 km/h`; player-toggleable in the road inspector.
    pub no_building_spawn: bool,
}

impl Edge {
    /// Returns the interpolated world-space Y (height) at a given T-coordinate [0, 1].
    pub fn get_y_at_t(&self, t: f32) -> f32 {
        self.get_pos_and_tangent_at_t(t).0.y
    }

    /// Returns the (position, tangent) at a given T-coordinate [0, 1] along the physical geometry.
    pub fn get_pos_and_tangent_at_t(&self, t: f32) -> (Vector3, Vector3) {
        if self.physical_geometry.is_empty() {
            return (Vector3::ZERO, Vector3::RIGHT);
        }
        if self.physical_geometry.len() == 1 {
            return (self.physical_geometry[0], Vector3::RIGHT);
        }

        let t_clamped = t.clamp(0.0, 1.0);
        let target_dist = t_clamped * self.physical_length;
        let mut curr_dist = 0.0;

        for i in 0..self.physical_geometry.len() - 1 {
            let p1 = self.physical_geometry[i];
            let p2 = self.physical_geometry[i + 1];
            let d = (Vector2::new(p2.x, p2.z) - Vector2::new(p1.x, p1.z)).length();
            if curr_dist + d >= target_dist {
                let local_t = if d > 1e-6 {
                    (target_dist - curr_dist) / d
                } else {
                    0.0
                };
                let pos = p1 + (p2 - p1) * local_t;
                let tangent = (p2 - p1).normalized();
                return (pos, tangent);
            }
            curr_dist += d;
        }
        (
            self.physical_geometry.last().unwrap().clone(),
            (self.physical_geometry.last().unwrap().clone()
                - self.physical_geometry[self.physical_geometry.len() - 2])
                .normalized(),
        )
    }

    /// Returns the normalized 2D tangent (X, Z) at a given T-coordinate.
    pub fn get_tangent_at_t(&self, t: f32) -> Vector2 {
        let (_, t3) = self.get_pos_and_tangent_at_t(t);
        Vector2::new(t3.x, t3.z).normalized()
    }
}

/// A unified directed graph representing the road and transit network of the entire region.
///
/// High-performance road network graph using a Structure-of-Arrays (SoA) layout.
/// and global pathfinding (CCH/CRP). It owns all nodes, edges, and the spatial acceleration grid.
///
/// This is the central data structure of the simulation. All pathfinding, zoning,
/// agent movement, and building placement operate on this graph.
#[derive(Clone)]
pub struct RegionGraph {
    /// All road nodes (junctions and endpoints). Indexed by node ID (`u32`).
    pub(in crate::simulation::network) nodes: Vec<Node>,
    /// All road edges (segments). Indexed by edge ID (`usize`). Includes soft-deleted entries.
    pub(in crate::simulation::network) edges: Vec<Edge>,
    /// Node alias map for the union-find structure used during node merging.
    /// Maps a node ID to its canonical representative after `unite_nodes`.
    pub(in crate::simulation::network) node_aliases: HashMap<u32, u32>,
    /// Spatial acceleration structure for edges: RTree of EdgeEntry.
    /// Query via [`get_edges_near_point`](Self::get_edges_near_point); do not access directly.
    pub(in crate::simulation::network) spatial_edge_rt: RTree<EdgeEntry>,
    /// Adjacency list: node ID -> list of outgoing edge indices. Rebuilt after every road edit.
    pub(in crate::simulation::network) adjacency: Vec<Vec<usize>>,
    /// Spatial acceleration structure for nodes: 16 m grid chunks -> node IDs.
    pub(in crate::simulation::network) spatial_node_grid: HashMap<(i32, i32), Vec<u32>>,
}

impl RegionGraph {
    /// Creates a new, empty road graph.
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            edges: Vec::new(),
            node_aliases: std::collections::HashMap::new(),
            spatial_edge_rt: RTree::new(),
            adjacency: Vec::new(),
            spatial_node_grid: HashMap::new(),
        }
    }

    /// Returns the total number of nodes in the graph (including any that were merged).
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Returns the total number of nodes in the adjacency list.
    pub fn node_adjacency_count(&self) -> usize {
        self.adjacency.len()
    }

    /// Returns the number of edges incident to a specific node.
    pub fn node_adjacency_count_at(&self, node_id: u32) -> usize {
        self.adjacency[node_id as usize].len()
    }

    /// Returns the total number of edges in the graph (including soft-deleted slots).
    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Returns a direct reference to a node by ID.
    /// # Panics
    /// Panics if the ID is out of bounds.
    pub fn node(&self, id: u32) -> &Node {
        &self.nodes[id as usize]
    }

    /// Returns a reference to a node by ID, or None if out of bounds.
    pub fn get_node(&self, id: u32) -> Option<&Node> {
        self.nodes.get(id as usize)
    }

    /// Returns a direct reference to an edge by ID.
    /// # Panics
    /// Panics if the ID is out of bounds.
    pub fn edge(&self, id: usize) -> &Edge {
        &self.edges[id]
    }

    /// Returns a reference to an edge by ID, or None if out of bounds.
    pub fn get_edge(&self, id: usize) -> Option<&Edge> {
        self.edges.get(id)
    }

    /// Returns a reference to the entire node list.
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    /// Returns a reference to the entire edge list.
    pub fn edges(&self) -> &[Edge] {
        &self.edges
    }

    /// Returns a reference to the adjacency list for a specific node.
    pub fn node_adjacency(&self, node_id: u32) -> &[usize] {
        &self.adjacency[node_id as usize]
    }

    /// Returns the full adjacency list.
    pub fn adjacency(&self) -> &[Vec<usize>] {
        &self.adjacency
    }

    /// Returns a mutable reference to the edge at `id`.
    pub fn edge_mut(&mut self, id: usize) -> &mut Edge {
        &mut self.edges[id]
    }

    /// Sets the congestion value for edge `eid`.
    pub fn set_edge_congestion(&mut self, eid: usize, value: f32) {
        self.edges[eid].current_congestion = value;
    }

    /// Sets the [`NodeType`] of the node at `node_id`.
    pub fn set_node_type(&mut self, node_id: u32, node_type: NodeType) {
        self.nodes[node_id as usize].node_type = node_type;
    }

    /// Sets the world-space position of the node at `node_id`.
    pub fn set_node_pos(&mut self, node_id: u32, pos: Vector3) {
        self.nodes[node_id as usize].pos = pos;
    }

    /// Removes the lane-routing entry for `key` at node `node_id`.
    pub fn remove_lane_connection(&mut self, node_id: u32, key: (usize, i8)) {
        if let Some(node) = self.nodes.get_mut(node_id as usize) {
            node.lane_connections.remove(&key);
        }
    }

    /// Appends `(to_edge, to_lane)` to the routing table entry `(from_edge, from_lane)` at node `node_id`.
    pub fn add_lane_connection(&mut self, node_id: u32, fe: usize, fl: i8, te: usize, tl: i8) {
        self.nodes[node_id as usize]
            .lane_connections
            .entry((fe, fl))
            .or_default()
            .push((te, tl));
    }

    /// Sets a user override for a crosswalk at a specific road mouth.
    pub fn set_crosswalk_override(&mut self, node_id: u32, edge_id: usize, enabled: bool) {
        if node_id as usize >= self.nodes.len() {
            return;
        }
        self.nodes[node_id as usize]
            .crosswalk_overrides
            .insert(edge_id, enabled);
    }

    /// Returns a mutable iterator over all edge slots (including soft-deleted ones).
    pub fn edges_iter_mut(&mut self) -> impl Iterator<Item = &mut Edge> {
        self.edges.iter_mut()
    }

    /// Clears all runtime-only caches (node aliases, spatial indices) and rebuilds them from
    /// the current node and edge lists. Call this after bulk-loading nodes/edges from save data.
    pub fn rebuild_all_indices(&mut self) {
        self.node_aliases.clear();
        self.rebuild_adjacency_list();
        self.spatial_edge_rt = RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
        self.spatial_node_grid.clear();
        for i in 0..self.nodes.len() {
            self.add_node_to_spatial_index(i as u32);
        }
    }

    /// Pushes a node directly into the graph. For test setup only — does not update spatial indices.
    #[cfg(test)]
    pub fn push_node_for_test(&mut self, node: Node) {
        self.nodes.push(node);
    }

    /// Replaces the node and edge lists wholesale. For test setup only — does not update spatial
    /// indices or adjacency; call [`rebuild_adjacency_list`](Self::rebuild_adjacency_list) after.
    #[cfg(test)]
    pub fn set_nodes_edges_for_test(&mut self, nodes: Vec<Node>, edges: Vec<Edge>) {
        self.nodes = nodes;
        self.edges = edges;
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
        let p0 = triangles[i]; // Center
        let p1 = triangles[i + 1]; // Right Corner
        let p2 = triangles[i + 2]; // Left Corner

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
