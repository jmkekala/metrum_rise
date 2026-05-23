//! Boundary height authority tests.

use super::*;

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
fn boundary_height_prefers_final_footprint_source_edge_over_internal_edge() {
    let internal_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        9,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut final_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(2.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    final_edge.final_footprint_boundary = true;
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![internal_edge, final_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let midpoint_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.12, 0.0));

    let height_mm = sources
        .boundary_height_mm_at_key(midpoint_key.xz_key())
        .expect("final footprint source edge should define terrain boundary height");

    assert_eq!(height_mm, Some(midpoint_key.y_mm));
}

#[test]
fn boundary_height_uses_raised_final_edge_at_final_material_conflict() {
    let mut lower_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        9,
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut raised_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        Vector3::new(0.0, 0.12, 0.0),
        Vector3::new(2.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    lower_edge.final_footprint_boundary = true;
    raised_edge.final_footprint_boundary = true;
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let midpoint_key = ArrangementBoundaryPointKey::from_world(Vector3::new(1.0, 0.12, 0.0));

    let height_mm = sources
        .boundary_height_mm_at_key(midpoint_key.xz_key())
        .expect("final footprint raised-step conflict should use the raised final edge");

    assert_eq!(height_mm, Some(midpoint_key.y_mm));
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
fn numeric_cleanup_support_rejects_non_endpoint_boundary_drift() {
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
    let drifted_midpoint = ArrangementBoundaryPointKey::from_world(Vector3::new(0.5, 0.0, 0.00005));

    assert!(sources.has_exact_final_owned_footprint_boundary_support_at_point(exact_midpoint));
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(drifted_midpoint),
        "numeric cleanup and sub-budget interpolation require source-edge support"
    );
}
