// SPDX-License-Identifier: GPL-2.0-only

//! Arrangement diagnostic mapping.

use super::*;

impl NodeGeometryDiagnostic {
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

    pub(in crate::simulation::network::surface::node::validation) fn from_arrangement_diagnostic(
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

    pub(in crate::simulation::network::surface::node::validation) fn from_owned_region_arrangement_diagnostic(
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
