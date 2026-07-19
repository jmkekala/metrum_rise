//! Elevated junction height-authority regression tests.

use super::*;

#[test]
fn elevated_four_way_junction_compiles_after_endpoint_profile_solve() {
    let terrain = planar_world_terrain(192, 192, 1.0, 150.0, 0.045, -0.018);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(
        0.0,
        terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE,
        0.0,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint_xz in [
        Vector2::new(-72.0, 0.0),
        Vector2::new(72.0, 0.0),
        Vector2::new(0.0, -72.0),
        Vector2::new(0.0, 72.0),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let points = if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            grounded_polyline_points_from_terrain(&terrain, endpoint_xz, Vector2::ZERO, 24)
        } else {
            grounded_polyline_points_from_terrain(&terrain, Vector2::ZERO, endpoint_xz, 24)
        };
        if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            graph.add_edge(test_edge(
                endpoint,
                center,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        } else {
            graph.add_edge(test_edge(
                center,
                endpoint,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
    }
    graph.rebuild_adjacency_list();
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn elevated_junction_rejects_contradictory_side_vertex_heights() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    for (endpoint_pos, starts_at_center) in [
        (Vector3::new(-80.0, 80.0, 0.0), false),
        (Vector3::new(80.0, -80.0, 0.0), true),
        (Vector3::new(0.0, 64.0, -80.0), false),
        (Vector3::new(0.0, -64.0, 80.0), true),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    for edge_idx in 0..graph.edge_count() {
        surface
            .compiled_sections
            .insert(edge_idx, surface.compile_edge_sections(&graph, edge_idx));
    }
    for edge_idx in 0..graph.edge_count() {
        let span_piece = surface
            .compile_visual_span_piece(&graph, &terrain, edge_idx)
            .expect("contradictory-height fixture spans must compile independently");
        surface.apply_span_compile_result(edge_idx, Some(span_piece));
    }
    let input = surface
        .visual_node_compile_input(&graph, center)
        .expect("contradictory-height fixture must produce a JunctionN input");
    let node_piece = surface.compile_visual_node_piece_from_input(&graph, &terrain, center, &input);
    surface.apply_node_compile_result(center, input, node_piece);

    let mut max_mouth_abs_y = 0.0_f64;
    for &edge_idx in graph.node_adjacency(center) {
        let edge = graph.edge(edge_idx);
        let span_piece = surface
            .compiled_visual_span_pieces()
            .get(&edge_idx)
            .expect("incident edge must compile a span piece");
        let mouth = if graph.get_valid_node(edge.start_node) == center {
            span_piece.start_mouth_profile.as_ref().unwrap()
        } else {
            span_piece.end_mouth_profile.as_ref().unwrap()
        };
        for point in &mouth.boundary_points_world {
            max_mouth_abs_y = max_mouth_abs_y.max(point.y.abs());
        }
    }
    assert!(
        max_mouth_abs_y >= 3.0,
        "test setup must put visible throats far above or below the endpoint; max_mouth_abs_y={max_mouth_abs_y:.3}"
    );
    if surface.compiled_visual_node_pieces().contains_key(&center) {
        let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0, 1, 2, 3]);
        assert!(
            !dump.contains("source_height_field_conflict")
                && !dump.contains("shared_source_height_conflict")
                && !dump.contains("height_conflict"),
            "steep JunctionN may compile only when same-XZ side vertices are resolved without unresolved height conflicts: {dump}"
        );
    }
}

#[test]
fn elevated_junction_endpoint_profile_limits_mouth_grade() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let raw_grade = 0.24;
    for endpoint_pos in [
        Vector3::new(10.0, 10.0 * raw_grade, 0.0),
        Vector3::new(-10.0, -10.0 * raw_grade, 0.0),
        Vector3::new(0.0, 0.0, 10.0),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            endpoint,
            vec![center_pos, endpoint_pos],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("steep but finite junction must expose a grade-limited endpoint plane");
    let origin_height_m = plane.height_at_xz(center_pos.x, center_pos.z);
    let grade_x = plane.height_at_xz(center_pos.x + 1.0, center_pos.z) - origin_height_m;
    let grade_z = plane.height_at_xz(center_pos.x, center_pos.z + 1.0) - origin_height_m;
    let limited_grade = grade_x.hypot(grade_z);

    assert!(
        limited_grade <= 0.161,
        "junction mouth profile grade must be capped before roadbed/node compilation; grade={limited_grade:.5}"
    );
    assert!(
        limited_grade >= 0.159,
        "test setup should exercise the cap instead of a naturally shallow plane; grade={limited_grade:.5}"
    );
}

#[test]
fn elevated_junction_endpoint_profile_preserves_source_grade_when_cap_would_move_samples_too_far() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let raw_grade = 0.24;
    for endpoint_pos in [
        Vector3::new(60.0, 60.0 * raw_grade, 0.0),
        Vector3::new(-60.0, -60.0 * raw_grade, 0.0),
        Vector3::new(0.0, 0.0, 60.0),
    ] {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            endpoint,
            vec![center_pos, endpoint_pos],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();

    let plane = graph
        .junction_endpoint_profile_plane(center)
        .expect("finite source-supported junction must expose an endpoint plane");
    let origin_height_m = plane.height_at_xz(center_pos.x, center_pos.z);
    let grade_x = plane.height_at_xz(center_pos.x + 1.0, center_pos.z) - origin_height_m;
    let grade_z = plane.height_at_xz(center_pos.x, center_pos.z + 1.0) - origin_height_m;
    let preserved_grade = grade_x.hypot(grade_z);

    assert!(
        preserved_grade >= 0.239,
        "profile cap must not move long incident source samples far enough to break ownership; grade={preserved_grade:.5}"
    );
    assert!(
        preserved_grade <= 0.241,
        "test setup should preserve the original source grade; grade={preserved_grade:.5}"
    );
}

#[test]
fn elevated_three_way_junction_compiles_after_endpoint_profile_solve() {
    let terrain = planar_world_terrain(192, 192, 1.0, 150.0, 0.045, -0.018);
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(
        0.0,
        terrain.sample_height_world(0.0, 0.0) * crate::config::HEIGHT_SCALE,
        0.0,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint_xz in [
        Vector2::new(-72.0, 0.0),
        Vector2::new(72.0, 0.0),
        Vector2::new(0.0, 72.0),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let points = if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            grounded_polyline_points_from_terrain(&terrain, endpoint_xz, Vector2::ZERO, 24)
        } else {
            grounded_polyline_points_from_terrain(&terrain, Vector2::ZERO, endpoint_xz, 24)
        };
        if endpoint_xz.x < 0.0 || endpoint_xz.y < 0.0 {
            graph.add_edge(test_edge(
                endpoint,
                center,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        } else {
            graph.add_edge(test_edge(
                center,
                endpoint,
                points,
                7.0,
                EdgeClass::Standard,
                TransitType::Road,
                TransitFlags::CAR | TransitFlags::FOOT,
            ));
        }
    }
    graph.rebuild_adjacency_list();
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn skewed_elevated_four_way_junction_compiles_with_explicit_height_carriers() {
    let terrain = planar_world_terrain(256, 256, 1.0, 148.0, -0.080, -0.035);
    let mut graph = RegionGraph::new();
    let center_xz = Vector2::new(14.096, -65.592);
    let center_pos = Vector3::new(
        center_xz.x,
        terrain.sample_height_world(center_xz.x, center_xz.y) * crate::config::HEIGHT_SCALE,
        center_xz.y,
    );
    let center = graph.add_node(center_pos, NodeType::Junction);
    for (endpoint_xz, starts_at_center) in [
        (Vector2::new(-15.703, -93.471), false),
        (Vector2::new(56.138, -72.850), false),
        (Vector2::new(-17.050, -60.215), true),
        (Vector2::new(50.308, -31.714), true),
    ] {
        let endpoint_pos = Vector3::new(
            endpoint_xz.x,
            terrain.sample_height_world(endpoint_xz.x, endpoint_xz.y) * crate::config::HEIGHT_SCALE,
            endpoint_xz.y,
        );
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (
                center,
                endpoint,
                grounded_polyline_points_from_terrain(&terrain, center_xz, endpoint_xz, 24),
            )
        } else {
            (
                endpoint,
                center,
                grounded_polyline_points_from_terrain(&terrain, endpoint_xz, center_xz, 24),
            )
        };
        graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_compiled_junction_piece(&surface, &graph, center);
}
