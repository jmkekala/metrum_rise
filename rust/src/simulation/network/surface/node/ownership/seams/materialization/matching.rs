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
    if constraint.source_boundary_index.is_none() {
        return false;
    }
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
    match (
        constraint.kind,
        material_contact_kind_for_owned_edge(owner, opposite_owner),
    ) {
        (
            NodeRailConstraintKind::RaisedStepContact,
            Some(NodeRailConstraintKind::RaisedStepContact),
        ) => true,
        (
            NodeRailConstraintKind::FootprintSeam { adjacent_kind },
            Some(NodeRailConstraintKind::BandBoundary {
                left_kind,
                right_kind,
            }),
        ) => {
            adjacent_kind == role_owner.kind()
                && (role_owner.kind() == left_kind || role_owner.kind() == right_kind)
        }
        _ => false,
    }
}

pub(super) fn material_contact_kind_for_owned_edge(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<NodeRailConstraintKind> {
    if owners_form_raised_step_contact(owner, opposite_owner) {
        return Some(NodeRailConstraintKind::RaisedStepContact);
    }
    if let Some(adjacent_kind) = asphalt_boundary_adjacent_kind(owner, opposite_owner) {
        return Some(NodeRailConstraintKind::BandBoundary {
            left_kind: RoadSurfaceBandKind::Carriageway,
            right_kind: adjacent_kind,
        });
    }
    if owners_form_sidewalk_footpath_contact(owner, opposite_owner) {
        return Some(NodeRailConstraintKind::BandBoundary {
            left_kind: RoadSurfaceBandKind::Sidewalk,
            right_kind: RoadSurfaceBandKind::Footpath,
        });
    }
    None
}

fn asphalt_boundary_adjacent_kind(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> Option<RoadSurfaceBandKind> {
    match (owner.kind(), opposite_owner.kind()) {
        (RoadSurfaceBandKind::Carriageway, adjacent_kind)
            if adjacent_kind != RoadSurfaceBandKind::Carriageway =>
        {
            Some(adjacent_kind)
        }
        (adjacent_kind, RoadSurfaceBandKind::Carriageway)
            if adjacent_kind != RoadSurfaceBandKind::Carriageway =>
        {
            Some(adjacent_kind)
        }
        _ => None,
    }
}

fn owners_form_sidewalk_footpath_contact(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
) -> bool {
    matches!(
        (owner.kind(), opposite_owner.kind()),
        (RoadSurfaceBandKind::Sidewalk, RoadSurfaceBandKind::Footpath)
            | (RoadSurfaceBandKind::Footpath, RoadSurfaceBandKind::Sidewalk)
    )
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
        return rail_constraint_band_contour_authorizes_owned_edge(
            constraint,
            owner,
            opposite_owner,
        ) && edge_lies_on_constraint_polyline_on_overlay_grid(start, end, constraint);
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
    if constraint.kind == NodeRailConstraintKind::RaisedStepContact
        && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
        && !materialized_edge_requires_exact_constraint_span(constraint, owner, opposite_owner)
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

#[cfg(test)]
mod tests {
    use crate::simulation::network::surface::RoadSurfaceBandKind;

    use super::*;

    #[test]
    fn bend_band_contour_materializes_noded_collinear_contact_edge() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 8);
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 7);
        let constraint = NodeRailConstraint {
            constraint_index: 28,
            kind: NodeRailConstraintKind::BandContour {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            },
            source_mouth_order_index: 0,
            source_band_index: Some(2),
            source_boundary_index: None,
            owner: Some(curb),
            opposite_owner: None,
            points_xz: vec![
                road_point_from_key((6_148_780, 3_650_000)),
                road_point_from_key((7_031_089, 5_178_204)),
                road_point_from_key((7_361_216, 5_750_000)),
            ],
        };

        assert!(owned_edge_lies_on_rail_constraint(
            (6_951_609, 5_040_541),
            (7_361_216, 5_750_000),
            &constraint,
            carriageway,
            curb,
            RoadSurfaceVisualNodePieceKind::Bend,
        ));
    }

    #[test]
    fn generated_carriageway_sidewalk_contact_is_material_only_boundary() {
        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 9);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);

        assert_eq!(
            material_contact_kind_for_owned_edge(carriageway, sidewalk),
            Some(NodeRailConstraintKind::BandBoundary {
                left_kind: RoadSurfaceBandKind::Carriageway,
                right_kind: RoadSurfaceBandKind::Sidewalk,
            })
        );
    }
}
