//! Boundary constraint validation for triangulated node regions.

use super::super::backend::RoadVec3;
use super::super::indices::normalized_vertex_edge;
use super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedVertex, NodeTriangulationSolution,
};
use super::report::{
    NodeGeometryBackend, NodeGeometryDiagnostic, NodeGeometryDiagnosticKind,
    NodeInvalidConstraintReason, push_validation_diagnostic,
};
use super::{
    BoundarySegment, NodeValidationPointKey, VALIDATION_MIN_SEGMENT_LENGTH_M, edge_key_for_indices,
    quantize_m,
};
use parry2d::math::{Pose, Vector};
use parry2d::query::PointQuery;
use parry2d::shape::Segment;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_boundary_constraints(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<BoundarySegment> {
    let mut seen_constraints = BTreeSet::new();
    let mut boundary_degree = BTreeMap::<NodeValidationPointKey, usize>::new();
    let mut boundary_segments = Vec::with_capacity(region.boundary_constraints.len());

    for (constraint_index, constraint) in region.boundary_constraints.iter().copied().enumerate() {
        if constraint[0] >= region.vertices.len() || constraint[1] >= region.vertices.len() {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::OutOfRange,
                },
            );
            continue;
        }
        if constraint[0] == constraint[1] {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            );
            continue;
        }
        let normalized = normalized_vertex_edge(constraint[0], constraint[1]);
        let key_edge = edge_key_for_indices(region, normalized);
        if key_edge.is_degenerate() {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            );
            continue;
        }
        if !seen_constraints.insert(key_edge) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Duplicate,
                },
            );
            continue;
        }

        // Constraint identity is the canonical vertex pair, not the f32 Parry segment length.
        // Overlay-grid-distinct endpoint connectors can collapse after the f32 conversion.
        let segment = parry_segment_for_edge(region, normalized);
        *boundary_degree.entry(key_edge.start).or_default() += 1;
        *boundary_degree.entry(key_edge.end).or_default() += 1;
        boundary_segments.push(BoundarySegment {
            index: constraint_index,
            edge: normalized,
            key_edge,
            segment,
        });
    }

    for (_point_key, degree) in boundary_degree {
        if degree != 2 {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::OpenBoundary {
                    region_index,
                    vertex_index: None,
                    degree,
                },
            );
        }
    }
    boundary_segments
}

fn parry_segment_for_edge(region: &NodeTriangulatedRegion, edge: [usize; 2]) -> Segment {
    Segment::new(
        parry_point_from_vertex(&region.vertices[edge[0]]),
        parry_point_from_vertex(&region.vertices[edge[1]]),
    )
}

fn parry_point_from_vertex(vertex: &NodeTriangulatedVertex) -> Vector {
    Vector::new(vertex.point_world.x as f32, vertex.point_world.z as f32)
}

pub(super) fn diagnostic_min_distance_to_boundary_mm(
    point: RoadVec3,
    boundary_segments: &[BoundarySegment],
) -> i64 {
    let point = Vector::new(point.x as f32, point.z as f32);
    boundary_segments
        .iter()
        .map(|segment| {
            segment
                .segment
                .distance_to_point(&Pose::identity(), point, false)
        })
        .min_by(|a, b| a.total_cmp(b))
        .map(|distance| quantize_m(f64::from(distance)))
        .unwrap_or(i64::MAX)
}

pub(super) fn edge_lies_on_explicit_boundary_constraint_or_backend_epsilon(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
    boundary_segments: &[BoundarySegment],
) -> bool {
    let edge_key = edge_key_for_indices(region, edge);
    if boundary_segments.iter().any(|boundary| {
        let start = boundary.key_edge.start.surface_key();
        let end = boundary.key_edge.end.surface_key();
        edge_key
            .start
            .surface_key()
            .lies_exactly_on_segment(start, end)
            && edge_key
                .end
                .surface_key()
                .lies_exactly_on_segment(start, end)
    }) {
        return true;
    }

    // This is a backend validation tolerance for Spade/Parry geometry checks only. It does not
    // change topology, pick a boundary owner, or supply heights.
    let edge_segment = parry_segment_for_edge(region, edge);
    [edge_segment.a, edge_segment.b]
        .into_iter()
        .all(|point| point_lies_on_boundary_constraint_by_backend_epsilon(point, boundary_segments))
}

fn point_lies_on_boundary_constraint_by_backend_epsilon(
    point: Vector,
    boundary_segments: &[BoundarySegment],
) -> bool {
    boundary_segments.iter().any(|boundary| {
        boundary
            .segment
            .distance_to_point(&Pose::identity(), point, false)
            <= VALIDATION_MIN_SEGMENT_LENGTH_M
    })
}
