//! Triangle validation router and shared triangle index helpers.

mod coverage;
mod edge_heights;

use super::super::indices::normalized_vertex_edge;
use super::super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulationSolution,
};
use super::boundaries::{
    diagnostic_min_distance_to_boundary_mm,
    edge_lies_on_explicit_boundary_constraint_or_backend_epsilon,
};
use super::report::{
    NodeBoundaryRegionDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, NodeInvalidConstraintReason, push_validation_diagnostic,
};
use super::{
    BoundarySegment, NodeValidationEdgeKey, VALIDATION_MIN_SEGMENT_LENGTH_M, edge_key_for_indices,
    point_key_from_world, quantize_m,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn validate_cross_region_triangle_edge_heights(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    edge_heights::validate_cross_region_triangle_edge_heights(solution, diagnostics);
}

pub(super) fn validate_triangle_area_coverage(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    coverage::validate_triangle_area_coverage(solution, region_index, region, diagnostics);
}

pub(super) fn validate_triangles(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    boundary_segments: &[BoundarySegment],
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<NodeValidationEdgeKey> {
    let boundary_edges = boundary_segments
        .iter()
        .map(|segment| segment.edge)
        .collect::<BTreeSet<_>>();
    let mut triangle_edge_counts = BTreeMap::<[usize; 2], usize>::new();
    for triangle in &region.triangles {
        if !triangle_indices_valid(triangle, region.vertices.len()) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::OutOfRange,
                },
            );
            continue;
        }
        for edge in triangle_edges(triangle) {
            *triangle_edge_counts.entry(edge).or_default() += 1;
        }
    }

    let mut exposed_edges = Vec::new();
    for (edge, count) in triangle_edge_counts {
        if count > 2 {
            let edge_key = edge_key_for_indices(region, edge);
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                    region_index: Some(region_index),
                    regions: vec![NodeBoundaryRegionDiagnostic {
                        region_index,
                        owner: region.owner.kind(),
                        owner_index: region.owner.owner_index(),
                        height_field_id: region.height_field_id,
                    }],
                    start_x_key: edge_key.start.x_key,
                    start_z_key: edge_key.start.z_key,
                    end_x_key: edge_key.end.x_key,
                    end_z_key: edge_key.end.z_key,
                    start_x_mm: edge_key.start.x_mm(),
                    start_z_mm: edge_key.start.z_mm(),
                    end_x_mm: edge_key.end.x_mm(),
                    end_z_mm: edge_key.end.z_mm(),
                    count,
                },
            );
            continue;
        }
        if count != 1 {
            continue;
        }
        let edge_key = edge_key_for_indices(region, edge);
        exposed_edges.push(edge_key);
        if boundary_edges.contains(&edge)
            || edge_lies_on_explicit_boundary_constraint_or_backend_epsilon(
                region,
                edge,
                boundary_segments,
            )
        {
            continue;
        }
        let start_distance_mm = diagnostic_min_distance_to_boundary_mm(
            region.vertices[edge[0]].point_world,
            boundary_segments,
        );
        let end_distance_mm = diagnostic_min_distance_to_boundary_mm(
            region.vertices[edge[1]].point_world,
            boundary_segments,
        );
        for (vertex_index, distance_mm) in
            [(edge[0], start_distance_mm), (edge[1], end_distance_mm)]
        {
            if distance_mm > quantize_m(f64::from(VALIDATION_MIN_SEGMENT_LENGTH_M)) {
                let key = point_key_from_world(region.vertices[vertex_index].point_world);
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::Parry2d,
                    NodeGeometryDiagnosticKind::NonExplicitBoundaryVertex {
                        region_index,
                        owner: region.owner.kind(),
                        owner_index: region.owner.owner_index(),
                        height_field_id: region.height_field_id,
                        x_key: key.x_key,
                        z_key: key.z_key,
                        x_mm: key.x_mm(),
                        z_mm: key.z_mm(),
                        min_boundary_distance_mm: distance_mm,
                    },
                );
            }
        }
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Parry2d,
            NodeGeometryDiagnosticKind::OpenBoundary {
                region_index,
                owner: region.owner.kind(),
                owner_index: region.owner.owner_index(),
                height_field_id: region.height_field_id,
                vertex_index: None,
                x_key: None,
                z_key: None,
                x_mm: None,
                z_mm: None,
                start_x_key: Some(edge_key.start.x_key),
                start_z_key: Some(edge_key.start.z_key),
                end_x_key: Some(edge_key.end.x_key),
                end_z_key: Some(edge_key.end.z_key),
                start_x_mm: Some(edge_key.start.x_mm()),
                start_z_mm: Some(edge_key.start.z_mm()),
                end_x_mm: Some(edge_key.end.x_mm()),
                end_z_mm: Some(edge_key.end.z_mm()),
                degree: 1,
            },
        );
    }
    exposed_edges
}

fn triangle_edges(triangle: &NodeTriangulatedTriangle) -> [[usize; 2]; 3] {
    [
        normalized_vertex_edge(triangle.vertices[0], triangle.vertices[1]),
        normalized_vertex_edge(triangle.vertices[1], triangle.vertices[2]),
        normalized_vertex_edge(triangle.vertices[2], triangle.vertices[0]),
    ]
}

fn triangle_indices_valid(triangle: &NodeTriangulatedTriangle, vertex_count: usize) -> bool {
    triangle.vertices.iter().all(|index| *index < vertex_count)
        && triangle.vertices[0] != triangle.vertices[1]
        && triangle.vertices[1] != triangle.vertices[2]
        && triangle.vertices[2] != triangle.vertices[0]
}

fn edge_indices_valid(edge: [usize; 2], vertex_count: usize) -> bool {
    edge[0] < vertex_count && edge[1] < vertex_count && edge[0] != edge[1]
}
