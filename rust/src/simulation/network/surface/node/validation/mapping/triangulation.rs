// SPDX-License-Identifier: GPL-2.0-only

//! CDT triangulation error diagnostic mapping.

use super::*;

impl NodeGeometryDiagnostic {
    pub(super) fn from_triangulation_error(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        error: &NodeTriangulationError,
    ) -> Self {
        let (backend, kind) = match error {
            NodeTriangulationError::InvalidConstraint {
                region_index,
                first_constraint_index,
                ..
            } => (
                NodeGeometryBackend::Spade,
                NodeGeometryDiagnosticKind::InvalidConstraint {
                    region_index: *region_index,
                    constraint_index: *first_constraint_index,
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
}
