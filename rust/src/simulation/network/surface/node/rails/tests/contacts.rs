//! Generated contact materialization tests.

use super::*;

#[test]
fn bend_side_join_contacts_name_exact_adjacent_owner_pair() {
    let contours =
        NodeRailContourSet::from_input(&bend_input_with_curb_side_join()).expect("valid contours");

    let side_join_owner = contours
        .contours
        .iter()
        .find(|contour| {
            contour.purpose == NodeGeneratedContourPurpose::BendSideJoin
                && contour.source_band_index == Some(4)
        })
        .and_then(|contour| contour.owner)
        .expect("side-join contour should have a band owner");
    assert!(contours.constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && constraint.source_band_index == Some(4)
            && (constraint.owner == Some(side_join_owner)
                || constraint.opposite_owner == Some(side_join_owner))
            && constraint_opposite_owner(constraint, side_join_owner)
                .is_some_and(|owner| owner.kind() == RoadSurfaceBandKind::Carriageway)
    }));
    assert!(!contours.constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && constraint.source_band_index == Some(4)
            && (constraint.owner == Some(side_join_owner)
                || constraint.opposite_owner == Some(side_join_owner))
            && constraint.opposite_owner.is_none()
    }));
}

#[test]
fn generated_contact_rejects_non_exact_owner_pair_authority() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        0,
        Some(0),
        Some(asphalt_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(2.0, 0.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(actual_curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(2.0, 1.0),
            RoadVec2::new(2.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: None,
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(2.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    assert!(!constraints.iter().any(|constraint| {
        let start = road_point_key(RoadVec2::new(0.0, 1.0));
        let end = road_point_key(RoadVec2::new(2.0, 1.0));
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn bend_side_join_point_contact_reowns_exact_source_rail_by_band_kind() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(1),
        Some(actual_curb_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(2.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(2.0, 1.5),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb side join contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(source_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    let mut junction_constraints = constraints.clone();
    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &contours,
        &mut junction_constraints,
    );
    assert!(!junction_constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
    }));

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(2.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    for point in [start, end] {
        assert!(constraints.iter().any(|constraint| {
            constraint.kind == NodeRailConstraintKind::RaisedStepContact
                && owners_match_unordered(
                    constraint.owner,
                    constraint.opposite_owner,
                    asphalt_owner,
                    actual_curb_owner,
                )
                && constraint.points_xz.len() == 2
                && road_point_key(constraint.points_xz[0]) == point
                && road_point_key(constraint.points_xz[1]) == point
        }));
    }
    assert!(!constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                asphalt_owner,
                actual_curb_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn source_authorized_point_contact_uses_deterministic_source_name() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let shared_point = RoadVec2::new(0.0, 0.0);
    let mut constraints = vec![
        NodeRailConstraint {
            constraint_index: 0,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: Some(1),
            source_boundary_index: Some(1),
            owner: Some(asphalt_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![shared_point, RoadVec2::new(1.0, 0.0)],
        },
        NodeRailConstraint {
            constraint_index: 1,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 1,
            source_band_index: Some(2),
            source_boundary_index: Some(2),
            owner: Some(asphalt_owner),
            opposite_owner: Some(curb_owner),
            points_xz: vec![shared_point, RoadVec2::new(0.0, 1.0)],
        },
    ];

    append_source_authorized_raised_step_point_contacts(
        RoadSurfaceVisualNodePieceKind::Bend,
        &[],
        &mut constraints,
    );

    assert!(constraints.iter().skip(2).any(|constraint| {
        constraint.source_mouth_order_index == 0
            && constraint.source_band_index == Some(1)
            && constraint.points_xz == vec![shared_point, shared_point]
    }));
}
