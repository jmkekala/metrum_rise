//! Exact quantized point-in-contour predicates for generated contact geometry.

use super::super::{
    NodeGeneratedContour, NodeOverlayShapes, NodeRailPointKey, generated_contour_directed_edges,
    generated_contour_keys, generated_point_key_lies_on_segment,
};
use super::overlay::generated_overlay_contour_keys;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GeneratedPointContourLocation {
    Outside,
    Boundary,
    Inside,
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contour_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    doubled_point_inside_or_on_generated_contour(
        i128::from(point.0) * 2,
        i128::from(point.1) * 2,
        contour,
    )
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contour_boundary_contains_key(
    contour: &NodeGeneratedContour,
    point: NodeRailPointKey,
) -> bool {
    generated_contour_directed_edges(contour)
        .into_iter()
        .any(|edge| generated_point_key_lies_on_segment(point, edge.start, edge.end))
}

pub(super) fn doubled_point_inside_or_on_generated_contour(
    point_x2: i128,
    point_z2: i128,
    contour: &NodeGeneratedContour,
) -> bool {
    let keys = generated_contour_keys(contour);
    doubled_point_inside_or_on_generated_keys(point_x2, point_z2, &keys)
}

fn doubled_point_inside_or_on_generated_keys(
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
        let Some((outer, holes)) = shape.split_first() else {
            return false;
        };
        let outer_keys = generated_overlay_contour_keys(outer);
        match doubled_point_location_in_generated_keys(point_x2, point_z2, &outer_keys) {
            GeneratedPointContourLocation::Outside => false,
            GeneratedPointContourLocation::Boundary => true,
            GeneratedPointContourLocation::Inside => holes.iter().all(|hole| {
                let hole_keys = generated_overlay_contour_keys(hole);
                doubled_point_location_in_generated_keys(point_x2, point_z2, &hole_keys)
                    != GeneratedPointContourLocation::Inside
            }),
        }
    })
}

fn doubled_point_location_in_generated_keys(
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

fn doubled_point_lies_on_generated_segment(
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
