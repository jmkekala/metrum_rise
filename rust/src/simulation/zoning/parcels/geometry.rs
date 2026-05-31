//! Parcel rectangle geometry, overlap tests, and road-corridor conflict checks.

use super::types::{ParcelGeometry, ZoningParcel};
use super::{OVERLAP_EPSILON_M, ROAD_OVERLAP_QUERY_PAD_M};
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

pub(crate) fn geometry_from_attachment(
    graph: &RegionGraph,
    edge_idx: usize,
    side: i8,
    frontage_center_t: f32,
    frontage_m: f32,
    depth_m: f32,
) -> ParcelGeometry {
    let edge = graph.edge(edge_idx);
    let s_m = frontage_center_t.clamp(0.0, 1.0) * edge.physical_length;
    let road_pos = sample_pos_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let tangent = sample_tangent_on_polyline(&edge.physical_geometry, edge.physical_length, s_m);
    let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
    let front_center = road_pos + normal * (edge.width * 0.5 + crate::config::SIDEWALK_WIDTH);
    let center = front_center + normal * (depth_m * 0.5);
    let half_frontage = frontage_m * 0.5;
    let front_left = front_center - tangent * half_frontage;
    let front_right = front_center + tangent * half_frontage;
    let rear_right = front_right + normal * depth_m;
    let rear_left = front_left + normal * depth_m;
    let corners = [front_left, front_right, rear_right, rear_left];
    let (aabb_min, aabb_max) = aabb_for_corners(&corners);
    ParcelGeometry {
        edge_idx,
        side,
        frontage_center_t,
        frontage_m,
        depth_m,
        front_center,
        center,
        tangent,
        normal,
        corners,
        aabb_min,
        aabb_max,
    }
}

pub(crate) fn geometry_inside_world(
    geometry: &ParcelGeometry,
    world_width_m: f32,
    world_height_m: f32,
) -> bool {
    let half_w = world_width_m * 0.5;
    let half_h = world_height_m * 0.5;
    geometry.corners.iter().all(|corner| {
        corner.x >= -half_w && corner.x <= half_w && corner.y >= -half_h && corner.y <= half_h
    })
}

pub(crate) fn geometries_overlap(a: &ParcelGeometry, b: &ParcelGeometry) -> bool {
    let axes = [a.tangent, a.normal, b.tangent, b.normal];
    axes.into_iter().all(|axis| {
        let (a_min, a_max) = project_corners(&a.corners, axis);
        let (b_min, b_max) = project_corners(&b.corners, axis);
        a_max > b_min + OVERLAP_EPSILON_M && b_max > a_min + OVERLAP_EPSILON_M
    })
}

pub(crate) fn geometries_have_overlap(geometries: &[ParcelGeometry]) -> bool {
    let mut chunk_index: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
    let mut visited = HashSet::new();
    for (index, geometry) in geometries.iter().enumerate() {
        let chunks = chunks_for_aabb(geometry.aabb_min, geometry.aabb_max);
        visited.clear();
        for chunk in &chunks {
            let Some(previous_indices) = chunk_index.get(chunk) else {
                continue;
            };
            for &previous_index in previous_indices {
                if !visited.insert(previous_index) {
                    continue;
                }
                if geometries_overlap(&geometries[previous_index], geometry) {
                    return true;
                }
            }
        }
        for chunk in chunks {
            chunk_index.entry(chunk).or_default().push(index);
        }
    }
    false
}

pub(crate) fn geometry_overlaps_road(graph: &RegionGraph, geometry: &ParcelGeometry) -> bool {
    let min = Vector3::new(
        geometry.aabb_min.x - ROAD_OVERLAP_QUERY_PAD_M,
        0.0,
        geometry.aabb_min.y - ROAD_OVERLAP_QUERY_PAD_M,
    );
    let max = Vector3::new(
        geometry.aabb_max.x + ROAD_OVERLAP_QUERY_PAD_M,
        0.0,
        geometry.aabb_max.y + ROAD_OVERLAP_QUERY_PAD_M,
    );
    graph
        .get_edges_near_aabb(min, max)
        .into_iter()
        .any(|edge_idx| {
            edge_idx != geometry.edge_idx && geometry_overlaps_edge(graph, geometry, edge_idx)
        })
}

pub(crate) fn any_geometry_overlaps_road(
    graph: &RegionGraph,
    geometries: &[ParcelGeometry],
) -> bool {
    geometries
        .iter()
        .any(|geometry| geometry_overlaps_road(graph, geometry))
}

pub(crate) fn geometry_for_parcel(parcel: &ZoningParcel) -> ParcelGeometry {
    ParcelGeometry {
        edge_idx: parcel.edge_idx(),
        side: parcel.side(),
        frontage_center_t: parcel.frontage_center_t(),
        frontage_m: parcel.frontage_m(),
        depth_m: parcel.depth_m(),
        front_center: parcel.front_center(),
        center: parcel.center(),
        tangent: parcel.tangent(),
        normal: parcel.normal(),
        corners: parcel.corners(),
        aabb_min: parcel.aabb_min(),
        aabb_max: parcel.aabb_max(),
    }
}

pub(super) fn point_inside_parcel(point: Vector2, parcel: &ZoningParcel) -> bool {
    let rel = point - parcel.center();
    let along = rel.dot(parcel.tangent());
    let depth = rel.dot(parcel.normal());
    along.abs() <= parcel.frontage_m() * 0.5 + OVERLAP_EPSILON_M
        && depth.abs() <= parcel.depth_m() * 0.5 + OVERLAP_EPSILON_M
}

pub(super) fn segment_touches_parcel(start: Vector2, end: Vector2, parcel: &ZoningParcel) -> bool {
    if point_inside_parcel(start, parcel) || point_inside_parcel(end, parcel) {
        return true;
    }
    let corners = parcel.corners();
    for index in 0..4 {
        if segments_intersect(start, end, corners[index], corners[(index + 1) % 4]) {
            return true;
        }
    }
    false
}

pub(super) fn rectangles_overlap_geometry(
    geometry: &ParcelGeometry,
    parcel: &ZoningParcel,
) -> bool {
    let parcel_geometry = geometry_for_parcel(parcel);
    geometries_overlap(geometry, &parcel_geometry)
}

fn geometry_overlaps_edge(graph: &RegionGraph, geometry: &ParcelGeometry, edge_idx: usize) -> bool {
    let edge = graph.edge(edge_idx);
    if edge.deleted || edge.physical_geometry.len() < 2 {
        return false;
    }
    let half_width = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;
    edge.physical_geometry.windows(2).any(|window| {
        parcel_overlaps_road_segment(
            geometry,
            Vector2::new(window[0].x, window[0].z),
            Vector2::new(window[1].x, window[1].z),
            half_width,
        )
    })
}

fn parcel_overlaps_road_segment(
    geometry: &ParcelGeometry,
    start: Vector2,
    end: Vector2,
    half_width: f32,
) -> bool {
    let segment = end - start;
    if segment.length_squared() <= OVERLAP_EPSILON_M * OVERLAP_EPSILON_M {
        return false;
    }
    let tangent = segment.normalized();
    let normal = Vector2::new(tangent.y, -tangent.x);
    let road_corners = [
        start - normal * half_width,
        end - normal * half_width,
        end + normal * half_width,
        start + normal * half_width,
    ];
    rectangles_overlap_on_axes(
        &geometry.corners,
        &road_corners,
        [geometry.tangent, geometry.normal, tangent, normal],
    )
}

fn rectangles_overlap_on_axes(
    a_corners: &[Vector2; 4],
    b_corners: &[Vector2; 4],
    axes: [Vector2; 4],
) -> bool {
    axes.into_iter().all(|axis| {
        let (a_min, a_max) = project_corners(a_corners, axis);
        let (b_min, b_max) = project_corners(b_corners, axis);
        a_max > b_min + OVERLAP_EPSILON_M && b_max > a_min + OVERLAP_EPSILON_M
    })
}

fn segments_intersect(a0: Vector2, a1: Vector2, b0: Vector2, b1: Vector2) -> bool {
    let o1 = cross2(a1 - a0, b0 - a0);
    let o2 = cross2(a1 - a0, b1 - a0);
    let o3 = cross2(b1 - b0, a0 - b0);
    let o4 = cross2(b1 - b0, a1 - b0);

    if ((o1 > OVERLAP_EPSILON_M && o2 < -OVERLAP_EPSILON_M)
        || (o1 < -OVERLAP_EPSILON_M && o2 > OVERLAP_EPSILON_M))
        && ((o3 > OVERLAP_EPSILON_M && o4 < -OVERLAP_EPSILON_M)
            || (o3 < -OVERLAP_EPSILON_M && o4 > OVERLAP_EPSILON_M))
    {
        return true;
    }

    (o1.abs() <= OVERLAP_EPSILON_M && point_on_segment(b0, a0, a1))
        || (o2.abs() <= OVERLAP_EPSILON_M && point_on_segment(b1, a0, a1))
        || (o3.abs() <= OVERLAP_EPSILON_M && point_on_segment(a0, b0, b1))
        || (o4.abs() <= OVERLAP_EPSILON_M && point_on_segment(a1, b0, b1))
}

fn point_on_segment(point: Vector2, start: Vector2, end: Vector2) -> bool {
    point.x >= start.x.min(end.x) - OVERLAP_EPSILON_M
        && point.x <= start.x.max(end.x) + OVERLAP_EPSILON_M
        && point.y >= start.y.min(end.y) - OVERLAP_EPSILON_M
        && point.y <= start.y.max(end.y) + OVERLAP_EPSILON_M
}

fn cross2(a: Vector2, b: Vector2) -> f32 {
    a.x * b.y - a.y * b.x
}

fn project_corners(corners: &[Vector2; 4], axis: Vector2) -> (f32, f32) {
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for corner in corners {
        let projected = corner.dot(axis);
        min = min.min(projected);
        max = max.max(projected);
    }
    (min, max)
}

fn aabb_for_corners(corners: &[Vector2; 4]) -> (Vector2, Vector2) {
    let mut min = Vector2::new(f32::INFINITY, f32::INFINITY);
    let mut max = Vector2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
    for corner in corners {
        min.x = min.x.min(corner.x);
        min.y = min.y.min(corner.y);
        max.x = max.x.max(corner.x);
        max.y = max.y.max(corner.y);
    }
    (min, max)
}

pub(super) fn sample_pos_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.is_empty() {
        return Vector2::ZERO;
    }
    if points.len() == 1 || total_len <= 1e-6 {
        return Vector2::new(points[0].x, points[0].z);
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            let local_t = ((target_s - acc_len) / seg_len).clamp(0.0, 1.0);
            let p0 = Vector2::new(window[0].x, window[0].z);
            let p1 = Vector2::new(window[1].x, window[1].z);
            return p0.lerp(p1, local_t);
        }
        acc_len += seg_len;
    }
    let last = points.last().unwrap();
    Vector2::new(last.x, last.z)
}

fn sample_tangent_on_polyline(points: &[Vector3], total_len: f32, s_m: f32) -> Vector2 {
    if points.len() <= 1 || total_len <= 1e-6 {
        return Vector2::RIGHT;
    }

    let target_s = s_m.clamp(0.0, total_len);
    let mut acc_len = 0.0;
    for window in points.windows(2) {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        let seg_len = window[0].distance_to(window[1]);
        if seg_len <= 1e-6 || seg.length_squared() <= 1e-12 {
            continue;
        }
        if acc_len + seg_len >= target_s {
            return seg.normalized();
        }
        acc_len += seg_len;
    }

    for window in points.windows(2).rev() {
        let seg = Vector2::new(window[1].x - window[0].x, window[1].z - window[0].z);
        if seg.length_squared() > 1e-12 {
            return seg.normalized();
        }
    }
    Vector2::RIGHT
}

pub(super) fn chunk_key(point: Vector2) -> (i32, i32) {
    (
        (point.x / RegionGraph::CHUNK_SIZE).floor() as i32,
        (point.y / RegionGraph::CHUNK_SIZE).floor() as i32,
    )
}

pub(super) fn chunks_for_aabb(min: Vector2, max: Vector2) -> Vec<(i32, i32)> {
    let min_chunk = chunk_key(min);
    let max_chunk = chunk_key(max);
    let mut chunks = Vec::new();
    for cx in min_chunk.0..=max_chunk.0 {
        for cz in min_chunk.1..=max_chunk.1 {
            chunks.push((cx, cz));
        }
    }
    chunks
}
