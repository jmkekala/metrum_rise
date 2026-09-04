//! Contact edge and point extraction from generated contour geometry.

use super::super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeGeneratedContour, NodeOverlayShapes,
    NodeRailPointKey, ROAD_OVERLAY_COORDINATE_SCALE, RoadSurfaceSystem,
    generated_contour_directed_edges, generated_point_key_lies_on_segment,
    generated_segment_parameter_key, quantized_proper_segment_intersection,
};
use super::overlay::GeneratedOverlayShapeKeys;
use super::overlay::generated_contour_overlay_shapes;
use super::point_location::{
    doubled_point_inside_or_on_generated_contour, doubled_point_inside_or_on_overlay_shape_keys,
};
use i_overlay::core::overlay_rule::OverlayRule;
use std::collections::BTreeSet;

fn generated_role_edge_segments_inside_contour(
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

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_inside_contour(
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

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_directed_edge_segments_inside_shape_keys(
    edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    containing_shapes: &GeneratedOverlayShapeKeys,
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

    let mut edges = Vec::with_capacity(keys.len().saturating_sub(1));
    for segment in keys.windows(2) {
        let start = segment[0];
        let end = segment[1];
        if start == end {
            continue;
        }
        let point_x2 = i128::from(start.0) + i128::from(end.0);
        let point_z2 = i128::from(start.1) + i128::from(end.1);
        if doubled_point_inside_or_on_overlay_shape_keys(point_x2, point_z2, containing_shapes) {
            edges.push(GeneratedContourEdgeKey::new(start, end));
        }
    }
    edges
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_shape_boundary_segments_on_source_edge(
    source_edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = Vec::new();
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
                edges.push(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_from_overlay_intersection(
    left: &NodeGeneratedContour,
    right: &NodeGeneratedContour,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(left_shapes) = generated_contour_overlay_shapes(left) else {
        return Vec::new();
    };
    let Some(right_shapes) = generated_contour_overlay_shapes(right) else {
        return Vec::new();
    };
    generated_contact_edges_from_overlay_shape_intersection(&left_shapes, &right_shapes)
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_from_overlay_shape_intersection(
    left_shapes: &NodeOverlayShapes,
    right_shapes: &NodeOverlayShapes,
) -> Vec<GeneratedContourEdgeKey> {
    let Some(intersection) =
        RoadSurfaceSystem::overlay_binary_shapes(left_shapes, right_shapes, OverlayRule::Intersect)
    else {
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

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_from_source_edges_inside_shape_key_intersection(
    source_edges: &[GeneratedContourDirectedEdge],
    left_shape_edges: &[GeneratedContourDirectedEdge],
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shape_edges: &[GeneratedContourDirectedEdge],
    right_shapes: &GeneratedOverlayShapeKeys,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    for source_edge in source_edges {
        edges.extend(generated_source_edge_segments_inside_shape_intersection(
            *source_edge,
            left_shape_edges,
            left_shapes,
            right_shape_edges,
            right_shapes,
        ));
    }
    edges.into_iter().collect()
}

fn generated_source_edge_segments_inside_shape_intersection(
    source_edge: GeneratedContourDirectedEdge,
    left_shape_edges: &[GeneratedContourDirectedEdge],
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shape_edges: &[GeneratedContourDirectedEdge],
    right_shapes: &GeneratedOverlayShapeKeys,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = BTreeSet::new();
    edges.extend(
        generated_shape_boundary_segments_on_source_edge(source_edge, left_shape_edges)
            .into_iter()
            .filter(|edge| generated_contact_edge_lies_inside_overlay_shapes(*edge, right_shapes)),
    );
    edges.extend(
        generated_shape_boundary_segments_on_source_edge(source_edge, right_shape_edges)
            .into_iter()
            .filter(|edge| generated_contact_edge_lies_inside_overlay_shapes(*edge, left_shapes)),
    );
    edges.retain(|edge| {
        generated_contact_edge_lies_inside_overlay_shapes(*edge, left_shapes)
            && generated_contact_edge_lies_inside_overlay_shapes(*edge, right_shapes)
    });
    edges.into_iter().collect()
}

fn generated_contact_edge_lies_inside_overlay_shapes(
    edge: GeneratedContourEdgeKey,
    shapes: &GeneratedOverlayShapeKeys,
) -> bool {
    let point_x2 = i128::from(edge.start.0) + i128::from(edge.end.0);
    let point_z2 = i128::from(edge.start.1) + i128::from(edge.end.1);
    doubled_point_inside_or_on_overlay_shape_keys(point_x2, point_z2, shapes)
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_points_from_contour_intersections(
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
