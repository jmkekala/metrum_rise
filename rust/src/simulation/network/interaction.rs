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

fn is_live_canonical_node(graph: &RegionGraph, node_id: u32) -> bool {
    is_canonical_node(graph, node_id) && graph.node_has_live_incident_edge(node_id)
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
    get_closest_point_impl(graph, world_pos, max_dist, false)
}

/// Finds the closest network point using only XZ distance for snap eligibility and scoring.
pub(crate) fn get_closest_point_xz(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
) -> Option<Vector3> {
    get_closest_point_impl(graph, world_pos, max_dist, true)
}

fn get_closest_point_impl(
    graph: &RegionGraph,
    world_pos: Vector3,
    max_dist: f32,
    xz_only: bool,
) -> Option<Vector3> {
    let mut closest_pos = None;
    let mut min_score = f32::MAX;

    // 1. Check nodes first (Higher priority/Sticky)
    let node_snap_dist = max_dist * 2.5;
    let mut closest_node_dist = f32::MAX;

    for node_id in nearby_node_ids(graph, world_pos, node_snap_dist) {
        if !is_live_canonical_node(graph, node_id) {
            continue;
        }
        let node = &graph.nodes()[node_id as usize];
        let d = snap_distance(node.pos, world_pos, xz_only);
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

            let Some(pos) = get_edge_snap_point_for_mode(world_pos, p0, p1, xz_only) else {
                continue;
            };
            let d_perp = snap_distance(pos, world_pos, xz_only);

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

fn snap_distance(a: Vector3, b: Vector3, xz_only: bool) -> f32 {
    if xz_only {
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        (dx * dx + dz * dz).sqrt()
    } else {
        a.distance_to(b)
    }
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

    for node_id in nearby_node_ids(graph, world_pos, max_dist) {
        if !is_live_canonical_node(graph, node_id) {
            continue;
        }
        let node = &graph.nodes()[node_id as usize];
        let d_sq = node.pos.distance_squared_to(world_pos);
        if d_sq < min_dist_sq {
            min_dist_sq = d_sq;
            closest_node = Some(node_id);
        }
    }
    closest_node
}

fn nearby_node_ids(
    graph: &RegionGraph,
    world_pos: Vector3,
    radius: f32,
) -> impl Iterator<Item = u32> + '_ {
    let min = Vector3::new(world_pos.x - radius, 0.0, world_pos.z - radius);
    let max = Vector3::new(world_pos.x + radius, 0.0, world_pos.z + radius);
    let min_chunk = RegionGraph::get_node_chunk_coords(min);
    let max_chunk = RegionGraph::get_node_chunk_coords(max);
    (min_chunk.0..=max_chunk.0)
        .flat_map(move |chunk_x| {
            (min_chunk.1..=max_chunk.1)
                .filter_map(move |chunk_z| graph.spatial_node_grid.get(&(chunk_x, chunk_z)))
        })
        .flatten()
        .copied()
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

fn get_edge_snap_point_for_mode(
    p: Vector3,
    a: Vector3,
    b: Vector3,
    xz_only: bool,
) -> Option<Vector3> {
    if !xz_only {
        return get_edge_snap_point(p, a, b);
    }

    let dx = b.x - a.x;
    let dz = b.z - a.z;
    let length_sq = dx * dx + dz * dz;
    if length_sq <= 0.000001 {
        return None;
    }

    let seg_len = length_sq.sqrt();
    let end_margin = (EDGE_SNAP_ENDPOINT_MARGIN_M / seg_len).min(0.49);
    let t = (((p.x - a.x) * dx + (p.z - a.z) * dz) / length_sq).clamp(end_margin, 1.0 - end_margin);
    Some(a + (b - a) * t)
}

/// Finds the intersection point of two 2D segments in the XZ plane.
///
/// Returns `Some((t_a, t_b))` if they intersect, where `t` is the distance along the segment in `[0, 1]`.
pub fn find_intersection_2d(
    p1: Vector3,
    p2: Vector3,
    p3: Vector3,
    p4: Vector3,
) -> Option<(f32, f32)> {
    fn cross_xz(a: Vector3, b: Vector3) -> f32 {
        a.x * b.z - a.z * b.x
    }

    let r = p2 - p1;
    let s = p4 - p3;
    let denom = cross_xz(r, s);
    if denom.abs() < 0.0001 {
        return None;
    }

    let qp = p3 - p1;
    let t = cross_xz(qp, s) / denom;
    let u = cross_xz(qp, r) / denom;

    const PARAM_EPSILON: f32 = 0.00001;
    if t >= -PARAM_EPSILON
        && t <= 1.0 + PARAM_EPSILON
        && u >= -PARAM_EPSILON
        && u <= 1.0 + PARAM_EPSILON
    {
        Some((t.clamp(0.0, 1.0), u.clamp(0.0, 1.0)))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{
        find_intersection_2d, get_closest_point, get_closest_point_xz, get_edge_snap_point,
        get_edge_snap_point_for_mode,
    };
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
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
        let far = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        graph.add_edge(test_edge(keep, far));
        graph.unite_nodes(keep, remove);

        let snapped = get_closest_point(&graph, Vector3::new(0.8, 0.0, 0.0), 5.0).unwrap();
        assert!(snapped.distance_to(Vector3::ZERO) <= 0.001);
    }

    #[test]
    fn xz_closest_point_ignores_height_delta_for_editor_snap() {
        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
        graph.add_edge(test_edge(start, end));

        assert!(get_closest_point(&graph, Vector3::new(0.1, 20.0, 0.1), 5.0).is_none());
        let snapped = get_closest_point_xz(&graph, Vector3::new(0.1, 20.0, 0.1), 5.0).unwrap();
        assert!(snapped.distance_to(Vector3::ZERO) <= 0.001);
    }

    #[test]
    fn xz_edge_snap_projects_along_horizontal_footprint() {
        let snapped = get_edge_snap_point_for_mode(
            Vector3::new(8.0, 30.0, 0.0),
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 10.0, 0.0),
            true,
        )
        .unwrap();

        assert!((snapped.x - 8.0).abs() < 0.001);
        assert!((snapped.y - 8.0).abs() < 0.001);
        assert!(snapped.z.abs() < 0.001);
    }

    fn test_edge(start_node: u32, end_node: u32) -> Edge {
        let start = Vector3::ZERO;
        let end = Vector3::new(0.0, 0.0, 20.0);
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 20.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![start, end],
            physical_geometry: vec![start, end],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        }
    }

    #[test]
    fn segment_intersection_returns_parameters_on_both_segments() {
        let (t, u) = find_intersection_2d(
            Vector3::new(0.0, 0.0, -100.0),
            Vector3::new(0.0, 0.0, 100.0),
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        )
        .expect("perpendicular centerlines should intersect");

        assert!((t - 0.5).abs() < 0.0001);
        assert!((u - 0.5).abs() < 0.0001);
    }
}
