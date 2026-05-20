//! Node boundary provenance and height-export tests.

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
        "boundary source recovery must block height drift instead of picking nearest top"
    );
}

#[test]
fn boundary_height_uses_exact_source_edge_without_adjacent_contour_support() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 2.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let midpoint_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.0));
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(midpoint_key.xz_key())
        .expect("exact final-owned boundary source edge should provide height");

    assert_eq!(height_mm, Some(midpoint_key.y_mm));
    assert!(
        sources.has_exact_final_owned_footprint_boundary_support_at_point(midpoint_key),
        "accepted source-edge midpoint must be recorded as final-owned boundary support"
    );
}

#[test]
fn boundary_height_rejects_project_quantization_drift_from_source_edge() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 2.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let drifted_midpoint =
        ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.000002));
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(drifted_midpoint.xz_key())
        .expect("drifted source edge lookup should not conflict");

    assert_eq!(height_mm, None);
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(
            ArrangementBoundaryPointKey {
                x_key: drifted_midpoint.x_key,
                z_key: drifted_midpoint.z_key,
                y_mm: 1000,
            }
        ),
        "canonical drift normalization must not become exact cleanup support"
    );
}

#[test]
fn boundary_height_rejects_direct_conflict_with_exact_source_edge() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let conflicting_point = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.2, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        conflicting_point,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 9,
                grade_authority_index: 90,
            }),
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        },
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(conflicting_point.xz_key())
        .expect_err("exact source edge and direct boundary height conflicts must reject");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn boundary_endpoint_candidate_rechecks_exact_conflicts_before_height_filter() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let conflicting_point = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.2, 0.0));
    let mut direct_vertex_sources = BTreeMap::new();
    direct_vertex_sources.insert(
        conflicting_point,
        NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 9,
                grade_authority_index: 90,
            }),
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

    let error = sources
        .height_candidate_at_point(conflicting_point.xz_key(), conflicting_point.y_mm)
        .expect_err("preselected endpoint height must not hide an exact same-XZ conflict");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn boundary_height_rejects_endpoint_scale_source_edge_extension() {
    let source_edge = test_source_edge(
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let near_extension = ArrangementBoundaryPointKey::from_world(Vector3::new(1.00005, 0.0, 0.0));

    let height_mm = sources
        .height_mm_at_key(near_extension.xz_key())
        .expect("endpoint-scale drift should not conflict");

    assert_eq!(height_mm, None);
}

#[test]
fn numeric_cleanup_support_requires_exact_final_owned_boundary_support() {
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
    let exact_midpoint = ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.0));
    let drifted_endpoint = ArrangementBoundaryPointKey::from_world(Vector3::new(1.00005, 0.0, 0.0));

    assert!(sources.has_exact_final_owned_footprint_boundary_support_at_point(exact_midpoint));
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(drifted_endpoint),
        "numeric cleanup and sub-budget interpolation require exact boundary support"
    );
}

#[test]
fn raised_step_footprint_height_requires_explicit_step_authority() {
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Carriageway, 0);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::CurbOrShoulder, 1);
    let step_start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        0,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut missing_authority = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = missing_authority
        .height_mm_at_key(step_start)
        .expect_err("material rank alone must not resolve same-XZ footprint height conflict");
    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));

    let mut authorized = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                step_start,
                step_end,
                lower_owner,
                raised_owner,
            )
            .expect("test step should be non-degenerate"),
        ],
    };

    let height_mm = authorized
        .height_mm_at_key(step_start)
        .expect("explicit owner-pair step should authorize raised footprint corner height");
    assert_eq!(height_mm, Some(120));
}

#[test]
fn terminal_raised_step_footprint_height_accepts_boundary_edge_authority() {
    let step_start = ArrangementBoundaryPointKey::from_world(Vector3::new(0.0, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(-1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(step_start)
        .expect("terminal source-edge endpoints should authorize raised footprint corner");

    assert_eq!(height_mm, Some(120));
}

#[test]
fn terminal_raised_step_footprint_height_rejects_endpoint_quantization_drift() {
    let drifted_step_start =
        ArrangementBoundaryPointKey::from_world(Vector3::new(0.000001, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(drifted_step_start)
        .expect_err("terminal endpoint-scale quantization drift must not authorize raised corners");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn terminal_raised_step_footprint_height_rejects_interior_edge_authority() {
    let step_midpoint =
        ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.0)).xz_key();
    let lower_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::Carriageway,
        0,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let raised_edge = test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::Terminal,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(1.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = sources
        .height_mm_at_key(step_midpoint)
        .expect_err("terminal source-edge authority is limited to footprint corners");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
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
        "contour-only interpolation must not make numeric cleanup depend on hidden support"
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
        .expect_err("over-budget missing topology must not be hidden by contour interpolation");

    assert!(matches!(
        error,
        NodeBoundaryExportError::MissingFootprintBoundaryHeight { .. }
    ));
    assert_eq!(vertices[1].1, None);
}

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
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(1.0, 1.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge(
            Vector3::new(1.0, 1.0, 0.0),
            Vector3::new(2.0, 2.0, 0.0),
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
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 2.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-height duplicate split points should merge without height repair");

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
        test_source_edge(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            6,
            Vector3::new(1.0, 0.12, 0.0),
            Vector3::new(1.0, 0.12, 1.0),
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
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
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
            point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.0)),
            source: None,
        },
    )
    .expect("first split point should insert");

    let error = insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0)),
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
            point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 2.0, 0.0)),
            source: Some(lower_order_source),
        },
    )
    .expect("first split point should insert");

    let error = insert_node_footprint_boundary_split_point(
        &mut split_points,
        parameter,
        NodeFootprintBoundarySplitPoint {
            point_key: ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 1.0, 0.0)),
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
fn overlapping_source_edges_with_distinct_provenance_are_rejected() {
    let source_edges = vec![
        test_source_edge(
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            6,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("coincident source edges with different provenance must not pick sorted first");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn overlapping_source_edges_with_identical_boundary_provenance_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("identical boundary provenance must merge without source priority");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::Direct(
                    NodeFootprintBoundaryDirectSource {
                        top_surface_source_index: 47,
                        grade_authority_index: 3,
                    },
                ),
                end: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                    top_surface_source_index: 23,
                    grade_authority_index: 11,
                },),
            }),
            ..
        }
    ));
}

#[test]
fn overlapping_adjacent_material_edges_with_one_same_height_handoff_source_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            11,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            69,
            4,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.12, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-height adjacent material handoff may share one canonical endpoint");

    assert_eq!(segments.len(), 1);
}

#[test]
fn overlapping_adjacent_material_edges_with_one_sloped_handoff_source_are_accepted() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::CurbOrShoulder,
            10,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            47,
            3,
            23,
            11,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            11,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            69,
            4,
            23,
            11,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.12, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-mouth adjacent material handoff may share one canonical endpoint");

    assert_eq!(segments.len(), 1);
}

#[test]
fn overlapping_source_edges_with_same_owner_distinct_provenance_are_rejected() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            3,
            30,
            3,
            31,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(2.0, 0.0, 0.0),
            4,
            40,
            4,
            41,
        ),
    ];
    let mut segments = Vec::new();

    let error = push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.0, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.0, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect_err("same owner must not hide distinct boundary provenance");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn overlapping_same_material_edges_with_equal_boundary_heights_use_canonical_segment_source() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            58,
            82,
            57,
            56,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            6,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            62,
            68,
            62,
            57,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.12, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("same-material equal-height boundary overlap should use canonical segment identity");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
            boundary_source: Some(NodeFootprintBoundarySegmentSource {
                start: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 0,
                    z_key: 0,
                    y_mm: 120,
                },
                end: NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
                    x_key: 2_000_000,
                    z_key: 0,
                    y_mm: 120,
                },
            }),
            ..
        }
    ));
}

#[test]
fn canonical_boundary_segment_source_matches_later_direct_source_at_same_keys() {
    let source_edges = vec![
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            58,
            82,
            57,
            56,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            6,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            62,
            68,
            62,
            57,
        ),
        test_source_edge_for_owner(
            RoadSurfaceBandKind::Sidewalk,
            5,
            Vector3::new(0.0, 0.12, 0.0),
            Vector3::new(2.0, 0.12, 0.0),
            91,
            92,
            91,
            93,
        ),
    ];
    let mut segments = Vec::new();

    push_sourced_node_earthwork_boundary_segments(
        11,
        RoadSurfaceVisualNodePieceKind::JunctionN,
        test_boundary_point(Vector3::new(0.0, 0.12, 0.0)),
        test_boundary_point(Vector3::new(2.0, 0.12, 0.0)),
        &source_edges,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &[],
        &mut segments,
    )
    .expect("canonical boundary point identity should absorb later direct source at same key");

    assert_eq!(segments.len(), 1);
}

#[test]
fn direct_boundary_segment_with_distinct_endpoint_owners_is_rejected() {
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
    .expect_err("direct fallback must reject endpoint owner ambiguity");

    assert!(matches!(
        error,
        NodeBoundaryExportError::AmbiguousEarthworkBoundarySegmentSource { .. }
    ));
}

#[test]
fn direct_boundary_segment_with_same_material_equal_height_uses_canonical_owner() {
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
    .expect("same-material equal-height boundary owners can normalize deterministically");

    assert_eq!(segments.len(), 1);
    assert!(matches!(
        segments[0].source,
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            owner_kind: RoadSurfaceBandKind::CurbOrShoulder,
            owner_index: 4,
            ..
        }
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

#[test]
fn same_height_boundary_point_with_distinct_source_identity_is_rejected() {
    let point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.0, 0.0));
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
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
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
fn same_height_boundary_point_accepts_reversed_interpolation_source_identity() {
    let point_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.12, 0.0));
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
        direct_vertex_sources,
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .height_mm_at_key(point_key.xz_key())
        .expect("reversed interpolation endpoints preserve the same undirected source identity");

    assert_eq!(height_mm, Some(point_key.y_mm));
}

fn test_boundary_point(point: Vector3) -> NodeFootprintBoundaryPoint {
    NodeFootprintBoundaryPoint::new(ArrangementBoundaryPointKey::from_world(point))
}

fn test_source_edge(
    start: Vector3,
    end: Vector3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    test_source_edge_for_owner(
        RoadSurfaceBandKind::Sidewalk,
        5,
        start,
        end,
        start_top_surface_source_index,
        start_grade_authority_index,
        end_top_surface_source_index,
        end_grade_authority_index,
    )
}

fn test_source_edge_for_owner(
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    start: Vector3,
    end: Vector3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    test_source_edge_for_owner_and_kind(
        RoadSurfaceVisualNodePieceKind::JunctionN,
        owner_kind,
        owner_index,
        start,
        end,
        start_top_surface_source_index,
        start_grade_authority_index,
        end_top_surface_source_index,
        end_grade_authority_index,
    )
}

fn test_source_edge_for_owner_and_kind(
    kind: RoadSurfaceVisualNodePieceKind,
    owner_kind: RoadSurfaceBandKind,
    owner_index: usize,
    start: Vector3,
    end: Vector3,
    start_top_surface_source_index: usize,
    start_grade_authority_index: usize,
    end_top_surface_source_index: usize,
    end_grade_authority_index: usize,
) -> NodeEarthworkBoundarySourceEdge {
    let start_point_key = ArrangementBoundaryPointKey::from_world(start);
    let end_point_key = ArrangementBoundaryPointKey::from_world(end);
    NodeEarthworkBoundarySourceEdge {
        start_point_key,
        end_point_key,
        start_key: start_point_key.xz_key(),
        end_key: end_point_key.xz_key(),
        final_footprint_boundary: false,
        node_id: 11,
        kind,
        owner_kind,
        owner_index,
        height_field_id: arrangement::NodeBandHeightFieldId::new(0, owner_index, owner_kind),
        start_source: NodeFootprintBoundaryDirectSource {
            top_surface_source_index: start_top_surface_source_index,
            grade_authority_index: start_grade_authority_index,
        },
        end_source: NodeFootprintBoundaryDirectSource {
            top_surface_source_index: end_top_surface_source_index,
            grade_authority_index: end_grade_authority_index,
        },
    }
}
