//! Constraint emission routing for source-authorized generated rail contacts.

mod point_contacts;
mod same_band;

use super::*;

#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) use point_contacts::append_source_authorized_raised_step_point_contacts;
pub(in crate::simulation::network::surface::node::rails) use point_contacts::{
    append_generated_material_point_contact_constraints,
    append_source_authorized_raised_step_point_contacts_with_reuse,
};
pub(in crate::simulation::network::surface::node::rails) use same_band::{
    NodeSameMaterialContactPairCache,
    append_generated_same_band_contact_constraints_with_source_reuse,
};
#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) use same_band::{
    append_generated_same_band_contact_constraints,
    append_generated_same_band_contact_constraints_with_reuse,
};

pub(super) fn source_authority_constraints_for_generated_contacts(
    constraints: &[NodeRailConstraint],
    source_constraint_count: usize,
) -> Vec<NodeRailConstraint> {
    let mut source_constraints = constraints
        .iter()
        .take(source_constraint_count)
        .cloned()
        .collect::<Vec<_>>();
    for constraint in constraints.iter().skip(source_constraint_count) {
        if constraint.kind != NodeRailConstraintKind::RaisedStepContact
            || constraint.points_xz.len() < 2
        {
            continue;
        }
        let start = constraint.points_xz[0];
        let end = *constraint
            .points_xz
            .last()
            .expect("raised-step contact has endpoint");
        if road_point_key(start) == road_point_key(end) {
            source_constraints.push(constraint.clone());
            continue;
        }
        for point in [start, end] {
            let mut endpoint_constraint = constraint.clone();
            endpoint_constraint.points_xz = vec![point, point];
            source_constraints.push(endpoint_constraint);
        }
    }
    source_constraints
}
