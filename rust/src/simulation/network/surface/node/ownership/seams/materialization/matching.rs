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
    if exact_owner_pair
        && piece_kind == RoadSurfaceVisualNodePieceKind::JunctionN
        && constraint.kind == NodeRailConstraintKind::RaisedStepContact
        && !raised_step_contact_requires_exact_constraint_span(owner, opposite_owner)
        && edge_lies_inside_single_constraint_segment_dust_envelope(start, end, constraint)
    {
        return true;
    }
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

fn edge_lies_inside_single_constraint_segment_dust_envelope(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    constraint: &NodeRailConstraint,
) -> bool {
    if edge_start == edge_end {
        return false;
    }
    constraint.points_xz.windows(2).any(|segment| {
        let segment_start = ownership_key_from_road_point(segment[0]);
        let segment_end = ownership_key_from_road_point(segment[1]);
        segment_start != segment_end
            && point_is_inside_constraint_segment_dust_envelope(
                edge_start,
                segment_start,
                segment_end,
            )
            && point_is_inside_constraint_segment_dust_envelope(
                edge_end,
                segment_start,
                segment_end,
            )
            && edge_is_longitudinal_to_constraint_segment(
                edge_start,
                edge_end,
                segment_start,
                segment_end,
            )
    })
}

fn edge_is_longitudinal_to_constraint_segment(
    edge_start: NodeOwnershipPointKey,
    edge_end: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
) -> bool {
    let edge_dx = i128::from(edge_end.0 - edge_start.0);
    let edge_dz = i128::from(edge_end.1 - edge_start.1);
    let segment_dx = i128::from(segment_end.0 - segment_start.0);
    let segment_dz = i128::from(segment_end.1 - segment_start.1);
    let longitudinal = edge_dx * segment_dx + edge_dz * segment_dz;
    if longitudinal == 0 {
        return false;
    }
    let transverse = edge_dx * segment_dz - edge_dz * segment_dx;
    // The envelope admits Boolean endpoint displacement, not a newly angled contact edge.
    const MAX_TRANSVERSE_RATIO_DENOMINATOR: u128 = 100;
    transverse
        .unsigned_abs()
        .saturating_mul(MAX_TRANSVERSE_RATIO_DENOMINATOR)
        <= longitudinal.unsigned_abs()
}

fn point_is_inside_constraint_segment_dust_envelope(
    point: NodeOwnershipPointKey,
    segment_start: NodeOwnershipPointKey,
    segment_end: NodeOwnershipPointKey,
) -> bool {
    let dx = i128::from(segment_end.0 - segment_start.0);
    let dz = i128::from(segment_end.1 - segment_start.1);
    let length_sq = dx * dx + dz * dz;
    if length_sq == 0 {
        return false;
    }
    let px = i128::from(point.0 - segment_start.0);
    let pz = i128::from(point.1 - segment_start.1);
    let parameter_numerator = px * dx + pz * dz;
    let dust_key_units = (f64::from(super::super::super::super::NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
        * super::super::super::super::super::keys::SURFACE_XZ_KEY_SCALE)
        .round() as i128;
    let dust_sq = dust_key_units * dust_key_units;
    if parameter_numerator <= 0 {
        return px * px + pz * pz <= dust_sq;
    }
    if parameter_numerator >= length_sq {
        let ex = i128::from(point.0 - segment_end.0);
        let ez = i128::from(point.1 - segment_end.1);
        return ex * ex + ez * ez <= dust_sq;
    }
    let cross = px * dz - pz * dx;
    cross * cross <= dust_sq * length_sq
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

    #[test]
    fn junctionn_exact_raised_step_pair_accepts_boolean_endpoint_dust() {
        let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
        let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
        let constraint = NodeRailConstraint {
            constraint_index: 23,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(4),
            source_boundary_index: Some(1),
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![
                road_point_from_key((0, 0)),
                road_point_from_key((1_000_000, 0)),
            ],
        };

        assert!(owned_edge_lies_on_rail_constraint(
            (15, 29),
            (500_000, 29),
            &constraint,
            curb,
            sidewalk,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ));
        assert!(!owned_edge_lies_on_rail_constraint(
            (15, 300),
            (500_000, 300),
            &constraint,
            curb,
            sidewalk,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ));
        assert!(!owned_edge_lies_on_rail_constraint(
            (500_000, -25),
            (500_000, 25),
            &constraint,
            curb,
            sidewalk,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ));

        let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 3);
        let carriageway_curb_constraint = NodeRailConstraint {
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            ..constraint
        };
        assert!(!owned_edge_lies_on_rail_constraint(
            (15, 29),
            (500_000, 29),
            &carriageway_curb_constraint,
            carriageway,
            curb,
            RoadSurfaceVisualNodePieceKind::JunctionN,
        ));
    }
}
