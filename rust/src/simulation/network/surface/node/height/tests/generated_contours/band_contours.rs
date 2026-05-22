//! Generated band contour validation tests.

use super::*;

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
