//! Source-authorized generated rail contact materialization.

mod geometry;
mod materialization;
mod noding;
mod source_authority;
mod validation;

pub(super) use materialization::{
    append_generated_material_point_contact_constraints,
    append_generated_same_band_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
};
pub(super) use noding::{
    node_generated_contact_contours, node_generated_contact_source_constraints,
    node_generated_contact_sources_from_contour_backed_contacts,
};
pub(super) use source_authority::{
    generated_raised_step_boundary_role_for_owner, raised_step_band_kinds_can_contact,
};
pub(super) use validation::{
    retain_source_authorized_generated_contact_constraints,
    validate_generated_contact_constraint_endpoints_from_sources,
};
