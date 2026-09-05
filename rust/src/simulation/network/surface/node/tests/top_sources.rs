// SPDX-License-Identifier: GPL-2.0-only

//! Node top-surface source provenance tests.

use super::*;

#[test]
fn node_top_surface_sources_preserve_explicit_material_seam_authority() {
    let owner = owner(RoadSurfaceBandKind::Carriageway, 6);
    let height_field_id = height_field(owner);
    let decision = NodeGradeCarrierDecision::ExplicitMaterialSeam;
    let heights = NodeHeightSolution {
        node_id: 82,
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
                    decision,
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(1.0, 0.0),
                    2.0,
                    owner,
                    height_field_id,
                    decision,
                ),
                heighted_vertex_with_grade_decision(
                    RoadVec2::new(0.0, 1.0),
                    2.0,
                    owner,
                    height_field_id,
                    decision,
                ),
            ]],
            area_m2: 0.5,
            seam_constraints: Vec::new(),
        }],
    };
    let mut arrangement = NodeArrangement::from_height_solution(&heights)
        .expect("grade-authorized explicit seam should arrange");
    let triangulation = RoadSurfaceSystem::build_node_triangulation_from_arrangement(&arrangement)
        .expect("grade-authorized explicit seam should triangulate");
    arrangement
        .attach_triangulation(&triangulation)
        .expect("grade-authorized explicit seam should attach triangulation");
    let footprint_shapes = footprint_shapes_from_points(&[
        RoadVec2::new(0.0, 0.0),
        RoadVec2::new(1.0, 0.0),
        RoadVec2::new(0.0, 1.0),
    ]);
    let regions =
        RoadSurfaceSystem::node_surface_regions_from_arrangement(&arrangement, &footprint_shapes)
            .expect("grade-authorized explicit seam should export node top provenance");

    assert_eq!(regions.node_top_surface_sources.len(), 1);
    let source = &regions.node_top_surface_sources[0];
    assert_eq!(source.kind, RoadSurfaceBandKind::Carriageway);
    assert_eq!(source.owner_index, owner.owner_index());
    assert_eq!(source.height_field_id, height_field_id);
    assert_eq!(source.vertex_sources.len(), 3);
    assert_eq!(source.triangle_sources.len(), 1);
    for grade_authority_index in source
        .vertex_sources
        .iter()
        .map(|source| source.grade_authority_index)
        .chain(
            source
                .triangle_sources
                .iter()
                .flat_map(|triangle| triangle.iter().map(|source| source.grade_authority_index)),
        )
    {
        assert_eq!(
            regions.node_grade_authorities[grade_authority_index].decision,
            NodeGradeCarrierDecision::ExplicitMaterialSeam
        );
    }
}
