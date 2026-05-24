//! Rail-source authority tests for node boolean ownership.

use super::*;

#[test]
fn source_local_owned_boundary_preserves_explicit_height_endpoint_authority() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, vec![canonical_endpoint])],
        )),
        height_points_by_source,
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("canonical rail point adoption should succeed");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&canonical_endpoint));
    assert!(contour_keys.contains(&local_endpoint));
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn duplicate_owner_source_candidate_cluster_preserves_overlay_owned_key() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("sub-grid duplicate source cluster should preserve the owned overlay key");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&drifted_vertex));
}

#[test]
fn duplicate_owner_source_candidate_cluster_preserves_same_mm_overlay_key() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("tight duplicate source cluster should preserve same-mm overlay ownership");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&same_mm_overlay_vertex));
}

#[test]
fn duplicate_owner_source_candidate_cluster_accepts_hill_junction_drift() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("sub-quarter-mm source cluster should preserve same-mm overlay ownership");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&drifted_vertex));
}

#[test]
fn source_height_rail_authorizes_same_mm_hill_junction_cluster() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        height_points_by_source: BTreeMap::from([(source_key, owner_points)]),
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
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn source_height_rail_scope_does_not_choose_single_candidate_from_owner_ambiguity() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(sidewalk, owner_points)],
        )),
        height_points_by_source: BTreeMap::from([(source_key, vec![source_candidate])]),
        paths_by_owner: BTreeMap::new(),
    };

    assert!(matches!(
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points),
        Err(NodeBooleanOwnershipError::AmbiguousCanonicalOwnedRegionVertex { owner, .. })
            if owner == sidewalk
    ));
}

#[test]
fn source_height_rail_preserves_source_scoped_same_mm_cluster_with_unrelated_owner_candidate() {
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(sidewalk, owner_points)],
        )),
        height_points_by_source: BTreeMap::from([(
            source_key,
            vec![first_source_candidate, second_source_candidate],
        )]),
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
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn source_segment_authorizes_regenerated_hill_same_mm_boolean_vertex() {
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
        vec![NodeRailSourceSegmentAuthority {
            owner: carriageway,
            source: source_key,
            segment: OwnedRegionEdgeKey::new(
                canonical_source_point,
                (canonical_source_point.0 - 1_000, canonical_source_point.1),
            ),
        }],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::from([(source_key, vec![canonical_source_point])]),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("unique source segment projection should authorize the same-mm boolean vertex");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&canonical_source_point));
    assert!(!contour_keys.contains(&boolean_vertex));
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
}

#[test]
fn multiple_source_segment_authorizations_report_ambiguous_projection() {
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
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new(left_candidate, (left_candidate.0, 1_000)),
            },
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new(right_candidate, (right_candidate.0, 1_000)),
            },
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("two source segment projections must remain ambiguous");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: 0,
            point_z_key: 0,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            ref candidates,
        } if owner == carriageway
            && candidates.len() == 2
            && candidates.iter().any(|candidate| candidate.canonical_point == left_candidate)
            && candidates.iter().any(|candidate| candidate.canonical_point == right_candidate)
    ));
}

#[test]
fn source_segment_projection_on_boolean_key_still_rejects_independent_second_rail() {
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
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            },
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            },
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("a boolean point on one source rail must not hide a second source rail");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: 0,
            point_z_key: 0,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            ref candidates,
        } if owner == carriageway
            && candidates.len() == 2
            && candidates.iter().any(|candidate| candidate.canonical_point == boolean_vertex)
            && candidates.iter().any(|candidate| candidate.canonical_point == (1, 0))
    ));
}

#[test]
fn split_source_segments_report_ambiguous_even_with_unique_same_mm_source_point() {
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
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            },
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            },
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::from([(source_key, vec![source_point])]),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("two source rails must not collapse to one source point by policy");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: 0,
            point_z_key: 8,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            ref candidates,
        } if owner == carriageway
            && candidates.len() == 2
    ));
}

#[test]
fn adjacent_source_endpoint_cluster_materializes_to_source_projection() {
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
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((-1_000, 0), alternate_projection),
            },
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new(alternate_projection, expected_projection),
            },
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        height_points_by_source: BTreeMap::from([(source_key, owner_points)]),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("adjacent source endpoint dust should materialize to a source projection");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&expected_projection));
    assert!(!contour_keys.contains(&boolean_vertex));
}

#[test]
fn split_source_segments_report_ambiguous_even_with_source_scoped_duplicate_cluster() {
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
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            },
            NodeRailSourceSegmentAuthority {
                owner: carriageway,
                source: source_key,
                segment: OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            },
        ],
    )]);
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner,
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        height_points_by_source: BTreeMap::from([(source_key, owner_points)]),
        paths_by_owner: BTreeMap::new(),
    };

    let error =
        canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
            .expect_err("two source rails must not preserve a same-mm duplicate cluster");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner,
            point_x_key: 0,
            point_z_key: 8,
            source_kind: RoadSurfaceBandKind::Carriageway,
            source_mouth_order_index: 1,
            source_band_index: 2,
            ref candidates,
        } if owner == carriageway
            && candidates.len() == 2
    ));
}

#[test]
fn ambiguous_owner_source_candidates_preserve_explicit_segment_point() {
    let carriageway = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let first_candidate = (1_000_000, 0);
    let second_candidate = (1_000_400, 0);
    let segment_point = (1_000_200, 0);
    let mut regions = vec![test_owned_region(
        RoadSurfaceBandKind::Carriageway,
        carriageway,
        vec![
            overlay_point_from_key(segment_point),
            overlay_point_from_key((0, 0)),
            overlay_point_from_key((2_000_000, 0)),
        ],
    )];
    let owner_points = vec![first_candidate, second_candidate];
    let mut segments_by_owner = BTreeMap::new();
    insert_open_source_segments(
        &mut segments_by_owner,
        carriageway,
        &[(0, 0), (2_000_000, 0)],
    );
    let rail_canonical_points = NodeRailCanonicalPointSet {
        all_points: owner_points.clone(),
        points_by_owner: BTreeMap::from([(carriageway, owner_points.clone())]),
        segments_by_owner,
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points)],
        )),
        height_points_by_source: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    canonicalize_owned_region_rings_with_rail_point_set(&mut regions, &rail_canonical_points)
        .expect("explicit source-segment point should not choose among ambiguous endpoints");

    let contour_keys = regions[0].shape[0]
        .iter()
        .copied()
        .map(ownership_key_from_overlay_point)
        .collect::<BTreeSet<_>>();
    assert!(contour_keys.contains(&segment_point));
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
    );
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
        segments_by_owner: BTreeMap::new(),
        source_segments_by_owner: BTreeMap::new(),
        canonical_points_by_mm_key_by_owner: canonical_points_by_mm_key_by_owner(&BTreeMap::from(
            [(carriageway, owner_points.clone())],
        )),
        height_points_by_source: BTreeMap::new(),
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
fn noncanonical_owned_region_vertex_reports_source_authority_error() {
    let curb = NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let drifted_endpoint = [1.000004, 0.0];
    let regions = vec![test_owned_region(
        RoadSurfaceBandKind::CurbOrShoulder,
        curb,
        vec![drifted_endpoint, [2.0, 0.0], [2.0, 2.0]],
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

    assert!(matches!(
        validate_owned_region_vertices_against_source_authority(
            &regions,
            &rail_canonical_points
        ),
        Err(NodeBooleanOwnershipError::NonCanonicalOwnedRegionVertex {
            owner,
            point_x_key,
            point_z_key,
            canonical_x_key,
            canonical_z_key,
        }) if owner == curb
            && point_x_key == ownership_key_from_overlay_point(drifted_endpoint).0
            && point_z_key == ownership_key_from_overlay_point(drifted_endpoint).1
            && canonical_x_key == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)).0
            && canonical_z_key == ownership_key_from_road_point(RoadVec2::new(1.0, 0.0)).1
    ));
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
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
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
    assert!(
        validate_owned_region_vertices_against_source_authority(&regions, &rail_canonical_points)
            .is_ok()
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
