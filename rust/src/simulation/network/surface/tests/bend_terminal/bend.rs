//! Bend node-surface regression tests.

use super::*;

#[test]
fn flat_logged_curve_bend_compiles_with_explicit_point_contact_curb_ownership() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-17.539, 0.0, 12.635), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(57.560, 0.0, 4.157), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(119.799, 0.0, 82.841), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-17.539, 0.0, 12.635),
            Vector3::new(0.259, 0.0, 10.625),
            Vector3::new(30.126, 0.0, 7.254),
            Vector3::new(49.571, 0.0, 5.059),
            Vector3::new(57.560, 0.0, 4.157),
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
            Vector3::new(57.560, 0.0, 4.157),
            Vector3::new(61.267, 0.0, 8.844),
            Vector3::new(71.839, 0.0, 22.209),
            Vector3::new(89.956, 0.0, 45.112),
            Vector3::new(105.986, 0.0, 65.379),
            Vector3::new(119.799, 0.0, 82.841),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    graph.rebuild_adjacency_list();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_bend_piece(&surface, &graph, bend);
}

#[test]
fn hillside_curve_bend_blends_from_short_horizontal_pin() {
    let terrain = flat_terrain(128, 128);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 10.0, 0.0);
    let west_pos = Vector3::new(-48.0, 10.0, 0.0);
    let north_pos = Vector3::new(0.0, 22.0, 48.0);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let bend = graph.add_node(center_pos, NodeType::Junction);
    let north = graph.add_node(north_pos, NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![west_pos, center_pos],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        north,
        vec![center_pos, north_pos],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    graph.rebuild_adjacency_list();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_bend_piece(&surface, &graph, bend);
    let asphalt_y_range_m = visual_polygon_y_range_m(&piece.road_surface_polygons);
    assert!(
        asphalt_y_range_m <= 0.75,
        "Bend asphalt should curve gently through the owned bend footprint instead of forming a hard ramp: range={asphalt_y_range_m:.6}"
    );

    let edge = graph.edge(1);
    let total_length_m = edge.physical_length;
    let hard_pin_m = RegionGraph::junction_profile_hard_zone_m(edge, total_length_m);
    let start_kind =
        surface.classify_surface_node_kind_from_graph_geometry(&graph, graph.get_valid_node(bend));
    let end_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.end_node),
    );
    let (start_handoff_m, _) = surface
        .visual_surface_handoff_range_for_edge(
            &graph,
            1,
            edge,
            total_length_m,
            start_kind,
            end_kind,
        )
        .expect("bend edge should expose a visible ownership handoff");
    assert!(
        start_handoff_m > hard_pin_m + 4.0,
        "test setup must keep ownership handoff farther than the profile hard pin: handoff={start_handoff_m:.3} hard_pin={hard_pin_m:.3}"
    );

    let sections = surface
        .compiled_sections()
        .get(&1)
        .expect("bend outbound edge should compile sections");
    let endpoint_height_m = sections
        .first()
        .expect("outbound edge should have an endpoint section")
        .center_height_m;
    let hard_pin_section = sections
        .iter()
        .find(|section| (section.s_m - hard_pin_m).abs() <= SAMPLE_EPSILON_M)
        .expect("profile hard pin should be an explicit section sample");
    assert!(
        (hard_pin_section.center_height_m - endpoint_height_m).abs() <= 0.01,
        "Bend profile must keep only the short hard pin horizontal: endpoint={endpoint_height_m:.3} hard_pin={:.3}",
        hard_pin_section.center_height_m
    );

    let handoff_section = sections
        .iter()
        .find(|section| (section.s_m - start_handoff_m).abs() <= SAMPLE_EPSILON_M)
        .expect("ownership handoff should remain an explicit section sample");
    assert!(
        handoff_section.center_height_m > endpoint_height_m + 0.25,
        "Bend mouth should already be easing toward the incident edge grade instead of staying pinned flat until ownership handoff: endpoint={endpoint_height_m:.3} handoff={:.3}",
        handoff_section.center_height_m
    );
}

#[test]
fn triangle_network_compiles_as_three_independent_bend_pieces() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-30.0, 0.0, -16.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(30.0, 0.0, -16.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 36.0), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        east,
        vec![
            Vector3::new(-30.0, 0.0, -16.0),
            Vector3::new(30.0, 0.0, -16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        east,
        north,
        vec![Vector3::new(30.0, 0.0, -16.0), Vector3::new(0.0, 0.0, 36.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        north,
        west,
        vec![
            Vector3::new(0.0, 0.0, 36.0),
            Vector3::new(-30.0, 0.0, -16.0),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    graph.rebuild_adjacency_list();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_eq!(
        surface.compiled_visual_node_pieces().len(),
        3,
        "closed triangle corridors must compile as one bend piece per graph node"
    );
    for bend in [west, east, north] {
        assert_compiled_bend_piece(&surface, &graph, bend);
    }
}

fn visual_polygon_y_range_m(polygons: &[RoadSurfaceVisualPolygon]) -> f64 {
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for point in polygons
        .iter()
        .flat_map(|polygon| polygon.points_world.iter())
    {
        min_y = min_y.min(point.y);
        max_y = max_y.max(point.y);
    }
    if min_y.is_finite() && max_y.is_finite() {
        max_y - min_y
    } else {
        0.0
    }
}
