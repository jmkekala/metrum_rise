//! Owned-edge to rail-constraint matching for seam materialization.

use super::*;

pub(super) fn rail_constraint_owner_pair_matches_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (constraint.owner, constraint.opposite_owner),
        (Some(left), Some(right))
            if (left == owner && right == opposite_owner)
                || (left == opposite_owner && right == owner)
    )
}

pub(super) fn rail_constraint_can_materialize_for_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner)
        || rail_constraint_owner_kinds_authorize_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_band_contour_authorizes_owned_edge(constraint, owner, opposite_owner)
        || rail_constraint_role_matches_owned_edge(constraint, owner, opposite_owner)
}

pub(super) fn rail_constraint_band_contour_authorizes_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    let NodeRailConstraintKind::BandContour { kind } = constraint.kind else {
        return false;
    };
    if material_contact_kind_for_owned_edge(owner, opposite_owner).is_none() {
        return false;
    }
    if kind != owner.kind() && kind != opposite_owner.kind() {
        return false;
    }
    constraint.owner.is_none_or(|constraint_owner| {
        constraint_owner == owner || constraint_owner == opposite_owner
    })
}

fn rail_constraint_owner_kinds_authorize_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if !constraint_is_material_transition(constraint) {
        return false;
    }
    let Some((constraint_owner, constraint_opposite_owner)) =
        constraint.owner.zip(constraint.opposite_owner)
    else {
        return false;
    };
    if ![constraint_owner, constraint_opposite_owner]
        .into_iter()
        .any(|constraint_owner| constraint_owner == owner || constraint_owner == opposite_owner)
    {
        return false;
    }
    owner_sets_match_by_kind(
        owner,
        opposite_owner,
        constraint_owner,
        constraint_opposite_owner,
    )
}

fn owner_sets_match_by_kind(
    left_owner: NodeBandOwner,
    left_opposite_owner: NodeBandOwner,
    right_owner: NodeBandOwner,
    right_opposite_owner: NodeBandOwner,
) -> bool {
    (left_owner.kind() == right_owner.kind()
        && left_opposite_owner.kind() == right_opposite_owner.kind())
        || (left_owner.kind() == right_opposite_owner.kind()
            && left_opposite_owner.kind() == right_owner.kind())
}

fn rail_constraint_role_matches_owned_edge(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    if constraint.owner.zip(constraint.opposite_owner).is_some() {
        return false;
    }
    let Some(role_owner) = constraint.owner.or(constraint.opposite_owner) else {
        return false;
    };
    if role_owner != owner && role_owner != opposite_owner {
        return false;
    }
    match constraint.kind {
        NodeRailConstraintKind::RaisedStepContact => {
            owners_form_raised_step_contact(owner, opposite_owner)
        }
        _ => false,
    }
}

pub(super) fn material_contact_kind_for_owned_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    owners_form_raised_step_contact(owner, opposite_owner)
        .then_some(NodeRailConstraintKind::RaisedStepContact)
}

pub(super) fn owned_edge_lies_on_rail_constraint(
    start: NodeOwnershipPointKey,
    end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    piece_kind: RoadSurfaceVisualNodePieceKind,
) -> bool {
    if start == end || constraint.points_xz.len() < 2 {
        return false;
    }
    if edge_lies_on_single_constraint_segment(start, end, constraint) {
        return true;
    }
    if matches!(constraint.kind, NodeRailConstraintKind::BandContour { .. }) {
        return false;
    }
    let exact_owner_pair =
        rail_constraint_owner_pair_matches_edge(constraint, owner, opposite_owner);
    if materialized_edge_requires_exact_constraint_span(constraint, owner, opposite_owner) {
        if exact_owner_pair && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN {
            return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
        }
        if !exact_owner_pair
            || (constraint.source_boundary_index.is_some()
                && piece_kind != RoadSurfaceVisualNodePieceKind::Terminal)
        {
            return false;
        }
    }
    if constraint.kind == NodeRailConstraintKind::RaisedStepContact
        && exact_owner_pair
        && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
    {
        return edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
    }
    matches!(
        piece_kind,
        RoadSurfaceVisualNodePieceKind::Bend | RoadSurfaceVisualNodePieceKind::Terminal
    ) && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint)
}

fn materialized_edge_requires_exact_constraint_span(
    constraint: &NodeRailConstraint,
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(constraint.kind, NodeRailConstraintKind::RaisedStepContact)
        && raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
}
