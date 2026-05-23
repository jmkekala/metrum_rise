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
