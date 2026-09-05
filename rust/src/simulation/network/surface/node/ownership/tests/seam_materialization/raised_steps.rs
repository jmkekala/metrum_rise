// SPDX-License-Identifier: GPL-2.0-only

//! Raised-step seam materialization tests.

use super::*;

#[test]
fn materializes_asymmetric_asphalt_curb_boundary_from_final_noded_edges() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let first_split = RoadVec2::new(1.0, 0.0);
    let second_split = RoadVec2::new(2.0, 0.0);
    let end = RoadVec2::new(3.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [3.0, 0.0], [3.0, -1.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![
                [0.0, 0.0],
                [1.0, 0.0],
                [2.0, 0.0],
                [3.0, 0.0],
                [3.0, 1.0],
                [0.0, 1.0],
            ],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 37,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, first_split, second_split, end],
    }];
    let footprint_shapes = Vec::new();

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_final_owned_region_boundary_edges(
        &mut regions,
        &footprint_shapes,
        &rail_canonical_points,
    )
    .expect("canonical boundary noding should succeed");
    materialize_noded_region_seam_constraints(
        &mut regions,
        &footprint_shapes,
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
    );

    let carriageway_contour = &regions[0].shape[0];
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(first_split))
    );
    assert!(
        carriageway_contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(second_split))
    );
    for (subedge_start, subedge_end) in [
        (start, first_split),
        (first_split, second_split),
        (second_split, end),
    ] {
        for region in &regions {
            assert!(
                region.seam_constraints.iter().any(|constraint| {
                    ownership_key_from_road_point(constraint.start_xz)
                        == ownership_key_from_road_point(subedge_start)
                        && ownership_key_from_road_point(constraint.end_xz)
                            == ownership_key_from_road_point(subedge_end)
                        && constraint.owner == Some(carriageway)
                        && constraint.opposite_owner == Some(curb)
                        && matches!(
                            constraint.seam_source,
                            NodeSeamSource::RaisedStepContact { .. }
                        )
                }),
                "final owned asphalt-curb subedge must carry the exact explicit step seam"
            );
        }
    }

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::Bend,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );
    assert!(arrangement.diagnostics().is_empty());
    assert!(!arrangement.edges().iter().any(|edge| {
        edge.owner == carriageway
            && edge.opposite_owner == Some(curb)
            && edge.start == NodeOwnedRegionArrangementKey::from_point(start)
            && edge.end == NodeOwnedRegionArrangementKey::from_point(end)
    }));
}

#[test]
fn junctionn_reports_unmaterialized_raised_step_authority_before_height_validation() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(3.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [3.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [3.0, 0.0], [3.0, 1.0], [0.0, 1.0]],
        ),
    ];
    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 7,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: Some(carriageway),
            opposite_owner: Some(curb),
            constrains_shared_height: true,
            is_material_transition: true,
            start_xz: start,
            end_xz: end,
        });
    }
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 41,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![
            start,
            RoadVec2::new(1.0, 0.000001),
            RoadVec2::new(2.0, -0.000001),
            end,
        ],
    }];

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &Vec::new(),
        &rail_constraints,
    );

    assert!(arrangement.diagnostics().iter().any(|diagnostic| matches!(
        diagnostic,
        NodeOwnedRegionArrangementDiagnostic::UnmaterializedRaisedStepAuthority {
            region_index: 0,
            owner,
            opposite_owner,
            start,
            end,
            source_constraint_indices,
        } if *owner == carriageway
            && *opposite_owner == curb
            && *start == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(0.0, 0.0))
            && *end == NodeOwnedRegionArrangementKey::from_point(RoadVec2::new(3.0, 0.0))
            && source_constraint_indices.as_slice() == [41]
    )));
    let report = NodeValidationReport::from_owned_region_arrangement_diagnostics(&arrangement)
        .expect("unmaterialized authority must block before height validation");
    let dump = report.debug_dump();
    assert!(dump.contains("\"kind\":\"unmaterialized_raised_step_authority\""));
    assert!(dump.contains("\"backend\":\"canonical_keys\""));
    let parsed: serde_json::Value =
        serde_json::from_str(&dump).expect("diagnostic dump must be valid JSON");
    assert_eq!(parsed["diagnostics"][0]["source_constraint_indices"][0], 41);
}

#[test]
fn materializes_role_only_raised_step_contact_as_exact_owned_edge_pair() {
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
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![start, end],
    }];

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &rail_constraints,
        RoadSurfaceVisualNodePieceKind::Bend,
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
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "role-only asphalt-curb contact must instantiate the actual owned edge pair"
        );
    }
}

#[test]
fn materializes_same_kind_reowned_raised_step_contact_as_exact_owned_edge_pair() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let source_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let final_curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 2);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            final_curb,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
        ),
    ];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 35,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(source_curb),
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
                        && constraint.opposite_owner == Some(final_curb))
                        || (constraint.owner == Some(final_curb)
                            && constraint.opposite_owner == Some(carriageway)))
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "final owned edge must instantiate its exact owner pair from a same-kind source rail"
        );
    }
}

#[test]
fn materializes_same_material_owned_boundary_as_explicit_height_split() {
    let first_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let second_sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let start = RoadVec2::new(0.0, 0.0);
    let end = RoadVec2::new(2.0, 0.0);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            first_sidewalk,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, -1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            second_sidewalk,
            vec![[0.0, 0.0], [2.0, 0.0], [0.0, 1.0]],
        ),
    ];
    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 208,
            seam_source: NodeSeamSource::FootprintBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: Some(region.owner),
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: false,
            start_xz: start,
            end_xz: end,
        });
    }

    materialize_noded_region_seam_constraints(
        &mut regions,
        &Vec::new(),
        &[],
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(region.seam_constraints.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(start)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(end)
                && ((constraint.owner == Some(first_sidewalk)
                    && constraint.opposite_owner == Some(second_sidewalk))
                    || (constraint.owner == Some(second_sidewalk)
                        && constraint.opposite_owner == Some(first_sidewalk)))
                && !constraint.constrains_shared_height
                && constraint.is_material_transition
        }));
    }
}

#[test]
fn asphalt_curb_shape_seams_use_exact_constraint_spans() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let start = RoadVec2::new(0.0, 0.0);
    let middle = RoadVec2::new(1.0, 1.0);
    let end = RoadVec2::new(2.0, 2.0);
    let shape = vec![vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0]]];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 36,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![start, middle, end],
    }];

    let seams = seam_constraints_for_shape(
        &shape,
        carriageway,
        &rail_constraints,
        ConstraintOverlapMode::ExactCanonical,
    );

    assert!(
        !seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(start)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(end)
        }),
        "asphalt-curb seams must not carry a full edge just because a rail polyline covers it"
    );
    assert!(
        seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(start)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(middle)
        }),
        "first exact rail span should be preserved"
    );
    assert!(
        seams.iter().any(|constraint| {
            ownership_key_from_road_point(constraint.start_xz)
                == ownership_key_from_road_point(middle)
                && ownership_key_from_road_point(constraint.end_xz)
                    == ownership_key_from_road_point(end)
        }),
        "second exact rail span should be preserved"
    );
}
