// SPDX-License-Identifier: GPL-2.0-only

//! Boundary split-point and source-identity tests.

use super::*;

#[test]
fn duplicate_split_point_same_height_preserves_sourced_subsegments() {
    let first_mid_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 3,
        grade_authority_index: 31,
    };
    let second_mid_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 4,
        grade_authority_index: 40,
    };
    let source_edges = vec![
        test_source_edge(
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(1.0, 1.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge(
            RoadVec3::new(1.0, 1.0, 0.0),
            RoadVec3::new(2.0, 2.0, 0.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 2.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-height duplicate split points should merge from matching explicit heights");

    assert_eq!(segments.len(), 2);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                end: NodeFootprintBoundaryVertexSource::Direct(source),
                ..
            }),
            ..
        } if source == first_mid_source
    ));
    assert!(matches!(
        segments[1].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(source),
                ..
            }),
            ..
        } if source == second_mid_source
    ));
}

#[test]
fn off_height_source_endpoint_does_not_split_boundary_segment() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Carriageway,
            3,
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            6,
            RoadVec3::new(1.0, 0.12, 0.0),
            RoadVec3::new(1.0, 0.12, 1.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(RoadVec3::new(0.0, 0.0, 0.0)),
        test_boundary_point(RoadVec3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("off-height source endpoints are not boundary split points");

    assert_eq!(segments.len(), 1);
}

#[test]
fn duplicate_split_point_conflicting_height_is_rejected() {
    let parameter =
        ArrangementSegmentParameter::new(1, 2).expect("test parameter should be canonical");
    let mut split_points = BTreeMap::new();
    insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 1.0, 0.0)),
            source: None,
        },
    )
    .expect("first split point should insert");

    let error = insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 2.0, 0.0)),
            source: None,
        },
    )
    .expect_err("duplicate split points with different heights must not pick max Y");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
            x_key: 1_000_000,
            z_key: 0,
            existing_y_mm: 1000,
            incoming_y_mm: 2000,
        }
    ));
}

#[test]
fn duplicate_split_point_conflicting_sourced_height_is_rejected() {
    let parameter =
        ArrangementSegmentParameter::new(1, 2).expect("test parameter should be canonical");
    let lower_order_source = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 1,
            grade_authority_index: 10,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let higher_order_source = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 2,
            grade_authority_index: 20,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut split_points = BTreeMap::new();
    insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 2.0, 0.0)),
            source: Some(lower_order_source),
        },
    )
    .expect("first split point should insert");

    let error = insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 1.0, 0.0)),
            source: Some(higher_order_source),
        },
    )
    .expect_err("duplicate sourced split points with different heights must reject");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundarySplitHeight {
            x_key: 1_000_000,
            z_key: 0,
            existing_y_mm: 2000,
            incoming_y_mm: 1000,
        }
    ));
}

#[test]
fn same_height_boundary_point_with_distinct_source_identity_is_rejected() {
    let point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0));
    let first = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 3,
            grade_authority_index: 30,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let second = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 4,
            grade_authority_index: 40,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(point_key, first);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(point_key, vec![first, second]);
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(point_key.xz_key())
        .expect_err("same-height point sources with distinct provenance must reject");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousFootprintBoundaryPointSource { .. }
    ));
}

#[test]
fn same_height_boundary_point_accepts_same_owner_direct_source_on_interpolated_edge() {
    let point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.12, 0.0));
    let direct = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 122,
            grade_authority_index: 42,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(point_key, direct);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(point_key, vec![direct]);
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![test_source_edge(
            RoadVec3::new(0.0, 0.12, 0.0),
            RoadVec3::new(2.0, 0.12, 0.0),
            121,
            53,
            121,
            37,
        )],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(point_key.xz_key())
        .expect("same-owner direct point and interpolated source edge prove one boundary point");

    assert_eq!(height_mm, Some(point_key.y_mm));
    assert!(matches!(
        sources.direct_vertex_sources.get(&point_key),
        Some(NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: 1_000_000,
                z_key: 0,
                y_mm: 120,
            },
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        })
    ));
}

#[test]
fn same_height_boundary_point_accepts_reversed_interpolation_source_identity() {
    let point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.12, 0.0));
    let first_start = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 105,
        grade_authority_index: 160,
    };
    let first_end = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 105,
        grade_authority_index: 162,
    };
    let second_start = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 107,
        grade_authority_index: 162,
    };
    let second_end = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 107,
        grade_authority_index: 160,
    };
    let first = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: first_start,
            owning_segment_end: first_end,
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let second = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: second_start,
            owning_segment_end: second_end,
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(point_key, first);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(point_key, vec![first, second]);
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(point_key.xz_key())
        .expect("reversed interpolation endpoints preserve the same undirected source identity");

    assert_eq!(height_mm, Some(point_key.y_mm));
}

#[test]
fn same_height_boundary_point_accepts_adjacent_interpolation_source_cluster() {
    let point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.12, 0.0));
    let first_start = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 1237,
        grade_authority_index: 1189,
    };
    let shared = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 1237,
        grade_authority_index: 1432,
    };
    let shared_second = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 1238,
        grade_authority_index: 1432,
    };
    let second_end = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 1238,
        grade_authority_index: 1168,
    };
    let first = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: first_start,
            owning_segment_end: shared,
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let second = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: shared_second,
            owning_segment_end: second_end,
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(point_key, first);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(point_key, vec![first, second]);
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(point_key.xz_key())
        .expect("same-owner adjacent source edges sharing one source vertex canonicalize");

    assert_eq!(height_mm, Some(point_key.y_mm));
    assert!(matches!(
        sources.direct_vertex_sources.get(&point_key),
        Some(NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: 1_000_000,
                z_key: 0,
                y_mm: 120,
            },
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        })
    ));
}

#[test]
fn same_height_boundary_point_accepts_same_owner_interpolation_cluster() {
    let point_key = ArrangementBoundaryPointKey {
        x_key: 572_943_237,
        z_key: 5_000_000,
        y_mm: 120,
    };
    let first = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1237,
                grade_authority_index: 1189,
            },
            owning_segment_end: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1237,
                grade_authority_index: 1432,
            },
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let second = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1269,
                grade_authority_index: 1461,
            },
            owning_segment_end: NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 1268,
                grade_authority_index: 1444,
            },
            height_mm: point_key.y_mm,
        },
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(point_key, first);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(point_key, vec![first, second]);
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(point_key.xz_key())
        .expect("same-owner post-boolean interpolation point should canonicalize by key");

    assert_eq!(height_mm, Some(point_key.y_mm));
    assert!(matches!(
        sources.direct_vertex_sources.get(&point_key),
        Some(NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                x_key: 572_943_237,
                z_key: 5_000_000,
                y_mm: 120,
            },
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        })
    ));
}
