//! Terminal raised-step source-edge authority tests.

use super::*;

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
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
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
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
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
        final_height_edges: Vec::new(),
        final_vertex_sources: BTreeMap::new(),
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
