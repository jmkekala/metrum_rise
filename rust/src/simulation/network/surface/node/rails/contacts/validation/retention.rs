//! Retention filter for exact source-authorized generated contacts.

use super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::{NodeGeneratedContour, NodeRailConstraint};
use super::authority::ExactGeneratedSourceAuthority;
use super::endpoints::generated_contact_constraint_has_exact_source_authority;

pub(in crate::simulation::network::surface::node::rails) fn retain_source_authorized_generated_contact_constraints(
    contours: &[NodeGeneratedContour],
    authority_constraints: &[NodeRailConstraint],
    constraints: &mut Vec<NodeRailConstraint>,
    generated_constraint_start_index: usize,
) {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        authority_constraints,
        generated_constraint_start_index,
    );
    let mut index = 0usize;
    constraints.retain(|constraint| {
        let retain = index < generated_constraint_start_index
            || generated_contact_kind_from_constraint(constraint.kind).is_none()
            || generated_contact_constraint_has_exact_source_authority(
                constraint,
                &source_authority,
            );
        index += 1;
        retain
    });
}
