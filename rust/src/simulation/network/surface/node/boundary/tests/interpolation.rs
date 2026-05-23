//! Boundary interpolation and missing-height rejection tests.

use super::*;

#[test]
fn boundary_only_vertex_source_records_explicit_interpolation() {
    let start_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 3,
        grade_authority_index: 30,
    };
    let end_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 3,
        grade_authority_index: 31,
    };
    let start_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 1.0, 0.0));
    let end_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(2.0, 3.0, 0.0));
    let source_edge = NodeEarthworkBoundarySourceEdge {
        start_point_key,
        end_point_key,
        start_key: start_point_key.xz_key(),
        end_key: end_point_key.xz_key(),
        final_footprint_boundary: false,
        node_id: 11,
        kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
        height_field_id: arrangement::NodeBandHeightFieldId::new(
            0,
            5,
            RoadSurfaceBandKind::Sidewalk,
        ),
        start_source,
        end_source,
    };

    let direct =
        node_footprint_boundary_vertex_source_for_edge_point(&source_edge, start_point_key)
            .expect("source edge endpoint should preserve direct top provenance");
    assert_eq!(
        direct,
        NodeFootprintBoundaryVertexSource::Direct(start_source)
    );

    let midpoint_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0));
    let interpolated =
        node_footprint_boundary_vertex_source_for_edge_point(&source_edge, midpoint_key)
            .expect("boundary-only midpoint should be authorized by owning source edge");
    assert_eq!(
        interpolated,
        NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
            owning_segment_start: start_source,
            owning_segment_end: end_source,
            height_mm: midpoint_key.y_mm,
        }
    );

    let wrong_height_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.25, 0.0));
    assert!(
        node_footprint_boundary_vertex_source_for_edge_point(&source_edge, wrong_height_key)
            .is_none(),
        "boundary source recovery must block height drift instead of selecting unrelated top support"
    );
}

#[test]
fn source_edge_endpoint_dust_authorizes_boundary_segment() {
    let start_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 3,
        grade_authority_index: 30,
    };
    let end_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 3,
        grade_authority_index: 31,
    };
    let start_point_key = ArrangementBoundaryPointKey {
        x_key: 37_978_775,
        z_key: 3_650_000,
        y_mm: 120,
    };
    let source_end_point_key = ArrangementBoundaryPointKey {
        x_key: 37_978_772,
        z_key: 5_000_000,
        y_mm: 120,
    };
    let final_end_point_key = ArrangementBoundaryPointKey {
        x_key: 37_978_771,
        z_key: 5_000_000,
        y_mm: 120,
    };
    let source_edge = NodeEarthworkBoundarySourceEdge {
        start_point_key,
        end_point_key: source_end_point_key,
        start_key: start_point_key.xz_key(),
        end_key: source_end_point_key.xz_key(),
        final_footprint_boundary: false,
        node_id: 11,
        kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        owner_kind: RoadSurfaceBandKind::Sidewalk,
        owner_index: 5,
        height_field_id: arrangement::NodeBandHeightFieldId::new(
            0,
            5,
            RoadSurfaceBandKind::Sidewalk,
        ),
        start_source,
        end_source,
    };
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        NodeFootprintBoundaryPoint::new(start_point_key),
        NodeFootprintBoundaryPoint::new(final_end_point_key),
        &[source_edge],
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("source endpoint overlay dust should preserve source-backed boundary ownership");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(_),
                end: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                    height_mm: 120,
                    ..
                },
            }),
            ..
        }
    ));
}

#[test]
fn numeric_cleanup_support_ignores_contour_only_interpolation_sources() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let start_source = source_edge.start_source;
    let end_source = source_edge.end_source;
    let unsupported_point = Vector3::new(1.0, 0.0, 0.0002);
    let unsupported_key = ArrangementBoundaryPointKey::from_world(unsupported_point);
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        unsupported_key,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: start_source,
                owning_segment_end: end_source,
                height_mm: unsupported_key.y_mm,
            },
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        },
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let supported_midpoint = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0));

    assert!(
        sources.has_exact_final_owned_footprint_boundary_support_at_point(supported_midpoint),
        "source-edge interpolation is valid footprint boundary support"
    );
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(unsupported_key),
        "contour-only interpolation must keep numeric cleanup independent of unowned support"
    );

    let mut points = vec![
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(unsupported_point),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 1.0)),
        test_boundary_point(Vector3::new(0.0, 0.0, 1.0)),
    ];
    remove_subbudget_unsupported_numeric_boundary_vertices(
        &mut points,
        |point_key, local_points| {
            sources.has_exact_final_owned_footprint_boundary_support_at_point(point_key)
                || RoadSurfaceSystem::signed_polygon_area_xz(&local_points).abs()
                    > boundary_points_numeric_area_budget_m2(&local_points)
        },
    );

    assert_eq!(points.len(), 4);
    assert!(
        points
            .iter()
            .copied()
            .all(|point| point.point_key != unsupported_key)
    );
}

#[test]
fn missing_boundary_height_rejects_subbudget_run() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let vertices = vec![
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0)).xz_key(),
            Some(0),
        ),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.0002)).xz_key(),
            None,
        ),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0)).xz_key(),
            Some(0),
        ),
    ];

    let error = sources
        .reject_missing_footprint_boundary_heights(&vertices)
        .expect_err("sub-budget boundary-only runs must not invent contour height");

    assert!(matches!(
        error,
        NodeBoundaryExportError::MissingFootprintBoundaryHeight { .. }
    ));
    assert_eq!(vertices[1].1, None);
}

#[test]
fn missing_boundary_height_interpolation_rejects_contour_only_endpoint_source() {
    let source_edge = test_source_edge(
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let start_key = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start_key,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::BoundaryInterpolation {
                owning_segment_start: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 8,
                    grade_authority_index: 80,
                },
                owning_segment_end: NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 9,
                    grade_authority_index: 90,
                },
                height_mm: start_key.y_mm,
            },
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        },
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let vertices = vec![
        (start_key.xz_key(), Some(start_key.y_mm)),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.00001)).xz_key(),
            None,
        ),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0)).xz_key(),
            Some(0),
        ),
    ];

    let error = sources
        .reject_missing_footprint_boundary_heights(&vertices)
        .expect_err("sub-budget interpolation must be bounded by final-owned source support");

    assert!(matches!(
        error,
        NodeBoundaryExportError::MissingFootprintBoundaryHeight { .. }
    ));
    assert_eq!(vertices[1].1, None);
}

#[test]
fn missing_boundary_height_interpolation_rejects_overbudget_same_owner_connector() {
    let direct_source = NodeFootprintBoundaryDirectSource {
        top_surface_source_index: 7,
        grade_authority_index: 70,
    };
    let start_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0));
    let source_edge = test_source_edge(
        Vector3::new(2.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 2.0),
        8,
        80,
        8,
        81,
    );
    let end_point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(2.0, 0.0, 1.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        start_point_key,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(direct_source),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        },
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let vertices = vec![
        (start_point_key.xz_key(), Some(start_point_key.y_mm)),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0)).xz_key(),
            None,
        ),
        (end_point_key.xz_key(), Some(end_point_key.y_mm)),
    ];

    let error = sources
        .reject_missing_footprint_boundary_heights(&vertices)
        .expect_err("same-owner gaps must not authorize over-budget contour interpolation");

    assert!(matches!(
        error,
        NodeBoundaryExportError::MissingFootprintBoundaryHeight { .. }
    ));
    assert_eq!(vertices[1].1, None);
}

#[test]
fn missing_boundary_height_interpolation_rejects_overbudget_run() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let vertices = vec![
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0)).xz_key(),
            Some(0),
        ),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.2)).xz_key(),
            None,
        ),
        (
            ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0)).xz_key(),
            Some(0),
        ),
    ];

    let error = sources
        .reject_missing_footprint_boundary_heights(&vertices)
        .expect_err("over-budget missing topology must remain a hard contour interpolation error");

    assert!(matches!(
        error,
        NodeBoundaryExportError::MissingFootprintBoundaryHeight { .. }
    ));
    assert_eq!(vertices[1].1, None);
}
