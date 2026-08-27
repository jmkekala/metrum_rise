//! Projection from world points to buildable road frontage positions.

use super::super::geometry::sample_pos_on_polyline;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::ParcelPlacementError;
use godot::prelude::{Vector2, Vector3};

pub(super) fn project_buildable_road_point_at(
    graph: &RegionGraph,
    world_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
) -> Result<ProjectedRoadPoint, ParcelPlacementError> {
    let search_radius = depth_m + frontage_m + 48.0;
    let mut nearby_edges =
        graph.get_edges_near_point(Vector3::new(world_pos.x, 0.0, world_pos.y), search_radius);
    nearby_edges.sort_unstable();
    let mut best: Option<ProjectedRoadPoint> = None;

    for edge_idx in nearby_edges {
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
            || !edge.frontage_class.can_address()
            || edge.physical_length <= frontage_m
            || edge.physical_geometry.len() < 2
        {
            continue;
        }
        let Some(projected) = project_point_to_edge(graph, edge_idx, world_pos) else {
            continue;
        };
        let max_centerline_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + depth_m + 8.0;
        if projected.dist_m > max_centerline_dist {
            continue;
        }
        let better = best
            .as_ref()
            .map(|current| projected.dist_m < current.dist_m)
            .unwrap_or(true);
        if better {
            best = Some(projected);
        }
    }

    let Some(projected) = best else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };
    Ok(projected)
}

#[derive(Clone, Copy)]
pub(super) struct ProjectedRoadPoint {
    pub(super) edge_idx: usize,
    pub(super) side: i8,
    pub(super) s_m: f32,
    pub(super) edge_len_m: f32,
    pub(super) dist_m: f32,
}

pub(super) fn project_point_to_edge(
    graph: &RegionGraph,
    edge_idx: usize,
    point: Vector2,
) -> Option<ProjectedRoadPoint> {
    let edge = graph.edge(edge_idx);
    if edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
        return None;
    }

    let mut best_dist2 = f32::INFINITY;
    let mut best_s = 0.0;
    let mut best_tangent = Vector2::RIGHT;
    let mut accumulated = 0.0;

    for window in edge.physical_geometry.windows(2) {
        let p0 = Vector2::new(window[0].x, window[0].z);
        let p1 = Vector2::new(window[1].x, window[1].z);
        let segment = p1 - p0;
        let len2 = segment.length_squared();
        let seg_len = window[0].distance_to(window[1]);
        if len2 <= 1e-12 || seg_len <= 1e-6 {
            continue;
        }
        let local_t = ((point - p0).dot(segment) / len2).clamp(0.0, 1.0);
        let closest = p0 + segment * local_t;
        let dist2 = (point - closest).length_squared();
        if dist2 < best_dist2 {
            best_dist2 = dist2;
            best_s = accumulated + seg_len * local_t;
            best_tangent = segment.normalized();
        }
        accumulated += seg_len;
    }

    if !best_dist2.is_finite() {
        return None;
    }
    let side_one_normal = Vector2::new(best_tangent.y, -best_tangent.x);
    let side = if (point
        - sample_pos_on_polyline(&edge.physical_geometry, edge.physical_length, best_s))
    .dot(side_one_normal)
        >= 0.0
    {
        1
    } else {
        -1
    };
    Some(ProjectedRoadPoint {
        edge_idx,
        side,
        s_m: best_s,
        edge_len_m: edge.physical_length,
        dist_m: best_dist2.sqrt(),
    })
}
