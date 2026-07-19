//! Boolean ownership domain tests.

use super::*;

#[test]
fn boolean_ownership_produces_asphalt_and_band_owned_regions() {
    let ownership =
        NodeBooleanOwnership::from_rails(&contour_set()).expect("valid ownership solve");

    assert_eq!(ownership.node_id, 42);
    assert_eq!(
        ownership.piece_kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(ownership.footprint_shapes.len(), 1);
    assert_eq!(ownership.asphalt_shapes.len(), 1);
    assert_eq!(ownership.non_road_shapes.len(), 2);
    assert_eq!(ownership.owned_regions.len(), 4);
    assert_eq!(ownership.owned_region_arrangement.region_count(), 4);
    assert!(ownership.owned_region_arrangement.diagnostics().is_empty());
    assert!(!ownership.owned_region_arrangement.edges().is_empty());
    assert!(
        ownership
            .owned_regions
            .iter()
            .any(|region| region.kind == RoadSurfaceBandKind::Carriageway
                && region.owner == NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2)
                && !region.seam_constraints.is_empty())
    );
    assert!(
        ownership.owned_regions.iter().any(|region| {
            region.seam_constraints.iter().any(|constraint| {
                matches!(
                    constraint.seam_source,
                    NodeSeamSource::RaisedStepContact { .. }
                        | NodeSeamSource::FootprintBoundary { .. }
                )
            })
        }),
        "owned regions must preserve source rail seam constraints"
    );
    assert_eq!(
        ownership
            .owned_regions
            .iter()
            .filter(|region| region.kind == RoadSurfaceBandKind::Sidewalk)
            .count(),
        2
    );
}

#[test]
fn exact_ownership_rebuild_reuses_cleanup_and_seam_contributors() {
    let rails = contour_set();
    let (cold, previous, cold_stats) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(&rails, None)
            .expect("cold ownership solve");
    assert!(cold_stats.cleanup_cache_misses > 0);
    assert!(cold_stats.seam_extraction_cache_misses > 0);
    assert!(cold_stats.edge_seam_cache_misses > 0);

    let (warm, _, warm_stats) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(&rails, Some(&previous))
            .expect("warm ownership solve");

    assert_eq!(warm, cold);
    assert!(warm_stats.cleanup_previous_hits > 0);
    assert!(warm_stats.seam_extraction_previous_hits > 0);
    assert!(warm_stats.edge_seam_previous_hits > 0);
}

#[test]
fn local_junction_mouth_addition_and_removal_reuse_unchanged_ownership_contributors() {
    let three_mouth_rails = junction_contour_set(false);
    let four_mouth_rails = junction_contour_set(true);

    let (three_mouth_cold, three_mouth_cache, _) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(&three_mouth_rails, None)
            .expect("three-mouth cold ownership solve");
    let (four_mouth_warm, four_mouth_cache, addition_stats) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(
            &four_mouth_rails,
            Some(&three_mouth_cache),
        )
        .expect("four-mouth incremental ownership solve");
    let four_mouth_cold = NodeBooleanOwnership::from_rails(&four_mouth_rails)
        .expect("four-mouth cold ownership solve");

    assert_eq!(four_mouth_warm, four_mouth_cold);
    assert!(
        addition_stats.cleanup_previous_hits > 0,
        "adding one mouth must reuse unchanged cleanup contributors: {addition_stats:?}"
    );

    let (three_mouth_warm, _, removal_stats) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(
            &three_mouth_rails,
            Some(&four_mouth_cache),
        )
        .expect("three-mouth incremental ownership solve after removal");

    assert_eq!(three_mouth_warm, three_mouth_cold);
    assert!(
        removal_stats.cleanup_previous_hits > 0,
        "removing one mouth must reuse unchanged cleanup contributors: {removal_stats:?}"
    );
}

#[test]
fn local_junction_height_edit_reuses_unchanged_final_boundary_points() {
    let baseline_rails = junction_contour_set_with_fourth_height(0.0);
    let changed_rails = junction_contour_set_with_fourth_height(0.75);
    let (_, baseline_cache, _) =
        NodeBooleanOwnership::from_rails_with_incremental_reuse(&baseline_rails, None)
            .expect("baseline four-mouth ownership solve");
    let (warm, _, stats) = NodeBooleanOwnership::from_rails_with_incremental_reuse(
        &changed_rails,
        Some(&baseline_cache),
    )
    .expect("height-edited incremental ownership solve");
    let cold = NodeBooleanOwnership::from_rails(&changed_rails)
        .expect("height-edited cold ownership solve");

    assert_eq!(warm, cold);
    assert!(
        stats.final_boundary_previous_hits > 0,
        "a one-mouth height edit must reuse unchanged final-boundary point decisions: {stats:?}"
    );
    assert!(
        stats.final_assembly_previous_hits > 0,
        "a one-mouth height edit with unchanged XZ ownership must reuse final-boundary assembly: {stats:?}"
    );
}

fn junction_contour_set(include_fourth_mouth: bool) -> NodeRailContourSet {
    junction_contour_set_with_optional_fourth_height(include_fourth_mouth.then_some(0.0))
}

fn junction_contour_set_with_fourth_height(height_delta: f64) -> NodeRailContourSet {
    junction_contour_set_with_optional_fourth_height(Some(height_delta))
}

fn junction_contour_set_with_optional_fourth_height(
    fourth_height_delta: Option<f64>,
) -> NodeRailContourSet {
    let mut mouths = vec![
        junction_mouth(
            symmetric_profile_x(10.0, RoadVec2::X),
            symmetric_profile_x(0.0, RoadVec2::X),
            0.0,
            RoadVec2::X,
            1,
        ),
        junction_mouth(
            symmetric_profile_z(12.0, RoadVec2::Y),
            symmetric_profile_z(0.0, RoadVec2::Y),
            std::f32::consts::FRAC_PI_2,
            RoadVec2::Y,
            2,
        ),
        junction_mouth(
            symmetric_profile_x(-10.0, RoadVec2::NEG_X),
            symmetric_profile_x(0.0, RoadVec2::NEG_X),
            std::f32::consts::PI,
            RoadVec2::NEG_X,
            3,
        ),
    ];
    if let Some(height_delta) = fourth_height_delta {
        let mut fourth = junction_mouth(
            symmetric_profile_z(-12.0, RoadVec2::NEG_Y),
            symmetric_profile_z(0.0, RoadVec2::NEG_Y),
            std::f32::consts::PI + std::f32::consts::FRAC_PI_2,
            RoadVec2::NEG_Y,
            4,
        );
        translate_mouth_height(&mut fourth, height_delta);
        mouths.push(fourth);
    }
    let input = NodeArrangementInput::from_ordered_mouths(
        42,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        &mouths,
    )
    .expect("junction mouths should produce canonical input");
    NodeRailContourSet::from_input(&input).expect("junction input should produce contours")
}

fn translate_mouth_height(mouth: &mut OrderedIncidentPieceMouth, delta: f64) {
    for profile in [&mut mouth.profile, &mut mouth.endpoint_profile] {
        for point in &mut profile.boundary_points_world {
            point.y += delta;
        }
        for band in &mut profile.bands {
            band.start_point_world.y += delta;
            band.end_point_world.y += delta;
        }
    }
}

fn junction_mouth(
    profile: IncidentMouthProfile,
    endpoint_profile: IncidentMouthProfile,
    direction_angle_ccw: f32,
    direction_xz: RoadVec2,
    edge_idx: usize,
) -> OrderedIncidentPieceMouth {
    OrderedIncidentPieceMouth {
        profile,
        endpoint_profile,
        boundary_paths_world: Vec::new(),
        band_start_paths_world: Vec::new(),
        band_end_paths_world: Vec::new(),
        uses_explicit_band_domain_paths: false,
        direction_angle_ccw,
        direction_xz,
        edge_idx,
        side: IncidentEdgeSide::Start,
    }
}

fn symmetric_profile_x(x: f64, inward_direction_xz: RoadVec2) -> IncidentMouthProfile {
    symmetric_profile(
        vec![
            RoadVec3::new(x, 4.0, -4.0),
            RoadVec3::new(x, 4.1, -3.0),
            RoadVec3::new(x, 4.2, -1.0),
            RoadVec3::new(x, 4.0, 0.0),
            RoadVec3::new(x, 4.2, 1.0),
            RoadVec3::new(x, 4.1, 3.0),
            RoadVec3::new(x, 4.0, 4.0),
        ],
        inward_direction_xz,
    )
}

fn symmetric_profile_z(z: f64, inward_direction_xz: RoadVec2) -> IncidentMouthProfile {
    symmetric_profile(
        vec![
            RoadVec3::new(4.0, 4.0, z),
            RoadVec3::new(3.0, 4.1, z),
            RoadVec3::new(1.0, 4.2, z),
            RoadVec3::new(0.0, 4.0, z),
            RoadVec3::new(-1.0, 4.2, z),
            RoadVec3::new(-3.0, 4.1, z),
            RoadVec3::new(-4.0, 4.0, z),
        ],
        inward_direction_xz,
    )
}

fn symmetric_profile(
    boundary_points_world: Vec<RoadVec3>,
    inward_direction_xz: RoadVec2,
) -> IncidentMouthProfile {
    let bands = vec![
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[0],
            boundary_points_world[1],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[1],
            boundary_points_world[2],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[2],
            boundary_points_world[3],
        ),
        band(
            RoadSurfaceBandKind::Carriageway,
            boundary_points_world[3],
            boundary_points_world[4],
        ),
        band(
            RoadSurfaceBandKind::CurbOrShoulder,
            boundary_points_world[4],
            boundary_points_world[5],
        ),
        band(
            RoadSurfaceBandKind::Sidewalk,
            boundary_points_world[5],
            boundary_points_world[6],
        ),
    ];
    IncidentMouthProfile {
        inward_direction_xz,
        boundary_points_world,
        bands,
    }
}

#[test]
fn boolean_ownership_rejects_unowned_non_road_residual() {
    let mut rails = contour_set();
    rails.contours.retain(|contour| {
        contour.kind == NodeGeneratedContourKind::FullRoadbed
            || contour.kind
                == NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::Carriageway,
                }
    });

    let error = NodeBooleanOwnership::from_rails(&rails)
        .expect_err("non-road footprint without band contours must be rejected");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::UnownedNonRoadResidual { .. }
    ));
}

#[test]
fn non_road_owner_regions_require_explicit_profile_seam_rails() {
    let mut rails = contour_set();
    rails.constraints.retain(|constraint| {
        matches!(
            constraint.kind,
            NodeRailConstraintKind::FullRoadbedContour | NodeRailConstraintKind::BandContour { .. }
        )
    });

    let error = NodeBooleanOwnership::from_rails(&rails)
        .expect_err("non-road owner carriers without profile seam rails must be rejected");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::UnownedBandResidual {
            kind: RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk,
            ..
        }
    ));
    let report =
        NodeValidationReport::from_boolean_ownership_error(rails.node_id, rails.piece_kind, &error);
    let dump = report.debug_dump();
    assert!(dump.contains("\"stage\":\"boolean_ownership\""));
    assert!(dump.contains("\"kind\":\"rejected_residual\""));
}

#[test]
fn side_join_non_road_authority_claims_before_mouth_band_carriers() {
    let mut rails = contour_set();
    let mut side_join = rails
        .contours
        .iter()
        .find(|contour| {
            contour.kind
                == (NodeGeneratedContourKind::Band {
                    kind: RoadSurfaceBandKind::CurbOrShoulder,
                })
                && contour.purpose == NodeGeneratedContourPurpose::NonRoadBand
        })
        .cloned()
        .expect("test rail set should include a curb/shoulder mouth carrier");
    side_join.purpose = NodeGeneratedContourPurpose::JunctionSideJoin;
    side_join.claim_priority = NodeGeneratedContourClaimPriority::SideJoin;
    let owner = side_join.owner.expect("band contour has an owner");
    let source_mouth_order_index = side_join.source_mouth_order_index;
    let source_band_index = side_join.source_band_index;
    rails.contours.push(side_join);

    let ownership =
        NodeBooleanOwnership::from_rails(&rails).expect("side-join ownership remains valid");

    assert!(
        ownership.owned_regions.iter().any(|region| {
            region.kind == RoadSurfaceBandKind::CurbOrShoulder
                && region.owner == owner
                && region.source_mouth_order_index == source_mouth_order_index
                && region.source_band_index == source_band_index
                && region.claim_priority == NodeGeneratedContourClaimPriority::SideJoin
        }),
        "overlapping side-join contours must own their final non-road region before ordinary mouth carriers"
    );
    assert!(
        !ownership.owned_regions.iter().any(|region| {
            region.kind == RoadSurfaceBandKind::CurbOrShoulder
                && region.owner == owner
                && region.source_mouth_order_index == source_mouth_order_index
                && region.source_band_index == source_band_index
                && region.claim_priority == NodeGeneratedContourClaimPriority::MouthBand
        }),
        "the same final region must not remain mouth-band-owned after side-join authority claims it"
    );
}

#[test]
fn contour_purpose_gates_junction_footprint_and_asphalt_authority() {
    let mut rails = contour_set();
    let baseline =
        NodeBooleanOwnership::from_rails(&rails).expect("baseline ownership solve is valid");
    let ignored_footprint_points = vec![
        RoadVec2::new(100.0, 100.0),
        RoadVec2::new(102.0, 100.0),
        RoadVec2::new(102.0, 102.0),
        RoadVec2::new(100.0, 102.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::FullRoadbed,
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: 0,
        source_band_index: None,
        owner: None,
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        points_xz: ignored_footprint_points.clone(),
        height_points_world: None,
        backend_polyline: road_points_to_polyline(ignored_footprint_points, true),
    });

    let outside_asphalt_points = vec![
        RoadVec2::new(110.0, 100.0),
        RoadVec2::new(112.0, 100.0),
        RoadVec2::new(112.0, 102.0),
        RoadVec2::new(110.0, 102.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band {
            kind: RoadSurfaceBandKind::Carriageway,
        },
        purpose: NodeGeneratedContourPurpose::CarriagewayCorridor,
        source_mouth_order_index: 99,
        source_band_index: Some(99),
        owner: Some(NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 99)),
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        points_xz: outside_asphalt_points.clone(),
        height_points_world: None,
        backend_polyline: road_points_to_polyline(outside_asphalt_points, true),
    });
    let ownership =
        NodeBooleanOwnership::from_rails(&rails).expect("extra gated contours remain valid");
    assert_eq!(ownership.footprint_shapes, baseline.footprint_shapes);
    assert_eq!(ownership.asphalt_shapes, baseline.asphalt_shapes);
    let asphalt_outside = overlay_difference(
        &ownership.asphalt_shapes,
        &ownership.footprint_shapes,
        "test_asphalt_outside_footprint",
    )
    .expect("test overlay difference succeeds");
    assert!(
        asphalt_outside.is_empty(),
        "asphalt authority must be clipped to node_footprint"
    );
}

#[test]
fn protected_span_handoff_dust_stays_owned() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
    let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];
    let constraints = vec![NodeRailConstraint {
        constraint_index: 7,
        kind: NodeRailConstraintKind::SpanHandoff {
            kind: RoadSurfaceBandKind::Sidewalk,
        },
        source_mouth_order_index: 0,
        source_band_index: Some(0),
        source_boundary_index: None,
        owner: Some(owner),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(0.0001, 0.0)],
    }];

    assert!(
        !owned_shape_is_discardable_numeric_dust(
            &shape,
            RoadSurfaceSystem::overlay_shape_area_m2(&shape),
            owner,
            &constraints,
        ),
        "span-handoff dust must remain an owned top region so mouth/skirt seams cannot point at missing top mesh"
    );
}

#[test]
fn protected_material_transition_dust_stays_owned() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];
    let constraints = vec![NodeRailConstraint {
        constraint_index: 9,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(carriageway),
        opposite_owner: Some(curb),
        points_xz: vec![RoadVec2::new(0.0, 0.0), RoadVec2::new(0.0001, 0.0)],
    }];

    assert!(
        !owned_shape_is_discardable_numeric_dust(
            &shape,
            RoadSurfaceSystem::overlay_shape_area_m2(&shape),
            curb,
            &constraints,
        ),
        "material-transition dust must remain owned so asphalt and sidewalk cannot become directly adjacent"
    );
}

#[test]
fn unprotected_numeric_dust_can_still_be_discarded() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 3);
    let shape = vec![vec![[0.0, 0.0], [0.0001, 0.0], [0.0, 0.0001]]];

    assert!(owned_shape_is_discardable_numeric_dust(
        &shape,
        RoadSurfaceSystem::overlay_shape_area_m2(&shape),
        owner,
        &[],
    ));
}

#[test]
fn unsupported_asphalt_adjacent_sidewalk_sliver_stays_rejected() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        ),
        NodeBooleanOwnedRegion {
            kind: RoadSurfaceBandKind::Sidewalk,
            owner: sidewalk,
            claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
            source_mouth_order_index: 1,
            source_band_index: Some(1),
            shape: vec![vec![[0.0, 0.0], [0.1, 0.0], [0.0, 0.002]]],
            area_m2: 0.0001,
            seam_constraints: Vec::new(),
        },
    ];
    let arrangement = unsupported_asphalt_contact_arrangement(sidewalk, carriageway, 1);

    assert_eq!(regions.len(), 2);
    let report = NodeValidationReport::from_owned_region_arrangement_diagnostics(&arrangement)
        .expect("missing seam diagnostic must be surfaced instead of repaired");
    assert!(report.has_blocking_diagnostics());
    assert!(report.debug_dump().contains("\"reason\":\"Missing\""));
}

fn unsupported_asphalt_contact_arrangement(
    owner: NodeBandOwner,
    opposite_owner: NodeBandOwner,
    region_index: usize,
) -> NodeOwnedRegionArrangement {
    let start = NodeOwnedRegionArrangementKey { x_key: 0, z_key: 0 };
    let end = NodeOwnedRegionArrangementKey {
        x_key: 1_000_000,
        z_key: 0,
    };
    NodeOwnedRegionArrangement {
        node_id: 7,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        region_count: 2,
        edges: Vec::new(),
        diagnostics: vec![
            NodeOwnedRegionArrangementDiagnostic::MissingSeamConstraint {
                region_index,
                owner,
                opposite_owner,
                start,
                end,
            },
        ],
    }
}
