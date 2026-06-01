//! Rail-source authority tests for node boolean ownership.

use super::*;

#[test]
fn source_local_owned_boundary_does_not_adopt_owner_wide_endpoint() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let local_endpoint = (1_000_001, 0);
    let canonical_endpoint = (1_000_000, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            [0.0, 0.0],
            overlay_point_from_key(local_endpoint),
            [0.0, 1.0],
        ],
    )];
    let mut height_points_by_source = BTreeMap::new();
    height_points_by_source.insert(
        (
            RoadSurfaceBandKind::Carriageway,
            carriageway.owner_index(),
            carriageway.owner_index(),
        ),
        vec![local_endpoint],
    );
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: vec![canonical_endpoint],
        points_by_owner: BTreeMap::from([(carriageway, vec![canonical_endpoint])]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner: BTreeMap::new(),
            height_points_by_source,
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, vec![canonical_endpoint])],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("canonical rail point adoption should succeed");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&local_endpoint));
    assert!(!contour_keys.contains(&canonical_endpoint));
}

#[test]
fn duplicate_owner_source_candidate_cluster_blocks_without_source_carrier() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let representative = (1_000_000, 0);
    let duplicate_source = (1_000_019, 18);
    let drifted_vertex = (1_000_006, -1);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(drifted_vertex),
            overlay_point_from_key((0, 0)),
            overlay_point_from_key((0, 1_000_000)),
        ],
    )];
    let owner_points = vec![representative, duplicate_source];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("owner-wide duplicate source clusters must not authorize backend drift");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: 1_000_006,
            point_z_key: -1,
            ref candidates,
        } if owner == carriageway && candidates == &vec![representative, duplicate_source]
    ));
}

#[test]
fn duplicate_owner_same_mm_cluster_blocks_without_source_carrier() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let representative = (1_000_000, 0);
    let duplicate_source = (1_000_001, 0);
    let same_mm_overlay_vertex = (1_000_200, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(same_mm_overlay_vertex),
            overlay_point_from_key((0, 0)),
            overlay_point_from_key((0, 1_000_000)),
        ],
    )];
    let owner_points = vec![representative, duplicate_source];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("same-mm duplicate source clusters require explicit source carriers");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: 1_000_200,
            point_z_key: 0,
            ref candidates,
        } if owner == carriageway && candidates == &vec![representative, duplicate_source]
    ));
}

#[test]
fn duplicate_owner_hill_junction_cluster_blocks_without_source_carrier() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let first_candidate = (-52_302_017, -49_839_396);
    let second_candidate = (-52_302_004, -49_839_347);
    let third_candidate = (-52_301_976, -49_839_236);
    let drifted_vertex = (-52_301_986, -49_839_418);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(drifted_vertex),
            overlay_point_from_key((-53_000_000, -50_000_000)),
            overlay_point_from_key((-53_000_000, -49_000_000)),
        ],
    )];
    let owner_points = vec![first_candidate, second_candidate, third_candidate];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("hilly duplicate source clusters require explicit source carriers");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: -52_301_986,
            point_z_key: -49_839_418,
            ref candidates,
        } if owner == carriageway
            && candidates == &vec![first_candidate, second_candidate, third_candidate]
    ));
}

#[test]
fn source_height_rail_defers_same_mm_hill_junction_cluster_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let first_candidate = (-36_231_967, -58_182_291);
    let second_candidate = (-36_231_967, -58_182_290);
    let third_candidate = (-36_231_843, -58_181_910);
    let drifted_vertex = (-36_231_956, -58_182_294);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let owner_points = vec![first_candidate, second_candidate, third_candidate];
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(drifted_vertex),
            overlay_point_from_key((-37_000_000, -59_000_000)),
            overlay_point_from_key((-37_000_000, -58_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner: BTreeMap::new(),
            height_points_by_source: BTreeMap::from([(source_key, owner_points.clone())]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-height rail must authorize the same-mm JunctionN ownership vertex");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&drifted_vertex));
}

#[test]
fn source_height_rail_scope_defers_single_candidate_owner_ambiguity_to_closure() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let unrelated_first_candidate = (-21_445_400, -48_035_391);
    let unrelated_second_candidate = (-21_445_399, -48_035_391);
    let source_candidate = (-21_445_222, -48_034_510);
    let boolean_vertex = (-21_445_337, -48_035_081);
    let source_key = (RoadSurfaceBandKind::Sidewalk, 1, 5);
    let owner_points = vec![
        unrelated_first_candidate,
        unrelated_second_candidate,
        source_candidate,
    ];
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Sidewalk,
        sidewalk,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((-22_000_000, -49_000_000)),
            overlay_point_from_key((-22_000_000, -48_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(sidewalk, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner: BTreeMap::new(),
            height_points_by_source: BTreeMap::from([(source_key, vec![source_candidate])]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(sidewalk, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-scoped owner ambiguity should be left for carrier closure");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
    assert!(!contour_keys.contains(&source_candidate));
}

#[test]
fn source_height_rail_adopts_unique_same_mm_source_vertex() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let source_candidate = (21_998_085, 27_803_471);
    let boolean_vertex = (21_998_084, 27_803_470);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((20_574_587, 31_000_916)),
            overlay_point_from_key((16_516_749, 25_363_188)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    regions[0].claim_priority = NodeGeneratedContourClaimPriority::JoinOrCap;
    let owner_points = vec![source_candidate];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner: BTreeMap::new(),
            height_points_by_source: BTreeMap::from([(source_key, vec![source_candidate])]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("unique source-scoped rail key should replace backend same-mm drift");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&source_candidate));
    assert!(!contour_keys.contains(&boolean_vertex));
}

#[test]
fn source_height_rail_defers_source_scoped_same_mm_cluster_with_unrelated_owner_candidate() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let unrelated_candidate = (-21_445_399, -48_035_391);
    let first_source_candidate = (-21_445_400, -48_035_391);
    let second_source_candidate = (-21_445_222, -48_034_510);
    let boolean_vertex = (-21_445_337, -48_035_081);
    let source_key = (RoadSurfaceBandKind::Sidewalk, 1, 5);
    let owner_points = vec![
        unrelated_candidate,
        first_source_candidate,
        second_source_candidate,
    ];
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Sidewalk,
        sidewalk,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((-22_000_000, -49_000_000)),
            overlay_point_from_key((-22_000_000, -48_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(sidewalk, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner: BTreeMap::new(),
            height_points_by_source: BTreeMap::from([(
                source_key,
                vec![first_source_candidate, second_source_candidate],
            )]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(sidewalk, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-scoped same-mm cluster should preserve the boolean vertex");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
}

#[test]
fn source_segment_projection_is_left_for_carrier_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let canonical_source_point = (-39_339_263, -57_072_175);
    let unrelated_same_mm_owner_point = (-39_339_147, -57_071_688);
    let boolean_vertex = (-39_339_253, -57_072_177);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((-40_000_000, -58_000_000)),
            overlay_point_from_key((-40_000_000, -57_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![canonical_source_point, unrelated_same_mm_owner_point];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![NodeRailSourceSegmentAuthority::new(
            carriageway,
            source_key,
            OwnedRegionEdgeKey::new(
                canonical_source_point,
                (canonical_source_point.0 - 1_000, canonical_source_point.1),
            ),
        )],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::from([(source_key, vec![canonical_source_point])]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("unique source segment projection should authorize the same-mm boolean vertex");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
}

#[test]
fn multiple_source_segment_authorizations_are_deferred_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let left_candidate = (-10, 0);
    let right_candidate = (10, 0);
    let boolean_vertex = (0, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((0, 1_000_000)),
            overlay_point_from_key((1_000_000, 1_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![left_candidate, right_candidate];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new(left_candidate, (left_candidate.0, 1_000)),
            ),
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new(right_candidate, (right_candidate.0, 1_000)),
            ),
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::new(),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-segment ambiguity should be reported by carrier closure");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
    assert!(!contour_keys.contains(&left_candidate));
    assert!(!contour_keys.contains(&right_candidate));
}

#[test]
fn source_segment_projection_on_boolean_key_is_deferred_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let first_owner_candidate = (-10, 0);
    let second_owner_candidate = (10, 0);
    let boolean_vertex = (0, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((0, 1_000_000)),
            overlay_point_from_key((1_000_000, 1_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![first_owner_candidate, second_owner_candidate];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            ),
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            ),
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::new(),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("independent source rails should be reported by carrier closure");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
}

#[test]
fn split_source_segments_with_unique_same_mm_source_point_are_deferred_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let source_point = (0, 0);
    let boolean_vertex = (0, 8);
    let first_owner_candidate = (-10, 0);
    let second_owner_candidate = (10, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((0, 1_000_000)),
            overlay_point_from_key((1_000_000, 1_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![first_owner_candidate, second_owner_candidate];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            ),
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            ),
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::from([(source_key, vec![source_point])]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("split source rails should be reported by carrier closure");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
}

#[test]
fn adjacent_source_endpoint_cluster_is_deferred_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let boolean_vertex = (10, 2);
    let alternate_projection = (0, 0);
    let expected_projection = (8, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((0, 1_000_000)),
            overlay_point_from_key((1_000_000, 1_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![expected_projection, alternate_projection];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((-1_000, 0), alternate_projection),
            ),
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new(alternate_projection, expected_projection),
            ),
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::from([(source_key, owner_points.clone())]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("adjacent source endpoint dust should materialize to a source projection");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
    assert!(!contour_keys.contains(&expected_projection));
}

#[test]
fn split_source_segments_with_source_scoped_duplicate_cluster_are_deferred_to_closure() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source_key = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let boolean_vertex = (0, 8);
    let first_source_point = (-2, 0);
    let second_source_point = (2, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key((0, 1_000_000)),
            overlay_point_from_key((1_000_000, 1_000_000)),
        ],
    )];
    regions[0].source_mouth_order_index = source_key.1;
    regions[0].source_band_index = Some(source_key.2);
    let owner_points = vec![first_source_point, second_source_point];
    let source_segments_by_owner = BTreeMap::from([(
        carriageway,
        vec![
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            ),
            NodeRailSourceSegmentAuthority::new(
                carriageway,
                source_key,
                OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            ),
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::from([(source_key, owner_points.clone())]),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("source-scoped duplicate cluster should be reported by carrier closure");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&boolean_vertex));
}

#[test]
fn ambiguous_owner_source_candidates_block_without_source_scoped_carrier() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let first_candidate = (1_000_000, 0);
    let second_candidate = (1_000_400, 0);
    let drifted_vertex = (1_000_200, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(drifted_vertex),
            overlay_point_from_key((0, 0)),
            overlay_point_from_key((2_000_000, 0)),
        ],
    )];
    let owner_points = vec![first_candidate, second_candidate];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("owner-wide segment authorization must not bypass ambiguity");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: 1_000_200,
            point_z_key: 0,
            ref candidates,
        } if owner == carriageway
            && candidates == &vec![first_candidate, second_candidate]
    ));
}

#[test]
fn ambiguous_owner_canonical_source_candidates_are_blocking() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let first_candidate = (1_000_000, 0);
    let second_candidate = (1_000_400, 0);
    let drifted_vertex = (1_000_200, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(drifted_vertex),
            overlay_point_from_key((0, 0)),
            overlay_point_from_key((0, 1_000_000)),
        ],
    )];
    let owner_points = vec![first_candidate, second_candidate];
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("ambiguous source candidates must not choose a lowest-key winner");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex {
            owner,
            point_x_key: 1_000_200,
            point_z_key: 0,
            ref candidates,
        } if owner == carriageway
            && candidates == &vec![first_candidate, second_candidate]
    ));
}

#[test]
fn closure_materializes_drifted_boundary_from_registered_source_segment() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let drifted_endpoint = [1.000004, 0.0];
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::CurbOrShoulder,
        curb,
        vec![drifted_endpoint, [1.0, 2.0], [1.0, 0.0]],
    )];
    regions[0].source_mouth_order_index = 0;
    regions[0].source_band_index = Some(1);
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];
    let mut rails = test_rail_contour_set_from_constraints(rail_constraints.clone());
    rails.height_carrier_points_by_source.insert(
        (RoadSurfaceBandKind::CurbOrShoulder, 0, 1),
        vec![RoadVec3::new(1.0, 0.0, 0.0), RoadVec3::new(1.0, 0.0, 2.0)],
    );
    rails.source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &rails.contours,
        &rails.constraints,
        &rails.height_carrier_paths_by_source,
        &rails.height_carrier_points_by_source,
    );
    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);

    let closure = validate_owned_region_vertices_against_carrier_closure(
        &regions,
        &rails,
        &rail_canonical_points,
    )
    .expect("drifted boundary support must be classified through source carrier closure");
    let drifted_key = ownership_key_from_overlay_point(drifted_endpoint);
    assert!(closure.records.iter().any(|record| {
        record.owner == curb
            && record.point.raw_tuple() == drifted_key
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment { .. }
            )
    }));
}

#[test]
fn canonicalizes_overlay_vertex_drift_to_unique_source_rail_key() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::CurbOrShoulder,
        curb,
        vec![[0.0, 0.0], [1.000004, 0.0], [1.000004, 2.0], [0.0, 2.0]],
    )];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 33,
        kind: NodeRailConstraintKind::RaisedStepContact,
        source_mouth_order_index: 0,
        source_band_index: Some(1),
        source_boundary_index: Some(1),
        owner: Some(curb),
        opposite_owner: None,
        points_xz: vec![RoadVec2::new(1.0, 0.0), RoadVec2::new(1.0, 2.0)],
    }];

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("canonical rail point adoption should succeed");

    let contour = &regions[0].shape[0];
    assert!(
        contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)))
    );
    assert!(
        contour
            .iter()
            .any(|point| ownership_key_from_overlay_point(*point)
                == ownership_key_from_road_point(RoadVec2::new(1.0, 2.0)))
    );
    assert!(
        contour.iter().all(|point| {
            ownership_key_from_overlay_point(*point)
                != ownership_key_from_overlay_point([1.000004, 0.0])
                && ownership_key_from_overlay_point(*point)
                    != ownership_key_from_overlay_point([1.000004, 2.0])
        }),
        "owned region vertices must use the owner-authorized source rail keys, not backend drift"
    );
}

#[test]
fn canonicalizes_closing_overlay_dust_to_source_rail_endpoint() {
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
    let endpoint = RoadVec2::new(15.169048, 5.0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Sidewalk,
        sidewalk,
        vec![
            [15.169047, 5.0],
            [15.169048, 3.65],
            [15.979047, 3.65],
            [15.596568, 4.287465],
            [15.169048, 4.999998],
        ],
    )];
    let rail_constraints = vec![NodeRailConstraint {
        constraint_index: 34,
        kind: NodeRailConstraintKind::FootprintSeam {
            adjacent_kind: RoadSurfaceBandKind::Sidewalk,
        },
        source_mouth_order_index: 1,
        source_band_index: Some(0),
        source_boundary_index: Some(0),
        owner: Some(sidewalk),
        opposite_owner: None,
        points_xz: vec![endpoint, RoadVec2::new(15.169048, 3.65)],
    }];

    let rail_canonical_points = test_rail_canonical_points_from_constraints(&rail_constraints);
    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("canonical rail point adoption should succeed");

    let contour = &regions[0].shape[0];
    let endpoint_key = ownership_key_from_road_point(endpoint);
    assert_eq!(ownership_key_from_overlay_point(contour[0]), endpoint_key);
    assert_eq!(
        contour
            .iter()
            .filter(|point| ownership_key_from_overlay_point(**point) == endpoint_key)
            .count(),
        1,
        "closing overlay dust must collapse onto the authorized source rail endpoint"
    );
}

#[test]
fn explicit_shared_point_constraints_preserve_endpoint_context_without_height_continuity() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let sidewalk = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 1);
    let mut regions = vec![
        test_owned_region(
            RoadSurfaceBandKind::Carriageway,
            carriageway,
            vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
        ),
        test_owned_region(
            RoadSurfaceBandKind::Sidewalk,
            sidewalk,
            vec![[1.0, 1.0], [2.0, 1.0], [2.0, 2.0], [1.0, 2.0]],
        ),
    ];

    for region in &mut regions {
        region.seam_constraints.push(NodeRegionSeamConstraint {
            constraint_index: 0,
            seam_source: NodeSeamSource::AsphaltBoundary {
                owner_index: region.owner.owner_index(),
            },
            owner: None,
            opposite_owner: None,
            constrains_shared_height: false,
            is_material_transition: true,
            start_xz: RoadVec2::new(1.0, 1.0),
            end_xz: RoadVec2::new(1.0, 1.0),
        });
        canonicalize_seam_constraints(&mut region.seam_constraints);
    }

    for region in &regions {
        assert!(
            region.seam_constraints.iter().any(|constraint| {
                let start = ownership_key_from_road_point(constraint.start_xz);
                let end = ownership_key_from_road_point(constraint.end_xz);
                start == ownership_key_from_road_point(RoadVec2::new(1.0, 1.0))
                    && end == start
                    && constraint.is_material_transition
                    && !constraint.constrains_shared_height
            }),
            "point-only material contacts must remain explicit seam endpoints without asserting one shared height"
        );
    }
}
