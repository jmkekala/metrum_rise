//! Seam extraction and materialization helpers for node boolean ownership.

mod extraction;
mod materialization;
mod predicates;

pub(super) use extraction::{owned_shape_is_discardable_numeric_dust, seam_constraints_for_shape};
pub(super) use materialization::{
    junctionn_unmaterialized_raised_step_authority_indices_for_edge,
    materialize_noded_region_seam_constraints, owned_boundary_requires_explicit_seam,
    owned_source_constraints_for_edge, source_constraints_materialize_raised_step_authority,
};
#[cfg(test)]
pub(super) use predicates::canonicalize_seam_constraints;
