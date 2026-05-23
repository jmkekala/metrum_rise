//! Boundary validation diagnostic tests.

use super::*;

#[test]
fn reports_open_boundaries_with_stage_and_backend() {
    let mut solution = solved_triangulation();
    solution.regions[0].boundary_constraints.pop();

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("missing explicit boundary constraint must fail validation");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::CanonicalKeys
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::OpenBoundary { .. }
            )
    }));
    let dump = error.report.debug_dump();
    assert!(dump.contains("\"stage\":\"validation\""));
    assert!(dump.contains("\"backend\":\"canonical_keys\""));
    assert!(dump.contains("\"kind\":\"open_boundary\""));
}

#[test]
fn reports_duplicate_exposed_edge_even_for_tiny_canonical_sliver() {
    let start = RoadVec3::new(0.0, 0.0, 0.0);
    let end = RoadVec3::new(0.005, 0.0, 0.0);
    let tiny_edge = key_edge(
        [f64::from(start.x), f64::from(start.z)],
        [f64::from(end.x), f64::from(end.z)],
    );
    let regions = [
        (
            RoadSurfaceBandKind::Carriageway,
            RoadVec3::new(0.0, 0.0, 1.0),
        ),
        (RoadSurfaceBandKind::Footpath, RoadVec3::new(0.0, 0.0, -1.0)),
        (RoadSurfaceBandKind::Parking, RoadVec3::new(0.005, 0.0, 1.0)),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (kind, apex))| {
        manual_region_with_constraints_and_triangles(
            kind,
            index,
            NodeBandHeightFieldId::new(0, index, kind),
            vec![start, end, apex],
            vec![[0, 1], [1, 2], [0, 2]],
            vec![NodeTriangulatedTriangle {
                vertices: [0, 1, 2],
            }],
            0.0025,
        )
    })
    .collect();
    let solution = NodeTriangulationSolution {
        node_id: 119,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions,
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("tiny duplicate exposed topology must be reported, not silently suppressed");

    assert!(error.report.diagnostics.iter().any(|diagnostic| {
        diagnostic.stage == NodeGeometryStage::Validation
            && diagnostic.backend == NodeGeometryBackend::Parry2d
            && matches!(
                diagnostic.kind,
                NodeGeometryDiagnosticKind::DuplicateExposedEdge {
                    region_index: None,
                    start_x_mm,
                    start_z_mm,
                    end_x_mm,
                    end_z_mm,
                    count: 3,
                } if start_x_mm == tiny_edge.start.x_mm()
                    && start_z_mm == tiny_edge.start.z_mm()
                    && end_x_mm == tiny_edge.end.x_mm()
                    && end_z_mm == tiny_edge.end.z_mm()
            )
    }));
}
