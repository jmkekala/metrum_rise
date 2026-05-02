//! Structured validation and diagnostics for canonical node surface compilation.

#![allow(dead_code)]

use super::height::NodeHeightSourceError;
use super::ownership::NodeBooleanOwnershipError;
use super::rails::NodeRailGenerationError;
use super::triangulation::{
    NodeTriangulatedRegion, NodeTriangulatedTriangle, NodeTriangulatedVertex,
    NodeTriangulationError, NodeTriangulationSolution,
};
use super::{
    NodeOverlayContour, RoadSurfaceBandKind, RoadSurfaceSystem, RoadSurfaceVisualNodePieceKind,
};
use parry2d::math::{Pose, Vector};
use parry2d::query::PointQuery;
use parry2d::shape::{Segment, SegmentPointLocation};
use parry2d::utils::{SegmentsIntersection, segments_intersection2d};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

const VALIDATION_KEY_SCALE: f64 = 1000.0;
const VALIDATION_MIN_SEGMENT_LENGTH_M: f32 = 0.000001;
const VALIDATION_PARALLEL_EPSILON_M: f32 = 0.001;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationReport {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) region_count: usize,
    pub(crate) triangle_count: usize,
    pub(crate) exposed_edge_count: usize,
    pub(crate) diagnostics: Vec<NodeGeometryDiagnostic>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeValidationError {
    pub(crate) report: NodeValidationReport,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeGeometryDiagnostic {
    pub(crate) node_id: u32,
    pub(crate) piece_kind: RoadSurfaceVisualNodePieceKind,
    pub(crate) stage: NodeGeometryStage,
    pub(crate) backend: NodeGeometryBackend,
    pub(crate) kind: NodeGeometryDiagnosticKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryStage {
    ContourGeneration,
    BooleanOwnership,
    HeightEvaluation,
    Validation,
    CdtTriangulation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryBackend {
    CavalierContours,
    IOverlay,
    Splines,
    Parry2d,
    Spade,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeGeometryDiagnosticKind {
    RejectedResidual {
        residual: NodeRejectedResidualKind,
        shape_count: usize,
        area_m2: f32,
    },
    NonExplicitBoundaryVertex {
        region_index: usize,
        x_mm: i64,
        z_mm: i64,
        min_boundary_distance_mm: i64,
    },
    HeightConflict {
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    OpenBoundary {
        region_index: usize,
        vertex_index: Option<usize>,
        degree: usize,
    },
    DuplicateExposedEdge {
        region_index: Option<usize>,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        count: usize,
    },
    InvalidConstraint {
        region_index: usize,
        constraint_index: Option<usize>,
        reason: NodeInvalidConstraintReason,
    },
    TriangleCoverageMismatch {
        region_index: usize,
        missing_area_m2: f32,
        extra_area_m2: f32,
    },
    TriangleOverlap {
        region_index: usize,
        overlap_area_m2: f32,
    },
    BackendFailure {
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeRejectedResidualKind {
    Asphalt,
    Band(RoadSurfaceBandKind),
    NonRoad,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeInvalidConstraintReason {
    Degenerate,
    OutOfRange,
    Crossing,
    Duplicate,
    CdtRejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationPointKey {
    x_mm: i64,
    z_mm: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationEdgeKey {
    start: NodeValidationPointKey,
    end: NodeValidationPointKey,
}

#[derive(Clone, Copy)]
struct BoundarySegment {
    index: usize,
    edge: [usize; 2],
    segment: Segment,
}

impl RoadSurfaceSystem {
    pub(super) fn validate_node_triangulation_solution(
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
            if region_indices.len() > 2 {
                diagnostics.push(NodeGeometryDiagnostic {
                    node_id: solution.node_id,
                    piece_kind: solution.piece_kind,
                    stage: NodeGeometryStage::Validation,
                    backend: NodeGeometryBackend::Parry2d,
                    kind: NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                        region_index: None,
                        start_x_mm: edge.start.x_mm,
                        start_z_mm: edge.start.z_mm,
                        end_x_mm: edge.end.x_mm,
                        end_z_mm: edge.end.z_mm,
                        count: region_indices.len(),
                    },
                });
            }
        }

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

    pub(crate) fn debug_dump(&self) -> String {
        let mut dump = String::new();
        let _ = write!(
            dump,
            "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"region_count\":{},\"triangle_count\":{},\"exposed_edge_count\":{},\"diagnostics\":[",
            self.node_id,
            self.piece_kind,
            self.region_count,
            self.triangle_count,
            self.exposed_edge_count
        );
        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            if index > 0 {
                let _ = write!(dump, ",");
            }
            let _ = write!(dump, "{}", diagnostic.debug_record());
        }
        let _ = write!(dump, "]}}");
        dump
    }

    pub(crate) fn has_blocking_diagnostics(&self) -> bool {
        self.diagnostics.iter().any(|diagnostic| {
            !matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
        })
    }

    pub(crate) fn from_rail_generation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeRailGenerationError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_rail_generation_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_boolean_ownership_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeBooleanOwnershipError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_boolean_ownership_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_height_source_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightSourceError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_height_source_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_triangulation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeTriangulationError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_triangulation_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_boundary_export_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        reason: &'static str,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Parry2d,
            kind: NodeGeometryDiagnosticKind::BackendFailure { reason },
        })
    }

    fn single_diagnostic(diagnostic: NodeGeometryDiagnostic) -> Self {
        Self {
            node_id: diagnostic.node_id,
            piece_kind: diagnostic.piece_kind,
            region_count: 0,
            triangle_count: 0,
            exposed_edge_count: 0,
            diagnostics: vec![diagnostic],
        }
    }
}

impl NodeGeometryDiagnostic {
    fn from_rail_generation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeRailGenerationError,
    ) -> Self {
        let kind = match error {
            NodeRailGenerationError::DegenerateConstraint { .. } => {
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: 0,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                }
            }
            NodeRailGenerationError::DegenerateContour { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "degenerate_contour",
                }
            }
            NodeRailGenerationError::EmptyInput { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_input",
                }
            }
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::ContourGeneration,
            backend: NodeGeometryBackend::CavalierContours,
            kind,
        }
    }

    fn from_boolean_ownership_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeBooleanOwnershipError,
    ) -> Self {
        let kind = match error {
            NodeBooleanOwnershipError::UnownedAsphaltResidual {
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::Asphalt,
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::UnownedBandResidual {
                kind,
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::Band(*kind),
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::UnownedNonRoadResidual {
                shape_count,
                area_m2,
            } => NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::NonRoad,
                shape_count: *shape_count,
                area_m2: *area_m2,
            },
            NodeBooleanOwnershipError::BooleanOperationFailed { stage } => {
                NodeGeometryDiagnosticKind::BackendFailure { reason: stage }
            }
            NodeBooleanOwnershipError::MissingBandOwner { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "missing_band_owner",
                }
            }
            NodeBooleanOwnershipError::EmptyContourSet { .. }
            | NodeBooleanOwnershipError::EmptyFootprint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_boolean_input",
                }
            }
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::BooleanOwnership,
            backend: NodeGeometryBackend::IOverlay,
            kind,
        }
    }

    fn from_height_source_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightSourceError,
    ) -> Self {
        let kind = match error {
            NodeHeightSourceError::MissingSourceBand { .. }
            | NodeHeightSourceError::MissingRegionBandIndex { .. }
            | NodeHeightSourceError::SourceBandKindMismatch { .. }
            | NodeHeightSourceError::InputOwnershipMismatch { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "invalid_height_source",
                }
            }
            NodeHeightSourceError::DegenerateHeightField { .. }
            | NodeHeightSourceError::VertexOutsideHeightField { .. }
            | NodeHeightSourceError::HeightSampleFailed { .. }
            | NodeHeightSourceError::DuplicateSourceBand { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "height_evaluation_failed",
                }
            }
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::HeightEvaluation,
            backend: NodeGeometryBackend::Splines,
            kind,
        }
    }

    fn from_triangulation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeTriangulationError,
    ) -> Self {
        let (backend, kind) = match error {
            NodeTriangulationError::InvalidConstraint { .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: 0,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::CdtRejected,
                },
            ),
            NodeTriangulationError::DuplicateVertexHeightConflict {
                x_mm,
                z_mm,
                existing_height_mm,
                incoming_height_mm,
                ..
            } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::HeightConflict {
                    x_mm: *x_mm,
                    z_mm: *z_mm,
                    existing_height_mm: *existing_height_mm,
                    incoming_height_mm: *incoming_height_mm,
                },
            ),
            NodeTriangulationError::TriangleCoverageMismatch {
                region_index,
                missing_area_m2,
                extra_area_m2,
                ..
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::TriangleCoverageMismatch {
                    region_index: *region_index,
                    missing_area_m2: *missing_area_m2,
                    extra_area_m2: *extra_area_m2,
                },
            ),
            NodeTriangulationError::BooleanOperationFailed { stage, .. } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::BackendFailure { reason: stage },
            ),
            NodeTriangulationError::DegenerateRegionContour { region_index, .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: *region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            ),
            NodeTriangulationError::EmptyHeightSolution { .. }
            | NodeTriangulationError::EmptyRegionShape { .. }
            | NodeTriangulationError::CdtBuildFailed { .. }
            | NodeTriangulationError::EmptyTriangulation { .. } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "cdt_triangulation_failed",
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::CdtTriangulation,
            backend,
            kind,
        }
    }

    fn debug_record(&self) -> String {
        format!(
            "{{\"node_id\":{},\"piece_kind\":\"{:?}\",\"stage\":\"{}\",\"backend\":\"{}\",\"kind\":\"{}\",\"detail\":\"{:?}\"}}",
            self.node_id,
            self.piece_kind,
            self.stage.as_str(),
            self.backend.as_str(),
            self.kind.as_str(),
            self.kind
        )
    }
}

impl NodeGeometryStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::ContourGeneration => "contour_generation",
            Self::BooleanOwnership => "boolean_ownership",
            Self::HeightEvaluation => "height_evaluation",
            Self::Validation => "validation",
            Self::CdtTriangulation => "cdt_triangulation",
        }
    }
}

impl NodeGeometryBackend {
    fn as_str(self) -> &'static str {
        match self {
            Self::CavalierContours => "cavalier_contours",
            Self::IOverlay => "i_overlay",
            Self::Splines => "splines",
            Self::Parry2d => "parry2d",
            Self::Spade => "spade",
        }
    }
}

impl NodeGeometryDiagnosticKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::RejectedResidual { .. } => "rejected_residual",
            Self::NonExplicitBoundaryVertex { .. } => "non_explicit_boundary_vertex",
            Self::HeightConflict { .. } => "height_conflict",
            Self::OpenBoundary { .. } => "open_boundary",
            Self::DuplicateExposedEdge { .. } => "duplicate_exposed_edge",
            Self::InvalidConstraint { .. } => "invalid_constraint",
            Self::TriangleCoverageMismatch { .. } => "triangle_coverage_mismatch",
            Self::TriangleOverlap { .. } => "triangle_overlap",
            Self::BackendFailure { .. } => "backend_failure",
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

fn validate_boundary_constraints(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) -> Vec<BoundarySegment> {
    let mut seen_constraints = BTreeSet::new();
    let mut boundary_degree = BTreeMap::<usize, usize>::new();
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
        if !seen_constraints.insert(normalized) {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index,
                    constraint_index: Some(constraint_index),
                    reason: NodeInvalidConstraintReason::Duplicate,
                },
            );
            continue;
        }

        let segment = parry_segment_for_edge(region, normalized);
        if segment.length() <= VALIDATION_MIN_SEGMENT_LENGTH_M {
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
        *boundary_degree.entry(normalized[0]).or_default() += 1;
        *boundary_degree.entry(normalized[1]).or_default() += 1;
        boundary_segments.push(BoundarySegment {
            index: constraint_index,
            edge: normalized,
            segment,
        });
    }

    for (vertex_index, degree) in boundary_degree {
        if degree != 2 {
            push_validation_diagnostic(
                solution,
                diagnostics,
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::OpenBoundary {
                    region_index,
                    vertex_index: Some(vertex_index),
                    degree,
                },
            );
        }
    }
    boundary_segments
}

fn validate_constraint_crossings(
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
            if segments_intersection2d(
                first.segment.a,
                first.segment.b,
                second.segment.a,
                second.segment.b,
                VALIDATION_PARALLEL_EPSILON_M,
            )
            .is_some_and(|intersection| strict_intersection(intersection))
            {
                push_validation_diagnostic(
                    solution,
                    diagnostics,
                    NodeGeometryBackend::Parry2d,
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

fn validate_triangles(
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
                    start_x_mm: edge_key.start.x_mm,
                    start_z_mm: edge_key.start.z_mm,
                    end_x_mm: edge_key.end.x_mm,
                    end_z_mm: edge_key.end.z_mm,
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
            || edge_lies_on_boundary_constraint(region, edge, boundary_segments)
        {
            continue;
        }
        let start_distance_mm =
            min_distance_to_boundary_mm(region.vertices[edge[0]].point_world, boundary_segments);
        let end_distance_mm =
            min_distance_to_boundary_mm(region.vertices[edge[1]].point_world, boundary_segments);
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
                        x_mm: key.x_mm,
                        z_mm: key.z_mm,
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
                vertex_index: None,
                degree: 1,
            },
        );
    }
    exposed_edges
}

fn validate_triangle_area_coverage(
    solution: &NodeTriangulationSolution,
    region_index: usize,
    region: &NodeTriangulatedRegion,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
) {
    if region.triangles.is_empty() {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::Spade,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "empty_triangle_set",
            },
        );
        return;
    }
    let triangle_contours = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_contour(region, triangle))
        .collect::<Vec<_>>();
    let Some(triangle_shapes) = RoadSurfaceSystem::overlay_union_contours(&triangle_contours)
    else {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::BackendFailure {
                reason: "triangle_union_failed",
            },
        );
        return;
    };
    let union_area = triangle_shapes
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .sum::<f32>();
    let triangle_area_sum = region
        .triangles
        .iter()
        .filter(|triangle| triangle_indices_valid(triangle, region.vertices.len()))
        .map(|triangle| triangle_area_m2(region, triangle))
        .sum::<f32>();
    let overlap_area_m2 = (triangle_area_sum - union_area).max(0.0);
    let area_budget_m2 =
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&triangle_shapes);
    if overlap_area_m2 > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleOverlap {
                region_index,
                overlap_area_m2,
            },
        );
    }

    let area_delta = union_area - region.area_m2;
    if area_delta.abs() > area_budget_m2 {
        push_validation_diagnostic(
            solution,
            diagnostics,
            NodeGeometryBackend::IOverlay,
            NodeGeometryDiagnosticKind::TriangleCoverageMismatch {
                region_index,
                missing_area_m2: (-area_delta).max(0.0),
                extra_area_m2: area_delta.max(0.0),
            },
        );
    }
}

fn push_validation_diagnostic(
    solution: &NodeTriangulationSolution,
    diagnostics: &mut Vec<NodeGeometryDiagnostic>,
    backend: NodeGeometryBackend,
    kind: NodeGeometryDiagnosticKind,
) {
    diagnostics.push(NodeGeometryDiagnostic {
        node_id: solution.node_id,
        piece_kind: solution.piece_kind,
        stage: NodeGeometryStage::Validation,
        backend,
        kind,
    });
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

fn min_distance_to_boundary_mm(
    point: super::backend::RoadVec3,
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

fn edge_lies_on_boundary_constraint(
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

fn strict_intersection(intersection: SegmentsIntersection) -> bool {
    match intersection {
        SegmentsIntersection::Point { loc1, loc2 } => {
            matches!(loc1, SegmentPointLocation::OnEdge(_))
                && matches!(loc2, SegmentPointLocation::OnEdge(_))
        }
        SegmentsIntersection::Segment { .. } => true,
    }
}

fn triangle_edges(triangle: &NodeTriangulatedTriangle) -> [[usize; 2]; 3] {
    [
        normalized_constraint(triangle.vertices[0], triangle.vertices[1]),
        normalized_constraint(triangle.vertices[1], triangle.vertices[2]),
        normalized_constraint(triangle.vertices[2], triangle.vertices[0]),
    ]
}

fn triangle_indices_valid(triangle: &NodeTriangulatedTriangle, vertex_count: usize) -> bool {
    triangle.vertices.iter().all(|index| *index < vertex_count)
        && triangle.vertices[0] != triangle.vertices[1]
        && triangle.vertices[1] != triangle.vertices[2]
        && triangle.vertices[2] != triangle.vertices[0]
}

fn triangle_contour(
    region: &NodeTriangulatedRegion,
    triangle: &NodeTriangulatedTriangle,
) -> NodeOverlayContour {
    let mut contour = triangle
        .vertices
        .iter()
        .map(|index| {
            let point = region.vertices[*index].point_world;
            [point.x, point.z]
        })
        .collect::<Vec<_>>();
    if signed_overlay_area_m2(&contour) < 0.0 {
        contour.swap(1, 2);
    }
    contour
}

fn triangle_area_m2(region: &NodeTriangulatedRegion, triangle: &NodeTriangulatedTriangle) -> f32 {
    signed_overlay_area_m2(&triangle_contour(region, triangle)).abs()
}

fn signed_overlay_area_m2(contour: &NodeOverlayContour) -> f32 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for index in 0..contour.len() {
        let start = contour[index];
        let end = contour[(index + 1) % contour.len()];
        area += start[0] * end[1] - end[0] * start[1];
    }
    (area * 0.5) as f32
}

fn edge_key_for_indices(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
) -> NodeValidationEdgeKey {
    NodeValidationEdgeKey::new(
        point_key_from_world(region.vertices[edge[0]].point_world),
        point_key_from_world(region.vertices[edge[1]].point_world),
    )
}

fn point_key_from_world(point: super::backend::RoadVec3) -> NodeValidationPointKey {
    NodeValidationPointKey {
        x_mm: quantize_m(point.x),
        z_mm: quantize_m(point.z),
    }
}

fn normalized_constraint(a: usize, b: usize) -> [usize; 2] {
    if a < b { [a, b] } else { [b, a] }
}

fn shares_endpoint(a: [usize; 2], b: [usize; 2]) -> bool {
    a[0] == b[0] || a[0] == b[1] || a[1] == b[0] || a[1] == b[1]
}

fn quantize_m(value: f64) -> i64 {
    (value * VALIDATION_KEY_SCALE).round() as i64
}

impl NodeValidationEdgeKey {
    fn new(a: NodeValidationPointKey, b: NodeValidationPointKey) -> Self {
        if a <= b {
            Self { start: a, end: b }
        } else {
            Self { start: b, end: a }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::surface::height::NodeHeightSolution;
    use crate::simulation::network::surface::input::NodeArrangementInput;
    use crate::simulation::network::surface::ownership::NodeBooleanOwnership;
    use crate::simulation::network::surface::rails::NodeRailContourSet;
    use crate::simulation::network::surface::{
        IncidentEdgeSide, IncidentMouthBand, IncidentMouthProfile, OrderedIncidentPieceMouth,
    };
    use godot::prelude::{Vector2, Vector3};

    fn band(kind: RoadSurfaceBandKind, start: Vector3, end: Vector3) -> IncidentMouthBand {
        IncidentMouthBand {
            kind,
            start_point_world: start,
            end_point_world: end,
        }
    }

    fn profile(x: f32, base_height: f32) -> IncidentMouthProfile {
        let boundary_points_world = vec![
            Vector3::new(x, base_height, -4.0),
            Vector3::new(x, base_height + 0.1, -2.0),
            Vector3::new(x, base_height + 0.2, 0.0),
            Vector3::new(x, base_height + 0.3, 2.0),
            Vector3::new(x, base_height + 0.4, 4.0),
        ];
        let bands = vec![
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[0],
                boundary_points_world[1],
            ),
            band(
                RoadSurfaceBandKind::CurbOrShoulder,
                boundary_points_world[1],
                boundary_points_world[2],
            ),
            band(
                RoadSurfaceBandKind::Carriageway,
                boundary_points_world[2],
                boundary_points_world[3],
            ),
            band(
                RoadSurfaceBandKind::Sidewalk,
                boundary_points_world[3],
                boundary_points_world[4],
            ),
        ];
        IncidentMouthProfile {
            inward_direction_xz: Vector2::RIGHT,
            boundary_points_world,
            bands,
        }
    }

    fn solved_triangulation() -> NodeTriangulationSolution {
        let mouth = OrderedIncidentPieceMouth {
            profile: profile(10.0, 4.0),
            endpoint_profile: profile(0.0, 2.0),
            direction_angle_ccw: 0.0,
            direction_xz: Vector2::RIGHT,
            edge_idx: 7,
            side: IncidentEdgeSide::Start,
        };
        let input = NodeArrangementInput::from_ordered_mouths(
            42,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &[mouth],
        )
        .expect("test mouth should produce canonical input");
        let rails =
            NodeRailContourSet::from_input(&input).expect("test input should produce rails");
        let ownership =
            NodeBooleanOwnership::from_rails(&rails).expect("test rails should produce ownership");
        let heights = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
            .expect("test ownership should height canonical regions");
        NodeTriangulationSolution::from_height_solution(&heights)
            .expect("test heights should triangulate")
    }

    #[test]
    fn validates_clean_triangulated_solution() {
        let solution = solved_triangulation();
        let report = NodeValidationReport::from_triangulation_solution(&solution)
            .expect("fresh triangulation should validate");

        assert_eq!(report.node_id, 42);
        assert_eq!(report.piece_kind, RoadSurfaceVisualNodePieceKind::JunctionN);
        assert_eq!(report.region_count, solution.regions.len());
        assert!(report.triangle_count > 0);
        assert!(report.exposed_edge_count > 0);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn reports_open_boundaries_with_stage_and_backend() {
        let mut solution = solved_triangulation();
        solution.regions[0].boundary_constraints.pop();

        let error = NodeValidationReport::from_triangulation_solution(&solution)
            .expect_err("missing explicit boundary constraint must fail validation");

        assert!(error.report.diagnostics.iter().any(|diagnostic| {
            diagnostic.stage == NodeGeometryStage::Validation
                && diagnostic.backend == NodeGeometryBackend::Parry2d
                && matches!(
                    diagnostic.kind,
                    NodeGeometryDiagnosticKind::OpenBoundary { .. }
                )
        }));
        let dump = error.report.debug_dump();
        assert!(dump.contains("\"stage\":\"validation\""));
        assert!(dump.contains("\"backend\":\"parry2d\""));
        assert!(dump.contains("\"kind\":\"open_boundary\""));
    }

    #[test]
    fn reports_crossing_constraints() {
        let mut solution = solved_triangulation();
        let region = &mut solution.regions[0];
        region.boundary_constraints = vec![[0, 2], [1, 3], [0, 1], [2, 3]];

        let error = NodeValidationReport::from_triangulation_solution(&solution)
            .expect_err("crossing constraints must fail validation");

        assert!(error.report.diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
        }));
    }

    #[test]
    fn maps_boolean_residual_to_structured_debug_record() {
        let report = NodeValidationReport::from_boolean_ownership_error(
            8,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &NodeBooleanOwnershipError::UnownedNonRoadResidual {
                shape_count: 2,
                area_m2: 0.5,
            },
        );

        let diagnostic = &report.diagnostics[0];
        assert_eq!(diagnostic.stage, NodeGeometryStage::BooleanOwnership);
        assert_eq!(diagnostic.backend, NodeGeometryBackend::IOverlay);
        assert!(matches!(
            diagnostic.kind,
            NodeGeometryDiagnosticKind::RejectedResidual {
                residual: NodeRejectedResidualKind::NonRoad,
                ..
            }
        ));
        assert!(
            report
                .debug_dump()
                .contains("\"kind\":\"rejected_residual\"")
        );
    }
}
