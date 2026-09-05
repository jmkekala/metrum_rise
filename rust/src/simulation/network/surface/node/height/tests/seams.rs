// SPDX-License-Identifier: GPL-2.0-only

//! Explicit seam height-continuity tests.

use super::*;

#[test]
fn same_material_seam_rejects_shared_height_disagreement() {
    let seam = manual_seam_constraint(
        88,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        false,
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

    assert!(matches!(
        apply_junctionn_height_authority_normalization(&mut regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}

#[test]
fn explicit_curb_sidewalk_seam_rejects_shared_height_disagreement() {
    let seam = manual_seam_constraint(
        12,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        true,
    );
    let regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            vec![seam],
        ),
    ];

    assert!(matches!(
        validate_explicit_material_seam_heights(&regions),
        Err(NodeHeightFieldError::SharedSourceHeightConflict { .. })
    ));
}

#[test]
fn explicit_curb_sidewalk_shared_height_seam_uses_sidewalk_height_authority() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 4);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let seam = manual_owned_pair_seam_constraint(21, curb_owner, sidewalk_owner, true);
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 138.184)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 138.210)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("curb / sidewalk shared-height seams should use sidewalk height authority");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        138_210
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        138_210
    );
    assert_eq!(
        regions[0].shape[0][0]
            .grade_authority
            .expect("curb vertex should record explicit seam authority")
            .decision,
        NodeGradeCarrierDecision::ExplicitMaterialSeam
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("sidewalk vertex should record explicit seam authority")
            .decision,
        NodeGradeCarrierDecision::ExplicitMaterialSeam
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("normalized curb / sidewalk seam should validate as shared height");
}

#[test]
fn fragmented_curb_sidewalk_shared_height_seam_uses_sidewalk_height_authority() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11);
    let curb_fragment = NodeRegionSeamConstraint {
        constraint_index: 62,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: curb_owner.owner_index(),
        },
        owner: Some(curb_owner),
        opposite_owner: Some(sidewalk_owner),
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(-1.0, 0.0),
        end_xz: RoadVec2::new(0.0, 0.0),
    };
    let sidewalk_fragment = NodeRegionSeamConstraint {
        constraint_index: 62,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: sidewalk_owner.owner_index(),
        },
        owner: Some(curb_owner),
        opposite_owner: Some(sidewalk_owner),
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    };
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 137.894)],
            vec![curb_fragment],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 137.911)],
            vec![sidewalk_fragment],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("fragmented curb / sidewalk shared-height seams should normalize");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        137_911
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        137_911
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("fragmented shared-height seam should validate after normalization");
}

#[test]
fn fragmented_curb_sidewalk_seam_infers_shared_height_from_owner_pair() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11);
    let curb_fragment = NodeRegionSeamConstraint {
        constraint_index: 62,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: curb_owner.owner_index(),
        },
        owner: Some(curb_owner),
        opposite_owner: Some(sidewalk_owner),
        constrains_shared_height: false,
        is_material_transition: true,
        start_xz: RoadVec2::new(-1.0, 0.0),
        end_xz: RoadVec2::new(0.0, 0.0),
    };
    let sidewalk_fragment = NodeRegionSeamConstraint {
        constraint_index: 62,
        seam_source: NodeSeamSource::RaisedStepContact {
            owner_index: sidewalk_owner.owner_index(),
        },
        owner: Some(curb_owner),
        opposite_owner: Some(sidewalk_owner),
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    };
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 151.339)],
            vec![curb_fragment],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 151.340)],
            vec![sidewalk_fragment],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions).expect(
        "curb / sidewalk owner pairs should normalize all fragmented shared-height contacts",
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        151_340
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        151_340
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("mixed-flag curb / sidewalk seam should validate after normalization");
}

#[test]
fn same_xz_curb_sidewalk_boundary_vertex_uses_sidewalk_height_authority() {
    let curb_owner = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11);
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb_owner.owner_index(),
            0.0,
            vec![manual_heighted_vertex(-42.867205, 73.035502, 137.845)],
            Vec::new(),
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk_owner.owner_index(),
            0.25,
            vec![manual_heighted_vertex(-42.867205, 73.035502, 137.836)],
            Vec::new(),
        ),
    ];

    apply_bend_height_authority_normalization(&mut regions)
        .expect("same-XZ curb / sidewalk boundary vertices should use sidewalk height authority");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        137_836
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        137_836
    );
    assert_eq!(
        regions[0].shape[0][0]
            .grade_authority
            .expect("curb vertex should record shared-height raised-step authority")
            .decision,
        NodeGradeCarrierDecision::ExplicitMaterialSeam
    );
}

#[test]
fn same_xz_carriageway_curb_boundary_vertex_keeps_vertical_height_split() {
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 12.0)],
            Vec::new(),
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 12.25)],
            Vec::new(),
        ),
    ];

    apply_bend_height_authority_normalization(&mut regions)
        .expect("carriageway / curb same-XZ contacts remain vertical steps");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        12_000
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        12_250
    );
}

#[test]
fn explicit_curb_sidewalk_seam_accepts_matching_quantized_shared_height() {
    let seam = manual_seam_constraint(
        12,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.2504)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25049)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit material seams may normalize only equal height keys");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m),
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m)
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("explicit seam authority may only accept matching height keys");
}

#[test]
fn explicit_curb_sidewalk_seam_normalizes_one_mm_height_dust() {
    let seam = manual_seam_constraint(
        12,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        true,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 150.274)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 150.275)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit material seams may snap one millimetre of source-height dust");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m),
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m)
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("dust snap should record explicit seam authority")
            .decision,
        NodeGradeCarrierDecision::ExplicitMaterialSeam
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("one millimetre dust must be normalized before seam validation");
}

#[test]
fn same_source_constraint_index_keeps_distinct_owner_pair_height_contexts() {
    let first_pair = manual_owned_pair_seam_constraint(
        12,
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0),
        NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1),
        true,
    );
    let second_pair = manual_owned_pair_seam_constraint(
        12,
        NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2),
        NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3),
        true,
    );
    let regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![first_pair.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 1.0)],
            vec![first_pair],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            2,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
            vec![second_pair.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            3,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 2.0)],
            vec![second_pair],
        ),
    ];

    validate_explicit_material_seam_heights(&regions)
        .expect("same source rail index may materialize distinct final owner-pair seams");
}

#[test]
fn asphalt_curb_seams_allow_explicit_vertical_height_step() {
    let seam = manual_seam_constraint(
        3,
        NodeSeamSource::RaisedStepContact { owner_index: 0 },
        false,
        true,
    );
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Carriageway,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::CurbOrShoulder,
            1,
            0.25,
            vec![manual_heighted_vertex(0.0, 0.0, 0.25)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("explicit vertical steps should not enter shared-height seam normalization");
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m).as_i64(),
        0
    );
    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m).as_i64(),
        250
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("asphalt / curb contact is a vertical material step, not shared-height correction");
}

#[test]
fn sidewalk_footpath_tie_in_uses_sidewalk_height_authority() {
    let sidewalk_owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let footpath_owner = NodeBandOwner::new(RoadSurfaceBandKind::Footpath, 1);
    let seam = NodeRegionSeamConstraint {
        constraint_index: 41,
        seam_source: NodeSeamSource::SidewalkOuter { owner_index: 0 },
        owner: Some(sidewalk_owner),
        opposite_owner: Some(footpath_owner),
        constrains_shared_height: true,
        is_material_transition: true,
        start_xz: RoadVec2::new(0.0, 0.0),
        end_xz: RoadVec2::new(1.0, 0.0),
    };
    let mut regions = vec![
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Sidewalk,
            0,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.12)],
            vec![seam.clone()],
        ),
        manual_heighted_region_with_seams(
            RoadSurfaceBandKind::Footpath,
            1,
            0.0,
            vec![manual_heighted_vertex(0.0, 0.0, 0.0)],
            vec![seam],
        ),
    ];

    apply_junctionn_height_authority_normalization(&mut regions)
        .expect("sidewalk-footpath tie-ins should use sidewalk height authority");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(regions[0].shape[0][0].height_m),
        SurfaceHeightMmKey::from_m_f64(regions[1].shape[0][0].height_m)
    );
    assert_eq!(
        regions[1].shape[0][0]
            .grade_authority
            .expect("footpath tie-in should record explicit grade authority")
            .decision,
        NodeGradeCarrierDecision::ExplicitMaterialSeam
    );
    validate_explicit_material_seam_heights(&regions)
        .expect("normalized sidewalk-footpath seam should validate as shared-height");
}
