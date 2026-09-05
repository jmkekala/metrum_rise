// SPDX-License-Identifier: GPL-2.0-only

//! Oblique bend and terminal regression tests.

use super::*;

#[test]
fn logged_sixty_degree_bend_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-131.350, 0.0, -31.215), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-21.350, 0.0, -31.215), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(13.650, 0.0, 29.406), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-131.350, 0.0, -31.215),
            Vector3::new(-21.350, 0.0, -31.215),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(-21.350, 0.0, -31.215),
            Vector3::new(13.650, 0.0, 29.406),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, &graph, bend);
}

#[test]
fn logged_flat_sixty_degree_bend_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-104.032, 0.0, -0.181), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-4.032, 0.0, -0.181), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(30.968, 0.0, 60.440), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-104.032, 0.0, -0.181),
            Vector3::new(-4.032, 0.0, -0.181),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(-4.032, 0.0, -0.181),
            Vector3::new(30.968, 0.0, 60.440),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, &graph, bend);
}

#[test]
fn logged_oblique_curve_bend_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-137.811, 0.0, -32.495), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-62.948, 0.0, -30.476), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-0.213, 0.0, 15.063), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-137.811, 0.0, -32.495),
            Vector3::new(-62.948, 0.0, -30.476),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        vec![
            Vector3::new(-62.948, 0.0, -30.476),
            Vector3::new(-0.213, 0.0, 15.063),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, &graph, bend);
}

#[test]
fn logged_oblique_terminal_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(256, 256);
    let points = road_points_from_json(
        "[[56.267,0.0,-24.078],[57.235,0.0,-24.012],[58.162,0.0,-23.950],\
        [59.047,0.0,-23.890],[59.889,0.0,-23.833],[60.687,0.0,-23.779],\
        [61.440,0.0,-23.728],[62.147,0.0,-23.680],[62.808,0.0,-23.635],\
        [63.421,0.0,-23.594],[63.985,0.0,-23.556],[64.501,0.0,-23.521],\
        [65.379,0.0,-23.462],[66.049,0.0,-23.416],[66.762,0.0,-23.368]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    graph.add_edge(test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    for node_id in [start, end] {
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .unwrap_or_else(|| {
                panic!(
                    "logged oblique road endpoint should compile a terminal piece: {}",
                    canonical_node_pipeline_report(
                        &surface,
                        &graph,
                        node_id,
                        RoadSurfaceVisualNodePieceKind::Terminal,
                    )
                )
            });
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_node_top_covers_footprint(terminal_piece);
    }
}
