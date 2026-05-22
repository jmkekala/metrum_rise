//! Generated raised-step owner-pair tests.

use super::*;

#[test]
fn generated_raised_step_owner_pair_splits_carriageway_boundary_at_overlay_contact() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
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
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
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
            RoadVec2::new(3.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(3.0, 1.5),
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
        opposite_owner: Some(actual_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(3.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
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
fn generated_raised_step_owner_pair_splits_curb_sidewalk_boundary_at_overlay_contact() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        0,
        Some(0),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("curb contour is valid");
    push_generated_contour(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        0,
        Some(1),
        Some(sidewalk_owner),
        NodeGeneratedContourClaimPriority::MouthBand,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        vec![
            RoadVec2::new(3.0, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(3.0, 1.5),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("sidewalk contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(sidewalk_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(3.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
        constraint.kind == NodeRailConstraintKind::RaisedStepContact
            && owners_match_unordered(
                constraint.owner,
                constraint.opposite_owner,
                curb_owner,
                sidewalk_owner,
            )
            && constraint.points_xz.len() == 2
            && road_point_key(constraint.points_xz[0]) == start
            && road_point_key(constraint.points_xz[1]) == end
    }));
}

#[test]
fn generated_raised_step_owner_pair_uses_source_authority_union_for_split_domains() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let actual_curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
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
            RoadVec2::new(4.0, 0.0),
            RoadVec2::new(4.0, 1.0),
            RoadVec2::new(0.0, 1.0),
        ],
        None,
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt contour is valid");
    for points in [
        vec![
            RoadVec2::new(2.0, 0.5),
            RoadVec2::new(3.2, 0.5),
            RoadVec2::new(3.2, 1.5),
            RoadVec2::new(2.0, 1.5),
        ],
        vec![
            RoadVec2::new(2.8, 0.5),
            RoadVec2::new(4.0, 0.5),
            RoadVec2::new(4.0, 1.5),
            RoadVec2::new(2.8, 1.5),
        ],
    ] {
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
            points,
            None,
            &mut contours,
            &mut constraints,
        )
        .expect("curb contour is valid");
    }
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(asphalt_owner),
        opposite_owner: Some(actual_curb_owner),
        points_xz: vec![RoadVec2::new(0.0, 1.0), RoadVec2::new(4.0, 1.0)],
    });

    append_generated_same_band_contact_constraints(
        RoadSurfaceVisualNodePieceKind::Bend,
        &contours,
        &mut constraints,
    );

    let start = road_point_key(RoadVec2::new(2.0, 1.0));
    let end = road_point_key(RoadVec2::new(4.0, 1.0));
    assert!(constraints.iter().any(|constraint| {
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
