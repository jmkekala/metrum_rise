//! Deterministic polygon clipping and patch-boundary geometry.

use super::*;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{IntOverlayOptions, Overlay};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;

// Keep overlay topology well below the authoritative 1 mm CDT identity grid. Relative i32
// coordinates at 0.01 mm still cover more than 21 km around one queried loop.
const CDT_OVERLAY_GRID_M: f64 = 0.000_01;

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
    clip_loop_to_patch_single(points, patch)
}

/// Intersects one loop with a rectangle without joining disconnected output components.
pub(super) fn clip_loop_to_patch_components(
    points: &[TerrainCdtVertex],
    patch: TerrainCdtPatch,
) -> Vec<Vec<TerrainCdtVertex>> {
    let Some(origin) = overlay_grid_origin(points, patch) else {
        return Vec::new();
    };
    let Some(subject_contour) = overlay_contour(points, origin) else {
        return Vec::new();
    };
    let Some(clip_contour) = overlay_patch_contour(patch, origin) else {
        return Vec::new();
    };
    let subject = vec![vec![subject_contour]];
    let clip = vec![vec![clip_contour]];
    let mut overlay = Overlay::with_shapes_options(
        &subject,
        &clip,
        IntOverlayOptions {
            preserve_input_collinear: true,
            preserve_output_collinear: true,
            min_output_area: 0,
            ..Default::default()
        },
        Default::default(),
    );
    let legacy_clipped = clip_loop_to_patch_single(points.to_vec(), patch);
    let overlay_shapes = overlay.overlay(OverlayRule::Intersect, FillRule::EvenOdd);
    let legacy_clipped = simplified_road_loop(legacy_clipped).unwrap_or_default();
    if overlay_shapes.len() == 1
        && legacy_clipped.len() >= 3
        && signed_area(&legacy_clipped).abs() > CDT_EPSILON_M * CDT_EPSILON_M
    {
        let vertices = legacy_clipped
            .iter()
            .map(|vertex| {
                clipped_vertex_from_overlay(points, &legacy_clipped, vertex.x, vertex.z, patch)
            })
            .collect();
        let mut vertices = ensure_ccw(vertices);
        rotate_loop_to_canonical_start(&mut vertices);
        return vec![vertices];
    }

    let mut components = overlay_shapes
        .into_iter()
        .filter_map(|shape| shape.into_iter().next())
        .filter_map(|contour| {
            let vertices = contour
                .into_iter()
                .map(|point| {
                    let x = overlay_coord(point.x, origin.0);
                    let z = overlay_coord(point.y, origin.1);
                    clipped_vertex_from_overlay(points, &legacy_clipped, x, z, patch)
                })
                .collect::<Vec<_>>();
            let vertices = simplified_road_loop(vertices).ok()?;
            (vertices.len() >= 3 && signed_area(&vertices).abs() > CDT_EPSILON_M * CDT_EPSILON_M)
                .then_some(ensure_ccw(vertices))
        })
        .collect::<Vec<_>>();
    for component in &mut components {
        rotate_loop_to_canonical_start(component);
    }
    components.sort_by_key(|component| {
        let first = component[0];
        (
            quantized_coord(first.x),
            quantized_coord(first.z),
            component.len(),
            quantized_coord(signed_area(component)),
        )
    });
    components
}

fn clipped_vertex_from_overlay(
    original: &[TerrainCdtVertex],
    legacy_clipped: &[TerrainCdtVertex],
    x: f64,
    z: f64,
    patch: TerrainCdtPatch,
) -> TerrainCdtVertex {
    let quantized = TerrainCdtVertex::new(x, 0.0, z);
    let exact_original = original
        .iter()
        .find(|vertex| same_xz(**vertex, quantized))
        .copied()
        .or_else(|| {
            original
                .iter()
                .filter_map(|vertex| {
                    let dx = vertex.x - x;
                    let dz = vertex.z - z;
                    let distance_squared = dx * dx + dz * dz;
                    (distance_squared <= CDT_EPSILON_M * CDT_EPSILON_M)
                        .then_some((distance_squared, *vertex))
                })
                .min_by(|left, right| {
                    left.0.total_cmp(&right.0).then_with(|| {
                        terrain_cdt_vertex_key(left.1).cmp(&terrain_cdt_vertex_key(right.1))
                    })
                })
                .map(|(_, vertex)| vertex)
        });
    if let Some(vertex) = exact_original {
        return vertex;
    }
    let exact_intersection = legacy_clipped
        .iter()
        .filter(|vertex| segment_height_at_sample(original, **vertex).is_some())
        .filter_map(|vertex| {
            let dx = vertex.x - x;
            let dz = vertex.z - z;
            let distance_squared = dx * dx + dz * dz;
            (same_xz(*vertex, quantized) || distance_squared <= CDT_EPSILON_M * CDT_EPSILON_M)
                .then_some((distance_squared, *vertex))
        })
        .min_by(|left, right| {
            left.0
                .total_cmp(&right.0)
                .then_with(|| terrain_cdt_vertex_key(left.1).cmp(&terrain_cdt_vertex_key(right.1)))
        })
        .map(|(_, vertex)| vertex);
    exact_intersection.unwrap_or_else(|| {
        TerrainCdtVertex::new(x, clipped_vertex_height(original, x, z, patch), z)
    })
}

fn clip_loop_to_patch_single(
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

fn overlay_grid_origin(points: &[TerrainCdtVertex], patch: TerrainCdtPatch) -> Option<(i64, i64)> {
    let min_x = points
        .iter()
        .map(|point| overlay_quantized_coord(point.x))
        .chain([
            overlay_quantized_coord(patch.min_x),
            overlay_quantized_coord(patch.max_x),
        ])
        .min()?;
    let min_z = points
        .iter()
        .map(|point| overlay_quantized_coord(point.z))
        .chain([
            overlay_quantized_coord(patch.min_z),
            overlay_quantized_coord(patch.max_z),
        ])
        .min()?;
    Some((min_x, min_z))
}

fn overlay_contour(points: &[TerrainCdtVertex], origin: (i64, i64)) -> Option<Vec<IntPoint>> {
    let mut contour = Vec::with_capacity(points.len());
    for point in points {
        let overlay_point = overlay_point(point.x, point.z, origin)?;
        if contour.last().is_none_or(|last| *last != overlay_point) {
            contour.push(overlay_point);
        }
    }
    if contour.len() >= 2 && contour.first() == contour.last() {
        contour.pop();
    }
    (contour.len() >= 3).then_some(contour)
}

fn overlay_patch_contour(patch: TerrainCdtPatch, origin: (i64, i64)) -> Option<Vec<IntPoint>> {
    [
        (patch.min_x, patch.min_z),
        (patch.max_x, patch.min_z),
        (patch.max_x, patch.max_z),
        (patch.min_x, patch.max_z),
    ]
    .into_iter()
    .map(|(x, z)| overlay_point(x, z, origin))
    .collect()
}

fn overlay_point(x: f64, z: f64, origin: (i64, i64)) -> Option<IntPoint> {
    Some(IntPoint::new(
        i32::try_from(overlay_quantized_coord(x) - origin.0).ok()?,
        i32::try_from(overlay_quantized_coord(z) - origin.1).ok()?,
    ))
}

fn overlay_coord(value: i32, origin: i64) -> f64 {
    (origin + i64::from(value)) as f64 * CDT_OVERLAY_GRID_M
}

fn overlay_quantized_coord(value: f64) -> i64 {
    (value / CDT_OVERLAY_GRID_M).round() as i64
}

fn clipped_vertex_height(
    original: &[TerrainCdtVertex],
    x: f64,
    z: f64,
    patch: TerrainCdtPatch,
) -> f32 {
    let sample = TerrainCdtVertex::new(x, 0.0, z);
    segment_height_at_sample(original, sample)
        .unwrap_or_else(|| patch_boundary_height(sample, patch))
}

fn segment_height_at_sample(points: &[TerrainCdtVertex], sample: TerrainCdtVertex) -> Option<f32> {
    if points.is_empty() {
        return None;
    }
    (0..points.len()).find_map(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        source_sample_parameter_on_road_constraint(start, end, sample)
            .map(|parameter| interpolate_vertex(start, end, parameter).height_m)
    })
}

fn patch_boundary_height(vertex: TerrainCdtVertex, patch: TerrainCdtPatch) -> f32 {
    if same_coord(vertex.x, patch.min_x) {
        return interpolate_height(
            patch.corner_heights_m[0],
            patch.corner_heights_m[1],
            boundary_parameter(vertex.z, patch.min_z, patch.max_z),
        );
    }
    if same_coord(vertex.z, patch.max_z) {
        return interpolate_height(
            patch.corner_heights_m[1],
            patch.corner_heights_m[2],
            boundary_parameter(vertex.x, patch.min_x, patch.max_x),
        );
    }
    if same_coord(vertex.x, patch.max_x) {
        return interpolate_height(
            patch.corner_heights_m[3],
            patch.corner_heights_m[2],
            boundary_parameter(vertex.z, patch.min_z, patch.max_z),
        );
    }
    interpolate_height(
        patch.corner_heights_m[0],
        patch.corner_heights_m[3],
        boundary_parameter(vertex.x, patch.min_x, patch.max_x),
    )
}

fn boundary_parameter(value: f64, min: f64, max: f64) -> f64 {
    ((value - min) / (max - min)).clamp(0.0, 1.0)
}

fn interpolate_height(start: f32, end: f32, parameter: f64) -> f32 {
    (f64::from(start) + f64::from(end - start) * parameter) as f32
}

pub(super) fn rotate_loop_to_canonical_start(vertices: &mut [TerrainCdtVertex]) {
    if let Some((start_index, _)) = vertices.iter().enumerate().min_by_key(|(_, vertex)| {
        (
            quantized_coord(vertex.x),
            quantized_coord(vertex.z),
            quantized_coord(f64::from(vertex.height_m)),
        )
    }) {
        vertices.rotate_left(start_index);
    }
}

pub(super) fn clip_segment_to_patch(
    start: TerrainCdtVertex,
    end: TerrainCdtVertex,
    patch: TerrainCdtPatch,
) -> Option<(TerrainCdtVertex, TerrainCdtVertex)> {
    let dx = end.x - start.x;
    let dz = end.z - start.z;
    let mut enter = 0.0_f64;
    let mut exit = 1.0_f64;
    for (direction, distance) in [
        (-dx, start.x - patch.min_x),
        (dx, patch.max_x - start.x),
        (-dz, start.z - patch.min_z),
        (dz, patch.max_z - start.z),
    ] {
        if direction.abs() <= f64::EPSILON {
            if distance < -CDT_EPSILON_M {
                return None;
            }
            continue;
        }
        let parameter = distance / direction;
        if direction < 0.0 {
            enter = enter.max(parameter);
        } else {
            exit = exit.min(parameter);
        }
        if enter > exit + f64::EPSILON {
            return None;
        }
    }
    let clipped_start = clamp_to_patch(interpolate_vertex(start, end, enter), patch);
    let clipped_end = clamp_to_patch(interpolate_vertex(start, end, exit), patch);
    (!same_xz(clipped_start, clipped_end)).then_some((clipped_start, clipped_end))
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
