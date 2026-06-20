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
        constraints.len(),
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
        constraints.len(),
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
        constraints.len(),
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

#[test]
fn shared_height_contact_syncs_generated_side_join_vertex_to_raised_owner_height() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour_with_purpose(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        NodeGeneratedContourPurpose::BendSideJoin,
        0,
        Some(4),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(-1.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(-1.0, 1.0),
        ],
        Some(vec![
            RoadVec3::new(-1.0, 10.0, 0.0),
            RoadVec3::new(1.0, 10.0, 0.0),
            RoadVec3::new(1.0, 10.001, 1.0),
            RoadVec3::new(0.0, 10.001, 1.0),
            RoadVec3::new(-1.0, 10.0, 1.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("curb side-join contour is valid");
    push_generated_contour_with_purpose(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        NodeGeneratedContourPurpose::BendSideJoin,
        0,
        Some(5),
        Some(sidewalk_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        vec![
            RoadVec2::new(-1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(1.0, 2.0),
            RoadVec2::new(-1.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(-1.0, 10.009, 1.0),
            RoadVec3::new(0.0, 10.009, 1.0),
            RoadVec3::new(1.0, 10.009, 1.0),
            RoadVec3::new(1.0, 10.010, 2.0),
            RoadVec3::new(-1.0, 10.010, 2.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("sidewalk side-join contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(5),
        source_boundary_index: Some(4),
        owner: Some(curb_owner),
        opposite_owner: Some(sidewalk_owner),
        points_xz: vec![
            RoadVec2::new(-1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(1.0, 1.0),
        ],
    });

    let shared_key = road_point_key(RoadVec2::new(0.0, 1.0));
    assert_ne!(
        contour_height_mm_at(&contours[0], shared_key),
        contour_height_mm_at(&contours[1], shared_key)
    );

    synchronize_shared_height_contact_vertices(&mut contours, &constraints);

    assert_eq!(
        contour_height_mm_at(&contours[0], shared_key),
        contour_height_mm_at(&contours[1], shared_key)
    );
    assert_eq!(
        contour_height_mm_at(&contours[0], shared_key),
        SurfaceHeightMmKey::from_m_f64(10.009).as_i64()
    );
}

#[test]
fn shared_height_contact_sync_preserves_carriageway_curb_step_height() {
    let asphalt_owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 3);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let mut contours = Vec::new();
    let mut constraints = Vec::new();

    push_generated_contour_with_purpose(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        NodeGeneratedContourPurpose::BendSideJoin,
        0,
        Some(3),
        Some(asphalt_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        vec![
            RoadVec2::new(-1.0, 0.0),
            RoadVec2::new(1.0, 0.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(-1.0, 1.0),
        ],
        Some(vec![
            RoadVec3::new(-1.0, 2.0, 0.0),
            RoadVec3::new(1.0, 2.0, 0.0),
            RoadVec3::new(1.0, 2.0, 1.0),
            RoadVec3::new(0.0, 2.0, 1.0),
            RoadVec3::new(-1.0, 2.0, 1.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("asphalt side-join contour is valid");
    push_generated_contour_with_purpose(
        NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        NodeGeneratedContourPurpose::BendSideJoin,
        0,
        Some(4),
        Some(curb_owner),
        NodeGeneratedContourClaimPriority::SideJoin,
        NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::CurbOrShoulder,
        },
        vec![
            RoadVec2::new(-1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(1.0, 1.0),
            RoadVec2::new(1.0, 2.0),
            RoadVec2::new(-1.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(-1.0, 2.15, 1.0),
            RoadVec3::new(0.0, 2.15, 1.0),
            RoadVec3::new(1.0, 2.15, 1.0),
            RoadVec3::new(1.0, 2.15, 2.0),
            RoadVec3::new(-1.0, 2.15, 2.0),
        ]),
        &mut contours,
        &mut constraints,
    )
    .expect("curb side-join contour is valid");
    constraints.push(NodeRailConstraint {
        constraint_index: constraints.len(),
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(4),
        source_boundary_index: Some(3),
        owner: Some(asphalt_owner),
        opposite_owner: Some(curb_owner),
        points_xz: vec![
            RoadVec2::new(-1.0, 1.0),
            RoadVec2::new(0.0, 1.0),
            RoadVec2::new(1.0, 1.0),
        ],
    });

    let shared_key = road_point_key(RoadVec2::new(0.0, 1.0));
    synchronize_shared_height_contact_vertices(&mut contours, &constraints);

    assert_eq!(contour_height_mm_at(&contours[0], shared_key), 2000);
    assert_eq!(contour_height_mm_at(&contours[1], shared_key), 2150);
}

fn contour_height_mm_at(contour: &NodeGeneratedContour, key: (i64, i64)) -> i64 {
    let point_index = contour
        .points_xz
        .iter()
        .position(|point| road_point_key(*point) == key)
        .expect("contour should contain the key");
    let height_point = &contour
        .height_points_world
        .as_ref()
        .expect("contour should have height points")[point_index];
    SurfaceHeightMmKey::from_m_f64(height_point.y).as_i64()
}
