// SPDX-License-Identifier: GPL-2.0-only

//! Validation report constructors from stage-specific diagnostics.

use super::*;

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
        Self::from_boundary_export_diagnostic(
            node_id,
            piece_kind,
            NodeGeometryDiagnosticKind::BackendFailure { reason },
        )
    }

    pub(crate) fn from_boundary_export_diagnostic(
        node_id: u32,
        piece_kind: RoadSurfaceVisualNodePieceKind,
        kind: NodeGeometryDiagnosticKind,
    ) -> Self {
        Self::single_diagnostic(NodeGeometryDiagnostic {
            node_id,
            piece_kind,
            stage: NodeGeometryStage::Validation,
            backend: NodeGeometryBackend::Parry2d,
            kind,
        })
    }
}
