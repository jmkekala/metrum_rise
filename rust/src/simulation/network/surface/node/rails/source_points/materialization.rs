//! Height-carrier point materialization from rail source provenance.

mod constraints;
mod owned_regions;

pub(in crate::simulation::network::surface::node::rails) use constraints::push_source_constraint_height_carrier_points;
pub(in crate::simulation::network::surface::node::rails) use owned_regions::push_owned_region_height_carrier_points;
