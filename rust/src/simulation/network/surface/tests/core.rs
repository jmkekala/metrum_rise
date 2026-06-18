//! Core surface cache, input, overlay, and section regression tests.

use super::*;

#[test]
fn overlay_numeric_area_budget_accepts_logged_sub_visual_cdt_residual() {
    let small_four_edge_region = vec![vec![[0.0, 0.0], [0.02, 0.0], [0.02, 0.02], [0.0, 0.02]]];
    let budget_m2 =
        RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&small_four_edge_region);

    assert!(
        budget_m2 > 1.6660093e-5,
        "the logged 60-degree T-junction CDT residual must be treated as numeric dust, budget={budget_m2:.8}"
    );
    assert!(
        budget_m2 <= 1.0e-3,
        "numeric dust acceptance must remain capped below visually meaningful polygon loss"
    );
}

#[test]
fn overlay_numeric_area_budget_accepts_logged_centimeter_scale_cdt_residual() {
    let meter_scale_region = vec![vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]]];
    let budget_m2 = RoadSurfaceSystem::overlay_numeric_area_budget_for_shape(&meter_scale_region);

    assert!(
        budget_m2 > 0.00020319251,
        "the logged oblique 3-way CDT residual must be treated as numeric dust, budget={budget_m2:.8}"
    );
    assert!(
        budget_m2 <= 1.0e-3,
        "numeric dust acceptance must remain capped at 10 cm^2"
    );
}

#[test]
fn hill_crossing_input_stays_standard_instead_of_auto_tunnel() {
    let terrain = ridge_terrain(97, 33);
    let raw_points = vec![
        Vector3::new(
            -20.0,
            terrain.sample_height_world(-20.0, 0.0) * crate::config::HEIGHT_SCALE,
            0.0,
        ),
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(
            20.0,
            terrain.sample_height_world(20.0, 0.0) * crate::config::HEIGHT_SCALE,
            0.0,
        ),
    ];

    let (grounded_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Standard);
    for point in grounded_points {
        let terrain_y = terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
        assert!(
            (point.y - terrain_y).abs() <= 0.001,
            "standard grounding should snap to terrain at x={:.2}: point_y={:.3} terrain_y={:.3}",
            point.x,
            point.y,
            terrain_y
        );
    }
}

#[test]
fn uniformly_submerged_input_stays_auto_tunnel() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![
        Vector3::new(-10.0, -2.5, 0.0),
        Vector3::new(0.0, -2.5, 0.0),
        Vector3::new(10.0, -2.5, 0.0),
    ];

    let (_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
    assert_eq!(class, EdgeClass::Tunnel);
}

#[test]
fn uniformly_elevated_input_stays_auto_bridge() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![
        Vector3::new(-10.0, 2.5, 0.0),
        Vector3::new(0.0, 2.5, 0.0),
        Vector3::new(10.0, 2.5, 0.0),
    ];

    let (_points, class) =
        RoadSurfaceSystem::classify_and_ground_road_points(&raw_points, &terrain);
    assert_eq!(class, EdgeClass::Bridge);
}

#[test]
fn mark_edge_dirty_tracks_edge_without_centerline_chunk_guess() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(5.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(5.0, 0.0, 0.0), Vector3::new(25.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.mark_edge_dirty(&graph, edge_idx);

    assert!(surface.dirty_edges().contains(&edge_idx));
    assert!(surface.dirty_surface_chunks().is_empty());
    assert!(surface.dirty_terrain_chunks().is_empty());
}

#[test]
fn terrain_edit_marks_nearby_edges_nodes_and_chunks() {
    let mut graph = RegionGraph::new();
    let near_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let near_b = graph.add_node(Vector3::new(8.0, 0.0, 0.0), NodeType::Junction);
    let far_a = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
    let far_b = graph.add_node(Vector3::new(60.0, 0.0, 0.0), NodeType::Junction);
    let near_edge = graph.add_edge(test_edge(
        near_a,
        near_b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(8.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let far_edge = graph.add_edge(test_edge(
        far_a,
        far_b,
        vec![Vector3::new(50.0, 0.0, 0.0), Vector3::new(60.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(10.0);
    surface.mark_terrain_edit_dirty(&graph, Vector2::new(4.0, 0.0), 5.0);

    assert!(surface.dirty_edges().contains(&near_edge));
    assert!(!surface.dirty_edges().contains(&far_edge));
    assert!(surface.dirty_nodes().contains(&near_a));
    assert!(surface.dirty_nodes().contains(&near_b));
    assert!(!surface.dirty_nodes().contains(&far_a));
    assert!(!surface.dirty_nodes().contains(&far_b));
    assert!(surface.dirty_surface_chunks().contains(&(-1, -1)));
    assert!(surface.dirty_surface_chunks().contains(&(0, 0)));
    assert_eq!(
        surface.dirty_surface_chunks(),
        surface.dirty_terrain_chunks()
    );
}

#[test]
fn section_refinement_is_deterministic() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let terrain = flat_terrain(64, 64);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
    let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
    assert_eq!(sections_a, sections_b);
    let s_values: Vec<f32> = sections_a.iter().map(|section| section.s_m).collect();
    assert_eq!(s_values, vec![0.0, 6.0, 8.0, 14.0, 16.0, 20.0]);
}

#[test]
fn standard_edge_sections_follow_solved_edge_profile_deterministically() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-16.0, 99.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(16.0, 99.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![
            Vector3::new(-16.0, 99.0, 0.0),
            Vector3::new(16.0, 99.0, 0.0),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let terrain = sloped_terrain(33, 9);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    let sections_a = surface_a.compiled_sections().get(&edge_idx).unwrap();
    let sections_b = surface_b.compiled_sections().get(&edge_idx).unwrap();
    assert_eq!(sections_a, sections_b);
    for section in sections_a {
        let expected = 99.0;
        assert!((section.center_height_m - expected).abs() <= 0.001);
    }
}

#[test]
fn junction_profile_transition_sections_use_dense_visual_cadence() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let branch_specs = [
        (Vector3::new(80.0, 16.0, 0.0), true),
        (Vector3::new(-80.0, -16.0, 0.0), false),
        (Vector3::new(0.0, 0.0, 80.0), true),
    ];
    let mut profiled_edge_idx = None;
    for (endpoint_pos, starts_at_center) in branch_specs {
        let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
        let (start, end, points) = if starts_at_center {
            (center, endpoint, vec![center_pos, endpoint_pos])
        } else {
            (endpoint, center, vec![endpoint_pos, center_pos])
        };
        let edge_idx = graph.add_edge(test_edge(
            start,
            end,
            points,
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
        if starts_at_center && endpoint_pos.x > 0.0 {
            profiled_edge_idx = Some(edge_idx);
        }
    }
    graph.rebuild_adjacency_list();
    let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
    graph.solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    graph.rebuild_intersection_clips();

    assert!(
        graph.junction_endpoint_profile_plane(center).is_some(),
        "test setup must create a JunctionN endpoint profile"
    );

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let edge_idx = profiled_edge_idx.expect("test setup should track the uphill branch");
    let edge = graph.edge(edge_idx);
    let total_length_m = edge.physical_length;
    let start_kind = surface
        .classify_surface_node_kind_from_graph_geometry(&graph, graph.get_valid_node(center));
    let end_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.end_node),
    );
    let mouth_policy = surface.visual_edge_mouth_policy_for_edge(
        &graph,
        edge_idx,
        edge,
        total_length_m,
        start_kind,
        end_kind,
        true,
        false,
    );
    let (start_profile_s_m, end_profile_s_m) = mouth_policy
        .profile_range
        .expect("profiled edge should expose a profile fade range");
    let fade_m = crate::simulation::network::graph::rebuild::JUNCTION_PROFILE_BLEND_ZONE_M
        .min((end_profile_s_m - start_profile_s_m) * 0.5);

    let sections = surface
        .compiled_sections()
        .get(&edge_idx)
        .expect("profiled edge should compile sections");
    let transition_s = sections
        .iter()
        .map(|section| section.s_m)
        .filter(|&s_m| {
            s_m >= start_profile_s_m - SAMPLE_EPSILON_M
                && s_m <= start_profile_s_m + fade_m + SAMPLE_EPSILON_M
        })
        .collect::<Vec<_>>();
    assert!(
        transition_s.len() >= 8,
        "JunctionN profile fade should have enough visible sections to avoid long planar facets: {transition_s:?}"
    );
    for pair in transition_s.windows(2) {
        let step_m = pair[1] - pair[0];
        assert!(
            step_m <= 2.0 + SAMPLE_EPSILON_M,
            "profile transition sections must stay dense enough for curved visual grade, got step {step_m:.3} in {transition_s:?}"
        );
    }
}

#[test]
fn no_sidewalk_standard_edge_sections_keep_explicit_curb_shoulder_bands() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(32.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        n0,
        n1,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(32.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR,
    ));

    let terrain = flat_terrain(64, 64);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let section = surface
        .compiled_sections()
        .get(&edge_idx)
        .and_then(|sections| sections.first())
        .expect("no-sidewalk standard edge should compile sections");
    let band_kinds = section
        .bands
        .iter()
        .map(|band| band.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        band_kinds,
        vec![
            RoadSurfaceBandKind::CurbOrShoulder,
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::Carriageway,
            RoadSurfaceBandKind::CurbOrShoulder,
        ],
        "no-sidewalk Standard roads must keep explicit curb/shoulder carriers instead of zero-width or asphalt-only profiles"
    );
    for shoulder in [
        section.bands.first().unwrap(),
        section.bands.last().unwrap(),
    ] {
        assert!(
            (shoulder.lateral_end_m - shoulder.lateral_start_m).abs() > 0.1,
            "curb/shoulder carrier must have non-zero lateral width"
        );
        assert!(
            (shoulder.height_start_m - CURB_STEP_HEIGHT_M).abs() <= 0.001
                && (shoulder.height_end_m - CURB_STEP_HEIGHT_M).abs() <= 0.001,
            "curb/shoulder carrier must use the solved raised profile height"
        );
    }
}

#[test]
fn node_piece_classification_matches_surface_profiles() {
    let terrain = flat_terrain(64, 64);

    let mut pass_graph = RegionGraph::new();
    let pa = pass_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let pb = pass_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let pc = pass_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    pass_graph.add_edge(test_edge(
        pa,
        pb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    pass_graph.add_edge(test_edge(
        pb,
        pc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut pass_surface = RoadSurfaceSystem::new(16.0);
    pass_surface.compile_dirty(&pass_graph, &terrain);
    assert!(
        pass_surface
            .compiled_visual_node_pieces()
            .get(&pb)
            .is_none()
    );

    let mut width_graph = RegionGraph::new();
    let wa = width_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let wb = width_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let wc = width_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    width_graph.add_edge(test_edge(
        wa,
        wb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    width_graph.add_edge(test_edge(
        wb,
        wc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        14.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut width_surface = RoadSurfaceSystem::new(16.0);
    width_surface.compile_dirty(&width_graph, &terrain);
    assert!(
        width_surface
            .compiled_visual_node_pieces()
            .get(&wb)
            .is_none()
    );

    let mut junction_graph = RegionGraph::new();
    let ja = junction_graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let jb = junction_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let jc = junction_graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
    junction_graph.add_edge(test_edge(
        ja,
        jb,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    junction_graph.add_edge(test_edge(
        jb,
        jc,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut junction_surface = RoadSurfaceSystem::new(16.0);
    junction_surface.compile_dirty(&junction_graph, &terrain);
    assert_eq!(
        junction_surface
            .compiled_visual_node_pieces()
            .get(&jb)
            .unwrap_or_else(|| panic!(
                "short right-angle bend should compile through raw corridor ownership: {}",
                canonical_node_pipeline_report(
                    &junction_surface,
                    &junction_graph,
                    jb,
                    RoadSurfaceVisualNodePieceKind::Bend,
                )
            ))
            .kind,
        RoadSurfaceVisualNodePieceKind::Bend
    );

    let mut terminal_graph = RegionGraph::new();
    let ta = terminal_graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let tb = terminal_graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    terminal_graph.add_edge(test_edge(
        ta,
        tb,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let mut terminal_surface = RoadSurfaceSystem::new(16.0);
    terminal_surface.compile_dirty(&terminal_graph, &terrain);
    assert_eq!(
        terminal_surface
            .compiled_visual_node_pieces()
            .get(&ta)
            .unwrap()
            .kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
}
