use super::*;

#[test]
fn materializes_seam_constraints_for_final_noded_owned_edges() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::CurbOrShoulder,
            curb,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 2.0], [0.0, 2.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 0.0], [2.0, 0.0], [2.0, 2.0], [1.0, 2.0]],
        ),
    ];
    let footprint_shapes = vec![vec![vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 2.0],
        [1.0, 1.0],
        [0.0, 2.0],
    ]]];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: Some(sidewalk),
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];

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
        RoadSurfaceVisualNodePieceKind::JunctionN,
    );

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0))
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && constraint.owner == Some(curb)
                    && constraint.opposite_owner == Some(sidewalk)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "first final subedge must carry the original raised-step seam"
        );
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                ownership_key_from_road_point(constraint.start_xz)
                    == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && ownership_key_from_road_point(constraint.end_xz)
                        == ownership_key_from_road_point(RoadVec2::new(1.0, 2.0))
                    && constraint.owner == Some(curb)
                    && constraint.opposite_owner == Some(sidewalk)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "second final subedge must carry the original raised-step seam"
        );
    }
    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::Terminal,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );

    assert!(arrangement.diagnostics().is_empty());
}

#[test]
fn materializes_owner_explicit_step_for_final_edge_on_exact_constraint_span() {
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
        constraint_index: 34,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
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
            "final shared asphalt-curb edge must carry the owner-explicit step seam"
        );
    }
}

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
fn junctionn_materializes_final_step_edge_from_exact_owner_pair_polyline_authority() {
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
    let footprint_shapes = Vec::new();

    materialize_noded_region_seam_constraints(
        &mut regions,
        &footprint_shapes,
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
                    && constraint.owner == Some(carriageway)
                    && constraint.opposite_owner == Some(curb)
                    && matches!(
                        constraint.seam_source,
                        NodeSeamSource::RaisedStepContact { .. }
                    )
            }),
            "JunctionN final asphalt-curb edge must materialize from exact source-pair polyline authority"
        );
    }

    let arrangement = NodeOwnedRegionArrangement::from_owned_regions(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &regions,
        &footprint_shapes,
        &rail_constraints,
    );
    assert!(arrangement.diagnostics().is_empty());
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
    assert!(dump.contains("source_constraint_indices: [41]"));
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
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

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
