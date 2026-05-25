//! Whole-solution validation orchestration for triangulated node surfaces.

use super::super::RoadSurfaceSystem;
use super::super::arrangement::owners_form_explicit_vertical_step_pair;
use super::super::triangulation::{NodeTriangulatedRegion, NodeTriangulationSolution};
use super::NodeValidationEdgeKey;
use super::boundaries::validate_boundary_constraints;
use super::crossings::validate_constraint_crossings;
use super::report::{
    NodeBoundaryRegionDiagnostic, NodeGeometryBackend, NodeGeometryDiagnostic,
    NodeGeometryDiagnosticKind, NodeGeometryStage, NodeValidationError, NodeValidationReport,
};
use super::triangles::{
    validate_cross_region_triangle_edge_heights, validate_triangle_area_coverage,
    validate_triangles,
};
use std::collections::{BTreeMap, BTreeSet};

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn validate_node_triangulation_solution(
        solution: &NodeTriangulationSolution,
    ) -> Result<NodeValidationReport, NodeValidationError> {
        NodeValidationReport::from_triangulation_solution(solution)
    }
}

impl NodeValidationReport {
    pub(crate) fn from_triangulation_solution(
        solution: &NodeTriangulationSolution,
    ) -> Result<Self, NodeValidationError> {
        let mut diagnostics = Vec::new();
        let mut exposed_edges = BTreeMap::<NodeValidationEdgeKey, Vec<usize>>::new();
        let mut triangle_count = 0usize;
        let mut exposed_edge_count = 0usize;

        for (region_index, region) in solution.regions.iter().enumerate() {
            let region_exposed_edges =
                validate_region(solution, region_index, region, &mut diagnostics);
            triangle_count += region.triangles.len();
            exposed_edge_count += region_exposed_edges.len();
            for edge in region_exposed_edges {
                exposed_edges.entry(edge).or_default().push(region_index);
            }
        }

        for (edge, region_indices) in exposed_edges {
            if region_indices.len() > 2
                && !duplicate_exposed_edge_has_explicit_owner_context(solution, &region_indices)
            {
                diagnostics.push(NodeGeometryDiagnostic {
                    node_id: solution.node_id,
                    piece_kind: solution.piece_kind,
                    stage: NodeGeometryStage::Validation,
                    backend: NodeGeometryBackend::Parry2d,
                    kind: NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                        region_index: None,
                        regions: region_indices
                            .iter()
                            .filter_map(|region_index| {
                                solution.regions.get(*region_index).map(|region| {
                                    NodeBoundaryRegionDiagnostic {
                                        region_index: *region_index,
                                        owner: region.owner.kind(),
                                        owner_index: region.owner.owner_index(),
                                        height_field_id: region.height_field_id,
                                    }
                                })
                            })
                            .collect(),
                        start_x_key: edge.start.x_key,
                        start_z_key: edge.start.z_key,
                        end_x_key: edge.end.x_key,
                        end_z_key: edge.end.z_key,
                        start_x_mm: edge.start.x_mm(),
                        start_z_mm: edge.start.z_mm(),
                        end_x_mm: edge.end.x_mm(),
                        end_z_mm: edge.end.z_mm(),
                        count: region_indices.len(),
                    },
                });
            }
        }
        validate_cross_region_triangle_edge_heights(solution, &mut diagnostics);

        let report = Self {
            node_id: solution.node_id,
            piece_kind: solution.piece_kind,
            region_count: solution.regions.len(),
            triangle_count,
            exposed_edge_count,
            diagnostics,
        };
        if report.diagnostics.is_empty() {
            Ok(report)
        } else {
            Err(NodeValidationError { report })
        }
    }
}

fn validate_region(
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

fn duplicate_exposed_edge_has_explicit_owner_context(
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
