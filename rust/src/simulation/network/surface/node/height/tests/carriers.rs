//! Node height carrier and source-edge authority tests.

use super::*;

#[test]
fn evaluates_owned_region_vertices_from_band_height_fields() {
    let input = solved_input();
    let ownership = solved_ownership(&input);
    let solution = NodeHeightSolution::from_ownership_and_input(&input, &ownership)
        .expect("valid ownership should height every canonical vertex");

    assert_eq!(solution.node_id, 42);
    assert_eq!(
        solution.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(solution.regions.len(), ownership.owned_regions.len());

    let carriageway = solution
        .regions
        .iter()
        .find(|region| region.kind == RoadSurfaceBandKind::Carriageway)
        .expect("test input has a carriageway band");
    assert!(has_vertex_height(carriageway, 0.0, 0.0, 2.2));
    assert!(has_vertex_height(carriageway, 10.0, 2.0, 4.3));
}

#[test]
fn rejects_missing_source_band() {
    let input = conflicting_manual_input();
    let owned_regions = vec![manual_region(RoadSurfaceBandKind::Carriageway, 99, 2.0)];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::Bend,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert_eq!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::MissingSourceBand {
            mouth_order_index: 0,
            band_index: 99,
        })
    );
}

#[test]
fn source_band_height_carrier_rejects_mismatched_explicit_paths() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![
        interval.mouth_end_world,
        RoadVec3::new(7.5, 4.0, 2.0),
        RoadVec3::new(2.5, 2.0, 2.0),
        interval.endpoint_end_world,
    ];

    let result = NodeBandHeightField::from_interval(0, &interval, None);

    assert!(matches!(
        result,
        Err(NodeHeightFieldError::InvalidSourceBandHeightCarrier {
            reason: "mismatched_source_band_path_lengths",
            ..
        })
    ));
}

#[test]
fn source_band_height_carrier_rejects_one_sided_explicit_path_even_with_support_points() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![interval.mouth_end_world, interval.endpoint_end_world];
    let source_support = vec![
        RoadVec3::new(10.0, 4.0, 2.0),
        RoadVec3::new(5.0, 3.0, 2.0),
        RoadVec3::new(0.0, 2.0, 2.0),
    ];

    let result = NodeBandHeightField::from_interval(0, &interval, Some(&source_support));

    assert!(matches!(
        result,
        Err(NodeHeightFieldError::InvalidSourceBandHeightCarrier {
            reason: "mismatched_source_band_path_lengths",
            ..
        })
    ));
}

#[test]
fn source_band_height_carrier_accepts_materialized_two_sided_explicit_paths() {
    let mut interval = manual_interval(0, RoadSurfaceBandKind::Sidewalk, 2.0, 4.0);
    interval.start_path_world = vec![
        interval.mouth_start_world,
        RoadVec3::new(5.0, 6.0, 0.0),
        interval.endpoint_start_world,
    ];
    interval.end_path_world = vec![
        interval.mouth_end_world,
        RoadVec3::new(5.0, 3.0, 2.0),
        interval.endpoint_end_world,
    ];
    let field = NodeBandHeightField::from_interval(0, &interval, None)
        .expect("paired explicit source rails are a valid carrier");

    let height_m = field
        .evaluate_height(RoadVec2::new(5.0, 1.0))
        .expect("canonical point inside explicit source carrier should evaluate");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(height_m),
        SurfaceHeightMmKey::from_m_f64(4.5)
    );
}

#[test]
fn source_support_points_reject_conflicting_duplicate_canonical_height() {
    let source_support = [RoadVec3::new(5.0, 0.5, 0.0), RoadVec3::new(5.0, 0.75, 0.0)];
    let result = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    );

    match result {
        Err(NodeHeightFieldError::SourceHeightFieldConflict {
            mouth_order_index,
            band_index,
            source_kind,
            height_field_id,
            owner,
            existing_authority,
            incoming_authority,
            point_x_mm,
            point_z_mm,
            existing_height_mm,
            incoming_height_mm,
        }) => {
            assert_eq!(mouth_order_index, 0);
            assert_eq!(band_index, 0);
            assert_eq!(source_kind, RoadSurfaceBandKind::Sidewalk);
            assert_eq!(
                height_field_id,
                NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk)
            );
            assert_eq!(owner, None);
            assert_eq!(
                existing_authority,
                NodeHeightAuthoritySource::SourceInterval
            );
            assert_eq!(
                incoming_authority,
                NodeHeightAuthoritySource::SourceInterval
            );
            assert_eq!(point_x_mm, 5000);
            assert_eq!(point_z_mm, 0);
            assert_eq!(existing_height_mm, 500);
            assert_eq!(incoming_height_mm, 750);
        }
        _ => panic!("conflicting source support should reject with source height conflict"),
    }
}

#[test]
fn source_band_height_field_uses_rail_materialized_outer_chord() {
    let mouth = OrderedIncidentPieceMouth {
        profile: profile(10.0, 4.0),
        endpoint_profile: profile(0.0, 2.0),
        boundary_paths_world: Vec::new(),
        band_start_paths_world: vec![vec![
            Vector3::new(10.0, 4.0, -4.0),
            Vector3::new(5.0, 3.0, -4.0),
            Vector3::new(0.0, 2.0, -4.0),
        ]],
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: true,
        direction_angle_ccw: 0.0,
        direction_xz: Vector2::RIGHT,
        edge_idx: 7,
        side: IncidentEdgeSide::Start,
    };
    let input = NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &[mouth],
    )
    .expect("test mouth should produce canonical input");
    let rails = NodeRailContourSet::from_input(&input).expect("rails should materialize carriers");
    let fields = height_fields_by_source(&input, Some(&rails))
        .expect("height fields should consume rail-materialized carrier paths");
    let field = fields
        .get(&NodeSourceBandKey {
            mouth_order_index: 0,
            band_index: 0,
        })
        .expect("test input has a first sidewalk field");

    let height_m = field
        .evaluate_height(RoadVec2::new(5.0, -3.0))
        .expect("materialized outer chord should make the band interior evaluable");

    assert_eq!(
        SurfaceHeightMmKey::from_m_f64(height_m),
        SurfaceHeightMmKey::from_m_f64(3.05)
    );
}

#[test]
fn height_carrier_rejects_duplicate_canonical_xz_with_different_height() {
    let points = [
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.25, 0.0),
        RoadVec3::new(10.0, 0.0, 0.0),
        RoadVec3::new(0.0, 0.0, 2.0),
    ];
    assert!(matches!(
        height_vertex_heights_from_vertices(&points),
        Err(HeightCarrierContourError::ConflictingDuplicateHeightVertex)
    ));

    let id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Sidewalk);
    assert!(matches!(
        height_triangles_from_contour(
            id,
            RoadSurfaceBandKind::Sidewalk,
            NodeHeightPatchAuthority::source_interval(),
            &points,
        ),
        Err(NodeHeightFieldError::InvalidHeightCarrierContour {
            reason: "conflicting_duplicate_height_vertex",
            ..
        })
    ));
}

#[test]
fn rejects_owned_region_vertex_outside_explicit_height_carrier() {
    let input = conflicting_manual_input();
    let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
    region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
    let owned_regions = vec![region];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::Bend,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::Bend,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert!(matches!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::MissingOwnedRegionCarrierSupport {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::Carriageway,
            point_x_mm: 11_000,
            point_z_mm: 1_000,
            ..
        })
    ));
}

#[test]
fn junctionn_canonical_height_authority_rejects_vertex_outside_explicit_carrier() {
    let mut input = conflicting_manual_input();
    input.piece_kind = RoadSurfaceVisualNodePieceKind::JunctionN;
    let mut region = manual_region(RoadSurfaceBandKind::Carriageway, 0, 2.0);
    region.shape = vec![vec![[0.0, 0.0], [10.0, 0.0], [11.0, 1.0], [0.0, 2.0]]];
    let owned_regions = vec![region];
    let ownership = NodeBooleanOwnership {
        node_id: 77,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        footprint_shapes: Vec::new(),
        asphalt_shapes: Vec::new(),
        non_road_shapes: Vec::new(),
        owned_region_arrangement: NodeOwnedRegionArrangement::from_owned_regions(
            77,
            RoadSurfaceVisualNodePieceKind::JunctionN,
            &owned_regions,
            &Vec::new(),
            &[],
        ),
        owned_regions,
    };

    assert!(matches!(
        NodeHeightSolution::from_ownership_and_input(&input, &ownership),
        Err(NodeHeightFieldError::MissingOwnedRegionCarrierSupport {
            mouth_order_index: 0,
            band_index: 0,
            source_kind: RoadSurfaceBandKind::Carriageway,
            point_x_mm: 11_000,
            point_z_mm: 1_000,
            ..
        })
    ));
}

#[test]
fn side_join_height_authority_reuses_source_rail_only_at_canonical_handoff_vertices() {
    let source_support = [RoadVec3::new(5.0, 0.5, 0.0)];
    let mut field = NodeBandHeightField::from_interval(
        0,
        &manual_interval(0, RoadSurfaceBandKind::Sidewalk, 0.0, 1.0),
        Some(&source_support),
    )
    .expect("manual interval is a valid source height carrier");
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 0);
    let points_xz = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(5.0, 0.0),
        RoadVec2::new(10.0, 0.0),
        RoadVec2::new(10.0, 2.0),
        RoadVec2::new(0.0, 2.0),
    ];
    let contour = NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        backend_polyline: road_points_to_polyline(points_xz.clone(), true),
        points_xz,
        height_points_world: Some(vec![
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(5.0, 0.5, 0.0),
            RoadVec3::new(10.0, 1.0, 0.0),
            RoadVec3::new(10.0, 1.0, 2.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ]),
    };
    field
        .extend_with_generated_contour(&contour)
        .expect("test generated contour is a valid height carrier");

    let handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.0),
        )
        .expect("side-join vertex on source rail should reuse source rail height");
    assert_eq!(
        handoff_height.authority,
        NodeHeightAuthoritySource::SourceInterval
    );
    assert!((handoff_height.height_m - 0.5).abs() <= 1.0e-6);

    let source_edge_non_handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(2.5, 0.0),
        )
        .expect(
            "source-edge point without explicit topology handoff should evaluate generated carrier",
        );
    assert_eq!(
        source_edge_non_handoff_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
    assert!(
        (source_edge_non_handoff_height.height_m - 0.25).abs() <= 1.0e-6,
        "exact boundary vertices may evaluate the generated contour, not a drifted source substitute"
    );

    let dedup_drifted_handoff_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.00005),
        )
        .expect("drifted side-join vertex should use generated authority, not source-edge handoff");
    assert_eq!(
        dedup_drifted_handoff_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );

    assert!(
        matches!(
            field.evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, -0.00005)
            ),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ),
        "canonical drift outside the contour must not be accepted as edge support"
    );

    assert!(
        matches!(
            field.evaluate_authorized_height(
                owner,
                NodeGeneratedContourClaimPriority::SideJoin,
                RoadVec2::new(5.0, -0.001)
            ),
            Err(NodeHeightFieldError::VertexOutsideHeightField { .. })
        ),
        "near-edge vertices outside the generated contour must not inherit source-rail height"
    );

    let near_generated_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 0.0002),
        )
        .expect("inside generated contour but off exact handoff should evaluate generated carrier");
    assert_eq!(
        near_generated_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );

    let interior_height = field
        .evaluate_authorized_height(
            owner,
            NodeGeneratedContourClaimPriority::SideJoin,
            RoadVec2::new(5.0, 1.0),
        )
        .expect("side-join interior should still use generated contour authority");
    assert_eq!(
        interior_height.authority,
        NodeHeightAuthoritySource::GeneratedContour {
            purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
            claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        }
    );
}
