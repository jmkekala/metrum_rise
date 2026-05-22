//! Exact source validation for generated rail contacts.

mod authority;
mod endpoints;
mod retention;

pub(in crate::simulation::network::surface::node::rails) use endpoints::validate_generated_contact_constraint_endpoints_from_sources;
pub(in crate::simulation::network::surface::node::rails) use retention::retain_source_authorized_generated_contact_constraints;
