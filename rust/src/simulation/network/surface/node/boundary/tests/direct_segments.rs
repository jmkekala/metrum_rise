//! Direct boundary segment owner-resolution tests.

use super::*;

#[test]
fn direct_boundary_segment_with_adjacent_material_endpoint_owners_uses_raised_owner() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(2.0, 0.0, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 3,
                grade_authority_index: 30,
            }),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        },
    );
    direct_vertex_sources.insert(
        end,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 4,
                grade_authority_index: 40,
            }),
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 6,
        },
    );
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("adjacent material direct endpoint sources should resolve to raised boundary owner");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            ..
        }
    ));
}

#[test]
fn direct_boundary_segment_with_same_material_distinct_endpoint_owners_is_rejected() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.12, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(2.0, 0.12, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 3,
                grade_authority_index: 30,
            }),
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 6,
        },
    );
    direct_vertex_sources.insert(
        end,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 4,
                grade_authority_index: 40,
            }),
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 4,
        },
    );
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("same-material direct endpoint sources must reject endpoint owner ambiguity");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn direct_boundary_segment_with_raised_step_connector_uses_raised_owner() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.12, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(0.15, 0.0, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 47,
                grade_authority_index: 3,
            }),
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 10,
        },
    );
    direct_vertex_sources.insert(
        end,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 23,
                grade_authority_index: 11,
            }),
            owner_kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 9,
        },
    );
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("raised-step connector should resolve to the raised boundary owner");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 10,
            ..
        }
    ));
}

#[test]
fn direct_boundary_segment_with_nonadjacent_raised_corner_uses_raised_owner() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.12, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(0.15, 0.0, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 47,
                grade_authority_index: 3,
            }),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 10,
        },
    );
    direct_vertex_sources.insert(
        end,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 23,
                grade_authority_index: 11,
            }),
            owner_kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 9,
        },
    );
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("raised/lower footprint corner should resolve to the raised owner");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 10,
            ..
        }
    ));
}

#[test]
fn raised_corner_with_equivalent_lower_material_candidates_uses_canonical_point_source() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.12, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(0.15, 0.0, 0.0));
    let raised = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 47,
            grade_authority_index: 3,
        }),
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 10,
    };
    let lower_a = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 23,
            grade_authority_index: 11,
        }),
        owner_kind: RoadSurfaceBandKind::Carriageway,
        owner_index: 9,
    };
    let lower_b = NodeFootprintBoundaryDirectVertex {
        source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
            top_surface_source_index: 29,
            grade_authority_index: 12,
        }),
        owner_kind: RoadSurfaceBandKind::Carriageway,
        owner_index: 12,
    };
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(start, raised);
    direct_vertex_sources.insert(end, lower_a);
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(end, vec![lower_a, lower_b]);
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &direct_vertex_source_candidates,
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-material lower endpoint candidates should keep canonical point provenance");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 150000,
                    z_key: 0,
                    y_mm: 0,
                },
                ..
            }),
            ..
        }
    ));
}

#[test]
fn direct_boundary_segment_with_explicit_step_owner_pair_is_accepted() {
    let start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.12, 0.0));
    let end = ArrangementBoundaryPointKey::from_world(Vector3::new(0.15, 0.0, 0.0));
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 10);
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 9);
    let explicit_vertical_step_segments = vec![
        arrangement::NodeExplicitVerticalStepSegment::new(
            start.xz_key(),
            end.xz_key(),
            raised_owner,
            lower_owner,
        )
        .expect("test step segment should be non-degenerate"),
    ];
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 47,
                grade_authority_index: 3,
            }),
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 10,
        },
    );
    direct_vertex_sources.insert(
        end,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 23,
                grade_authority_index: 11,
            }),
            owner_kind: RoadSurfaceBandKind::Carriageway,
            owner_index: 9,
        },
    );
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start),
        NodeFootprintBoundaryPoint::new(end),
        &[],
        &direct_vertex_sources,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &explicit_vertical_step_segments,
        &mut segments,
    )
    .expect("explicit owner-pair step must resolve direct boundary ownership");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 10,
            ..
        }
    ));
}
