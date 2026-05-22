//! Generated contour height-carrier tests.

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
fn contour_edge_height_requires_precomputed_support_key() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(5.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.8, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    );
    let point_xz = RoadVec2::new(5.0, 0.0);

    let mut without_support = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    without_support
        .extend_with_generated_contour(&contour)
        .expect("generated contour without source handoff support should still be valid");
    let generated_height = without_support
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::MouthBand,
            point_xz,
        )
        .expect("generated contour owns its explicit boundary vertex");
    assert_eq!(
        generated_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );

    let source_support = [RoadVec3::new(5.0, 0.8, 0.0)];
    let mut with_support = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    with_support
        .extend_with_generated_contour(&contour)
        .expect("matching precomputed source handoff support should authorize");
    let source_height = with_support
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::MouthBand,
            point_xz,
        )
        .expect("explicit source handoff support should own the handoff height");
    assert_eq!(
        source_height.authority,
        NodeHeightAuthoritySource::SourceInterval
    );
    assert!((source_height.height_m - 0.8).abs() <= 1.0e-6);
}

#[test]
fn contour_edge_height_rejects_drifted_support_key() {
    let source_support = [RoadVec3::new(5.0, 0.8, 0.00005)];
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(5.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.8, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    );
    field
        .extend_with_generated_contour(&contour)
        .expect("drifted source support must not be treated as this contour handoff");

    let height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::MouthBand,
            RoadVec2::new(5.0, 0.0),
        )
        .expect("generated contour owns the exact boundary vertex");
    assert_eq!(
        height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );
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

#[test]
fn height_solution_has_no_post_overlay_height_repair_path() {
    let source = [
        include_str!("../../height.rs"),
        include_str!("../build.rs"),
        include_str!("../carriers.rs"),
        include_str!("../evaluate.rs"),
        include_str!("../field.rs"),
        include_str!("../grade.rs"),
        include_str!("../authority.rs"),
        include_str!("../handoff.rs"),
        include_str!("../model.rs"),
        include_str!("../patch.rs"),
        include_str!("../seams.rs"),
        include_str!("../source_edges.rs"),
        include_str!("../triangles.rs"),
        include_str!("../vertices.rs"),
    ]
    .join("\n");
    for forbidden in [
        concat!("heighted_shape_with_", "canonical_contour_insertions"),
        concat!("heighted_contour_with_", "canonical_insertions"),
        concat!("fill_canonical_contour_", "height_insertions"),
        concat!("reheight_terminal_", "cap_band_from_base"),
        concat!("reheight_point_", "from_base"),
        concat!("from_terminal_cap_band_", "with_base"),
        concat!("evaluate_region_", "scoped_height"),
        concat!("bounded_region_", "scoped_edge_height"),
        concat!("region_scoped_", "carrier"),
        concat!("HEIGHT_SOURCE_EDGE_", "NEIGHBOR_UNITS"),
        concat!("HEIGHT_SOURCE_EDGE_", "DEDUP_DRIFT_UNITS"),
        concat!("allow_missing_height_points_", "backfill"),
        concat!("subdivided_", "height_chord"),
    ] {
        assert!(
            !source.contains(forbidden),
            "canonical arrangement vertices must be inside their explicit height carrier, not repaired by `{forbidden}`"
        );
    }
}

#[test]
fn generated_band_contour_requires_explicit_height_points() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let contour = generated_band_contour(
        RoadSurfaceBandKind::CurbOrShoulder,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        None,
    );

    assert_eq!(
        field.extend_with_generated_contour(&contour),
        Err(NodeHeightFieldError::MissingGeneratedContourHeightPoints {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id: field.id,
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        })
    );
    assert_eq!(
        field.patches.len(),
        1,
        "missing generated heights must not add an unsourced substitute patch"
    );
}

#[test]
fn generated_band_contour_requires_source_band_index_for_height_carrier() {
    let input = conflicting_manual_input();
    let mut contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 5.0, 0.0),
            RoadVec3::new(10.0, 7.0, 0.0),
            RoadVec3::new(10.0, 7.0, 2.0),
            RoadVec3::new(0.0, 5.0, 2.0),
        ]),
    );
    contour.source_band_index = None;
    let rails = manual_rail_contours(input.node_id, input.piece_kind, vec![contour]);

    assert!(matches!(
        height_fields_by_source(&input, Some(&rails)),
        Err(
            NodeHeightFieldError::GeneratedContourMissingSourceBandIndex {
                mouth_order_index: 0,
                source_kind: RoadSurfaceBandKind::Sidewalk,
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
                owner: Some(_),
            }
        )
    ));
}

#[test]
fn generated_band_contour_rejects_missing_source_band() {
    let input = conflicting_manual_input();
    let mut contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 5.0, 0.0),
            RoadVec3::new(10.0, 7.0, 0.0),
            RoadVec3::new(10.0, 7.0, 2.0),
            RoadVec3::new(0.0, 5.0, 2.0),
        ]),
    );
    contour.source_band_index = Some(99);
    let rails = manual_rail_contours(input.node_id, input.piece_kind, vec![contour]);

    assert!(matches!(
        height_fields_by_source(&input, Some(&rails)),
        Err(NodeHeightFieldError::GeneratedContourMissingSourceBand {
            mouth_order_index: 0,
            band_index: 99,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            owner: Some(_),
        })
    ));
}

#[test]
fn generated_contour_source_handoff_height_mismatch_keeps_generated_owner_contour() {
    let source_support = [RoadVec3::new(5.0, 0.5, 0.0)];
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    let contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(5.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.75, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    );

    field
        .extend_with_generated_contour(&contour)
        .expect("mismatched source support should not poison the owner contour");
    let height = field
        .evaluate_authorized_height(
            NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0),
            NodeGeneratedContourClaimPriority::MouthBand,
            RoadVec2::new(5.0, 0.0),
        )
        .expect("generated contour owns the mismatched handoff vertex");
    assert_eq!(
        height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );
    assert!((height.height_m - 0.75).abs() <= 1.0e-6);
}

#[test]
fn generated_contour_source_handoff_mismatched_explicit_vertex_keeps_generated_owner_contour() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let contour = generated_band_contour(
        RoadSurfaceBandKind::Sidewalk,
        vec![
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(10.0, 0.0),
            RoadVec2::new(10.0, 2.0),
            RoadVec2::new(0.0, 2.0),
        ],
        Some(vec![
            RoadVec3::new(0.0, 0.25, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    );

    field
        .extend_with_generated_contour(&contour)
        .expect("mismatched explicit source vertex should not poison the owner contour");
    let height = field
        .evaluate_authorized_height(
            NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0),
            NodeGeneratedContourClaimPriority::MouthBand,
            RoadVec2::new(0.0, 0.0),
        )
        .expect("generated contour owns the mismatched explicit source vertex");
    assert_eq!(
        height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::NonRoadBand,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        }
    );
    assert!((height.height_m - 0.25).abs() <= 1.0e-6);
}

#[test]
fn generated_band_contour_rejects_invalid_height_carrier_contour() {
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::CurbOrShoulder, 0.0, 0.0),
        None,
    )
    .expect("manual interval is a valid source height carrier");
    let points_xz = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(10.0, 2.0),
        RoadVec2::new(0.0, 2.0),
        RoadVec2::new(10.0, 0.0),
    ];
    let height_points_world = points_xz
        .iter()
        .map(|point| RoadVec3::new(point.x, 0.0, point.y))
        .collect();
    let contour = generated_band_contour(
        RoadSurfaceBandKind::CurbOrShoulder,
        points_xz,
        Some(height_points_world),
    );

    assert!(matches!(
        field.extend_with_generated_contour(&contour),
        Err(NodeHeightFieldError::InvalidHeightCarrierContour {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::CurbOrShoulder,
            height_field_id,
            authority: NodeHeightAuthoritySource::GeneratedContour {
                purpose: NodeGeneratedContourPurpose::NonRoadBand,
                claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            },
            ..
        }) if height_field_id == field.id
    ));
}
