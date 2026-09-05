// SPDX-License-Identifier: GPL-2.0-only

//! Exact source validation for generated rail contacts.

mod authority;
mod endpoints;
mod retention;

#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) use endpoints::validate_generated_contact_constraint_endpoints_from_sources;
pub(in crate::simulation::network::surface::node::rails) use endpoints::validate_generated_contact_constraint_endpoints_with_authority;
pub(in crate::simulation::network::surface::node::rails) use retention::{
    NodeRetainedContactCache, NodeRetainedContactReuseStats,
    retain_source_authorized_generated_contact_constraint_sets_with_reuse,
};
