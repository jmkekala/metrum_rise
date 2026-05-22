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
