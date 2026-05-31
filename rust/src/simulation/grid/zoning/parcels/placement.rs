//! Road attachment projection and parcel-run placement.

use super::geometry::{geometries_overlap, geometry_from_attachment, sample_pos_on_polyline};
use super::types::{ParcelGeometry, ParcelPlacementError};
use super::{CURVE_RUN_SPACING_BINARY_STEPS, CURVE_RUN_SPACING_SEARCH_STEP_M, OVERLAP_EPSILON_M};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::{Vector2, Vector3};

pub(crate) fn project_default_parcel_at(
    graph: &RegionGraph,
    world_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
) -> Result<ParcelGeometry, ParcelPlacementError> {
    let projected = project_buildable_road_point_at(graph, world_pos, frontage_m, depth_m)?;
    if projected.s_m < frontage_m * 0.5 || projected.s_m > projected.edge_len_m - frontage_m * 0.5 {
        return Err(ParcelPlacementError::FrontageOutOfBounds);
    }

    Ok(geometry_from_attachment(
        graph,
        projected.edge_idx,
        projected.side,
        projected.s_m / projected.edge_len_m,
        frontage_m,
        depth_m,
    ))
}

pub(crate) fn project_parcel_run_at(
    graph: &RegionGraph,
    start_pos: Vector2,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
    let start = project_buildable_road_point_at(graph, start_pos, frontage_m, depth_m)?;
    project_parcel_run_from_projected_start(graph, start, end_pos, frontage_m, depth_m, gap_m)
}

pub(crate) fn project_parcel_run_from_existing(
    graph: &RegionGraph,
    existing: &ParcelGeometry,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
    let edge_idx = existing.edge_idx;
    if edge_idx >= graph.edge_count() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    let edge = graph.edge(edge_idx);
    if edge.deleted
        || edge.no_building_spawn
        || edge.physical_length <= frontage_m
        || edge.physical_geometry.len() < 2
    {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    let s_m = existing.frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
    if s_m < existing.frontage_m * 0.5 || s_m > edge.physical_length - existing.frontage_m * 0.5 {
        return Err(ParcelPlacementError::FrontageOutOfBounds);
    }
    let Some(end) = project_point_to_edge(graph, edge_idx, end_pos) else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };

    let max_centerline_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + depth_m + 8.0;
    if end.dist_m > max_centerline_dist {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }

    let spacing_m = frontage_m + gap_m;
    if spacing_m <= 0.0 || !spacing_m.is_finite() {
        return Err(ParcelPlacementError::InvalidGap);
    }

    let direction = if end.s_m >= s_m { 1.0 } else { -1.0 };
    let first_offset_m = existing.frontage_m * 0.5 + gap_m + frontage_m * 0.5;
    let limit_s = end.s_m;
    let mut center_s = s_m + direction * first_offset_m;
    let mut geometries = Vec::new();

    while directed_s_within_limit(center_s, limit_s, direction) {
        if center_s < frontage_m * 0.5 || center_s > edge.physical_length - frontage_m * 0.5 {
            return Err(ParcelPlacementError::FrontageOutOfBounds);
        }
        let Some((accepted_s, geometry)) = next_non_overlapping_run_geometry_directed(
            graph,
            edge_idx,
            if existing.side >= 0 { 1 } else { -1 },
            center_s,
            limit_s,
            direction,
            edge.physical_length,
            frontage_m,
            depth_m,
            &geometries,
            Some(existing),
        ) else {
            break;
        };
        geometries.push(geometry);
        center_s = accepted_s + direction * spacing_m;
    }

    if geometries.is_empty() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    Ok(geometries)
}

fn project_parcel_run_from_projected_start(
    graph: &RegionGraph,
    start: ProjectedRoadPoint,
    end_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
    gap_m: f32,
) -> Result<Vec<ParcelGeometry>, ParcelPlacementError> {
    let edge = graph.edge(start.edge_idx);
    let Some(end) = project_point_to_edge(graph, start.edge_idx, end_pos) else {
        return Err(ParcelPlacementError::NoRoadAttachment);
    };

    let max_centerline_dist = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH + depth_m + 8.0;
    if end.dist_m > max_centerline_dist {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }

    let spacing_m = frontage_m + gap_m;
    if spacing_m <= 0.0 || !spacing_m.is_finite() {
        return Err(ParcelPlacementError::InvalidGap);
    }

    let min_s = start.s_m.min(end.s_m);
    let max_s = start.s_m.max(end.s_m);
    let mut center_s = min_s;
    let mut geometries = Vec::new();
    while center_s <= max_s + 0.001 {
        if center_s < frontage_m * 0.5 || center_s > start.edge_len_m - frontage_m * 0.5 {
            return Err(ParcelPlacementError::FrontageOutOfBounds);
        }
        let Some((accepted_s, geometry)) = next_non_overlapping_run_geometry(
            graph,
            start.edge_idx,
            start.side,
            center_s,
            max_s,
            start.edge_len_m,
            frontage_m,
            depth_m,
            &geometries,
        ) else {
            break;
        };
        geometries.push(geometry);
        center_s = accepted_s + spacing_m;
    }

    if geometries.is_empty() {
        return Err(ParcelPlacementError::NoRoadAttachment);
    }
    Ok(geometries)
}

fn next_non_overlapping_run_geometry(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    min_center_s: f32,
    max_center_s: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    previous_geometries: &[ParcelGeometry],
) -> Option<(f32, ParcelGeometry)> {
    next_non_overlapping_run_geometry_directed(
        graph,
        edge_idx,
        side,
        min_center_s,
        max_center_s,
        1.0,
        edge_len_m,
        frontage_m,
        depth_m,
        previous_geometries,
        None,
    )
}

fn next_non_overlapping_run_geometry_directed(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    min_center_s: f32,
    limit_s: f32,
    direction: f32,
    edge_len_m: f32,
    frontage_m: f32,
    depth_m: f32,
    previous_geometries: &[ParcelGeometry],
    blocking_geometry: Option<&ParcelGeometry>,
) -> Option<(f32, ParcelGeometry)> {
    let mut low_s = min_center_s;
    let mut high_s = min_center_s;
    loop {
        if !directed_s_within_limit(high_s, limit_s, direction) {
            return None;
        }
        let geometry = geometry_from_attachment(
            graph,
            edge_idx,
            side,
            high_s / edge_len_m,
            frontage_m,
            depth_m,
        );
        if !geometry_overlaps_previous_or_blocking(
            &geometry,
            previous_geometries,
            blocking_geometry,
        ) {
            break;
        }
        low_s = high_s;
        high_s += direction * CURVE_RUN_SPACING_SEARCH_STEP_M;
    }

    if (high_s - min_center_s).abs() <= OVERLAP_EPSILON_M {
        return Some((
            high_s,
            geometry_from_attachment(
                graph,
                edge_idx,
                side,
                high_s / edge_len_m,
                frontage_m,
                depth_m,
            ),
        ));
    }

    for _ in 0..CURVE_RUN_SPACING_BINARY_STEPS {
        let mid_s = (low_s + high_s) * 0.5;
        let geometry = geometry_from_attachment(
            graph,
            edge_idx,
            side,
            mid_s / edge_len_m,
            frontage_m,
            depth_m,
        );
        if geometry_overlaps_previous_or_blocking(&geometry, previous_geometries, blocking_geometry)
        {
            low_s = mid_s;
        } else {
            high_s = mid_s;
        }
    }

    Some((
        high_s,
        geometry_from_attachment(
            graph,
            edge_idx,
            side,
            high_s / edge_len_m,
            frontage_m,
            depth_m,
        ),
    ))
}

fn directed_s_within_limit(s_m: f32, limit_s: f32, direction: f32) -> bool {
    if direction >= 0.0 {
        s_m <= limit_s + OVERLAP_EPSILON_M
    } else {
        s_m >= limit_s - OVERLAP_EPSILON_M
    }
}

fn geometry_overlaps_previous_or_blocking(
    geometry: &ParcelGeometry,
    previous_geometries: &[ParcelGeometry],
    blocking_geometry: Option<&ParcelGeometry>,
) -> bool {
    blocking_geometry.is_some_and(|blocking| geometries_overlap(blocking, geometry))
        || previous_geometries
            .iter()
            .any(|previous| geometries_overlap(previous, geometry))
}

fn project_buildable_road_point_at(
    graph: &RegionGraph,
    world_pos: Vector2,
    frontage_m: f32,
    depth_m: f32,
) -> Result<ProjectedRoadPoint, ParcelPlacementError> {
    let search_radius = depth_m + frontage_m + 48.0;
    let nearby_edges =
        graph.get_edges_near_point(Vector3::new(world_pos.x, 0.0, world_pos.y), search_radius);
    let mut best: Option<ProjectedRoadPoint> = None;

    for edge_idx in nearby_edges {
        let edge = graph.edge(edge_idx);
        if edge.deleted
            || edge.no_building_spawn
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
struct ProjectedRoadPoint {
    edge_idx: usize,
    side: i8,
    s_m: f32,
    edge_len_m: f32,
    dist_m: f32,
}

fn project_point_to_edge(
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
