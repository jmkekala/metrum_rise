//! Cross-region triangle edge height validation.

use super::super::super::arrangement::{
    NodeBandOwner, NodeExplicitVerticalStepSegment, owners_form_explicit_vertical_step_pair,
};
use super::super::super::keys::SurfaceSegmentParameter;
use super::super::super::segments;
use super::super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::super::report::{
    NodeExplicitStepSegmentDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, push_validation_diagnostic,
};
use super::super::{
    NodeValidationEdgeKey, NodeValidationPointKey, point_key_from_world, quantize_m,
};
use super::{edge_indices_valid, triangle_edges, triangle_indices_valid};
use std::collections::BTreeMap;

mod diagnostics;
mod index;
mod steps;

use diagnostics::push_triangle_edge_height_conflict;
use steps::cross_region_edges_form_explicit_vertical_step;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedTriangleEdge {
    region_index: usize,
    start_height_mm: i64,
    end_height_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct HeightedOwnedCoverageEdge {
    edge: NodeValidationEdgeKey,
    heighted_edge: HeightedTriangleEdge,
}

#[derive(Default)]
struct ValidationTriangleEdgeIndex {
    by_edge: BTreeMap<NodeValidationEdgeKey, Vec<HeightedTriangleEdge>>,
    by_owner_coverage: BTreeMap<NodeBandOwner, Vec<HeightedOwnedCoverageEdge>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct HeightedEdgeCoverageInterval {
    start: SurfaceSegmentParameter,
    end: SurfaceSegmentParameter,
}

pub(super) fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    let edge_index = ValidationTriangleEdgeIndex::from_solution(solution);

    for (edge_key, heighted_edges) in &edge_index.by_edge {
        let mut heighted_edges = heighted_edges.clone();
        heighted_edges.sort_unstable();
        heighted_edges.dedup();
        'edge: for left_index in 0..heighted_edges.len() {
            for right_index in left_index + 1..heighted_edges.len() {
                let left = heighted_edges[left_index];
                let right = heighted_edges[right_index];
                if left.region_index == right.region_index
                    || (left.start_height_mm == right.start_height_mm
                        && left.end_height_mm == right.end_height_mm)
                    || cross_region_edges_form_explicit_vertical_step(
                        solution,
                        &edge_index,
                        *edge_key,
                        left,
                        right,
                    )
                {
                    continue;
                }
                push_triangle_edge_height_conflict(solution, diagnostics, *edge_key, left, right);
                break 'edge;
            }
        }
    }
}
