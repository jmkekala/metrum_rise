//! Boundary constraint validation for triangulated node regions.

use super::super::arrangement::owners_form_explicit_vertical_step_pair;
use super::super::backend::RoadVec3;
use super::super::keys::SURFACE_XZ_KEY_SCALE;
use super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedVertex, NodeTriangulationSolution,
};
use super::crossings::validate_constraint_crossings;
use super::report::{
    NodeGeometryBackend, NodeGeometryDiagnostic, NodeGeometryDiagnosticKind,
    NodeInvalidConstraintReason, push_validation_diagnostic,
};
use super::triangles::{validate_triangle_area_coverage, validate_triangles};
use super::{
    BoundarySegment, NodeValidationEdgeKey, NodeValidationPointKey,
    VALIDATION_DUPLICATE_EXPOSED_EDGE_CANONICAL_DRIFT_M, VALIDATION_MIN_SEGMENT_LENGTH_M,
    edge_key_for_indices, normalized_constraint, point_key_from_world, quantize_m,
};
use parry2d::math::{Pose, Vector};
use parry2d::query::PointQuery;
use parry2d::shape::Segment;
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn duplicate_exposed_edge_has_explicit_owner_context(
    solution: &NodeTriangulationSolution,
    region_indices: &[usize],
) -> bool {
    let mut owners = BTreeSet::new();
    for region_index in region_indices {
        let Some(region) = solution.regions.get(*region_index) else {
            return false;
        };
        owners.insert(region.owner);
    }
    let owners = owners.into_iter().collect::<Vec<_>>();
    if owners.is_empty() {
        return false;
    }
    for (left_index, left) in owners.iter().copied().enumerate() {
        for right in owners.iter().copied().skip(left_index + 1) {
            if left.kind() == right.kind() || owners_form_explicit_vertical_step_pair(left, right) {
                continue;
            }
            return false;
        }
    }
    true
}

pub(super) fn duplicate_exposed_edge_is_canonical_drift(
    solution: &NodeTriangulationSolution,
    edge: NodeValidationEdgeKey,
    region_indices: &[usize],
) -> bool {
    if validation_edge_length_m(edge) > VALIDATION_DUPLICATE_EXPOSED_EDGE_CANONICAL_DRIFT_M {
        return false;
    }

    let mut start_heights = BTreeSet::new();
    let mut end_heights = BTreeSet::new();
    for region_index in region_indices {
        let Some(region) = solution.regions.get(*region_index) else {
            return false;
        };
        let Some(start_height_mm) = region_height_mm_at_key(region, edge.start) else {
            return false;
        };
        let Some(end_height_mm) = region_height_mm_at_key(region, edge.end) else {
            return false;
        };
        start_heights.insert(start_height_mm);
        end_heights.insert(end_height_mm);
    }

    start_heights.len() == 1 && end_heights.len() == 1
}

fn validation_edge_length_m(edge: NodeValidationEdgeKey) -> f64 {
    let dx = (edge.end.x_key - edge.start.x_key) as f64 / SURFACE_XZ_KEY_SCALE;
    let dz = (edge.end.z_key - edge.start.z_key) as f64 / SURFACE_XZ_KEY_SCALE;
    dx.hypot(dz)
}

fn region_height_mm_at_key(
    region: &NodeTriangulatedRegion,
    point: NodeValidationPointKey,
) -> Option<i64> {
    region.vertices.iter().find_map(|vertex| {
        (point_key_from_world(vertex.point_world) == point)
            .then(|| quantize_m(vertex.point_world.y))
    })
}

pub(super) fn validate_region(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<NodeValidationEdgeKey> {
    let boundary_segments =
        validate_boundary_constraints(solution, region_index, region, diagnostics);
    validate_constraint_crossings(solution, region_index, &boundary_segments, diagnostics);
    let exposed_edges = validate_triangles(
        solution,
        region_index,
        region,
        &boundary_segments,
        diagnostics,
    );
    validate_triangle_area_coverage(solution, region_index, region, diagnostics);
    exposed_edges
}

fn validate_boundary_constraints(
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
        let normalized = normalized_constraint(constraint[0], constraint[1]);
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

pub(super) fn min_distance_to_boundary_mm(
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

pub(super) fn edge_lies_on_boundary_constraint(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
    boundary_segments: &[BoundarySegment],
) -> bool {
    let edge_segment = parry_segment_for_edge(region, edge);
    [edge_segment.a, edge_segment.b]
        .into_iter()
        .all(|point| point_lies_on_boundary_constraint(point, boundary_segments))
}

fn point_lies_on_boundary_constraint(point: Vector, boundary_segments: &[BoundarySegment]) -> bool {
    boundary_segments.iter().any(|boundary| {
        boundary
            .segment
            .distance_to_point(&Pose::identity(), point, false)
            <= VALIDATION_MIN_SEGMENT_LENGTH_M
    })
}
