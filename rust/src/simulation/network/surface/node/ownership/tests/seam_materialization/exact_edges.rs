//! Exact final-edge seam materialization tests.

use super::*;

#[test]
fn terminal_overlap_mode_uses_exact_canonical_constraints_without_grid_bounded_overlap() {
    let terminal_mode =
        ConstraintOverlapMode::for_piece_kind(RoadSurfaceVisualNodePieceKind::Terminal);
    let bend_mode = ConstraintOverlapMode::for_piece_kind(RoadSurfaceVisualNodePieceKind::Bend);
    let junction_mode =
        ConstraintOverlapMode::for_piece_kind(RoadSurfaceVisualNodePieceKind::JunctionN);

    assert_eq!(terminal_mode, ConstraintOverlapMode::ExactCanonical);
    assert_eq!(bend_mode, ConstraintOverlapMode::ExactCanonical);
    assert_eq!(junction_mode, ConstraintOverlapMode::GridBounded);
    assert!(!terminal_mode.allows_grid_bounded_constraint_overlap());
    assert!(junction_mode.allows_grid_bounded_constraint_overlap());
    assert_eq!(
        terminal_mode.cleans_overlay_numeric_spikes(),
        bend_mode.cleans_overlay_numeric_spikes()
    );
}

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
