//! Boolean ownership error diagnostic mapping.

use super::*;

impl NodeGeometryDiagnostic {
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
}
