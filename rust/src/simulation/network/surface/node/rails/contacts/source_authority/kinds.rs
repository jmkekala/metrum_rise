// SPDX-License-Identifier: GPL-2.0-only

//! Generated contact-kind and constraint conversion helpers.

use super::super::{
    GeneratedRaisedStepOwnerPair, GeneratedSameBandBoundaryRole, NodeBandOwner, NodeRailConstraint,
    NodeRailConstraintKind, RoadSurfaceBandKind, raised_step_band_rank,
    raised_step_kinds_can_contact as band_kinds_can_contact, road_point_key,
};
use super::types::{GeneratedSameBandContactConstraint, GeneratedSameBandContactConstraintKey};

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_raised_step_contact_kind_for_owners(
    left_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    GeneratedRaisedStepOwnerPair::new(left_owner, right_owner)
        .map(|_| NodeRailConstraintKind::RaisedStepContact)
}

pub(in crate::simulation::network::surface::node::rails) fn raised_step_band_kinds_can_contact(
    left_kind: RoadSurfaceBandKind,
    right_kind: RoadSurfaceBandKind,
) -> bool {
    band_kinds_can_contact(left_kind, right_kind)
}

pub(in crate::simulation::network::surface::node::rails) fn generated_raised_step_boundary_role_for_owner(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<GeneratedSameBandBoundaryRole> {
    GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
    let owner_rank = raised_step_band_rank(owner.kind())?;
    let opposite_rank = raised_step_band_rank(opposite_owner.kind())?;
    if opposite_rank < owner_rank {
        Some(GeneratedSameBandBoundaryRole::LowerSide)
    } else if opposite_rank > owner_rank {
        Some(GeneratedSameBandBoundaryRole::RaisedSide)
    } else {
        None
    }
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_same_band_contact_constraint_key(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraintKey> {
    generated_same_band_contact_constraint(constraint).map(GeneratedSameBandContactConstraint::key)
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_same_band_contact_constraint(
    constraint: &NodeRailConstraint,
) -> Option<GeneratedSameBandContactConstraint> {
    let Some(kind) = generated_contact_kind_from_constraint(constraint.kind) else {
        return None;
    };
    let owner = constraint.owner?;
    let opposite_owner = constraint.opposite_owner?;
    if owner == opposite_owner {
        return None;
    }
    let points = constraint.points_xz.as_slice();
    if points.len() != 2 {
        return None;
    }
    let (owner, opposite_owner) = if kind == NodeRailConstraintKind::RaisedStepContact {
        let pair = GeneratedRaisedStepOwnerPair::new(owner, opposite_owner)?;
        (pair.owner, pair.opposite_owner)
    } else {
        (owner.min(opposite_owner), owner.max(opposite_owner))
    };
    Some(GeneratedSameBandContactConstraint {
        kind,
        owner,
        opposite_owner,
        start: road_point_key(points[0]),
        end: road_point_key(points[1]),
        source_mouth_order_index: constraint.source_mouth_order_index,
        source_band_index: constraint.source_band_index,
    })
}

pub(in crate::simulation::network::surface::node::rails::contacts) fn generated_contact_kind_from_constraint(
    kind: NodeRailConstraintKind,
) -> Option<NodeRailConstraintKind> {
    match kind {
        NodeRailConstraintKind::AsphaltBoundary { .. }
        | NodeRailConstraintKind::RaisedStepContact => Some(kind),
        NodeRailConstraintKind::BandBoundary {
            left_kind,
            right_kind,
        } => raised_step_band_kinds_can_contact(left_kind, right_kind).then_some(kind),
        NodeRailConstraintKind::FullRoadbedContour
        | NodeRailConstraintKind::BandContour { .. }
        | NodeRailConstraintKind::SpanHandoff { .. }
        | NodeRailConstraintKind::FootprintSeam { .. } => None,
    }
}
