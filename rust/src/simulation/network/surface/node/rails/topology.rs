// SPDX-License-Identifier: GPL-2.0-only

//! Generated contour topology helpers for node rails.

use super::super::backend::polyline_to_road_points;
use super::contours::{align_height_points_to_source_contours, cleaned_closed_contour};
use super::geometry::{road_point_from_key, road_point_key};
use super::owners::generated_contour_band_kind;
use super::{
    NodeGeneratedContour, NodeRailConstraint, NodeRailConstraintKind, NodeRailGenerationError,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedContourEdgeKey {
    pub(super) start: NodeRailPointKey,
    pub(super) end: NodeRailPointKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(super) struct GeneratedContourDirectedEdge {
    pub(super) start: NodeRailPointKey,
    pub(super) end: NodeRailPointKey,
}

pub(super) type NodeRailPointKey = (i64, i64);
impl GeneratedContourEdgeKey {
    pub(super) fn new(a: NodeRailPointKey, b: NodeRailPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}
pub(super) fn set_generated_contour_from_keys(
    contour: &mut NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
    keys: Vec<NodeRailPointKey>,
) -> Result<(), NodeRailGenerationError> {
    let points = keys
        .into_iter()
        .map(road_point_from_key)
        .collect::<Vec<_>>();
    let polyline = cleaned_closed_contour(
        contour.kind,
        contour.source_mouth_order_index,
        contour.source_band_index,
        points,
    )?;
    contour.points_xz = polyline_to_road_points(&polyline);
    contour.backend_polyline = polyline;
    if let Some(height_points_world) = contour.height_points_world.as_deref() {
        contour.height_points_world =
            align_height_points_to_source_contours(&contour.points_xz, &[height_points_world]);
    }
    update_generated_band_contour_constraint(contour, constraints);
    Ok(())
}
fn update_generated_band_contour_constraint(
    contour: &NodeGeneratedContour,
    constraints: &mut [NodeRailConstraint],
) {
    let Some(kind) = generated_contour_band_kind(contour) else {
        return;
    };
    for constraint in constraints {
        if matches!(
            constraint.kind,
            NodeRailConstraintKind::BandContour { kind: constraint_kind }
                if constraint_kind == kind
        ) && constraint.source_mouth_order_index == contour.source_mouth_order_index
            && constraint.source_band_index == contour.source_band_index
            && constraint.owner == contour.owner
        {
            constraint.points_xz = contour.points_xz.clone();
        }
    }
}
pub(super) fn generated_contour_directed_edges(
    contour: &NodeGeneratedContour,
) -> Vec<GeneratedContourDirectedEdge> {
    let keys = generated_contour_keys(contour);
    let mut edges = Vec::new();
    for index in 0..keys.len() {
        let start = keys[index];
        let end = keys[(index + 1) % keys.len()];
        if start != end {
            edges.push(GeneratedContourDirectedEdge { start, end });
        }
    }
    edges
}
pub(super) fn generated_contour_keys(contour: &NodeGeneratedContour) -> Vec<NodeRailPointKey> {
    contour
        .points_xz
        .iter()
        .copied()
        .map(road_point_key)
        .collect()
}
