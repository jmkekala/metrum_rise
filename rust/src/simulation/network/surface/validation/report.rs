//! Validation report types and debug serialization.

use super::super::arrangement::{NodeBandHeightFieldId, NodeBandOwner};
use super::super::height::NodeHeightAuthoritySource;
use super::super::triangulation::NodeTriangulationSolution;
use super::super::{RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind};
use std::fmt::Write as _;

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
    NodeGrade,
    HeightEvaluation,
    Validation,
    CdtTriangulation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NodeGeometryBackend {
    CavalierContours,
    IOverlay,
    HeightCarrier,
    CanonicalKeys,
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
    SourceHeightFieldConflict {
        mouth_order_index: usize,
        band_index: usize,
        source_kind: RoadSurfaceBandKind,
        height_field_id: NodeBandHeightFieldId,
        owner: Option<NodeBandOwner>,
        existing_authority: NodeHeightAuthoritySource,
        incoming_authority: NodeHeightAuthoritySource,
        x_mm: i64,
        z_mm: i64,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    SharedSourceHeightConflict {
        x_mm: i64,
        z_mm: i64,
        kind: RoadSurfaceBandKind,
        owner: NodeBandOwner,
        opposite_owner: Option<NodeBandOwner>,
        height_field_id: Option<NodeBandHeightFieldId>,
        incoming_owner: NodeBandOwner,
        incoming_height_field_id: Option<NodeBandHeightFieldId>,
        constraint_index: Option<usize>,
        existing_authority: Option<NodeHeightAuthoritySource>,
        incoming_authority: Option<NodeHeightAuthoritySource>,
        existing_height_mm: i64,
        incoming_height_mm: i64,
    },
    CrossRegionHeightConflict {
        edge_start_x_key: i64,
        edge_start_z_key: i64,
        edge_end_x_key: i64,
        edge_end_z_key: i64,
        edge_start_x_mm: i64,
        edge_start_z_mm: i64,
        edge_end_x_mm: i64,
        edge_end_z_mm: i64,
        conflict_x_key: i64,
        conflict_z_key: i64,
        conflict_x_mm: i64,
        conflict_z_mm: i64,
        existing_region_index: usize,
        existing_owner: RoadSurfaceBandKind,
        existing_owner_index: usize,
        existing_start_height_mm: i64,
        existing_end_height_mm: i64,
        existing_conflict_height_mm: i64,
        incoming_region_index: usize,
        incoming_owner: RoadSurfaceBandKind,
        incoming_owner_index: usize,
        incoming_start_height_mm: i64,
        incoming_end_height_mm: i64,
        incoming_conflict_height_mm: i64,
        matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
        non_matching_explicit_step_segments: Vec<NodeExplicitStepSegmentDiagnostic>,
    },
    HeightFieldFailure {
        reason: &'static str,
        mouth_order_index: Option<usize>,
        band_index: Option<usize>,
        kind: Option<RoadSurfaceBandKind>,
        source_kind: Option<RoadSurfaceBandKind>,
        height_field_id: Option<NodeBandHeightFieldId>,
        owner: Option<NodeBandOwner>,
        point_x_mm: Option<i64>,
        point_z_mm: Option<i64>,
        axis: Option<&'static str>,
        raw_parameter: Option<f64>,
    },
    MissingGradeAuthority {
        region_index: usize,
        contour_index: usize,
        x_mm: i64,
        z_mm: i64,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        height_field_id: NodeBandHeightFieldId,
        height_mm: i64,
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
    SeamConstraintFailure {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        reason: NodeSeamConstraintFailureReason,
    },
    AmbiguousOwnedBoundaryEdge {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owners: Vec<(RoadSurfaceBandKind, usize)>,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
    },
    UnmaterializedRaisedStepAuthority {
        region_index: usize,
        owner: RoadSurfaceBandKind,
        owner_index: usize,
        opposite_owner: RoadSurfaceBandKind,
        opposite_owner_index: usize,
        start_x_mm: i64,
        start_z_mm: i64,
        end_x_mm: i64,
        end_z_mm: i64,
        source_constraint_indices: Vec<usize>,
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

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum NodeSeamConstraintFailureReason {
    Missing,
    Ambiguous,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NodeExplicitStepSegmentDiagnostic {
    pub(crate) segment_index: usize,
    pub(crate) start_x_key: i64,
    pub(crate) start_z_key: i64,
    pub(crate) end_x_key: i64,
    pub(crate) end_z_key: i64,
    pub(crate) start_x_mm: i64,
    pub(crate) start_z_mm: i64,
    pub(crate) end_x_mm: i64,
    pub(crate) end_z_mm: i64,
    pub(crate) owner: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) opposite_owner: RoadSurfaceBandKind,
    pub(crate) opposite_owner_index: usize,
    pub(crate) owners_match_regions: bool,
    pub(crate) edge_lies_on_segment: bool,
}

impl NodeValidationReport {
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
            // Parry crossing checks are diagnostic only once Spade accepted the constraints and
            // the overlay coverage checks passed. Missing coverage and ownership failures still
            // block export.
            !matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    reason: NodeInvalidConstraintReason::Crossing,
                    ..
                }
            )
        })
    }

    pub(super) fn single_diagnostic(diagnostic: NodeGeometryDiagnostic) -> Self {
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
    pub(super) fn debug_record(&self) -> String {
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
            Self::NodeGrade => "node_grade",
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
            Self::HeightCarrier => "height_carrier",
            Self::CanonicalKeys => "canonical_keys",
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
            Self::HeightConflict { .. } | Self::CrossRegionHeightConflict { .. } => {
                "height_conflict"
            }
            Self::SourceHeightFieldConflict { .. } => "source_height_field_conflict",
            Self::SharedSourceHeightConflict { .. } => "shared_source_height_conflict",
            Self::HeightFieldFailure { .. } => "height_field_failure",
            Self::MissingGradeAuthority { .. } => "missing_grade_authority",
            Self::OpenBoundary { .. } => "open_boundary",
            Self::DuplicateExposedEdge { .. } => "duplicate_exposed_edge",
            Self::InvalidConstraint { .. } => "invalid_constraint",
            Self::TriangleCoverageMismatch { .. } => "triangle_coverage_mismatch",
            Self::TriangleOverlap { .. } => "triangle_overlap",
            Self::SeamConstraintFailure { .. } => "seam_constraint_failure",
            Self::AmbiguousOwnedBoundaryEdge { .. } => "ambiguous_owned_boundary_edge",
            Self::UnmaterializedRaisedStepAuthority { .. } => {
                "unmaterialized_raised_step_authority"
            }
            Self::BackendFailure { .. } => "backend_failure",
        }
    }
}

pub(super) fn push_validation_diagnostic(
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
