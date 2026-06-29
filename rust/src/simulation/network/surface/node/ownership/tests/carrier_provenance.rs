//! Carrier-provenance closure tests for post-boolean owned vertices.

use super::*;
use crate::simulation::network::surface::backend::RoadVec3;
use crate::simulation::network::surface::rails::NodeRailHeightCarrierPaths;

#[test]
fn carrier_provenance_closure_records_source_segment_projection() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let point = ownership_key_from_road_point(RoadVec2::new(5.0, 0.0));
    let region = region_for_source(owner, source, vec![[5.0, 0.0]]);
    let rails = rails_with_source_height_keys(
        source,
        [
            ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
            ownership_key_from_road_point(RoadVec2::new(10.0, 0.0)),
        ],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![NodeRailSourceSegmentAuthority::new(
            owner,
            source,
            OwnedRegionEdgeKey::new(
                ownership_key_from_road_point(RoadVec2::new(0.0, 0.0)),
                ownership_key_from_road_point(RoadVec2::new(10.0, 0.0)),
            ),
        )],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("source path segment should explicitly authorize the boolean vertex");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment { .. }
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_generated_carrier_vertex() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let point = ownership_key_from_road_point(RoadVec2::new(5.0, 5.0));
    let mut region = region_for_source(owner, source, vec![[0.0, 0.0], [5.0, 5.0], [0.0, 10.0]]);
    region.claim_priority = NodeGeneratedContourClaimPriority::SideJoin;
    let rails = rails_with_generated_carrier_vertex(owner, source);
    let rail_points = NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("source surface should explicitly authorize the interior boolean vertex");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::GeneratedCarrierVertex { .. }
            )
    }));
}

#[test]
fn carrier_provenance_closure_rejects_unemitted_source_surface_interior() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let region = region_for_source(owner, source, vec![[0.0, 0.0], [5.0, 5.0], [0.0, 10.0]]);
    let rails = rails_with_source_surface(source);
    let rail_points = NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    };

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("interior source-surface containment alone is not carrier provenance");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::MissingCarrierProvenance { .. }
    ));
}

#[test]
fn carrier_provenance_closure_rejects_generated_surface_interior_without_carrier_origin() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 0, 2);
    let point = (0, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key((-1_000, 0)),
            overlay_point_from_key(point),
            overlay_point_from_key((1_000, 0)),
        ],
    );
    let mut rails = rails_with_source_height_keys(source, [(-1_000, 0), (1_000, 0)]);
    push_generated_carrier_surface(&mut rails, owner, source);
    let rail_points = empty_rail_points();

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("generated surface containment alone is not carrier provenance");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::MissingCarrierProvenance {
            owner: diagnostic_owner,
            point_x_key: 0,
            point_z_key: 0,
            ..
        } if diagnostic_owner == owner
    ));
}

#[test]
fn carrier_provenance_closure_reports_missing_source_rail_details() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let source = (RoadSurfaceBandKind::Sidewalk, 2, 5);
    let point = ownership_key_from_road_point(RoadVec2::new(-1.785, -5.4396));
    let region = region_for_source(owner, source, vec![[-1.785, -5.4396]]);
    let rails =
        rails_with_source_height_keys(source, [(-10, 0), (-10, 1_000), (10, 0), (1_000, 10)]);
    let rail_points = empty_rail_points();

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("missing source rails must fail as missing carrier provenance");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::MissingCarrierProvenance {
            owner: diagnostic_owner,
            point_x_key,
            point_z_key,
            source_kind: RoadSurfaceBandKind::Sidewalk,
            source_mouth_order_index: 2,
            source_band_index: 5,
            height_field_id,
        } if diagnostic_owner == owner
            && (point_x_key, point_z_key) == point
            && height_field_id == NodeBandHeightFieldId::new(2, 5, RoadSurfaceBandKind::Sidewalk)
    ));
}

#[test]
fn carrier_provenance_closure_reports_ambiguous_independent_source_segments() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let point = (0, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(point),
            overlay_point_from_key((-10, 0)),
            overlay_point_from_key((-10, 1_000)),
        ],
    );
    let rails = rails_with_source_height_keys(
        source,
        [(-10, 0), (-10, 1_000), (-1_000, -10), (1_000, -10)],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-10, 0), (-10, 1_000)),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, -10), (1_000, -10)),
            ),
        ],
    );

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("independent source rails must report ambiguous carrier provenance");

    assert!(
        matches!(
            error,
            NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
                owner: diagnostic_owner,
                point_x_key: 0,
                point_z_key: 0,
                source_kind: RoadSurfaceBandKind::Carriageway,
                source_mouth_order_index: 1,
                source_band_index: 2,
                ref candidates,
            } if diagnostic_owner == owner && candidates.len() == 2
        ),
        "{error:?}"
    );
}

#[test]
fn carrier_provenance_closure_rejects_independent_second_rail_on_boolean_key() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let point = (0, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(point),
            overlay_point_from_key((-1_000, 0)),
            overlay_point_from_key((1_000, 0)),
        ],
    );
    let rails =
        rails_with_source_height_keys(source, [(-1_000, 0), (1_000, 0), (1, -1_000), (1, 1_000)]);
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            ),
        ],
    );

    let error = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect_err("an exact first rail must not hide an independent second source rail");

    assert!(matches!(
        error,
        NodeBooleanOwnershipError::AmbiguousSourceSegmentAuthorizedOwnedRegionVertex {
            owner: diagnostic_owner,
            point_x_key: 0,
            point_z_key: 0,
            ref candidates,
            ..
        } if diagnostic_owner == owner && candidates.len() == 2
    ));
}

#[test]
fn carrier_provenance_closure_records_generated_source_carrier_intersection() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let point = (0, 0);
    let mut region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(point),
            overlay_point_from_key((-1_000, 0)),
            overlay_point_from_key((1_000, 0)),
        ],
    );
    region.claim_priority = NodeGeneratedContourClaimPriority::SideJoin;
    let mut rails =
        rails_with_source_height_keys(source, [(-1_000, 0), (1_000, 0), (1, -1_000), (1, 1_000)]);
    push_generated_carrier_surface(&mut rails, owner, source);
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, 0), (1_000, 0)),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((1, -1_000), (1, 1_000)),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("generator-declared source carrier intersections are explicit provenance");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceIntersection { peer_count: 2 }
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_connected_endpoint_dust_cluster() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let boolean_vertex = (10, 2);
    let alternate_projection = (0, 0);
    let expected_projection = (8, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key(alternate_projection),
            overlay_point_from_key(expected_projection),
        ],
    );
    let rails = rails_with_source_height_keys(
        source,
        [(-1_000, 0), alternate_projection, expected_projection],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, 0), alternate_projection),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(alternate_projection, expected_projection),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("connected endpoint source-carrier dust should resolve deterministically");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == boolean_vertex
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment {
                    canonical_point,
                    distance_key_units_sq: 8,
                    ..
                } if canonical_point.raw_tuple() == expected_projection
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_nearest_parallel_projection_noise() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let boolean_vertex = (10, 2);
    let alternate_projection = (8, 0);
    let expected_projection = (10, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key(alternate_projection),
            overlay_point_from_key(expected_projection),
        ],
    );
    let rails = rails_with_source_height_keys(
        source,
        [(-1_000, 0), alternate_projection, (-900, 0), (11, 0)],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, 0), alternate_projection),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-900, 0), (11, 0)),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("nearest parallel source-carrier dust should resolve deterministically");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == boolean_vertex
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment {
                    canonical_point,
                    distance_key_units_sq: 4,
                    ..
                } if canonical_point.raw_tuple() == expected_projection
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_single_exact_same_direction_projection_noise() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 11);
    let source = (RoadSurfaceBandKind::Sidewalk, 1, 5);
    let boolean_vertex = (0, 0);
    let exact_start = (-1_000, 0);
    let exact_end = (1_000, 0);
    let noisy_start = (-1_000, -100);
    let noisy_end = (1_000, 90);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key(exact_start),
            overlay_point_from_key(exact_end),
        ],
    );
    let rails =
        rails_with_source_height_keys(source, [exact_start, exact_end, noisy_start, noisy_end]);
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(exact_start, exact_end),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(noisy_start, noisy_end),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("single exact same-direction projection noise should choose the exact carrier");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == boolean_vertex
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment {
                    canonical_point,
                    distance_key_units_sq: 0,
                    ..
                } if canonical_point.raw_tuple() == boolean_vertex
            )
    }));
}

#[test]
fn carrier_provenance_closure_tie_breaks_equal_parallel_projection_noise() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 2);
    let source = (RoadSurfaceBandKind::Carriageway, 1, 2);
    let boolean_vertex = (10, 2);
    let expected_projection = (8, 0);
    let alternate_projection = (12, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(boolean_vertex),
            overlay_point_from_key(expected_projection),
            overlay_point_from_key(alternate_projection),
        ],
    );
    let rails = rails_with_source_height_keys(
        source,
        [(0, 0), expected_projection, alternate_projection, (20, 0)],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((0, 0), expected_projection),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(alternate_projection, (20, 0)),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("equal-distance parallel source-carrier dust should resolve by stable key");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == boolean_vertex
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment {
                    canonical_point,
                    distance_key_units_sq: 8,
                    ..
                } if canonical_point.raw_tuple() == expected_projection
            )
    }));
}

#[test]
fn carrier_provenance_closure_records_exact_source_intersection_with_connected_secondary_cluster() {
    let owner = NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 17);
    let source = (RoadSurfaceBandKind::Sidewalk, 2, 5);
    let point = (0, 0);
    let secondary_projection = (1, 0);
    let region = region_for_source(
        owner,
        source,
        vec![
            overlay_point_from_key(point),
            overlay_point_from_key(secondary_projection),
            overlay_point_from_key((2, 0)),
        ],
    );
    let rails = rails_with_source_height_keys(
        source,
        [
            (0, -1_000),
            (0, 1_000),
            (-1_000, 0),
            secondary_projection,
            (1_000, 0),
        ],
    );
    let rail_points = rail_points_with_source_segments(
        owner,
        vec![
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((0, -1_000), (0, 1_000)),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new((-1_000, 0), secondary_projection),
            ),
            NodeRailSourceSegmentAuthority::new(
                owner,
                source,
                OwnedRegionEdgeKey::new(secondary_projection, (1_000, 0)),
            ),
        ],
    );

    let closure = NodeCarrierProvenanceClosure::from_owned_regions(&[region], &rails, &rail_points)
        .expect("exact carrier intersection with connected secondary cluster is deterministic");

    assert!(closure.records.iter().any(|record| {
        record.point.raw_tuple() == point
            && matches!(
                record.origin,
                NodeCarrierProvenanceOrigin::SourceSegment {
                    canonical_point,
                    distance_key_units_sq: 0,
                    ..
                } if canonical_point.raw_tuple() == point
            )
    }));
}

fn empty_rail_points() -> NodeRailCanonicalPointSet {
    NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        source_carriers: NodeSourceCarrierRegistry::default(),
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    }
}

fn rails_with_source_height_keys(
    source: (RoadSurfaceBandKind, usize, usize),
    keys: impl IntoIterator<Item = NodeOwnershipPointKey>,
) -> NodeRailContourSet {
    let height_carrier_points_by_source = BTreeMap::from([(
        source,
        keys.into_iter()
            .map(|key| {
                let point = road_point_from_key(key);
                RoadVec3::new(point.x, 0.0, point.y)
            })
            .collect::<Vec<_>>(),
    )]);
    let source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &[],
        &[],
        &BTreeMap::new(),
        &height_carrier_points_by_source,
    );
    NodeRailContourSet {
        node_id: 42,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        contours: Vec::new(),
        corner_trims: Vec::new(),
        side_join_gaps: Vec::new(),
        constraints: Vec::new(),
        height_carrier_paths_by_source: BTreeMap::new(),
        height_carrier_points_by_source,
        source_carriers,
    }
}

fn rail_points_with_source_segments(
    owner: NodeBandOwner,
    segments: Vec<NodeRailSourceSegmentAuthority>,
) -> NodeRailCanonicalPointSet {
    let mut source_segments_by_owner = BTreeMap::from([(owner, segments)]);
    for segments in source_segments_by_owner.values_mut() {
        segments.sort_unstable();
        segments.dedup();
    }
    NodeRailCanonicalPointSet {
        all_points: Vec::new(),
        points_by_owner: BTreeMap::new(),
        source_carriers: NodeSourceCarrierRegistry {
            source_segments_by_owner,
            height_points_by_source: BTreeMap::new(),
            numeric_dust_canonicalized_sources: BTreeSet::new(),
        },
        canonical_points_by_mm_key_by_owner: BTreeMap::new(),
        paths_by_owner: BTreeMap::new(),
    }
}

fn rails_with_source_surface(source: (RoadSurfaceBandKind, usize, usize)) -> NodeRailContourSet {
    let start_path_world = vec![
        RoadVec3::new(0.0, 10.0, 0.0),
        RoadVec3::new(10.0, 20.0, 0.0),
    ];
    let end_path_world = vec![
        RoadVec3::new(0.0, 10.0, 10.0),
        RoadVec3::new(10.0, 20.0, 10.0),
    ];
    let mut height_carrier_paths_by_source = BTreeMap::new();
    height_carrier_paths_by_source.insert(
        source,
        vec![NodeRailHeightCarrierPaths {
            start_path_world: start_path_world.clone(),
            end_path_world: end_path_world.clone(),
        }],
    );
    let height_carrier_points_by_source = BTreeMap::from([(
        source,
        start_path_world
            .into_iter()
            .chain(end_path_world)
            .collect::<Vec<_>>(),
    )]);
    let source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &[],
        &[],
        &height_carrier_paths_by_source,
        &height_carrier_points_by_source,
    );
    NodeRailContourSet {
        node_id: 42,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        contours: Vec::new(),
        corner_trims: Vec::new(),
        side_join_gaps: Vec::new(),
        constraints: Vec::new(),
        height_carrier_paths_by_source,
        height_carrier_points_by_source,
        source_carriers,
    }
}

fn rails_with_generated_carrier_vertex(
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
) -> NodeRailContourSet {
    let mut rails = rails_with_source_surface(source);
    let contour_points = vec![
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(5.0, 5.0),
        RoadVec2::new(0.0, 10.0),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band { kind: source.0 },
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: source.1,
        source_band_index: Some(source.2),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        points_xz: contour_points.clone(),
        height_points_world: Some(vec![
            RoadVec3::new(0.0, 10.0, 0.0),
            RoadVec3::new(5.0, 15.0, 5.0),
            RoadVec3::new(0.0, 20.0, 10.0),
        ]),
        backend_polyline: road_points_to_polyline(contour_points.clone(), true),
    });
    rails.source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &rails.contours,
        &rails.constraints,
        &rails.height_carrier_paths_by_source,
        &rails.height_carrier_points_by_source,
    );
    rails
}

fn push_generated_carrier_surface(
    rails: &mut NodeRailContourSet,
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
) {
    let contour_points = vec![
        RoadVec2::new(-0.01, -0.01),
        RoadVec2::new(0.01, -0.01),
        RoadVec2::new(0.01, 0.01),
        RoadVec2::new(-0.01, 0.01),
    ];
    rails.contours.push(NodeGeneratedContour {
        kind: NodeGeneratedContourKind::Band { kind: source.0 },
        purpose: NodeGeneratedContourPurpose::JunctionSideJoin,
        source_mouth_order_index: source.1,
        source_band_index: Some(source.2),
        owner: Some(owner),
        claim_priority: NodeGeneratedContourClaimPriority::SideJoin,
        points_xz: contour_points.clone(),
        height_points_world: Some(vec![
            RoadVec3::new(-0.01, 10.0, -0.01),
            RoadVec3::new(0.01, 10.0, -0.01),
            RoadVec3::new(0.01, 10.0, 0.01),
            RoadVec3::new(-0.01, 10.0, 0.01),
        ]),
        backend_polyline: road_points_to_polyline(contour_points, true),
    });
    rails.source_carriers = NodeSourceCarrierRegistry::from_rail_parts(
        &rails.contours,
        &rails.constraints,
        &rails.height_carrier_paths_by_source,
        &rails.height_carrier_points_by_source,
    );
}

fn region_for_source(
    owner: NodeBandOwner,
    source: (RoadSurfaceBandKind, usize, usize),
    contour: NodeOverlayContour,
) -> NodeBooleanOwnedRegion {
    NodeBooleanOwnedRegion {
        kind: source.0,
        owner,
        claim_priority: NodeGeneratedContourClaimPriority::MouthBand,
        source_mouth_order_index: source.1,
        source_band_index: Some(source.2),
        shape: vec![contour],
        area_m2: 1.0,
        seam_constraints: Vec::new(),
    }
}
