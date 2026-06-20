//! Shared XZ vertex height authority tests.

use super::*;

#[test]
fn shared_xz_vertices_keep_distinct_owner_source_heights() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    validate_shared_source_height_agreement(&regions)
        .expect("different owner/source contexts are explicit seams, not height corrections");

    assert_eq!(regions[0].shape[0][0].height_m, 0.0);
    assert_eq!(regions[1].shape[0][0].height_m, 0.25);
}

#[test]
fn shared_xz_vertices_without_explicit_seam_are_not_height_constrained() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    validate_explicit_material_seam_heights(&regions)
        .expect("missing explicit seam must not trigger coincident-XZ height correction");

    assert_eq!(regions[0].shape[0][0].height_m, 0.0);
    assert_eq!(regions[1].shape[0][0].height_m, 0.25);
}

#[test]
fn junctionn_same_material_shared_vertices_reject_height_conflict() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            9,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            14,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 1.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 0.25)],
        ),
    ];

    assert!(matches!(
        apply_junctionn_height_authority_normalization(&mut regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));

    assert_eq!(regions[0].shape[0][0].height_m, 2.0);
    assert_eq!(
        regions[1].shape[0][0].height_m, 1.0,
        "same-material owner priority must not rewrite conflicting sampled heights"
    );
    assert_eq!(
        regions[2].shape[0][0].height_m, 0.25,
        "different materials must not be pulled into the same-material tie-break"
    );
}

#[test]
fn junctionn_same_material_shared_vertices_keep_distinct_source_carrier_provenance() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            11,
            0.0,
            vec![manual_heighted_vertex_with_source_provenance(
                RoadSurfaceBandKind::Sidewalk,
                11,
                -1.0,
                0.0,
                1.52523,
            )],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            12,
            0.0,
            vec![manual_heighted_vertex_with_source_provenance(
                RoadSurfaceBandKind::Sidewalk,
                12,
                -1.0,
                0.0,
                1.52522,
            )],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("distinct source-carrier provenance must not be merged as one shared vertex");

    assert_eq!(regions[0].shape[0][0].height_m, 1.52523);
    assert_eq!(regions[1].shape[0][0].height_m, 1.52522);
}

#[test]
fn junctionn_same_material_shared_edge_keeps_distinct_source_carrier_provenance() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            17,
            0.0,
            vec![
                manual_heighted_vertex_with_source_provenance(
                    RoadSurfaceBandKind::Sidewalk,
                    17,
                    -1.0,
                    0.0,
                    1.49242,
                ),
                manual_heighted_vertex_with_source_provenance(
                    RoadSurfaceBandKind::Sidewalk,
                    17,
                    1.0,
                    0.0,
                    1.49242,
                ),
            ],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            18,
            0.0,
            vec![
                manual_heighted_vertex_with_source_provenance(
                    RoadSurfaceBandKind::Sidewalk,
                    18,
                    -1.0,
                    0.0,
                    1.49243,
                ),
                manual_heighted_vertex_with_source_provenance(
                    RoadSurfaceBandKind::Sidewalk,
                    18,
                    1.0,
                    0.0,
                    1.49243,
                ),
            ],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("distinct source-carrier provenance must keep shared-edge heights independent");

    assert_eq!(regions[0].shape[0][0].height_m, 1.49242);
    assert_eq!(regions[1].shape[0][0].height_m, 1.49243);
}

#[test]
fn junctionn_same_material_point_seam_selects_canonical_height_owner() {
    let seam = manual_seam_constraint(
        91,
        NodeSeamSource::AsphaltBoundary { owner_index: 0 },
        true,
        false,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0004)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            1,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.00049)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit same-material point seam should select a canonical height owner");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m),
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m)
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("point seam selection should record grade authority")
            .decision,
        NodeGradeCarrierDecision::SameMaterialSeam
    );
}

#[test]
fn junctionn_same_material_raised_step_contact_allows_vertical_height_split() {
    let seam = manual_seam_constraint(
        88,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        false,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.25)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions).expect(
        "explicit same-material raised-step contacts are height splits, not shared-height corrections",
    );

    assert_eq!(regions[0].shape[0][0].height_m, 1.0);
    assert_eq!(regions[1].shape[0][0].height_m, 1.25);
}

#[test]
fn junctionn_same_material_point_seam_uses_canonical_segment_membership() {
    let mut seam = manual_seam_constraint(
        89,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        false,
        true,
    );
    seam.start_xz = RoadVec2::new(0.0, 0.0);
    seam.end_xz = RoadVec2::new(0.01, 0.01);
    let vertex = RoadVec2::new(0.005, 0.005001);
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(vertex.x, vertex.y, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.0,
            vec![manual_heighted_vertex(vertex.x, vertex.y, 1.25)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions).expect(
        "canonical quantized seam membership should keep source-authorized point height splits",
    );

    assert_eq!(regions[0].shape[0][0].height_m, 1.0);
    assert_eq!(regions[1].shape[0][0].height_m, 1.25);
}

#[test]
fn junctionn_same_material_shared_vertices_share_authority_when_height_keys_match() {
    let mut regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            9,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.0004)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Carriageway,
            14,
            0.0,
            vec![manual_heighted_vertex(-1.0, 0.0, 2.00049)],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("matching height keys may share deterministic same-material authority");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        2000
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("carrier should record deterministic same-material authority")
            .decision,
        NodeGradeCarrierDecision::SameMaterialVertex
    );
}

#[test]
fn junctionn_explicit_material_seam_does_not_prefer_generated_contour_height() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let seam = manual_owned_pair_seam_constraint(37, curb, sidewalk, true);
    let mut mouth_band_vertex = manual_heighted_vertex(0.0, 0.0, 1.002);
    mouth_band_vertex.height_authority = Some(NodeHeightAuthoritySource::GeneratedContour {
        purpose: NodeGeneratedContourPurpose::NonRoadBand,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
    });
    let mut side_join_vertex = manual_heighted_vertex(0.0, 0.0, 1.0);
    side_join_vertex.height_authority = Some(NodeHeightAuthoritySource::GeneratedContour {
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
    });
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk.owner_index(),
            0.0,
            vec![mouth_band_vertex],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk.owner_index(),
            0.0,
            vec![side_join_vertex],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit material seam vertices must not use generated-contour priority repair");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        1002
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        1000
    );
}

#[test]
fn junctionn_node_grade_carrier_does_not_adopt_explicit_material_seam_for_same_material_vertex() {
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let other_sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 5);
    let seam = manual_owned_pair_seam_constraint(77, curb_owner, sidewalk_owner, true);
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            other_sidewalk_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("unconstrained same-material vertex should remain independently heighted");

    assert_eq!(
        regions[0].shape[0][0].height_m, 1.0,
        "explicit curb/sidewalk seam containment must outrank same-material tie-breaks"
    );
    assert_eq!(
        regions[1].shape[0][0].height_m, 2.0,
        "unconstrained same-material vertices must not be pulled to explicit seam height"
    );
    assert!(regions[1].shape[0][0].grade_authority.is_none());
    assert_eq!(regions[2].shape[0][0].height_m, 1.0);
    validate_explicit_material_seam_heights(&regions)
        .expect("preserved seam heights should still validate");
}

#[test]
fn shared_xz_vertices_reject_same_source_height_conflict() {
    let regions = vec![
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
        ),
        manual_heighted_region(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
        ),
    ];

    assert!(matches!(
        validate_shared_source_height_agreement(&regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}
