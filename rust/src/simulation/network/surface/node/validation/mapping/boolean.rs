// SPDX-License-Identifier: GPL-2.0-only

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
            NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                candidates,
            } => NodeGeometryDiagnosticKind::AmbiguousCanonicalOwnedRegionVertex {
                owner: *owner,
                point_x_key: *point_x_key,
                point_z_key: *point_z_key,
                point_x_mm: SurfaceXzKey::coordinate_key_to_mm(*point_x_key),
                point_z_mm: SurfaceXzKey::coordinate_key_to_mm(*point_z_key),
                candidates: candidates
                    .iter()
                    .copied()
                    .map(NodeCanonicalPointDiagnostic::from_key)
                    .collect(),
            },
            NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
                owner,
                point_x_key,
                point_z_key,
                source_kind,
                source_mouth_order_index,
                source_band_index,
                candidates,
            } => NodeGeometryDiagnosticKind::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
                owner: *owner,
                point_x_key: *point_x_key,
                point_z_key: *point_z_key,
                point_x_mm: SurfaceXzKey::coordinate_key_to_mm(*point_x_key),
                point_z_mm: SurfaceXzKey::coordinate_key_to_mm(*point_z_key),
                source_kind: *source_kind,
                source_mouth_order_index: *source_mouth_order_index,
                source_band_index: *source_band_index,
                candidates: candidates.clone(),
            },
            NodeBooleanOwnershipError::MissingCarrierProvenance {
                owner,
                point_x_key,
                point_z_key,
                source_kind,
                source_mouth_order_index,
                source_band_index,
                height_field_id,
            } => NodeGeometryDiagnosticKind::MissingCarrierProvenance {
                owner: *owner,
                point_x_key: *point_x_key,
                point_z_key: *point_z_key,
                point_x_mm: SurfaceXzKey::coordinate_key_to_mm(*point_x_key),
                point_z_mm: SurfaceXzKey::coordinate_key_to_mm(*point_z_key),
                source_kind: *source_kind,
                source_mouth_order_index: *source_mouth_order_index,
                source_band_index: *source_band_index,
                height_field_id: *height_field_id,
            },
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
