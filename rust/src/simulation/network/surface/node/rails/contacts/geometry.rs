//! Canonical generated-contact geometry helpers.

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GeneratedPointContourLocation {
    Outside,
    Boundary,
    Inside,
}

pub(super) fn generated_role_edge_segments_inside_contour(
    role_edge: GeneratedContourDirectedEdge,
    target: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![role_edge.start, role_edge.end];
    for target_edge in generated_contour_directed_edges(target) {
        if let Some(point) = quantized_proper_segment_intersection(
            role_edge.start,
            role_edge.end,
            target_edge.start,
            target_edge.end,
        ) {
            keys.push(point);
        }
        for point in [target_edge.start, target_edge.end] {
            if generated_point_key_lies_on_segment(point, role_edge.start, role_edge.end) {
                keys.push(point);
            }
        }
        for point in [role_edge.start, role_edge.end] {
            if generated_point_key_lies_on_segment(point, target_edge.start, target_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| {
        generated_segment_parameter_key(role_edge.start, role_edge.end, *point)
    });
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_generated_contour(point_x2, point_z2, target) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

pub(super) fn generated_contour_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    doubled_point_inside_or_on_generated_contour(
        i128::from(point.0) * 2,
        i128::from(point.1) * 2,
        contour,
    )
}

pub(super) fn generated_contour_boundary_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    generated_contour_directed_edges(contour)
        .into_iter()
        .any(|edge| generated_point_key_lies_on_segment(point, edge.start, edge.end))
}

pub(super) fn generated_contact_edges_inside_contour(
    edge_contour: &NodeGeneratedContour,
    containing_contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for edge in generated_contour_directed_edges(edge_contour) {
        edges.extend(generated_role_edge_segments_inside_contour(
            edge,
            containing_contour,
        ));
    }
    edges.into_iter().collect()
}

pub(super) fn generated_directed_edge_segments_inside_shape_edges(
    edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    containing_shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourEdgeKey> {
    let mut keys = vec![edge.start, edge.end];
    for shape_edge in shape_edges {
        if let Some(point) = quantized_proper_segment_intersection(
            edge.start,
            edge.end,
            shape_edge.start,
            shape_edge.end,
        ) {
            keys.push(point);
        }
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, edge.start, edge.end) {
                keys.push(point);
            }
        }
        for point in [edge.start, edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
    }
    keys.sort_by_key(|point| generated_segment_parameter_key(edge.start, edge.end, *point));
    keys.dedup();

    let mut edges = BTreeSet::new();
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_overlay_shapes(point_x2, point_z2, containing_shapes) {
            edges.insert(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges.into_iter().collect()
}

pub(super) fn generated_shape_boundary_segments_on_source_edge(
    source_edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for shape_edge in shape_edges {
        let mut keys = Vec::new();
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, source_edge.start, source_edge.end) {
                keys.push(point);
            }
        }
        for point in [source_edge.start, source_edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys.push(point);
            }
        }
        keys.sort_by_key(|point| {
            generated_segment_parameter_key(source_edge.start, source_edge.end, *point)
        });
        keys.dedup();
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if start != end {
                edges.insert(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
    edges.into_iter().collect()
}

pub(super) fn generated_contact_edges_from_overlay_intersection(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(left_shapes) = generated_contour_overlay_shapes(left) else {
        return Vec::new();
    };
    let Some(right_shapes) = generated_contour_overlay_shapes(right) else {
        return Vec::new();
    };
    let Some(intersection) = RoadSurfaceSystem::overlay_binary_shapes(
        &left_shapes,
        &right_shapes,
        OverlayRule::Intersect,
    ) else {
        return Vec::new();
    };
    let mut edges = intersection
        .into_iter()
        .flat_map(|shape| shape.into_iter())
        .flat_map(|contour| {
            let keys = contour
                .into_iter()
                .map(|point| {
                    (
                        (point[0] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                        (point[1] * ROAD_OVERLAY_COORDINATE_SCALE).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            let mut edges = Vec::new();
            for index in 0..keys.len() {
                let start = keys[index];
                let end = keys[(index + 1) % keys.len()];
                if start != end {
                    edges.push(GeneratedContourEdgeKey::new(start, end));
                }
            }
            edges
        })
        .collect::<Vec<_>>();
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub(super) fn generated_contact_points_from_contour_intersections(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<NodeRailPointKey> {
    let mut points = Vec::new();
    for left_edge in generated_contour_directed_edges(left) {
        for right_edge in generated_contour_directed_edges(right) {
            if let Some(point) = quantized_proper_segment_intersection(
                left_edge.start,
                left_edge.end,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(point);
            }
            if generated_point_key_lies_on_segment(
                left_edge.start,
                right_edge.start,
                right_edge.end,
            ) {
                points.push(left_edge.start);
            }
            if generated_point_key_lies_on_segment(left_edge.end, right_edge.start, right_edge.end)
            {
                points.push(left_edge.end);
            }
            if generated_point_key_lies_on_segment(right_edge.start, left_edge.start, left_edge.end)
            {
                points.push(right_edge.start);
            }
            if generated_point_key_lies_on_segment(right_edge.end, left_edge.start, left_edge.end) {
                points.push(right_edge.end);
            }
        }
    }
    points.sort_unstable();
    points.dedup();
    points
}

pub(super) fn generated_contour_overlay_shapes(
    contour: &NodeGeneratedContour,
) -> Option<NodeOverlayShapes> {
    RoadSurfaceSystem::overlay_union_contours(&[generated_overlay_contour(contour)])
}

pub(super) fn generated_overlay_contour(contour: &NodeGeneratedContour) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

pub(super) fn generated_overlay_shapes_directed_edges(
    shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = Vec::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        let keys = generated_overlay_contour_keys(contour);
        for index in 0..keys.len() {
            let start = keys[index];
            let end = keys[(index + 1) % keys.len()];
            if start != end {
                edges.push(GeneratedContourDirectedEdge { start, end });
            }
        }
    }
    edges
}

pub(super) fn generated_overlay_contour_keys(
    contour: &NodeOverlayContour,
) -> Vec<NodeRailPointKey> {
    contour
        .iter()
        .copied()
        .map(generated_overlay_point_key)
        .collect()
}

pub(super) fn generated_overlay_point_key(point: [f64; 2]) -> NodeRailPointKey {
    let key = SurfaceXzKey::from_overlay_point(point);
    (key.x_key(), key.z_key())
}

pub(super) fn doubled_point_inside_or_on_generated_contour(
    point_x2: i128,
    point_z2: i128,
    contour: &NodeGeneratedContour,
) -> bool {
    let keys = generated_contour_keys(contour);
    doubled_point_inside_or_on_generated_keys(point_x2, point_z2, &keys)
}

pub(super) fn doubled_point_inside_or_on_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> bool {
    doubled_point_location_in_generated_keys(point_x2, point_z2, keys)
        != GeneratedPointContourLocation::Outside
}

pub(super) fn doubled_point_inside_or_on_overlay_shapes(
    point_x2: i128,
    point_z2: i128,
    shapes: &NodeOverlayShapes,
) -> bool {
    shapes.iter().any(|shape| {
        let Some(outer) = shape.first() else {
            return false;
        };
        let outer_keys = generated_overlay_contour_keys(outer);
        match doubled_point_location_in_generated_keys(point_x2, point_z2, &outer_keys) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => shape.iter().skip(1).all(|hole| {
                let hole_keys = generated_overlay_contour_keys(hole);
                doubled_point_location_in_generated_keys(point_x2, point_z2, &hole_keys)
                    != GeneratedPointContourLocation::Inside
            }),
        }
    })
}

pub(super) fn doubled_point_location_in_generated_keys(
    point_x2: i128,
    point_z2: i128,
    keys: &[NodeRailPointKey],
) -> GeneratedPointContourLocation {
    if keys.len() < 3 {
        return GeneratedPointContourLocation::Outside;
    }
    let mut inside = false;
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if doubled_point_lies_on_generated_segment(point_x2, point_z2, start, end) {
            return GeneratedPointContourLocation::Boundary;
        }
        let start_z2 = i128::from(start.1) * 2;
        let end_z2 = i128::from(end.1) * 2;
        if (start_z2 > point_z2) == (end_z2 > point_z2) {
            continue;
        }
        let start_x2 = i128::from(start.0) * 2;
        let end_x2 = i128::from(end.0) * 2;
        let denominator = end_z2 - start_z2;
        let lhs = (point_x2 - start_x2) * denominator;
        let rhs = (point_z2 - start_z2) * (end_x2 - start_x2);
        let crosses = if denominator > 0 {
            lhs < rhs
        } else {
            lhs > rhs
        };
        if crosses {
            inside = !inside;
        }
    }
    if inside {
        GeneratedPointContourLocation::Inside
    } else {
        GeneratedPointContourLocation::Outside
    }
}

pub(super) fn doubled_point_lies_on_generated_segment(
    point_x2: i128,
    point_z2: i128,
    start: NodeRailPointKey,
    end: NodeRailPointKey,
) -> bool {
    let start_x2 = i128::from(start.0) * 2;
    let start_z2 = i128::from(start.1) * 2;
    let end_x2 = i128::from(end.0) * 2;
    let end_z2 = i128::from(end.1) * 2;
    let dx = end_x2 - start_x2;
    let dz = end_z2 - start_z2;
    let px = point_x2 - start_x2;
    let pz = point_z2 - start_z2;
    if px * dz - pz * dx != 0 {
        return false;
    }
    point_x2 >= start_x2.min(end_x2)
        && point_x2 <= start_x2.max(end_x2)
        && point_z2 >= start_z2.min(end_z2)
        && point_z2 <= start_z2.max(end_z2)
}
