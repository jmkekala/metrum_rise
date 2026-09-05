// SPDX-License-Identifier: GPL-2.0-only

//! Rail generation error diagnostic mapping.

use super::*;

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
            NodeRailGenerationError::ConflictingHeightCarrierPoint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "conflicting_height_carrier_point",
                }
            }
            NodeRailGenerationError::MissingCarrierProvenanceHeight { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "missing_carrier_provenance_height",
                }
            }
            NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint { .. } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: "noncanonical_generated_contact_endpoint",
                }
            }
            NodeRailGenerationError::SideJoinGeneration { error } => {
                NodeGeometryDiagnosticKind::BackendFailure {
                    reason: error.reason,
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
}
