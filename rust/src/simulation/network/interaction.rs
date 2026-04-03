//! Spatial interaction and querying utilities for the road network.
//!
//! Provides functions for snapping world positions to nodes and edges,
//! finding the closest entities, and geometrical intersections.

use super::graph::RegionGraph;
use godot::prelude::*;

const EDGE_SNAP_ENDPOINT_MARGIN_M: f32 = 0.25;

fn is_canonical_node(graph: &RegionGraph, node_id: u32) -> bool {
    graph.get_valid_node(node_id) == node_id
}

/// Stores the result of a point projection onto a road segment.
pub struct ProjectionData {
    /// Normalized distance along the segment `[0, 1]`.
    pub t: f32,
    /// Which side of the road the point is on: `1` = Left, `-1` = Right.
    pub side: i8,
    /// Perpendicular distance from the road centerline.
    pub dist_from_road: f32,
}

/// Finds the world-space position on the network closest to `world_pos`.
///
/// Snaps to nodes with higher priority than segments. Returns `None` if
/// no part of the network is within `max_dist`.
pub fn get_closest_point(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
) -> Option<Vector3> {
    let mut closest_pos = None;
    let mut min_score = f32::MAX;

    // 1. Check nodes first (Higher priority/Sticky)
    let node_snap_dist = max_dist * 2.5;
    let mut closest_node_dist = f32::MAX;

    for (i, node) in graph.nodes().iter().enumerate() {
        if !is_canonical_node(graph, i as u32) {
            continue;
        }
        let d = node.pos.distance_to(world_pos);
        if d < node_snap_dist {
            let score = d * 0.4; // Nodes are 2.5x more "attractive" than segments
            if score < min_score {
                min_score = score;
                closest_pos = Some(node.pos);
            }
            if d < closest_node_dist {
                closest_node_dist = d;
            }
        }
    }

    // ABSOLUTE NODE PRIORITY:
    // If the cursor is close to an existing intersection node (within the standard snap tolerance),
    // absolutely lock to it and return early. This prevents the segment distance calculator from
    // mathematically overriding the node with a tiny fractional margin point (e.g. 0.1m away on an attached edge),
    // which results in overlapping collision fragments and chaotic geometric splitting.
    if closest_node_dist <= max_dist {
        return closest_pos;
    }

    // 2. Check nearby edges using Spatial Index
    let nearby_edges = graph.get_edges_near_point(world_pos, max_dist.max(20.0));

    for edge_idx in nearby_edges {
        let edge = graph.edge(edge_idx);
        if edge.deleted {
            continue;
        }

        let half_width = edge.width * 0.5;
        let edge_snap_dist = f32::max(max_dist, half_width + 1.0);

        for i in 0..edge.geometry.len() - 1 {
            let p0 = edge.geometry[i];
            let p1 = edge.geometry[i + 1];

            let Some(pos) = get_edge_snap_point(world_pos, p0, p1) else {
                continue;
            };
            let d_perp = pos.distance_to(world_pos);

            if d_perp < edge_snap_dist {
                let score = d_perp;
                if score < min_score {
                    min_score = score;
                    closest_pos = Some(pos);
                }
            }
        }
    }
    closest_pos
}

/// Finds the index of the edge closest to a given world position.
///
/// Returns the `(edge_index, distance)` if found within `max_dist`.
pub fn find_closest_edge(graph: &RegionGraph, pos: Vector3, max_dist: f32) -> Option<(usize, f32)> {
    let mut closest_edge_idx = None;
    let mut min_dist_sq = max_dist * max_dist;

    // Iterate over all edges to find the closest point on any segment
    for (edge_idx, edge) in graph.edges().iter().enumerate() {
        if edge.deleted {
            continue;
        }

        for i in 0..edge.geometry.len() - 1 {
            let p0 = edge.geometry[i];
            let p1 = edge.geometry[i + 1];

            let closest_point_on_segment = get_closest_point_on_segment(pos, p0, p1);
            let d_sq = closest_point_on_segment.distance_squared_to(pos);

            if d_sq < min_dist_sq {
                min_dist_sq = d_sq;
                closest_edge_idx = Some(edge_idx);
            }
        }
    }

    closest_edge_idx.map(|idx| (idx, min_dist_sq.sqrt()))
}

/// Finds the index of the graph node closest to `world_pos`.
pub fn get_closest_node(graph: &RegionGraph, world_pos: Vector3, max_dist: f32) -> Option<u32> {
    let mut closest_node = None;
    let mut min_dist_sq = max_dist * max_dist;

    for (i, node) in graph.nodes().iter().enumerate() {
        if !is_canonical_node(graph, i as u32) {
            continue;
        }
        let d_sq = node.pos.distance_squared_to(world_pos);
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
            closest_node = Some(i as u32);
        }
    }
    closest_node
}

/// Projects a 3D point onto a line segment defined by two points.
pub fn get_closest_point_on_segment(p: Vector3, a: Vector3, b: Vector3) -> Vector3 {
    let ab = b - a;
    let t = (p - a).dot(ab) / ab.length_squared();
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    a + ab * t
}

fn get_edge_snap_point(p: Vector3, a: Vector3, b: Vector3) -> Option<Vector3> {
    let ab = b - a;
    let length_sq = ab.length_squared();
    if length_sq <= 0.000001 {
        return None;
    }

    let seg_len = length_sq.sqrt();
    let end_margin = (EDGE_SNAP_ENDPOINT_MARGIN_M / seg_len).min(0.49);
    let t = ((p - a).dot(ab) / length_sq).clamp(end_margin, 1.0 - end_margin);
    Some(a + ab * t)
}

/// Finds the intersection point of two 2D segments (XZ plane)
/// Returns (t_a, t_b) if they intersect, where t is the factor along the segment [0, 1]
/// Finds the intersection point of two 2D segments in the XZ plane.
///
/// Returns `Some((t_a, t_b))` if they intersect, where `t` is the distance along the segment in `[0, 1]`.
pub fn find_intersection_2d(
    p1: Vector3,
    p2: Vector3,
    p3: Vector3,
    p4: Vector3,
) -> Option<(f32, f32)> {
    let x1 = p1.x;
    let z1 = p1.z;
    let x2 = p2.x;
    let z2 = p2.z;
    let x3 = p3.x;
    let z3 = p3.z;
    let x4 = p4.x;
    let z4 = p4.z;

    let denom = (x1 - x2) * (z3 - z4) - (z1 - z2) * (x3 - x4);
    if denom.abs() < 0.0001 {
        return None;
    }

    let t = ((x1 - x3) * (z3 - z4) - (z1 - z3) * (x3 - x4)) / denom;
    let u = ((x1 - x2) * (z1 - z3) - (z1 - z2) * (x1 - x3)) / denom;

    if t >= 0.0 && t <= 1.0 && u >= 0.0 && u <= 1.0 {
        Some((t, u))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{get_closest_point, get_edge_snap_point};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    #[test]
    fn edge_snap_uses_exact_projection_instead_of_quantized_steps() {
        let snapped = get_edge_snap_point(
            Vector3::new(3.7, 0.0, 1.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
        )
        .unwrap();

        assert!((snapped.x - 3.7).abs() < 0.001);
        assert!(snapped.z.abs() < 0.001);
    }

    #[test]
    fn closest_point_ignores_aliased_merged_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let remove = graph.add_node(Vector3::new(1.0, 0.0, 0.0), NodeType::Junction);
        graph.unite_nodes(keep, remove);

        let snapped = get_closest_point(&graph, Vector3::new(0.8, 0.0, 0.0), 5.0).unwrap();
        assert!(snapped.distance_to(Vector3::ZERO) <= 0.001);
    }
}
