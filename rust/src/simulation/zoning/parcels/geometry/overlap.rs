//! Parcel rectangle and stroke overlap tests.

use super::bounds::geometry_for_parcel;
use super::spatial::chunks_for_aabb;
use crate::simulation::zoning::parcels::{OVERLAP_EPSILON_M, ParcelGeometry, ZoningParcel};
use godot::prelude::Vector2;
use std::collections::{HashMap, HashSet};

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

pub(crate) fn point_inside_parcel(point: Vector2, parcel: &ZoningParcel) -> bool {
    let rel = point - parcel.center();
    let along = rel.dot(parcel.tangent());
    let depth = rel.dot(parcel.normal());
    along.abs() <= parcel.frontage_m() * 0.5 + OVERLAP_EPSILON_M
        && depth.abs() <= parcel.depth_m() * 0.5 + OVERLAP_EPSILON_M
}

pub(crate) fn segment_touches_parcel(start: Vector2, end: Vector2, parcel: &ZoningParcel) -> bool {
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

pub(crate) fn rectangles_overlap_geometry(
    geometry: &ParcelGeometry,
    parcel: &ZoningParcel,
) -> bool {
    let parcel_geometry = geometry_for_parcel(parcel);
    geometries_overlap(geometry, &parcel_geometry)
}

pub(super) fn rectangles_overlap_on_axes(
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
