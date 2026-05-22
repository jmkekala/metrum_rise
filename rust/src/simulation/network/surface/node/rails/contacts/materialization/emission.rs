//! Constraint emission routing for source-authorized generated rail contacts.

mod point_contacts;
mod same_band;

pub(in crate::simulation::network::surface::node::rails) use point_contacts::{
    append_generated_material_point_contact_constraints,
    append_source_authorized_raised_step_point_contacts,
};
pub(in crate::simulation::network::surface::node::rails) use same_band::append_generated_same_band_contact_constraints;
