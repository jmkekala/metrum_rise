//! Generated contour owner-carrier authority tests.

use super::*;

#[test]
fn junctionn_canonical_height_authority_prefers_owner_generated_carrier_over_base_interval() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let patch = NodeBandHeightPatch::from_heighted_contour(
        field.id,
        field.kind,
        &[
            RoadVec3::new(0.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 1.0, 2.0),
        ],
        NodeHeightPatchAuthority {
            owner: Some(owner),
            role: NodeHeightPatchAuthorityRole::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            },
        },
    )
    .expect("test generated contour is a valid height carrier");
    field.patches.push(patch);

    assert!(matches!(
        field.evaluate_height(RoadVec2::new(5.0, 1.0)),
        Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
    ));
    let height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("owner-generated carrier is explicit height authority for JunctionN");
    assert!((height.height_m - 1.0).abs() <= 1.0e-6);
    assert_eq!(
        height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
}

#[test]
fn bend_pre_height_gate_uses_owner_generated_carrier_before_heighting_region() {
    let input = conflicting_manual_input();
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 0);
    let contour_points = vec![
        RoadVec2::new(2.0, 0.5),
        RoadVec2::new(8.0, 0.5),
        RoadVec2::new(8.0, 1.5),
        RoadVec2::new(2.0, 1.5),
    ];
    let rails = manual_rail_contours(
        77,
        RoadSurfaceVisualNodePieceKind::Bend,
        vec![NodeGeneratedContour {
            kind: NodeGeneratedContourKind::Band { kind: source.0 },
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            source_mouth_order_index: source.1,
            source_band_index: Some(source.2),
            owner: Some(owner),
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            backend_polyline: road_points_to_polyline(contour_points.clone(), true),
            points_xz: contour_points,
            height_points_world: Some(vec![
                RoadVec3::new(2.0, 9.0, 0.5),
                RoadVec3::new(8.0, 9.0, 0.5),
                RoadVec3::new(8.0, 9.0, 1.5),
                RoadVec3::new(2.0, 9.0, 1.5),
            ]),
        }],
    );
    let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 20.0);
    region.claim_priority = NodeGeneratedContourClaimPriority::SideJoin;
    region.shape = vec![vec![[2.0, 0.5], [8.0, 0.5], [8.0, 1.5], [2.0, 1.5]]];
    let owned_regions = vec![region];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::Bend,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
        carrier_provenance: NodeCarrierProvenanceClosure::empty(),
    };

    let solution =
        NodeHeightSolution::from_ownership_input_and_rails(&input, Some(&rails), &ownership)
            .expect("pre-height gate must use owner-generated Bend carrier");
    let region = solution
        .regions
        .first()
        .expect("test ownership should produce one heighted region");

    assert!(region.shape.iter().flatten().all(|vertex| {
        vertex.height_authority
            == Some(NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
                claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
            })
            && SurfaceHeightMmKey::from_m_f64(vertex.height_m)
                == SurfaceHeightMmKey::from_m_f64(9.0)
    }));
}

#[test]
fn junctionn_canonical_height_authority_scopes_generated_carriers_to_owned_region_claim() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    for (height_m, purpose, claim_priority) in [
        (
            1.0,
            NodeGeneratedContourPurpose::NonRoadBand,
            NodeGeneratedContourClaimPriority::MouthBand,
        ),
        (
            2.0,
            NodeGeneratedContourPurpose::JunctionSideJoin,
            NodeGeneratedContourClaimPriority::SideJoin,
        ),
    ] {
        let patch = NodeBandHeightPatch::from_heighted_contour(
            field.id,
            field.kind,
            &[
                RoadVec3::new(0.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 2.0),
                RoadVec3::new(0.0, height_m, 2.0),
            ],
            NodeHeightPatchAuthority {
                owner: Some(owner),
                role: NodeHeightPatchAuthorityRole::GeneratedContour {
                    purpose,
                    claim_priority,
                },
            },
        )
        .expect("test generated contour is a valid height carrier");
        field.patches.push(patch);
    }

    let mouth_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::MouthBand,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("mouth-owned region should use mouth-band generated carrier");
    assert_eq!(
        mouth_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );
    assert!((mouth_height.height_m - 1.0).abs() <= 1.0e-6);

    let side_join_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("side-join-owned region should use side-join generated carrier");
    assert_eq!(
        side_join_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
    assert!((side_join_height.height_m - 2.0).abs() <= 1.0e-6);
}

#[test]
fn junctionn_canonical_height_authority_rejects_conflicting_owner_generated_carriers() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Carriageway, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let authority = NodeHeightPatchAuthority {
        owner: Some(owner),
        role: NodeHeightPatchAuthorityRole::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        },
    };
    for height_m in [1.0, 2.0] {
        let patch = NodeBandHeightPatch::from_heighted_contour(
            field.id,
            field.kind,
            &[
                RoadVec3::new(0.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 0.0),
                RoadVec3::new(10.0, height_m, 2.0),
                RoadVec3::new(0.0, height_m, 2.0),
            ],
            authority,
        )
        .expect("test generated contour is a valid height carrier");
        field.patches.push(patch);
    }

    assert!(matches!(
        field.evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0)
        ),
        Err(NodeHeightFieldError::SourceHeightFieldConflict { .. })
    ));
}
