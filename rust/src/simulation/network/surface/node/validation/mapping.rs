//! Error-to-validation-diagnostic mapping.

use super::super::RoadSurfaceVisualNodePieceKind;
use super::super::arrangement::{NodeArrangement, NodeArrangementDiagnostic, NodeArrangementError};
use super::super::height::NodeHeightFieldError;
use super::super::ownership::{
    NodeBooleanOwnershipError, NodeOwnedRegionArrangement, NodeOwnedRegionArrangementDiagnostic,
};
use super::super::rails::NodeRailGenerationError;
use super::super::triangulation::NodeTriangulationError;
use super::report::{
    NodeGeometryBackend, NodeGeometryDiagnostic, NodeGeometryDiagnosticKind, NodeGeometryStage,
    NodeInvalidConstraintReason, NodeRejectedResidualKind, NodeSeamConstraintFailureReason,
    NodeValidationReport,
};

impl NodeValidationReport {
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

    pub(crate) fn from_height_field_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightFieldError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_height_field_error(
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

    pub(crate) fn from_arrangement_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeArrangementError,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic::from_arrangement_error(
            node_id, piece_kind, error,
        ))
    }

    pub(crate) fn from_arrangement_diagnostics(arrangement: &NodeArrangement) -> Option<Self> {
        if arrangement.diagnostics().is_empty() {
            return None;
        }
        Some(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            region_count: arrangement.regions().len(),
            triangle_count: arrangement.faces().len(),
            exposed_edge_count: arrangement.edges().len(),
            diagnostics: arrangement
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    NodeGeometryDiagnostic::from_arrangement_diagnostic(
                        arrangement.node_id(),
                        arrangement.piece_kind(),
                        diagnostic,
                    )
                })
                .collect(),
        })
    }

    pub(crate) fn from_owned_region_arrangement_diagnostics(
        arrangement: &NodeOwnedRegionArrangement,
    ) -> Option<Self> {
        if arrangement.diagnostics().is_empty() {
            return None;
        }
        Some(Self {
            node_id: arrangement.node_id(),
            piece_kind: arrangement.piece_kind(),
            region_count: arrangement.region_count(),
            triangle_count: 0,
            exposed_edge_count: arrangement.edges().len(),
            diagnostics: arrangement
                .diagnostics()
                .iter()
                .map(|diagnostic| {
                    NodeGeometryDiagnostic::from_owned_region_arrangement_diagnostic(
                        arrangement.node_id(),
                        arrangement.piece_kind(),
                        diagnostic,
                    )
                })
                .collect(),
        })
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
}

impl NodeGeometryDiagnostic {
    pub(super) fn from_rail_generation_error(
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
            NodeRailGenerationError::InvalidHeightCarrier { reason, .. } => {
                NodeGeometryDiagnosticKind::BackendFailure { reason }
            }
            NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "noncanonical_generated_contact_endpoint",
                }
            }
            NodeRailGenerationError::TerminalCapGeneration { error } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: error.reason.diagnostic_reason(),
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

    pub(super) fn from_boolean_ownership_error(
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
            NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "noncanonical_owned_region_vertex",
                }
            }
            NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "ambiguous_canonical_owned_region_vertex",
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

    pub(super) fn from_height_field_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeHeightFieldError,
    ) -> Self {
        let kind = match error {
            NodeHeightFieldError::InputOwnershipMismatch { .. } => {
                NodeGeometryDiagnosticKind::HeightFieldFailure {
                    reason: "input_ownership_mismatch",
                    mouth_order_index: None,
                    band_index: None,
                    kind: None,
                    source_kind: None,
                    height_field_id: None,
                    owner: None,
                    point_x_mm: None,
                    point_z_mm: None,
                    axis: None,
                    raw_parameter: None,
                }
            }
            NodeHeightFieldError::DuplicateSourceBand {
                mouth_order_index,
                band_index,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "duplicate_source_band",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::MissingRegionBandIndex {
                mouth_order_index,
                kind,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "missing_region_band_index",
                mouth_order_index: Some(*mouth_order_index),
                band_index: None,
                kind: Some(*kind),
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::MissingSourceBand {
                mouth_order_index,
                band_index,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "missing_source_band",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: None,
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::SourceBandKindMismatch {
                mouth_order_index,
                band_index,
                region_kind,
                source_kind,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "source_band_kind_mismatch",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: Some(*region_kind),
                source_kind: Some(*source_kind),
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::InvalidSourceBandHeightCarrier {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                reason,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason,
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::MissingGeneratedContourHeightPoints {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                ..
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "missing_generated_contour_height_points",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::GeneratedContourMissingSourceBandIndex {
                mouth_order_index,
                source_kind,
                ..
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "generated_contour_missing_source_band_index",
                mouth_order_index: Some(*mouth_order_index),
                band_index: None,
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: None,
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::GeneratedContourMissingSourceBand {
                mouth_order_index,
                band_index,
                source_kind,
                owner,
                ..
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "generated_contour_source_band_missing",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: None,
                owner: *owner,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::GeneratedContourSourceHandoffMismatch {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                point_x_mm,
                point_z_mm,
                ..
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "generated_contour_source_handoff_height_mismatch",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: *owner,
                point_x_mm: Some(*point_x_mm),
                point_z_mm: Some(*point_z_mm),
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::InvalidHeightCarrierContour {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                reason,
                ..
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason,
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: None,
                point_x_mm: None,
                point_z_mm: None,
                axis: None,
                raw_parameter: None,
            },
            NodeHeightFieldError::VertexOutsideHeightField {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                point_x_mm,
                point_z_mm,
                axis,
                raw_parameter,
            } => NodeGeometryDiagnosticKind::HeightFieldFailure {
                reason: "vertex_outside_height_field",
                mouth_order_index: Some(*mouth_order_index),
                band_index: Some(*band_index),
                kind: None,
                source_kind: Some(*source_kind),
                height_field_id: Some(*height_field_id),
                owner: *owner,
                point_x_mm: Some(*point_x_mm),
                point_z_mm: Some(*point_z_mm),
                axis: Some(*axis),
                raw_parameter: Some(*raw_parameter),
            },
            NodeHeightFieldError::TerminalCapGeneration { error } => {
                NodeGeometryDiagnosticKind::HeightFieldFailure {
                    reason: error.reason.diagnostic_reason(),
                    mouth_order_index: Some(error.mouth_order_index),
                    band_index: error.source_band_index,
                    kind: error.band_kind,
                    source_kind: error.band_kind,
                    height_field_id: None,
                    owner: None,
                    point_x_mm: None,
                    point_z_mm: None,
                    axis: None,
                    raw_parameter: None,
                }
            }
            NodeHeightFieldError::SourceHeightFieldConflict {
                mouth_order_index,
                band_index,
                source_kind,
                height_field_id,
                owner,
                existing_authority,
                incoming_authority,
                point_x_mm,
                point_z_mm,
                existing_height_mm,
                incoming_height_mm,
            } => NodeGeometryDiagnosticKind::SourceHeightFieldConflict {
                mouth_order_index: *mouth_order_index,
                band_index: *band_index,
                source_kind: *source_kind,
                height_field_id: *height_field_id,
                owner: *owner,
                existing_authority: *existing_authority,
                incoming_authority: *incoming_authority,
                x_mm: *point_x_mm,
                z_mm: *point_z_mm,
                existing_height_mm: *existing_height_mm,
                incoming_height_mm: *incoming_height_mm,
            },
            NodeHeightFieldError::SharedSourceHeightConflict {
                point_x_mm,
                point_z_mm,
                kind,
                owner,
                opposite_owner,
                height_field_id,
                incoming_owner,
                incoming_height_field_id,
                constraint_index,
                existing_authority,
                incoming_authority,
                existing_height_mm,
                incoming_height_mm,
            } => NodeGeometryDiagnosticKind::SharedSourceHeightConflict {
                x_mm: *point_x_mm,
                z_mm: *point_z_mm,
                kind: *kind,
                owner: *owner,
                opposite_owner: *opposite_owner,
                height_field_id: *height_field_id,
                incoming_owner: *incoming_owner,
                incoming_height_field_id: *incoming_height_field_id,
                constraint_index: *constraint_index,
                existing_authority: *existing_authority,
                incoming_authority: *incoming_authority,
                existing_height_mm: *existing_height_mm,
                incoming_height_mm: *incoming_height_mm,
            },
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::HeightEvaluation,
            backend: NodeGeometryBackend::HeightCarrier,
            kind,
        }
    }

    pub(super) fn from_triangulation_error(
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

    pub(super) fn from_arrangement_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeArrangementError,
    ) -> Self {
        let (backend, kind) = match error {
            NodeArrangementError::DuplicateVertexHeightConflict {
                key,
                existing_height_mm,
                incoming_height_mm,
            } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::HeightConflict {
                    x_mm: key.x_mm(),
                    z_mm: key.z_mm(),
                    existing_height_mm: *existing_height_mm,
                    incoming_height_mm: *incoming_height_mm,
                },
            ),
            NodeArrangementError::EmptyOwnerSet { .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "empty_arrangement_owner_set",
                },
            ),
            NodeArrangementError::MissingGradeAuthority {
                region_index,
                contour_index,
                key,
                owner,
                height_field_id,
                height_mm,
            } => (
                NodeGeometryBackend::HeightCarrier,
                NodeGeometryDiagnosticKind::MissingGradeAuthority {
                    region_index: *region_index,
                    contour_index: *contour_index,
                    x_mm: key.x_mm(),
                    z_mm: key.z_mm(),
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    height_field_id: *height_field_id,
                    height_mm: *height_mm,
                },
            ),
            NodeArrangementError::DegenerateRegionContour { region_index, .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: *region_index,
                    constraint_index: None,
                    reason: NodeInvalidConstraintReason::Degenerate,
                },
            ),
            NodeArrangementError::InputSolutionMismatch { .. }
            | NodeArrangementError::TriangulationRegionCountMismatch { .. }
            | NodeArrangementError::MissingHeightRegion { .. }
            | NodeArrangementError::RegionOwnerMismatch { .. }
            | NodeArrangementError::MissingTriangulatedVertex { .. } => (
                NodeGeometryBackend::Parry2d,
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "arrangement_build_failed",
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: match error {
                NodeArrangementError::MissingGradeAuthority { .. } => NodeGeometryStage::NodeGrade,
                _ => NodeGeometryStage::Validation,
            },
            backend,
            kind,
        }
    }

    pub(super) fn from_arrangement_diagnostic(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        diagnostic: &NodeArrangementDiagnostic,
    ) -> Self {
        let kind = match diagnostic {
            NodeArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => NodeGeometryDiagnosticKind::SeamConstraintFailure {
                region_index: *region_index,
                owner: owner.kind(),
                owner_index: owner.owner_index(),
                opposite_owner: opposite_owner.kind(),
                opposite_owner_index: opposite_owner.owner_index(),
                start_x_mm: start.x_mm(),
                start_z_mm: start.z_mm(),
                end_x_mm: end.x_mm(),
                end_z_mm: end.z_mm(),
                reason: NodeSeamConstraintFailureReason::Missing,
            },
            NodeArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => NodeGeometryDiagnosticKind::SeamConstraintFailure {
                region_index: *region_index,
                owner: owner.kind(),
                owner_index: owner.owner_index(),
                opposite_owner: opposite_owner.kind(),
                opposite_owner_index: opposite_owner.owner_index(),
                start_x_mm: start.x_mm(),
                start_z_mm: start.z_mm(),
                end_x_mm: end.x_mm(),
                end_z_mm: end.z_mm(),
                reason: NodeSeamConstraintFailureReason::Ambiguous,
            },
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Parry2d,
            kind,
        }
    }

    pub(super) fn from_owned_region_arrangement_diagnostic(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        diagnostic: &NodeOwnedRegionArrangementDiagnostic,
    ) -> Self {
        let (backend, kind) = match diagnostic {
            NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::SeamConstraintFailure {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    reason: NodeSeamConstraintFailureReason::Missing,
                },
            ),
            NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
                source_constraint_indices,
            } => (
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::UnmaterializedRaisedStepAuthority {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    source_constraint_indices: source_constraint_indices.clone(),
                },
            ),
            NodeOwnedRegionArrangementDiagnostic::AmbiguousSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            } => (
                NodeGeometryBackend::IOverlay,
                NodeGeometryDiagnosticKind::SeamConstraintFailure {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owner: opposite_owner.kind(),
                    opposite_owner_index: opposite_owner.owner_index(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                    reason: NodeSeamConstraintFailureReason::Ambiguous,
                },
            ),
            NodeOwnedRegionArrangementDiagnostic::AmbiguousOwnedBoundaryEdge {
                region_index,
                owner,
                opposite_owners,
                start,
                end,
            } => (
                NodeGeometryBackend::CanonicalKeys,
                NodeGeometryDiagnosticKind::AmbiguousOwnedBoundaryEdge {
                    region_index: *region_index,
                    owner: owner.kind(),
                    owner_index: owner.owner_index(),
                    opposite_owners: opposite_owners
                        .iter()
                        .map(|owner| (owner.kind(), owner.owner_index()))
                        .collect(),
                    start_x_mm: start.x_mm(),
                    start_z_mm: start.z_mm(),
                    end_x_mm: end.x_mm(),
                    end_z_mm: end.z_mm(),
                },
            ),
        };
        Self {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::BooleanOwnership,
            backend,
            kind,
        }
    }
}
