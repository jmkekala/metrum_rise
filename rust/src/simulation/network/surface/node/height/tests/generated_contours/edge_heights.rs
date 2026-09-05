// SPDX-License-Identifier: GPL-2.0-only

//! Generated contour edge-height support tests.

use super::*;

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
