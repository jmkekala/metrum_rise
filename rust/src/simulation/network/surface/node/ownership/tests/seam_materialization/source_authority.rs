// SPDX-License-Identifier: GPL-2.0-only

//! Source-authorized material boundary tests.

use super::*;

#[test]
fn reowned_raised_step_contact_does_not_inherit_source_pair_shared_height_contract() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let source_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let final_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(0.0, 3.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [0.0, 3.0], [-1.0, 0.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            final_sidewalk,
            vec![[0.0, 0.0], [1.0, 0.0], [0.0, 3.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: Some(source_sidewalk),
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(final_sidewalk))
                        || (constraint.owner == Some(final_sidewalk)
                            && constraint.opposite_owner == Some(curb)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "reowned side-join contact must authorize the exact final edge without forcing source-pair shared height"
        );
    }
}

#[test]
fn endpoint_pair_raised_step_contacts_do_not_constrain_full_edge_shared_height() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[0.0, 0.0], [2.0, 1.0], [2.0, 0.0]],
        ),
    ];
    let rail_constraints = vec![
        NodeRailConstraint {
            constraint_index: 70,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: None,
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![start, start],
        },
        NodeRailConstraint {
            constraint_index: 71,
            kind: NodeRailConstraintKind::RaisedStepContact,
            source_mouth_order_index: 0,
            source_band_index: None,
            source_boundary_index: None,
            owner: Some(curb),
            opposite_owner: Some(sidewalk),
            points_xz: vec![end, end],
        },
    ];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                let constraint_start = ownership_key_from_road_point(constraint.start_xz);
                let constraint_end = ownership_key_from_road_point(constraint.end_xz);
                ((constraint_start == ownership_key_from_road_point(start)
                    && constraint_end == ownership_key_from_road_point(end))
                    || (constraint_start == ownership_key_from_road_point(end)
                        && constraint_end == ownership_key_from_road_point(start)))
                    && ((constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(sidewalk))
                        || (constraint.owner == Some(sidewalk)
                            && constraint.opposite_owner == Some(curb)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
            }),
            "endpoint-pair seam evidence must not impose a full-edge shared height"
        );
    }
}

#[test]
fn junctionn_materializes_reowned_curb_sidewalk_step_from_source_polyline_coverage() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 0);
    let source_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let final_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let middle = RoadVec2::new(1.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            final_sidewalk,
            vec![[0.0, 0.0], [2.0, 1.0], [2.0, 0.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 53,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: Some(source_sidewalk),
        points_xz: vec![start, middle, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(curb)
                        && constraint.opposite_owner == Some(final_sidewalk))
                        || (constraint.owner == Some(final_sidewalk)
                            && constraint.opposite_owner == Some(curb)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "source-authorized curb-sidewalk polyline must materialize the final noded JunctionN edge"
        );
    }
}

#[test]
fn materializes_cross_material_contact_from_exact_final_owner_band_contour_edge() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 1.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 1.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, -1.0], [2.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::BandContour {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(carriageway),
        opposite_owner: None,
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(start)
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(end)
                    && ((constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb))
                        || (constraint.owner == Some(curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && !constraint.constrains_shared_height
                    && constraint.is_material_transition
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "exact final owner band contour edge must authorize the asphalt-curb step"
        );
    }
}

#[test]
fn projected_material_boundary_canonicalizes_source_authorized_endpoint() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let source_start = RoadVec2::new(1.0, 0.0);
    let drifted_start = [1.000004, 0.0];
    let end = RoadVec2::new(1.0, 2.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![drifted_start, [2.0, 0.0], [2.0, 2.0], [1.000004, 2.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![source_start, end],
    }];
    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("canonical rail point adoption should succeed");

    let curb_points = regions[1].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(curb_points.contains(&ownership_key_from_road_point(source_start)));
    assert!(!curb_points.contains(&ownership_key_from_overlay_point(drifted_start)));
}
