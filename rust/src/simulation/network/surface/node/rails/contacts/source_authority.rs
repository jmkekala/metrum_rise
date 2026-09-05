// SPDX-License-Identifier: GPL-2.0-only

//! Explicit source-authority support for generated rail contacts.

mod kinds;
mod raised_steps;
mod target_groups;
mod types;

pub(super) use kinds::{
    generated_contact_kind_from_constraint, generated_raised_step_contact_kind_for_owners,
    generated_same_band_contact_constraint, generated_same_band_contact_constraint_key,
};
pub(in crate::simulation::network::surface::node::rails) use kinds::{
    generated_raised_step_boundary_role_for_owner, raised_step_band_kinds_can_contact,
};
pub(in crate::simulation::network::surface::node::rails) use raised_steps::{
    NodeSourceAuthorizedContactCache, SourceAuthorizedContactReuseStats,
    collect_source_authorized_raised_step_contacts_with_reuse,
};
pub(super) use types::GeneratedSameBandContactConstraint;
