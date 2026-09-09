// SPDX-License-Identifier: GPL-2.0-only

//! Node top-surface quality validation tests.

use super::*;
use crate::simulation::network::surface::NODE_OVERLAY_NUMERIC_DUST_WIDTH_M;

#[test]
fn adjacent_grade_checks_distinguish_overlay_slivers_from_resolved_cliffs() {
    for (width_m, origin, must_reject) in [
        (0.000001, 0.0, false),
        (0.000014, 60.0, false),
        (0.25, 60.0, true),
    ] {
        let id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
        let region = manual_region_with_constraints_and_triangles(
            RoadSurfaceBandKind::Carriageway,
            0,
            id,
            vec![
                RoadVec3::new(origin, 0.0, 0.0),
                RoadVec3::new(origin + 4.0, 0.0, 0.0),
                RoadVec3::new(origin, 0.0, 4.0),
                RoadVec3::new(origin + 2.0, 4.0, -width_m),
            ],
            vec![[0, 2], [1, 2], [0, 3], [1, 3]],
            vec![
                NodeTriangulatedTriangle {
                    vertices: [0, 1, 2],
                },
                NodeTriangulatedTriangle {
                    vertices: [1, 0, 3],
                },
            ],
            (8.0 + 2.0 * width_m) as f32,
        );
        let solution = NodeTriangulationSolution {
            node_id: 121,
            piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            regions: vec![region],
            explicit_vertical_step_segments: Vec::new(),
        };
        let mut diagnostics = Vec::new();
        super::super::triangles::validate_top_surface_triangle_quality(
            &solution,
            0,
            &solution.regions[0],
            &mut diagnostics,
        );
        assert_eq!(
            !diagnostics.is_empty(),
            must_reject,
            "width={width_m}: {diagnostics:?}"
        );
    }
}

#[test]
fn rejects_pathological_carriageway_triangle_with_numeric_dust_edge() {
    let height_field_id = NodeBandHeightFieldId::new(0, 0, RoadSurfaceBandKind::Carriageway);
    let solution = NodeTriangulationSolution {
        node_id: 120,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![manual_region_with_constraints_and_triangles(
            RoadSurfaceBandKind::Carriageway,
            0,
            height_field_id,
            vec![
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(0.00001, 0.0, 0.0),
                RoadVec3::new(0.0, 0.0, 3000.0),
            ],
            vec![[0, 1], [1, 2], [0, 2]],
            vec![NodeTriangulatedTriangle {
                vertices: [0, 1, 2],
            }],
            0.015,
        )],
        explicit_vertical_step_segments: Vec::new(),
    };

    let error = NodeValidationReport::from_triangulation_solution(&solution)
        .expect_err("pathological top-surface triangle should block node export");
    let diagnostic = error
        .report
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.stage == NodeGeometryStage::Validation
                && diagnostic.backend == NodeGeometryBackend::Spade
                && matches!(
                    diagnostic.kind,
                    NodeGeometryDiagnosticKind::PathologicalTopSurfaceTriangle { .. }
                )
        })
        .expect("pathological triangle diagnostic should name the bad face");

    let NodeGeometryDiagnosticKind::PathologicalTopSurfaceTriangle {
        region_index,
        triangle_index,
        reason,
        min_edge_m,
        plane_residual_max_m,
        ..
    } = &diagnostic.kind
    else {
        unreachable!("diagnostic kind matched above");
    };
    assert_eq!(*region_index, 0);
    assert_eq!(*triangle_index, 0);
    assert_eq!(*reason, "numeric_dust_edge");
    assert!(*min_edge_m < f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M));
    assert_eq!(*plane_residual_max_m, Some(0.0));
}
