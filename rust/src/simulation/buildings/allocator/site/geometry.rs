// SPDX-License-Identifier: GPL-2.0-only

//! Polygon, hull, bounds, and plane-intersection primitives.

use super::model::BuildingSiteClient;
use godot::prelude::{Vector2, Vector3};

pub(super) const SITE_POINT_EPS_M: f32 = 0.001;
pub(super) const SITE_POINT_EPS_SQUARED_M2: f32 = SITE_POINT_EPS_M * SITE_POINT_EPS_M;

pub(super) fn convex_hull_from_sorted_points(points: Vec<Vector2>) -> Vec<Vector2> {
    if points.len() <= 3 {
        return points;
    }
    let mut lower = Vec::new();
    for point in points.iter().copied() {
        while lower.len() >= 2
            && local_cross(lower[lower.len() - 2], lower[lower.len() - 1], point)
                <= SITE_POINT_EPS_SQUARED_M2
        {
            lower.pop();
        }
        lower.push(point);
    }
    let mut upper = Vec::new();
    for point in points.iter().rev().copied() {
        while upper.len() >= 2
            && local_cross(upper[upper.len() - 2], upper[upper.len() - 1], point)
                <= SITE_POINT_EPS_SQUARED_M2
        {
            upper.pop();
        }
        upper.push(point);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

pub(super) fn local_cross(a: Vector2, b: Vector2, c: Vector2) -> f32 {
    let ab = b - a;
    let ac = c - a;
    ab.x * ac.y - ab.y * ac.x
}

pub(super) fn signed_polygon_area(points: &[Vector2]) -> f32 {
    let mut area = 0.0;
    for idx in 0..points.len() {
        let start = points[idx];
        let end = points[(idx + 1) % points.len()];
        area += start.x * end.y - end.x * start.y;
    }
    area * 0.5
}

pub(super) fn polygon_quad_bounds(points: [Vector2; 4]) -> (f32, f32, f32, f32) {
    polygon_slice_bounds(&points)
}

pub(super) fn polygon_slice_bounds(points: &[Vector2]) -> (f32, f32, f32, f32) {
    let mut min_x = f32::INFINITY;
    let mut min_z = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_z = f32::NEG_INFINITY;
    for &point in points {
        min_x = min_x.min(point.x);
        min_z = min_z.min(point.y);
        max_x = max_x.max(point.x);
        max_z = max_z.max(point.y);
    }
    (min_x, min_z, max_x, max_z)
}

pub(super) fn site_radius_m(site: &BuildingSiteClient) -> f32 {
    let lot_center = site.lot_footprint_world.iter().copied().sum::<Vector2>()
        / site.lot_footprint_world.len() as f32;
    site.footprint_world
        .iter()
        .map(|point| point.distance_to(lot_center))
        .fold(0.0, f32::max)
}

pub(super) fn update_site_plane_ray_hit(
    best: &mut Option<(f32, Vector3)>,
    ray_origin: Vector3,
    ray_dir: Vector3,
    height_m: f32,
    polygon: &[Vector2],
) {
    if ray_dir.y.abs() <= f32::EPSILON {
        return;
    }
    let t = (height_m - ray_origin.y) / ray_dir.y;
    if t < 0.0 {
        return;
    }
    let hit = ray_origin + ray_dir * t;
    if !point_in_polygon_slice(Vector2::new(hit.x, hit.z), polygon) {
        return;
    }
    if best.as_ref().is_none_or(|(best_t, _)| t < *best_t) {
        *best = Some((t, hit));
    }
}

pub(super) fn point_in_polygon_slice(pos: Vector2, polygon: &[Vector2]) -> bool {
    if polygon.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut prev = polygon[polygon.len() - 1];
    for &current in polygon {
        if point_on_segment(pos, prev, current) {
            return true;
        }
        let crosses = (current.y > pos.y) != (prev.y > pos.y);
        if crosses {
            let t = (pos.y - current.y) / (prev.y - current.y);
            let x_at_y = current.x + (prev.x - current.x) * t;
            if pos.x < x_at_y {
                inside = !inside;
            }
        }
        prev = current;
    }
    inside
}

pub(super) fn point_on_segment(pos: Vector2, a: Vector2, b: Vector2) -> bool {
    let ab = b - a;
    let ap = pos - a;
    let length_sq = ab.length_squared();
    if length_sq <= f32::EPSILON {
        return pos.distance_to(a) <= SITE_POINT_EPS_M;
    }
    let cross = ab.x * ap.y - ab.y * ap.x;
    let projected_tolerance_m2 = SITE_POINT_EPS_M * length_sq.sqrt().max(SITE_POINT_EPS_M);
    if cross.abs() > projected_tolerance_m2 {
        return false;
    }
    let dot = ap.dot(ab);
    dot >= -projected_tolerance_m2 && dot <= length_sq + projected_tolerance_m2
}
