//! Negative seam materialization tests.

use super::*;

#[test]
fn does_not_materialize_cross_material_contact_from_band_contour_chord() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let middle = RoadVec2::new(1.0, 1.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 42,
        kind: NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(carriageway),
        opposite_owner: None,
        points_xz: vec![start, middle, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            !region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "band contours authorize final contacts only on exact source segments"
        );
    }
}

#[test]
fn does_not_materialize_asphalt_curb_step_from_bend_polyline_coverage() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 2.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [2.0, 2.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, RoadVec2::new(1.0, 1.0), end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    for region in &regions {
        assert!(
            !region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "asphalt-curb vertical steps must come from an exact rail span, not Bend polyline coverage"
        );
    }
}
