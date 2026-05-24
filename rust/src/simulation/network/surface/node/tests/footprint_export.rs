//! Final node footprint export tests.

use super::*;

#[test]
fn node_export_uses_final_boolean_footprint_boundary_vertices() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let heights = NodeHeightSolution {
        node_id: 83,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id,
            shape: vec![vec![
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    2.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement =
        NodeArrangement::from_height_solution(&heights).expect("test triangle should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("test triangle should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("test triangle should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(0.5, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);

    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("footprint export should use final boolean footprint contour vertices");

    assert!(
        regions.outer_boundary_loops.iter().any(|polygon| {
            footprint_loop_contains_xz(&polygon.points_world, RoadVec2::new(0.5, 0.0))
        }),
        "node footprint export must consume final boolean node_footprint vertices"
    );
}

#[test]
fn node_export_rejects_unsupported_final_boolean_footprint_vertices() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let heights = NodeHeightSolution {
        node_id: 85,
        piece_kind: RoadSurfaceVisualNodePieceKind::JunctionN,
        regions: vec![NodeHeightedRegion {
            kind: RoadSurfaceBandKind::Carriageway,
            owner,
            height_field_id,
            shape: vec![vec![
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 0.0),
                    0.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    1.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    0.0,
                    owner,
                    height_field_id,
                    NodeGradeCarrierDecision::SourceCarrier { authority: None },
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement =
        NodeArrangement::from_height_solution(&heights).expect("test triangle should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("test triangle should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("test triangle should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.2, 0.2),
        RoadVec2::new(0.6, 0.2),
        RoadVec2::new(0.2, 0.6),
    ]);

    let error =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect_err("unsupported final footprint vertices must not be hidden by top triangles");

    assert!(
        matches!(
            error,
            NodeBoundaryExportError::MissingFootprintBoundaryHeight {
                x_key: 200000,
                z_key: 200000
            }
        ),
        "unsupported final node_footprint vertices must fail before terrain seam export: {error:?}"
    );
}
