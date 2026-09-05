// SPDX-License-Identifier: GPL-2.0-only

//! Overlay-shape conversion helpers for generated contact geometry.

use super::super::{
    GeneratedContourDirectedEdge, NodeGeneratedContour, NodeOverlayContour, NodeOverlayShapes,
    NodeRailPointKey, RoadSurfaceSystem, SurfaceXzKey,
};
use super::point_location::PreparedGeneratedPointLocationContour;

#[derive(Clone, Debug, Default)]
pub(in crate::simulation::network::surface::node::rails::contacts) struct GeneratedOverlayShapeKeys
{
    pub(super) shapes: Vec<Vec<Vec<NodeRailPointKey>>>,
    pub(super) prepared_shapes: Vec<Vec<PreparedGeneratedPointLocationContour>>,
    pub(super) doubled_bounds: Vec<Option<(i128, i128, i128, i128)>>,
}

impl GeneratedOverlayShapeKeys {
    pub(in crate::simulation::network::surface::node::rails::contacts) fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    pub(in crate::simulation::network::surface::node::rails::contacts) fn iter(
        &self,
    ) -> impl Iterator<Item = &Vec<Vec<NodeRailPointKey>>> {
        self.shapes.iter()
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contour_overlay_shapes(
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

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_overlay_shapes_keys(
    shapes: &NodeOverlayShapes,
) -> GeneratedOverlayShapeKeys {
    let shapes = shapes
        .iter()
        .map(|shape| shape.iter().map(generated_overlay_contour_keys).collect())
        .collect::<Vec<Vec<Vec<NodeRailPointKey>>>>();
    let doubled_bounds = shapes
        .iter()
        .map(|shape| {
            generated_overlay_shape_key_bounds(shape).map(|(min_x, min_z, max_x, max_z)| {
                (
                    i128::from(min_x) * 2,
                    i128::from(min_z) * 2,
                    i128::from(max_x) * 2,
                    i128::from(max_z) * 2,
                )
            })
        })
        .collect();
    let prepared_shapes = shapes
        .iter()
        .map(|shape| {
            shape
                .iter()
                .map(|contour| PreparedGeneratedPointLocationContour::new(contour))
                .collect()
        })
        .collect();
    GeneratedOverlayShapeKeys {
        shapes,
        prepared_shapes,
        doubled_bounds,
    }
}

fn generated_overlay_shape_key_bounds(
    shape: &[Vec<NodeRailPointKey>],
) -> Option<(i64, i64, i64, i64)> {
    let mut points = shape.iter().flat_map(|contour| contour.iter().copied());
    let first = points.next()?;
    let (mut min_x, mut min_z, mut max_x, mut max_z) = (first.0, first.1, first.0, first.1);
    for point in points {
        min_x = min_x.min(point.0);
        min_z = min_z.min(point.1);
        max_x = max_x.max(point.0);
        max_z = max_z.max(point.1);
    }
    Some((min_x, min_z, max_x, max_z))
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_overlay_shape_keys_directed_edges(
    shapes: &GeneratedOverlayShapeKeys,
) -> Vec<GeneratedContourDirectedEdge> {
    let mut edges = Vec::new();
    for contour in shapes.iter().flat_map(|shape| shape.iter()) {
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
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
