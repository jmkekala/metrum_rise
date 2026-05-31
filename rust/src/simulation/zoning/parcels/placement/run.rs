//! Same-road parcel-run projection.

mod spacing;

use super::projection::{
    ProjectedRoadPoint, project_buildable_road_point_at, project_point_to_edge,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::parcels::{ParcelGeometry, ParcelPlacementError};
use godot::prelude::Vector2;
use spacing::{
    directed_s_within_limit, next_non_overlapping_run_geometry,
    next_non_overlapping_run_geometry_directed,
};

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
