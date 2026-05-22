//! Generated contour source-handoff tests.

use super::*;

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
