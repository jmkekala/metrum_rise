// SPDX-License-Identifier: GPL-2.0-only

//! Seam extraction and materialization helpers for node boolean ownership.

mod extraction;
mod materialization;
mod predicates;

use super::RoadSurfaceVisualNodePieceKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConstraintOverlapMode {
    ExactCanonical,
    GridBounded,
}

impl ConstraintOverlapMode {
    pub(super) fn for_piece_kind(piece_kind: RoadSurfaceVisualNodePieceKind) -> Self {
        match piece_kind {
            RoadSurfaceVisualNodePieceKind::JunctionN => Self::GridBounded,
            RoadSurfaceVisualNodePieceKind::Terminal | RoadSurfaceVisualNodePieceKind::Bend => {
                Self::ExactCanonical
            }
        }
    }

    pub(super) fn allows_grid_bounded_constraint_overlap(self) -> bool {
        matches!(self, Self::GridBounded)
    }

    pub(super) fn cleans_overlay_numeric_spikes(self) -> bool {
        let _ = self;
        true
    }
}

#[cfg(test)]
pub(super) use extraction::seam_constraints_for_shape;
pub(super) use extraction::{
    PreparedOwnedShape, PreparedRailConstraintQueryScratch, PreparedRailConstraints,
    owned_shape_is_discardable_numeric_dust,
};
#[cfg(test)]
pub(super) use materialization::materialize_noded_region_seam_constraints;
pub(super) use materialization::{
    OwnedEdgeRailConstraintIndex, junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    materialize_noded_region_seam_constraints_from_boundary_refs_with_reuse,
    materialized_endpoint_pair_constraint_indices_for_owned_edge,
    owned_boundary_requires_explicit_seam, owned_source_constraints_for_edge,
    source_constraints_materialize_raised_step_authority,
};
#[cfg(test)]
pub(super) use predicates::canonicalize_seam_constraints;
