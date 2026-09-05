// SPDX-License-Identifier: GPL-2.0-only

//! Canonical boundary segment crossing checks.

use super::super::keys::SurfaceXzKey;
use super::super::triangulation::NodeTriangulationSolution;
use super::report::{
    NodeGeometryBackend, NodeGeometryDiagnostic, NodeGeometryDiagnosticKind,
    NodeInvalidConstraintReason, push_validation_diagnostic,
};
use super::{BoundarySegment, NodeValidationEdgeKey, NodeValidationPointKey};

pub(super) fn validate_constraint_crossings(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    boundary_segments: &[BoundarySegment],
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    for first_index in 0..boundary_segments.len() {
        for second_index in first_index + 1..boundary_segments.len() {
            let first = boundary_segments[first_index];
            let second = boundary_segments[second_index];
            if shares_endpoint(first.edge, second.edge) {
                continue;
            }
            if key_edges_share_endpoint(first.key_edge, second.key_edge) {
                continue;
            }
            if canonical_key_segments_strictly_intersect(first.key_edge, second.key_edge) {
                let region = &solution.regions[region_index];
                crate::debug_log!(
                    "road",
                    "node_constraint_crossing node_id={} piece_kind={:?} region={} kind={:?} owner={:?} backend=canonical_keys first_constraint={} second_constraint={} first_key=({},{})->({},{}) second_key=({},{})->({},{}) first=({:.6},{:.6})->({:.6},{:.6}) second=({:.6},{:.6})->({:.6},{:.6})",
                    solution.node_id,
                    solution.piece_kind,
                    region_index,
                    region.kind,
                    region.owner,
                    first.index,
                    second.index,
                    first.key_edge.start.x_key,
                    first.key_edge.start.z_key,
                    first.key_edge.end.x_key,
                    first.key_edge.end.z_key,
                    second.key_edge.start.x_key,
                    second.key_edge.start.z_key,
                    second.key_edge.end.x_key,
                    second.key_edge.end.z_key,
                    first.segment.a.x,
                    first.segment.a.y,
                    first.segment.b.x,
                    first.segment.b.y,
                    second.segment.a.x,
                    second.segment.a.y,
                    second.segment.b.x,
                    second.segment.b.y
                );
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::CanonicalKeys,
                    NodeGeometryDiagnosticKind::InvalidConstraint {
                        region_index,
                        constraint_index: Some(first.index.min(second.index)),
                        reason: NodeInvalidConstraintReason::Crossing,
                    },
                );
            }
        }
    }
}

fn key_edges_share_endpoint(a: NodeValidationEdgeKey, b: NodeValidationEdgeKey) -> bool {
    a.start == b.start || a.start == b.end || a.end == b.start || a.end == b.end
}

pub(super) fn canonical_key_segments_strictly_intersect(
    first: NodeValidationEdgeKey,
    second: NodeValidationEdgeKey,
) -> bool {
    let [a, b] = first.endpoints();
    let [c, d] = second.endpoints();
    if key_edges_share_endpoint(first, second) {
        return false;
    }

    let ab_c = orientation(a, b, c);
    let ab_d = orientation(a, b, d);
    let cd_a = orientation(c, d, a);
    let cd_b = orientation(c, d, b);

    if ab_c == 0 && ab_d == 0 && cd_a == 0 && cd_b == 0 {
        return collinear_segments_overlap_with_positive_length(a, b, c, d);
    }

    if ab_c == 0 || ab_d == 0 || cd_a == 0 || cd_b == 0 {
        return false;
    }

    signs_differ(ab_c, ab_d) && signs_differ(cd_a, cd_b)
}

fn orientation(
    a: NodeValidationPointKey,
    b: NodeValidationPointKey,
    c: NodeValidationPointKey,
) -> i128 {
    SurfaceXzKey::triangle_area2(a.surface_key(), b.surface_key(), c.surface_key())
}

fn signs_differ(a: i128, b: i128) -> bool {
    (a < 0 && b > 0) || (a > 0 && b < 0)
}

fn collinear_segments_overlap_with_positive_length(
    a: NodeValidationPointKey,
    b: NodeValidationPointKey,
    c: NodeValidationPointKey,
    d: NodeValidationPointKey,
) -> bool {
    if a.x_key != b.x_key || c.x_key != d.x_key {
        intervals_overlap_with_positive_length(a.x_key, b.x_key, c.x_key, d.x_key)
    } else {
        intervals_overlap_with_positive_length(a.z_key, b.z_key, c.z_key, d.z_key)
    }
}

fn intervals_overlap_with_positive_length(a0: i64, a1: i64, b0: i64, b1: i64) -> bool {
    let a_min = a0.min(a1);
    let a_max = a0.max(a1);
    let b_min = b0.min(b1);
    let b_max = b0.max(b1);
    a_min.max(b_min) < a_max.min(b_max)
}

fn shares_endpoint(a: [usize; 2], b: [usize; 2]) -> bool {
    a[0] == b[0] || a[0] == b[1] || a[1] == b[0] || a[1] == b[1]
}
