//! Deterministic polygon clipping and patch-boundary geometry.

use super::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TerrainCdtLoopBounds {
    pub(super) min_x: f64,
    pub(super) min_z: f64,
    pub(super) max_x: f64,
    pub(super) max_z: f64,
}

pub(super) fn simplified_road_loop(
    points: Vec<TerrainCdtVertex>,
) -> Result<Vec<TerrainCdtVertex>, TerrainCdtError> {
    let mut deduplicated = Vec::with_capacity(points.len());
    for point in points {
        if let Some(last) = deduplicated.last() {
            let last: &TerrainCdtVertex = last;
            if same_xz(*last, point) {
                if !same_height(last.height_m, point.height_m) {
                    return Err(TerrainCdtError::ConflictingRoadBoundaryHeight);
                }
                continue;
            }
        }
        deduplicated.push(point);
    }
    if deduplicated.len() > 1 {
        let first = deduplicated[0];
        let last = deduplicated[deduplicated.len() - 1];
        if same_xz(first, last) {
            if !same_height(first.height_m, last.height_m) {
                return Err(TerrainCdtError::ConflictingRoadBoundaryHeight);
            }
            deduplicated.pop();
        }
    }
    Ok(deduplicated)
}

pub(super) fn clip_loop_to_patch(
    points: Vec<TerrainCdtVertex>,
    patch: TerrainCdtPatch,
) -> Vec<TerrainCdtVertex> {
    let points = clip_loop_to_boundary(
        points,
        |point| point.x >= patch.min_x - CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.min_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.x <= patch.max_x + CDT_EPSILON_M,
        |a, b| intersect_at_x(a, b, patch.max_x),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z >= patch.min_z - CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.min_z),
    );
    let points = clip_loop_to_boundary(
        points,
        |point| point.z <= patch.max_z + CDT_EPSILON_M,
        |a, b| intersect_at_z(a, b, patch.max_z),
    );
    points
        .into_iter()
        .map(|point| clamp_to_patch(point, patch))
        .collect()
}

fn clip_loop_to_boundary(
    points: Vec<TerrainCdtVertex>,
    inside: impl Fn(TerrainCdtVertex) -> bool,
    intersection: impl Fn(TerrainCdtVertex, TerrainCdtVertex) -> TerrainCdtVertex,
) -> Vec<TerrainCdtVertex> {
    if points.is_empty() {
        return points;
    }

    let mut clipped = Vec::new();
    let Some(mut previous) = points.last().copied() else {
        return Vec::new();
    };
    let mut previous_inside = inside(previous);
    for current in points {
        let current_inside = inside(current);
        if current_inside {
            if !previous_inside {
                clipped.push(intersection(previous, current));
            }
            clipped.push(current);
        } else if previous_inside {
            clipped.push(intersection(previous, current));
        }
        previous = current;
        previous_inside = current_inside;
    }
    clipped
}

fn intersect_at_x(a: TerrainCdtVertex, b: TerrainCdtVertex, x: f64) -> TerrainCdtVertex {
    let denominator = b.x - a.x;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(x, a.height_m, a.z);
    }
    interpolate_vertex(a, b, (x - a.x) / denominator)
}

fn intersect_at_z(a: TerrainCdtVertex, b: TerrainCdtVertex, z: f64) -> TerrainCdtVertex {
    let denominator = b.z - a.z;
    if denominator.abs() <= CDT_EPSILON_M {
        return TerrainCdtVertex::new(a.x, a.height_m, z);
    }
    interpolate_vertex(a, b, (z - a.z) / denominator)
}

pub(super) fn interpolate_vertex(
    a: TerrainCdtVertex,
    b: TerrainCdtVertex,
    t: f64,
) -> TerrainCdtVertex {
    let t = t.clamp(0.0, 1.0);
    TerrainCdtVertex::new(
        a.x + (b.x - a.x) * t,
        (f64::from(a.height_m) + f64::from(b.height_m - a.height_m) * t) as f32,
        a.z + (b.z - a.z) * t,
    )
}

fn clamp_to_patch(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> TerrainCdtVertex {
    TerrainCdtVertex::new(
        vertex.x.clamp(patch.min_x, patch.max_x),
        vertex.height_m,
        vertex.z.clamp(patch.min_z, patch.max_z),
    )
}

pub(super) fn patch_contains(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> bool {
    vertex.x >= patch.min_x - CDT_EPSILON_M
        && vertex.x <= patch.max_x + CDT_EPSILON_M
        && vertex.z >= patch.min_z - CDT_EPSILON_M
        && vertex.z <= patch.max_z + CDT_EPSILON_M
}

pub(super) fn ensure_ccw(mut points: Vec<TerrainCdtVertex>) -> Vec<TerrainCdtVertex> {
    if signed_area(&points) < 0.0 {
        points.reverse();
    }
    points
}

pub(super) fn same_xz(a: TerrainCdtVertex, b: TerrainCdtVertex) -> bool {
    quantized_coord(a.x) == quantized_coord(b.x) && quantized_coord(a.z) == quantized_coord(b.z)
}

pub(super) fn same_coord(a: f64, b: f64) -> bool {
    quantized_coord(a) == quantized_coord(b)
}

pub(super) fn same_height(a: f32, b: f32) -> bool {
    quantized_coord(f64::from(a)) == quantized_coord(f64::from(b))
}

pub(super) fn shared_road_constraint_height(a: f32, b: f32) -> Option<f32> {
    same_height(a, b).then_some(a)
}

pub(super) fn quantized_coord(value: f64) -> i64 {
    (value / CDT_EPSILON_M).round() as i64
}

pub(super) fn signed_area(points: &[TerrainCdtVertex]) -> f64 {
    let mut area = 0.0;
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        area += points[index].x * points[next].z - points[next].x * points[index].z;
    }
    area * 0.5
}
