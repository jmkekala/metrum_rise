//! Final node footprint export tests.

use super::*;

#[test]
fn node_export_uses_final_owned_top_boundary_vertices() {
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
            .expect("footprint export should use final owned top contour vertices");

    assert!(
        !regions.outer_boundary_loops.iter().any(|polygon| {
            footprint_loop_contains_xz(&polygon.points_world, RoadVec2::new(0.5, 0.0))
        }),
        "node footprint export must consume final owned top vertices, not stale boolean-only contour points"
    );
}

#[test]
fn node_export_ignores_stale_post_triangulation_footprint_vertices() {
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

    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("footprint export should consume final owned top vertices");

    assert!(
        regions.outer_boundary_loops.iter().any(|polygon| {
            footprint_loop_contains_xz(&polygon.points_world, RoadVec2::new(1.0, 0.0))
        }),
        "final top vertex must be exported with its explicit source height"
    );
    assert!(
        !regions.outer_boundary_loops.iter().any(|polygon| {
            footprint_loop_contains_xz(&polygon.points_world, RoadVec2::new(0.2, 0.2))
        }),
        "stale footprint vertices must not trigger post-triangulation height sampling"
    );
}
