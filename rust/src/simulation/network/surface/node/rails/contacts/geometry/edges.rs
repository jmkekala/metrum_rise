// SPDX-License-Identifier: GPL-2.0-only

//! Contact edge and point extraction from generated contour geometry.

use super::super::{
    GeneratedContourDirectedEdge, GeneratedContourEdgeKey, NodeGeneratedContour, NodeOverlayShapes,
    NodeRailPointKey, ROAD_OVERLAY_COORDINATE_SCALE, RoadSurfaceSystem,
    generated_point_key_lies_on_segment, generated_segment_parameter_key,
    quantized_proper_segment_intersection,
};
use super::overlay::GeneratedOverlayShapeKeys;
use super::overlay::generated_contour_overlay_shapes;
use super::point_location::{
    PreparedGeneratedPointLocationContour, doubled_point_inside_or_on_overlay_shape_keys,
};
use crate::simulation::network::surface::NODE_OVERLAY_MIN_AREA_M2;
use i_overlay::core::fill_rule::FillRule;
use i_overlay::core::overlay::{Overlay, ShapeType};
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::i_float::int::point::IntPoint;
use i_overlay::i_shape::flat::buffer::FlatContoursBuffer;

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::node::rails::contacts) struct PreparedGeneratedContourEdge
{
    pub(in crate::simulation::network::surface::node::rails::contacts) edge:
        GeneratedContourEdgeKey,
    pub(in crate::simulation::network::surface::node::rails::contacts) min_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts) min_z: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts) max_x: i64,
    pub(in crate::simulation::network::surface::node::rails::contacts) max_z: i64,
}

impl PreparedGeneratedContourEdge {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn new(
        edge: GeneratedContourEdgeKey,
    ) -> Self {
        Self {
            edge,
            min_x: edge.start.0.min(edge.end.0),
            min_z: edge.start.1.min(edge.end.1),
            max_x: edge.start.0.max(edge.end.0),
            max_z: edge.start.1.max(edge.end.1),
        }
    }
}

/// Reusable integer overlay state for one same-band pair batch.
pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedContactOverlayScratch
{
    overlay: Overlay,
    output: FlatContoursBuffer,
    edges: Vec<GeneratedContourEdgeKey>,
}

impl Default for GeneratedContactOverlayScratch {
    fn default() -> Self {
        Self {
            overlay: Overlay::new(0),
            output: FlatContoursBuffer::default(),
            edges: Vec::new(),
        }
    }
}

impl GeneratedContactOverlayScratch {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn edges(
        &self,
    ) -> &[GeneratedContourEdgeKey] {
        &self.edges
    }

    pub(in crate::simulation::network::surface::node::rails::contacts) fn replace_edges(
        &mut self,
        edges: Vec<GeneratedContourEdgeKey>,
    ) {
        self.edges = edges;
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn append_generated_contact_edges_inside_prepared_contour(
    role_edges: &[PreparedGeneratedContourEdge],
    target_edges_by_min_x: &[PreparedGeneratedContourEdge],
    target_edges_by_min_z: &[PreparedGeneratedContourEdge],
    target_point_location: &PreparedGeneratedPointLocationContour,
    target_bounds: Option<(i64, i64, i64, i64)>,
    edges: &mut Vec<GeneratedContourEdgeKey>,
    keys: &mut Vec<NodeRailPointKey>,
) {
    let Some((target_min_x, target_min_z, target_max_x, target_max_z)) = target_bounds else {
        return;
    };
    let target_min_x2 = i128::from(target_min_x) * 2;
    let target_min_z2 = i128::from(target_min_z) * 2;
    let target_max_x2 = i128::from(target_max_x) * 2;
    let target_max_z2 = i128::from(target_max_z) * 2;
    for prepared_role_edge in role_edges {
        let role_edge = prepared_role_edge.edge;
        let role_min_x = prepared_role_edge.min_x;
        let role_max_x = prepared_role_edge.max_x;
        let role_min_z = prepared_role_edge.min_z;
        let role_max_z = prepared_role_edge.max_z;
        if role_max_x < target_min_x
            || target_max_x < role_min_x
            || role_max_z < target_min_z
            || target_max_z < role_min_z
        {
            continue;
        }
        let x_last = target_edges_by_min_x.partition_point(|edge| edge.min_x <= role_max_x);
        let z_last = target_edges_by_min_z.partition_point(|edge| edge.min_z <= role_max_z);
        let target_edges = if x_last <= z_last {
            &target_edges_by_min_x[..x_last]
        } else {
            &target_edges_by_min_z[..z_last]
        };
        keys.clear();
        keys.extend([role_edge.start, role_edge.end]);
        for target_edge in target_edges {
            if target_edge.max_x < role_min_x
                || role_max_x < target_edge.min_x
                || target_edge.max_z < role_min_z
                || role_max_z < target_edge.min_z
            {
                continue;
            }
            let target_edge = target_edge.edge;
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
        }
        if keys.len() == 2 {
            let point_x2 = i128::from(role_edge.start.0) + i128::from(role_edge.end.0);
            let point_z2 = i128::from(role_edge.start.1) + i128::from(role_edge.end.1);
            if point_x2 < target_min_x2
                || target_max_x2 < point_x2
                || point_z2 < target_min_z2
                || target_max_z2 < point_z2
            {
                continue;
            }
            if target_point_location.contains_doubled_point(point_x2, point_z2) {
                edges.push(GeneratedContourEdgeKey::new(role_edge.start, role_edge.end));
            }
            continue;
        }
        keys.sort_by_key(|point| {
            generated_segment_parameter_key(role_edge.start, role_edge.end, *point)
        });
        keys.dedup();
        for segment in keys.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if start == end {
                continue;
            }
            let point_x2 = i128::from(start.0) + i128::from(end.0);
            let point_z2 = i128::from(start.1) + i128::from(end.1);
            if point_x2 < target_min_x2
                || target_max_x2 < point_x2
                || point_z2 < target_min_z2
                || target_max_z2 < point_z2
            {
                continue;
            }
            if target_point_location.contains_doubled_point(point_x2, point_z2) {
                edges.push(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn append_generated_directed_edge_segments_inside_shape_keys(
    edge: GeneratedContourDirectedEdge,
    shape_edges_by_min_x: &[GeneratedContourDirectedEdge],
    shape_edges_by_min_z: &[GeneratedContourDirectedEdge],
    containing_shapes: &GeneratedOverlayShapeKeys,
    edges: &mut Vec<GeneratedContourEdgeKey>,
    keys: &mut Vec<NodeRailPointKey>,
) {
    keys.clear();
    keys.extend([edge.start, edge.end]);
    let edge_min_x = edge.start.0.min(edge.end.0);
    let edge_max_x = edge.start.0.max(edge.end.0);
    let edge_min_z = edge.start.1.min(edge.end.1);
    let edge_max_z = edge.start.1.max(edge.end.1);
    let x_last = shape_edges_by_min_x
        .partition_point(|shape_edge| shape_edge.start.0.min(shape_edge.end.0) <= edge_max_x);
    let z_last = shape_edges_by_min_z
        .partition_point(|shape_edge| shape_edge.start.1.min(shape_edge.end.1) <= edge_max_z);
    let shape_edges = if x_last <= z_last {
        &shape_edges_by_min_x[..x_last]
    } else {
        &shape_edges_by_min_z[..z_last]
    };
    for shape_edge in shape_edges {
        let shape_min_x = shape_edge.start.0.min(shape_edge.end.0);
        let shape_max_x = shape_edge.start.0.max(shape_edge.end.0);
        let shape_min_z = shape_edge.start.1.min(shape_edge.end.1);
        let shape_max_z = shape_edge.start.1.max(shape_edge.end.1);
        if shape_max_x < edge_min_x
            || edge_max_x < shape_min_x
            || shape_max_z < edge_min_z
            || edge_max_z < shape_min_z
        {
            continue;
        }
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
    }
    keys.sort_by_key(|point| generated_segment_parameter_key(edge.start, edge.end, *point));
    keys.dedup();

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
}

fn append_generated_shape_boundary_segments_on_source_edge(
    source_edge: GeneratedContourDirectedEdge,
    shape_edges: &[GeneratedContourDirectedEdge],
    edges: &mut Vec<GeneratedContourEdgeKey>,
) {
    let source_min_x = source_edge.start.0.min(source_edge.end.0);
    let source_max_x = source_edge.start.0.max(source_edge.end.0);
    let source_min_z = source_edge.start.1.min(source_edge.end.1);
    let source_max_z = source_edge.start.1.max(source_edge.end.1);
    for shape_edge in shape_edges {
        let shape_min_x = shape_edge.start.0.min(shape_edge.end.0);
        let shape_max_x = shape_edge.start.0.max(shape_edge.end.0);
        let shape_min_z = shape_edge.start.1.min(shape_edge.end.1);
        let shape_max_z = shape_edge.start.1.max(shape_edge.end.1);
        if shape_max_x < source_min_x
            || source_max_x < shape_min_x
            || shape_max_z < source_min_z
            || source_max_z < shape_min_z
        {
            continue;
        }
        let mut keys = [((0, 0), 0_i128); 4];
        let mut key_count = 0_usize;
        for point in [shape_edge.start, shape_edge.end] {
            if generated_point_key_lies_on_segment(point, source_edge.start, source_edge.end) {
                keys[key_count] = (
                    point,
                    generated_segment_parameter_key(source_edge.start, source_edge.end, point),
                );
                key_count += 1;
            }
        }
        for point in [source_edge.start, source_edge.end] {
            if generated_point_key_lies_on_segment(point, shape_edge.start, shape_edge.end) {
                keys[key_count] = (
                    point,
                    generated_segment_parameter_key(source_edge.start, source_edge.end, point),
                );
                key_count += 1;
            }
        }
        let keys = &mut keys[..key_count];
        keys.sort_by_key(|(_, parameter)| *parameter);
        let mut previous = None;
        for &(point, _) in keys.iter() {
            if previous == Some(point) {
                continue;
            }
            if let Some(start) = previous {
                edges.push(GeneratedContourEdgeKey::new(start, point));
            }
            previous = Some(point);
        }
    }
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

/// Intersects already-quantized overlay shapes while reusing solver and output buffers.
///
/// Returns `false` only when the node-local coordinates cannot be represented by the
/// integer backend; callers can then use the general floating-point adapter.
pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_from_overlay_shape_key_intersection(
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shapes: &GeneratedOverlayShapeKeys,
    scratch: &mut GeneratedContactOverlayScratch,
) -> bool {
    let Some(origin) = overlay_shape_key_origin(left_shapes, right_shapes) else {
        scratch.edges.clear();
        return true;
    };
    if !overlay_shape_keys_fit_i32(left_shapes, origin)
        || !overlay_shape_keys_fit_i32(right_shapes, origin)
    {
        return false;
    }

    scratch.overlay.clear();
    for contour in left_shapes.iter().flat_map(|shape| shape.iter()) {
        if contour.len() >= 3 {
            scratch.overlay.add_path_iter(
                contour
                    .iter()
                    .copied()
                    .map(|key| overlay_int_point(key, origin)),
                ShapeType::Subject,
            );
        }
    }
    for contour in right_shapes.iter().flat_map(|shape| shape.iter()) {
        if contour.len() >= 3 {
            scratch.overlay.add_path_iter(
                contour
                    .iter()
                    .copied()
                    .map(|key| overlay_int_point(key, origin)),
                ShapeType::Clip,
            );
        }
    }
    scratch.output.points.clear();
    scratch.output.ranges.clear();
    scratch.overlay.overlay_into(
        OverlayRule::Intersect,
        FillRule::Positive,
        &mut scratch.output,
    );

    scratch.edges.clear();
    for range in &scratch.output.ranges {
        let contour = &scratch.output.points[range.clone()];
        if !int_overlay_contour_passes_area_floor(contour) {
            continue;
        }
        for index in 0..contour.len() {
            let start = overlay_key_from_int_point(contour[index], origin);
            let end = overlay_key_from_int_point(contour[(index + 1) % contour.len()], origin);
            if start != end {
                scratch.edges.push(GeneratedContourEdgeKey::new(start, end));
            }
        }
    }
    scratch.edges.sort_unstable();
    scratch.edges.dedup();
    true
}

fn overlay_shape_key_origin(
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shapes: &GeneratedOverlayShapeKeys,
) -> Option<NodeRailPointKey> {
    left_shapes
        .iter()
        .chain(right_shapes.iter())
        .flat_map(|shape| shape.iter())
        .flat_map(|contour| contour.iter().copied())
        .reduce(|left, right| (left.0.min(right.0), left.1.min(right.1)))
}

fn overlay_shape_keys_fit_i32(
    shapes: &GeneratedOverlayShapeKeys,
    origin: NodeRailPointKey,
) -> bool {
    shapes
        .iter()
        .flat_map(|shape| shape.iter())
        .flat_map(|contour| contour.iter())
        .all(|&(x, z)| i32::try_from(x - origin.0).is_ok() && i32::try_from(z - origin.1).is_ok())
}

fn overlay_int_point(key: NodeRailPointKey, origin: NodeRailPointKey) -> IntPoint {
    IntPoint::new(
        i32::try_from(key.0 - origin.0).expect("overlay coordinates were range checked"),
        i32::try_from(key.1 - origin.1).expect("overlay coordinates were range checked"),
    )
}

fn overlay_key_from_int_point(point: IntPoint, origin: NodeRailPointKey) -> NodeRailPointKey {
    (origin.0 + i64::from(point.x), origin.1 + i64::from(point.y))
}

fn int_overlay_contour_passes_area_floor(contour: &[IntPoint]) -> bool {
    if contour.len() < 3 {
        return false;
    }
    let doubled_area = contour
        .iter()
        .zip(contour.iter().cycle().skip(1))
        .take(contour.len())
        .map(|(start, end)| {
            i128::from(start.x) * i128::from(end.y) - i128::from(end.x) * i128::from(start.y)
        })
        .sum::<i128>();
    let area_m2 = (doubled_area.unsigned_abs() as f64
        / (2.0 * ROAD_OVERLAY_COORDINATE_SCALE * ROAD_OVERLAY_COORDINATE_SCALE))
        as f32;
    area_m2 > NODE_OVERLAY_MIN_AREA_M2
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_edges_from_source_edges_inside_shape_key_intersection(
    source_edges: &[GeneratedContourDirectedEdge],
    left_shape_edges: &[GeneratedContourDirectedEdge],
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shape_edges: &[GeneratedContourDirectedEdge],
    right_shapes: &GeneratedOverlayShapeKeys,
) -> Vec<GeneratedContourEdgeKey> {
    let mut edges = Vec::new();
    let mut boundary_edges = Vec::new();
    for source_edge in source_edges {
        append_generated_source_edge_segments_inside_shape_intersection(
            *source_edge,
            left_shape_edges,
            left_shapes,
            right_shape_edges,
            right_shapes,
            &mut boundary_edges,
            &mut edges,
        );
    }
    edges.sort_unstable();
    edges.dedup();
    edges
}

fn append_generated_source_edge_segments_inside_shape_intersection(
    source_edge: GeneratedContourDirectedEdge,
    left_shape_edges: &[GeneratedContourDirectedEdge],
    left_shapes: &GeneratedOverlayShapeKeys,
    right_shape_edges: &[GeneratedContourDirectedEdge],
    right_shapes: &GeneratedOverlayShapeKeys,
    boundary_edges: &mut Vec<GeneratedContourEdgeKey>,
    edges: &mut Vec<GeneratedContourEdgeKey>,
) {
    boundary_edges.clear();
    append_generated_shape_boundary_segments_on_source_edge(
        source_edge,
        left_shape_edges,
        boundary_edges,
    );
    append_generated_shape_boundary_segments_on_source_edge(
        source_edge,
        right_shape_edges,
        boundary_edges,
    );
    boundary_edges.sort_unstable();
    boundary_edges.dedup();
    edges.extend(boundary_edges.iter().copied().filter(|edge| {
        generated_contact_edge_lies_inside_overlay_shapes(*edge, left_shapes)
            && generated_contact_edge_lies_inside_overlay_shapes(*edge, right_shapes)
    }));
}

fn generated_contact_edge_lies_inside_overlay_shapes(
    edge: GeneratedContourEdgeKey,
    shapes: &GeneratedOverlayShapeKeys,
) -> bool {
    let point_x2 = i128::from(edge.start.0) + i128::from(edge.end.0);
    let point_z2 = i128::from(edge.start.1) + i128::from(edge.end.1);
    doubled_point_inside_or_on_overlay_shape_keys(point_x2, point_z2, shapes)
}
