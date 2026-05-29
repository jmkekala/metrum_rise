//! Raised-step segment geometry helpers.

use super::*;

pub(in crate::simulation::network::surface::tests) fn explicit_vertical_step_segment_len_squared_m2(
    segment: super::arrangement::NodeExplicitVerticalStepSegment,
) -> f64 {
    let dx = (segment.end().x_key() - segment.start().x_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (segment.end().z_key() - segment.start().z_key()) as f64
        / super::backend::ROAD_OVERLAY_COORDINATE_SCALE;
    dx * dx + dz * dz
}

pub(in crate::simulation::network::surface::tests) fn test_boundary_edge_contains_edge_at_height(
    boundary_edge: [RoadVec3; 2],
    edge: [RoadVec3; 2],
) -> bool {
    let boundary_start = TestRenderVertexKey::from_point(boundary_edge[0]);
    let boundary_end = TestRenderVertexKey::from_point(boundary_edge[1]);
    let edge_start = TestRenderVertexKey::from_point(edge[0]);
    let edge_end = TestRenderVertexKey::from_point(edge[1]);
    let Some((start_numerator, start_denominator)) =
        test_boundary_segment_parameter_xz(edge_start, boundary_start, boundary_end)
    else {
        return false;
    };
    let Some((end_numerator, end_denominator)) =
        test_boundary_segment_parameter_xz(edge_end, boundary_start, boundary_end)
    else {
        return false;
    };
    let start_tolerance = (start_denominator / 1_000_000).max(1);
    let end_tolerance = (end_denominator / 1_000_000).max(1);
    if start_numerator < -start_tolerance
        || start_numerator > start_denominator + start_tolerance
        || end_numerator < -end_tolerance
        || end_numerator > end_denominator + end_tolerance
    {
        return false;
    }
    (test_interpolated_height_mm(
        boundary_start,
        boundary_end,
        start_numerator,
        start_denominator,
    ) - edge_start.y_mm)
        .abs()
        <= 1
        && (test_interpolated_height_mm(
            boundary_start,
            boundary_end,
            end_numerator,
            end_denominator,
        ) - edge_end.y_mm)
            .abs()
            <= 1
}

pub(in crate::simulation::network::surface::tests) fn test_boundary_segment_parameter_xz(
    point: TestRenderVertexKey,
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
) -> Option<(i128, i128)> {
    let point_key = test_surface_xz_key(point.x_key, point.z_key);
    let start_key = test_surface_xz_key(start.x_key, start.z_key);
    let end_key = test_surface_xz_key(end.x_key, end.z_key);
    if let Some(parameter) = segments::overlay_segment_parameter(point_key, start_key, end_key) {
        return Some((parameter.numerator, parameter.denominator));
    }
    let denominator = segment_denominator((start.x_key, start.z_key), (end.x_key, end.z_key));
    if xz_keys_nearly_equal(point, start) {
        return (denominator > 0).then_some((0, denominator));
    }
    if xz_keys_nearly_equal(point, end) {
        return (denominator > 0).then_some((denominator, denominator));
    }
    if !segments::key_collinear_with_overlay_grid_segment(point_key, start_key, end_key) {
        return None;
    }
    (denominator > 0).then_some((
        segments::segment_parameter_key(start_key, end_key, point_key),
        denominator,
    ))
}

fn xz_keys_nearly_equal(left: TestRenderVertexKey, right: TestRenderVertexKey) -> bool {
    (left.x_key - right.x_key).abs() <= 2 && (left.z_key - right.z_key).abs() <= 2
}

pub(in crate::simulation::network::surface::tests) fn test_interpolated_height_mm(
    start: TestRenderVertexKey,
    end: TestRenderVertexKey,
    numerator: i128,
    denominator: i128,
) -> i64 {
    let value =
        i128::from(start.y_mm) * denominator + i128::from(end.y_mm - start.y_mm) * numerator;
    if value >= 0 {
        ((value + denominator / 2) / denominator) as i64
    } else {
        -(((-value + denominator / 2) / denominator) as i64)
    }
}

pub(in crate::simulation::network::surface::tests) fn test_xz_segments_overlap_with_length(
    a_start: (i64, i64),
    a_end: (i64, i64),
    b_start: (i64, i64),
    b_end: (i64, i64),
) -> bool {
    if a_start == a_end || b_start == b_end {
        return false;
    }
    let a_start_key = test_surface_xz_key(a_start.0, a_start.1);
    let a_end_key = test_surface_xz_key(a_end.0, a_end.1);
    let b_start_key = test_surface_xz_key(b_start.0, b_start.1);
    let b_end_key = test_surface_xz_key(b_end.0, b_end.1);
    let denominator = segment_denominator(a_start, a_end);
    if denominator == 0 {
        return false;
    }
    let mut candidates = Vec::new();
    if segments::key_lies_on_segment(a_start_key, b_start_key, b_end_key) {
        candidates.push(0);
    }
    if segments::key_lies_on_segment(a_end_key, b_start_key, b_end_key) {
        candidates.push(denominator);
    }
    if let Some(parameter) =
        segments::overlay_segment_parameter(b_start_key, a_start_key, a_end_key)
    {
        candidates.push(parameter.numerator);
    }
    if let Some(parameter) = segments::overlay_segment_parameter(b_end_key, a_start_key, a_end_key)
    {
        candidates.push(parameter.numerator);
    }
    candidates.sort_unstable();
    candidates.dedup();
    candidates
        .first()
        .zip(candidates.last())
        .is_some_and(|(start, end)| end > start)
}

fn test_surface_xz_key(x_key: i64, z_key: i64) -> surface_keys::SurfaceXzKey {
    surface_keys::SurfaceXzKey::from_raw_keys(x_key, z_key)
}

fn segment_denominator(start: (i64, i64), end: (i64, i64)) -> i128 {
    let dx = i128::from(end.0 - start.0);
    let dz = i128::from(end.1 - start.1);
    dx * dx + dz * dz
}

pub(in crate::simulation::network::surface::tests) fn polygon_centroid_for_test(
    polygon: &RoadSurfaceVisualPolygon,
) -> Option<RoadVec3> {
    let mut sum = RoadVec3::ZERO;
    let mut count = 0usize;
    for point in &polygon.points_world {
        sum += RoadVec3::new(point.x, 0.0, point.z);
        count += 1;
    }
    (count > 0).then_some(sum / count as f64)
}
