//! Overlay-shape conversion helpers for generated contact geometry.

use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeOverlayContour, NodeOverlayShapes,
    NodeRailPointKey, RoadSurfaceSystem, SurfaceXzKey,
};

pub(super) fn generated_contour_overlay_shapes(
    contour: &NodeGeneratedContour,
) -> Option<NodeOverlayShapes> {
    RoadSurfaceSystem::overlay_union_contours(&[generated_overlay_contour(contour)])
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_overlay_contour(
    contour: &NodeGeneratedContour,
) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_overlay_shapes_directed_edges(
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

fn generated_overlay_point_key(point: [f64; 2]) -> NodeRailPointKey {
    let key = SurfaceXzKey::from_overlay_point(point);
    (key.x_key(), key.z_key())
}
