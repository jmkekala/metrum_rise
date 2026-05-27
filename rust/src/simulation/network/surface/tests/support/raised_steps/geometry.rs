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

pub(in crate::simulation::network::surface::tests) fn polygon_boundary_overlaps_edge_at_height_for_test(
    polygon: &RoadSurfaceVisualPolygon,
    edge: [RoadVec3; 2],
) -> bool {
    if !polygon.triangles_world.is_empty() {
        let mut triangle_edges = BTreeMap::<TestRenderEdgeKey, (usize, [RoadVec3; 2])>::new();
        for triangle in &polygon.triangles_world {
            for edge_index in 0..3 {
                let start = triangle[edge_index];
                let end = triangle[(edge_index + 1) % 3];
                let Some(key) = TestRenderEdgeKey::normalized(start, end) else {
                    continue;
                };
                triangle_edges
                    .entry(key)
                    .and_modify(|entry| entry.0 += 1)
                    .or_insert((1, [start, end]));
            }
        }
        return triangle_edges
            .into_values()
            .filter_map(|(count, boundary_edge)| (count == 1).then_some(boundary_edge))
            .any(|boundary_edge| test_boundary_edge_contains_edge_at_height(boundary_edge, edge));
    }

    let points = &polygon.points_world;
    if points.len() < 2 {
        return false;
    }
    (0..points.len()).any(|index| {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        test_boundary_edge_contains_edge_at_height([start, end], edge)
    })
}

pub(in crate::simulation::network::surface::tests) fn test_boundary_edge_contains_edge_at_height(
    boundary_edge: [RoadVec3; 2],
    edge: [RoadVec3; 2],
) -> bool {
    let boundary_start = TestRenderVertexKey::from_point(boundary_edge[0]);
    let boundary_end = TestRenderVertexKey::from_point(boundary_edge[1]);
    let edge_start = TestRenderVertexKey::from_point(edge[0]);
    let edge_end = TestRenderVertexKey::from_point(edge[1]);
    if !test_xz_segments_overlap_with_length(
        (boundary_start.x_key, boundary_start.z_key),
        (boundary_end.x_key, boundary_end.z_key),
        (edge_start.x_key, edge_start.z_key),
        (edge_end.x_key, edge_end.z_key),
    ) {
        return false;
    }
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
    if start_numerator < 0
        || start_numerator > start_denominator
        || end_numerator < 0
        || end_numerator > end_denominator
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
    let dx = end.x_key - start.x_key;
    let dz = end.z_key - start.z_key;
    let px = point.x_key - start.x_key;
    let pz = point.z_key - start.z_key;
    let length_squared = i128::from(dx) * i128::from(dx) + i128::from(dz) * i128::from(dz);
    if length_squared == 0 || i128::from(dx) * i128::from(pz) - i128::from(dz) * i128::from(px) != 0
    {
        return None;
    }
    Some((
        i128::from(px) * i128::from(dx) + i128::from(pz) * i128::from(dz),
        length_squared,
    ))
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
    let a_dx = i128::from(a_end.0 - a_start.0);
    let a_dz = i128::from(a_end.1 - a_start.1);
    let b_dx = i128::from(b_end.0 - b_start.0);
    let b_dz = i128::from(b_end.1 - b_start.1);
    if a_dx * b_dz - a_dz * b_dx != 0 {
        return false;
    }
    if !test_xz_key_lies_on_segment(a_start, b_start, b_end)
        && !test_xz_key_lies_on_segment(a_end, b_start, b_end)
        && !test_xz_key_lies_on_segment(b_start, a_start, a_end)
        && !test_xz_key_lies_on_segment(b_end, a_start, a_end)
    {
        return false;
    }
    let use_x = (a_end.0 - a_start.0).abs() >= (a_end.1 - a_start.1).abs();
    let coordinate = |key: (i64, i64)| {
        if use_x { key.0 } else { key.1 }
    };
    let a0 = coordinate(a_start);
    let a1 = coordinate(a_end);
    let b0 = coordinate(b_start);
    let b1 = coordinate(b_end);
    a0.min(a1).max(b0.min(b1)) < a0.max(a1).min(b0.max(b1))
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
