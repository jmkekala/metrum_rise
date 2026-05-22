//! Node-piece source provenance assertions.

use super::*;

pub(in crate::simulation::network::surface::tests) fn assert_node_top_surface_sources_have_grade_authority(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert_eq!(
        piece.node_top_surface_sources.len(),
        piece.owned_regions.len(),
        "every emitted node top region must carry one provenance record"
    );
    assert!(
        !piece.node_grade_authorities.is_empty(),
        "node top provenance must reference a non-empty grade-authority table"
    );
    for source in &piece.node_top_surface_sources {
        assert!(
            !source.vertex_sources.is_empty(),
            "node top provenance must name polygon vertex sources"
        );
        assert!(
            !source.triangle_sources.is_empty(),
            "node top provenance must name emitted triangle sources"
        );
        for grade_authority_index in
            source
                .vertex_sources
                .iter()
                .map(|source| source.grade_authority_index)
                .chain(source.triangle_sources.iter().flat_map(|triangle| {
                    triangle.iter().map(|source| source.grade_authority_index)
                }))
        {
            assert!(
                grade_authority_index < piece.node_grade_authorities.len(),
                "node top provenance index {grade_authority_index} must reference an emitted grade-authority row"
            );
        }
    }
}

pub(in crate::simulation::network::surface::tests) fn assert_node_terrain_clip_sources_have_footprint_provenance(
    piece: &RoadSurfaceVisualNodePiece,
) {
    for edge in piece
        .terrain_clip_boundary_loops
        .iter()
        .flat_map(|boundary_loop| boundary_loop.source_edges.iter())
    {
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            boundary_source,
        } = edge.source
        else {
            panic!(
                "node terrain clip edge must carry node footprint provenance, got {:?}",
                edge.source
            );
        };
        assert_eq!(node_id, piece.node_id);
        assert_eq!(kind, piece.kind);
        assert!(
            piece
                .owned_regions
                .iter()
                .any(|region| region.kind == owner_kind && region.owner_index == owner_index),
            "node terrain clip edge owner must refer to a canonical owned top region"
        );
        let boundary_source =
            boundary_source.expect("node terrain clip edge must carry exact endpoint provenance");
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.start);
        assert_node_footprint_boundary_vertex_source_is_valid(piece, boundary_source.end);
    }
}
