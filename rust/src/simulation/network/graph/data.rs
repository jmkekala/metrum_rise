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
    #[allow(dead_code)]
    /// Classification of the node (regular junction, cul-de-sac end, highway border, etc.).
    pub node_type: NodeType,
    /// Turn restriction table for vehicles at this junction.
    ///
    /// Key `(from_edge, from_lane)` -> list of `(to_edge, to_lane)` pairs that are permitted.
    /// If the key is absent, all turns from that edge/lane are allowed (open junction).
    /// Pedestrians bypass this table entirely.
    pub lane_connections: HashMap<(usize, i8), Vec<(usize, i8)>>,
}

/// A directed road segment connecting two [`Node`]s.
#[allow(dead_code)]
#[derive(Clone)]
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
    /// Whether the left side of this edge (relative to travel direction) is enabled for zoning.
    pub zoning_left: bool,
    /// Whether the right side of this edge is enabled for zoning.
    pub zoning_right: bool,
    /// All O(E) scans must skip edges where `deleted == true`.
    pub deleted: bool,
}

impl Edge {
    /// Returns the interpolated world-space Y (height) at a given T-coordinate [0, 1].
    pub fn get_y_at_t(&self, t: f32) -> f32 {
        if self.physical_geometry.is_empty() {
            return 0.0;
        }
        if self.physical_geometry.len() == 1 {
            return self.physical_geometry[0].y;
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
                return p1.y + (p2.y - p1.y) * local_t;
            }
            curr_dist += d;
        }
        self.physical_geometry.last().unwrap().y
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
    pub nodes: Vec<Node>,
    /// All road edges (segments). Indexed by edge ID (`usize`). Includes soft-deleted entries.
    pub edges: Vec<Edge>,
    /// Node alias map for the union-find structure used during node merging.
    /// Maps a node ID to its canonical representative after `unite_nodes`.
    pub node_aliases: HashMap<u32, u32>,
    /// Spatial acceleration structure for edges: RTree of EdgeEntry.
    /// Query via [`get_edges_near_point`](Self::get_edges_near_point); do not access directly.
    pub spatial_edge_rt: RTree<EdgeEntry>,
    /// Adjacency list: node ID -> list of outgoing edge indices. Rebuilt after every road edit.
    pub adjacency: Vec<Vec<usize>>,
    /// Spatial acceleration structure for nodes: 16 m grid chunks -> node IDs.
    pub spatial_node_grid: HashMap<(i32, i32), Vec<u32>>,
}

impl RegionGraph {
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
