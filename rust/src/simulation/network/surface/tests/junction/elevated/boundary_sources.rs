//! Elevated junction canonical boundary-source regression tests.

use super::*;

#[test]
fn logged_elevated_three_way_oblique_junction_compiles_with_canonical_boundary_sources() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-5.708, 139.500, 43.670), NodeType::Junction);
    let center = graph.add_node(Vector3::new(51.778, 146.820, 55.467), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(126.913, 143.009, 5.921), NodeType::Junction);
    let east = graph.add_node(Vector3::new(161.991, 147.143, 78.086), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-5.708, 139.500, 43.670),
            Vector3::new(51.778, 146.820, 55.467),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        vec![
            Vector3::new(51.778, 146.820, 55.467),
            Vector3::new(126.913, 143.009, 5.921),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![
            Vector3::new(51.778, 146.820, 55.467),
            Vector3::new(161.991, 147.143, 78.086),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "logged elevated oblique JunctionN should compile with canonical boundary sources: {}",
                canonical_junction_pipeline_report(&surface, &graph, center)
            )
        });
    assert_eq!(piece.kind, RoadSurfaceVisualNodePieceKind::JunctionN);
    assert_node_piece_uses_band_owned_regions(piece);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
}
