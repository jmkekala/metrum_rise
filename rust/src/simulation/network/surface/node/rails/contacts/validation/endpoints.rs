//! Generated-contact endpoint validation against exact source authority.

#[cfg(test)]
use super::super::NodeGeneratedContour;
use super::super::source_authority::generated_contact_kind_from_constraint;
use super::super::{
    NodeBandOwner, NodeRailConstraint, NodeRailGenerationError, NodeRailPointKey, road_point_key,
};
use super::authority::ExactGeneratedSourceAuthority;

#[cfg(test)]
pub(in crate::simulation::network::surface::node::rails) fn validate_generated_contact_constraint_endpoints_from_sources(
    contours: &[NodeGeneratedContour],
    constraints: &[NodeRailConstraint],
    generated_constraint_start_index: usize,
) -> Result<(), NodeRailGenerationError> {
    let source_authority = ExactGeneratedSourceAuthority::from_sources(
        contours,
        constraints,
        generated_constraint_start_index,
    );
    validate_generated_contact_constraint_endpoints_with_authority(
        constraints,
        generated_constraint_start_index,
        &source_authority,
    )
}

pub(in crate::simulation::network::surface::node::rails) fn validate_generated_contact_constraint_endpoints_with_authority(
    constraints: &[NodeRailConstraint],
    generated_constraint_start_index: usize,
    source_authority: &ExactGeneratedSourceAuthority,
) -> Result<(), NodeRailGenerationError> {
    for constraint in constraints.iter().skip(generated_constraint_start_index) {
        if generated_contact_kind_from_constraint(constraint.kind).is_none() {
            continue;
        }
        let source_band_index = constraint.source_band_index;
        if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
            continue;
        }
        let owners = [constraint.owner, constraint.opposite_owner];
        if !source_authority.has_any_source(
            owners,
            constraint.source_mouth_order_index,
            source_band_index,
        ) {
            continue;
        }
        for point in &constraint.points_xz {
            let key = road_point_key(*point);
            if generated_contact_constraint_endpoint_has_exact_source_authority(
                constraint,
                source_authority,
                owners,
                source_band_index,
                key,
            ) {
                continue;
            }
            return Err(
                NodeRailGenerationError::NonCanonicalGeneratedContactEndpoint {
                    kind: constraint.kind,
                    mouth_order_index: constraint.source_mouth_order_index,
                    band_index: constraint.source_band_index,
                    owner: constraint.owner,
                    opposite_owner: constraint.opposite_owner,
                    point_x_key: key.0,
                    point_z_key: key.1,
                },
            );
        }
    }
    Ok(())
}

pub(super) fn generated_contact_constraint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
) -> bool {
    let source_band_index = constraint.source_band_index;
    if constraint.owner.is_none() || constraint.opposite_owner.is_none() {
        return true;
    }
    let owners = [constraint.owner, constraint.opposite_owner];
    if !source_authority.has_any_source(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
    ) {
        return true;
    }
    constraint.points_xz.iter().copied().all(|point| {
        generated_contact_constraint_endpoint_has_exact_source_authority(
            constraint,
            source_authority,
            owners,
            source_band_index,
            road_point_key(point),
        )
    })
}

fn generated_contact_constraint_endpoint_has_exact_source_authority(
    constraint: &NodeRailConstraint,
    source_authority: &ExactGeneratedSourceAuthority,
    owners: [Option<NodeBandOwner>; 2],
    source_band_index: Option<usize>,
    key: NodeRailPointKey,
) -> bool {
    source_authority.has_exact_point(
        owners,
        constraint.source_mouth_order_index,
        source_band_index,
        key,
    ) || source_authority.has_exact_source_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_same_kind_source_handoff_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    ) || source_authority.has_exact_cross_source_same_kind_contact_key(
        constraint.kind,
        owners,
        constraint.source_mouth_order_index,
        constraint.source_band_index,
        key,
    )
}
