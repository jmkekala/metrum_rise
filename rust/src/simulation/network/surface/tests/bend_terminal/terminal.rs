//! Terminal node-surface regression tests.

use super::*;

#[test]
fn bend_and_terminal_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);

    let mut bend_graph = RegionGraph::new();
    let bend_center = bend_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let bend_a = bend_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let bend_b = bend_graph.add_node(Vector3::new(0.0, 0.0, 20.0), NodeType::Junction);
    bend_graph.add_edge(test_edge(
        bend_center,
        bend_a,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    bend_graph.add_edge(test_edge(
        bend_center,
        bend_b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 20.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    bend_graph.rebuild_intersection_clips();
    let mut bend_surface = RoadSurfaceSystem::new(16.0);
    bend_surface.compile_dirty(&bend_graph, &terrain);
    let bend_piece = bend_surface
        .compiled_visual_node_pieces()
        .get(&bend_center)
        .expect("bend should compile once generated curb join ownership is explicit");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_piece_uses_band_owned_regions(bend_piece);
    assert_node_piece_has_curb_and_sidewalk_owners(bend_piece);
    assert_material_triangles_do_not_overlap(bend_piece);
    assert!(!bend_piece.outer_boundary_loops.is_empty());
    assert!(!bend_piece.road_surface_polygons.is_empty());
    assert!(!bend_piece.curb_surface_polygons.is_empty());
    assert!(!bend_piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(bend_piece);
    assert_outer_boundary_vertices_match_visible_top(bend_piece);

    let mut terminal_graph = RegionGraph::new();
    let terminal_center = terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let terminal_end = terminal_graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let terminal_edge_idx = terminal_graph.add_edge(test_edge(
        terminal_center,
        terminal_end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut terminal_surface = RoadSurfaceSystem::new(16.0);
    terminal_surface.compile_dirty(&terminal_graph, &terrain);
    let terminal_piece = terminal_surface
        .compiled_visual_node_pieces()
        .get(&terminal_center)
        .unwrap();
    assert_eq!(
        terminal_piece.kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
    assert_node_piece_uses_band_owned_regions(terminal_piece);
    assert_node_piece_has_curb_and_sidewalk_owners(terminal_piece);
    assert_material_triangles_do_not_overlap(terminal_piece);
    assert!(!terminal_piece.outer_boundary_loops.is_empty());
    assert!(!terminal_piece.road_surface_polygons.is_empty());
    assert!(!terminal_piece.curb_surface_polygons.is_empty());
    assert!(!terminal_piece.sidewalk_surface_polygons.is_empty());
    assert_top_mesh_centroids_inside_outer_boundary(terminal_piece);
    assert_outer_boundary_vertices_match_visible_top(terminal_piece);
    assert_node_top_covers_footprint(terminal_piece);
    assert!(
        terminal_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .curb_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    let terminal_span_piece = terminal_surface
        .compiled_visual_span_pieces()
        .get(&terminal_edge_idx)
        .unwrap();
    assert!(!terminal_span_piece.road_surface_polygons.is_empty());
    assert!(!terminal_piece.earthwork_surface_polygons.is_empty());
    assert!(!terminal_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!terminal_piece.render_earthwork_faces.is_empty());
    assert_node_earthwork_faces_have_footprint_provenance(terminal_piece);
    assert!(
        terminal_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        terminal_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        terminal_piece.earthwork_outer_boundary_loops,
        terminal_piece.outer_boundary_loops
    );
}

#[test]
fn angled_terminal_keeps_curb_strip_covered_on_both_sides() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(40.0, 0.0, 5.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 5.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("angled terminal should compile a terminal piece");
    let end_terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&end)
        .expect("opposite angled terminal should compile a terminal piece");
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("terminal road should keep a visible span after terminal handoff");

    let travel = backend::RoadVec2::new(40.0, 5.0).normalize();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = backend::RoadVec2::new(0.0, 0.0);
    for side in [-1.0, 1.0] {
        let curb_mid = center + lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, curb_mid),
            "angled terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, curb_mid),
            "terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );

        let sidewalk_corner = center - travel * 0.075 + lateral * side * 4.325;
        assert!(
            point_inside_visual_polygons(
                &terminal_piece.sidewalk_surface_polygons,
                sidewalk_corner
            ),
            "terminal sidewalk must close the endpoint-to-cap curb-depth corner on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, sidewalk_corner),
            "terminal sidewalk corner closure must not be owned by curb on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, sidewalk_corner),
            "terminal sidewalk corner closure must not be owned by asphalt on side {side}; point={sidewalk_corner:?}"
        );
    }

    let end_travel = backend::RoadVec2::new(-40.0, -5.0).normalize();
    let end_lateral = RoadSurfaceSystem::left_normal_xz(end_travel);
    let end_center = backend::RoadVec2::new(40.0, 5.0);
    for side in [-1.0, 1.0] {
        let curb_mid = end_center + end_lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&end_terminal_piece.curb_surface_polygons, curb_mid),
            "opposite angled terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&end_terminal_piece.road_surface_polygons, curb_mid),
            "opposite terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "opposite terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );

        let sidewalk_corner = end_center - end_travel * 0.075 + end_lateral * side * 4.325;
        assert!(
            point_inside_visual_polygons(
                &end_terminal_piece.sidewalk_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk must close the endpoint-to-cap curb-depth corner on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(
                &end_terminal_piece.curb_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk corner closure must not be owned by curb on side {side}; point={sidewalk_corner:?}"
        );
        assert!(
            !point_inside_visual_polygons(
                &end_terminal_piece.road_surface_polygons,
                sidewalk_corner
            ),
            "opposite terminal sidewalk corner closure must not be owned by asphalt on side {side}; point={sidewalk_corner:?}"
        );
    }
}

#[test]
fn straight_terminal_keeps_curb_strip_covered_on_both_sides() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let end = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[edge_idx]);
    assert_debug_dump_mouth_seams_are_clean(&dump);
    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("straight terminal should compile a terminal piece");
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("terminal road should keep a visible span after terminal handoff");
    let start_mouth = span_piece
        .start_mouth_profile
        .as_ref()
        .expect("terminal span should expose a start mouth profile");

    let left_curb_upper = start_mouth.bands[1].end_point_world;
    let left_road_lower = start_mouth.bands[2].start_point_world;
    assert_eq!(test_xz_key(left_curb_upper), test_xz_key(left_road_lower));
    assert!(
        (left_curb_upper.y - left_road_lower.y - f64::from(CURB_STEP_HEIGHT_M)).abs() <= 0.004,
        "left asphalt-curb mouth seam should keep the explicit vertical step"
    );
    assert_material_top_supports_point(
        &terminal_piece.curb_surface_polygons,
        left_curb_upper,
        "straight terminal left curb upper mouth seam",
    );
    assert_material_top_supports_point(
        &terminal_piece.road_surface_polygons,
        left_road_lower,
        "straight terminal left asphalt lower mouth seam",
    );

    let right_road_lower = start_mouth.bands[3].end_point_world;
    let right_curb_upper = start_mouth.bands[4].start_point_world;
    assert_eq!(test_xz_key(right_road_lower), test_xz_key(right_curb_upper));
    assert!(
        (right_curb_upper.y - right_road_lower.y - f64::from(CURB_STEP_HEIGHT_M)).abs() <= 0.004,
        "right asphalt-curb mouth seam should keep the explicit vertical step"
    );
    assert_material_top_supports_point(
        &terminal_piece.road_surface_polygons,
        right_road_lower,
        "straight terminal right asphalt lower mouth seam",
    );
    assert_material_top_supports_point(
        &terminal_piece.curb_surface_polygons,
        right_curb_upper,
        "straight terminal right curb upper mouth seam",
    );

    let travel = backend::RoadVec2::new(40.0, 0.0).normalize();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = backend::RoadVec2::new(0.0, 0.0);
    for side in [-1.0, 1.0] {
        let curb_mid = center + lateral * side * 3.575;
        assert!(
            point_inside_visual_polygons(&terminal_piece.curb_surface_polygons, curb_mid),
            "straight terminal curb strip must be owned by curb surface on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&terminal_piece.road_surface_polygons, curb_mid),
            "terminal curb strip must not be owned by asphalt on side {side}; point={curb_mid:?}"
        );
        assert!(
            !point_inside_visual_polygons(&span_piece.curb_surface_polygons, curb_mid),
            "terminal curb strip must not be duplicated by the span on side {side}; point={curb_mid:?}"
        );
    }
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        1,
        2,
        "left curb at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        0,
        1,
        "left sidewalk at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb at handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        terminal_piece,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk at handoff",
    );
}

#[test]
fn steep_standard_terminal_compiles_legal_height_ownership() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let points = vec![
        Vector3::new(178.256, 203.772, -564.088),
        Vector3::new(178.174, 203.724, -563.275),
        Vector3::new(178.103, 203.674, -562.575),
        Vector3::new(178.045, 203.619, -561.999),
        Vector3::new(177.978, 203.551, -561.337),
        Vector3::new(177.903, 203.462, -560.595),
        Vector3::new(177.820, 203.350, -559.774),
        Vector3::new(177.729, 203.220, -558.879),
        Vector3::new(177.656, 203.082, -558.161),
        Vector3::new(177.606, 202.946, -557.661),
        Vector3::new(177.554, 202.818, -557.143),
        Vector3::new(170.931, 183.624, -491.661),
    ];
    let start = graph.add_node(*points.first().unwrap(), NodeType::Junction);
    let end = graph.add_node(*points.last().unwrap(), NodeType::Junction);
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
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "steep terminal should compile with explicit terminal cap height ownership; dump={dump}"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "opposite steep terminal should compile with explicit terminal cap height ownership; dump={dump}"
    );
}
