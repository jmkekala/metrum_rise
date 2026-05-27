//! Boundary height authority tests.

use super::*;

#[test]
fn boundary_height_uses_exact_source_edge_without_adjacent_contour_support() {
    let mut source_edge = test_source_edge(
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 2.0, 0.0),
        3,
        30,
        3,
        31,
    );
    source_edge.final_footprint_boundary = true;
    let midpoint_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 1.0, 0.0));
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
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
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 2.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let drifted_midpoint =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 1.0, 0.000002));
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
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
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut final_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(2.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    final_edge.final_footprint_boundary = true;
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![internal_edge, final_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let midpoint_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.12, 0.0));

    let height_mm = sources
        .boundary_height_mm_at_key(midpoint_key.xz_key())
        .expect("final footprint source edge should define terrain boundary height");

    assert_eq!(height_mm, Some(midpoint_key.y_mm));
}

#[test]
fn final_footprint_height_edge_rejects_overlay_grid_drift() {
    let start_point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.0, 0.0, 0.0));
    let end_point_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(2.0, 2.0, 2.0));
    let drifted_midpoint = ArrangementBoundaryPointKey {
        x_key: 1_000_000,
        z_key: 1_000_001,
        y_mm: 1000,
    };
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: vec![NodeFinalFootprintBoundaryHeightEdge {
            start_point_key,
            end_point_key,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 5,
        }],
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };

    let height_mm = sources
        .boundary_height_mm_at_contour_key(
            drifted_midpoint.xz_key(),
            start_point_key.xz_key(),
            end_point_key.xz_key(),
        )
        .expect("final footprint height lookup should reject non-exact boundary drift");

    assert_eq!(height_mm, None);
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(drifted_midpoint),
        "terrain and earthwork export must not repair non-exact final-footprint drift"
    );
}

#[test]
fn boundary_height_rejects_unauthorized_final_material_height_conflict() {
    let mut lower_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        9,
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let mut raised_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        RoadVec3::new(0.0, 0.12, 0.0),
        RoadVec3::new(2.0, 0.12, 0.0),
        4,
        40,
        4,
        41,
    );
    lower_edge.final_footprint_boundary = true;
    raised_edge.final_footprint_boundary = true;
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![lower_edge, raised_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let midpoint_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.12, 0.0));

    let error = sources
        .boundary_height_mm_at_key(midpoint_key.xz_key())
        .expect_err("final footprint raised-step conflict still needs explicit step authority");

    assert!(matches!(
        error,
        NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. }
    ));
}

#[test]
fn boundary_height_accepts_source_authorized_same_kind_vertical_step() {
    let key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.0, 0.0)).xz_key();
    let step_end = ArrangementBoundaryPointKey::from_world(RoadVec3::new(2.0, 0.0, 0.0)).xz_key();
    let lower_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 5);
    let raised_owner = arrangement::NodeBandOwner::new(RoadSurfaceBandKind::Sidewalk, 6);
    let lower_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 170046,
    };
    let raised_point = ArrangementBoundaryPointKey {
        x_key: key.x_key(),
        z_key: key.z_key(),
        y_mm: 170092,
    };
    let mut direct_vertex_source_candidates = BTreeMap::new();
    direct_vertex_source_candidates.insert(
        lower_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 340,
                grade_authority_index: 514,
            }),
            owner_kind: lower_owner.kind(),
            owner_index: lower_owner.owner_index(),
        }],
    );
    direct_vertex_source_candidates.insert(
        raised_point,
        vec![NodeFootprintBoundaryDirectVertex {
            source: NodeFootprintBoundaryVertexSource::Direct(NodeFootprintBoundaryDirectSource {
                top_surface_source_index: 342,
                grade_authority_index: 515,
            }),
            owner_kind: raised_owner.kind(),
            owner_index: raised_owner.owner_index(),
        }],
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: Vec::new(),
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates,
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: vec![
            arrangement::NodeExplicitVerticalStepSegment::new(
                key,
                step_end,
                lower_owner,
                raised_owner,
            )
            .expect("same-kind test step should be non-degenerate"),
        ],
    };

    let height_mm = sources
        .boundary_height_mm_at_key(key)
        .expect("same-kind vertical step owner pair is explicit source authority");

    assert_eq!(height_mm, Some(170092));
}

#[test]
fn boundary_height_rejects_same_owner_generated_edge_height_conflict() {
    let first_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        8,
        RoadVec3::new(0.0, 151.044, 0.0),
        RoadVec3::new(2.0, 151.044, 0.0),
        44,
        214,
        44,
        244,
    );
    let second_edge = test_source_edge_for_owner(
        RoadSurfaceBandKind::Carriageway,
        8,
        RoadVec3::new(0.0, 151.045, 0.0),
        RoadVec3::new(2.0, 151.045, 0.0),
        45,
        246,
        45,
        214,
    );
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![first_edge, second_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let midpoint_key = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 151.044, 0.0));

    assert!(matches!(
        sources.boundary_height_mm_at_key(midpoint_key.xz_key()),
        Err(NodeBoundaryExportError::ConflictingFootprintBoundaryHeight { .. })
    ));
}

#[test]
fn boundary_height_rejects_direct_conflict_with_exact_source_edge() {
    let source_edge = test_source_edge(
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let conflicting_point = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.2, 0.0));
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
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
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
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(2.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    let conflicting_point = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.0, 0.2, 0.0));
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
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources,
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
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
    let mut source_edge = test_source_edge(
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    source_edge.final_footprint_boundary = true;
    let mut sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let near_extension = ArrangementBoundaryPointKey::from_world(RoadVec3::new(1.00005, 0.0, 0.0));

    let height_mm = sources
        .height_mm_at_key(near_extension.xz_key())
        .expect("endpoint-scale drift should not conflict");

    assert_eq!(height_mm, None);
}

#[test]
fn numeric_cleanup_support_rejects_non_endpoint_boundary_drift() {
    let mut source_edge = test_source_edge(
        RoadVec3::new(0.0, 0.0, 0.0),
        RoadVec3::new(1.0, 0.0, 0.0),
        3,
        30,
        3,
        31,
    );
    source_edge.final_footprint_boundary = true;
    let sources = NodeFootprintBoundaryExportSources {
        source_edges: vec![source_edge],
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
        direct_vertex_sources: BTreeMap::new(),
        direct_vertex_source_candidates: BTreeMap::new(),
        direct_vertex_source_conflicts: BTreeMap::new(),
        grade_authority_source_provenance: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
    };
    let exact_midpoint = ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.5, 0.0, 0.0));
    let drifted_midpoint =
        ArrangementBoundaryPointKey::from_world(RoadVec3::new(0.5, 0.0, 0.00005));

    assert!(sources.has_exact_final_owned_footprint_boundary_support_at_point(exact_midpoint));
    assert!(
        !sources.has_exact_final_owned_footprint_boundary_support_at_point(drifted_midpoint),
        "numeric cleanup and sub-budget interpolation require source-edge support"
    );
}
