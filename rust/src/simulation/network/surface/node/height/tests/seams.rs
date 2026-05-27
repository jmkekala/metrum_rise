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
    let regions = vec![
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
