//! Seam extraction and materialization helpers for node boolean ownership.

mod extraction;
mod materialization;
mod predicates;

use super::RoadSurfaceVisualNodePieceKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ConstraintOverlapMode {
    ExactCanonical,
    TerminalOverlayGridBounded,
}

impl ConstraintOverlapMode {
    pub(super) fn for_piece_kind(piece_kind: RoadSurfaceVisualNodePieceKind) -> Self {
        match piece_kind {
            RoadSurfaceVisualNodePieceKind::Terminal => Self::TerminalOverlayGridBounded,
            RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::JunctionN => {
                Self::ExactCanonical
            }
        }
    }

    pub(super) fn allows_grid_bounded_constraint_overlap(self) -> bool {
        matches!(self, Self::TerminalOverlayGridBounded)
    }

    pub(super) fn cleans_overlay_numeric_spikes(self) -> bool {
        matches!(self, Self::TerminalOverlayGridBounded)
    }
}

pub(super) use extraction::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
pub(super) use materialization::{
    junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    materialize_noded_region_seam_constraints, owned_boundary_requires_explicit_seam,
    owned_source_constraints_for_edge, source_constraints_materialize_raised_step_authority,
};
#[cfg(test)]
pub(super) use predicates::canonicalize_seam_constraints;
