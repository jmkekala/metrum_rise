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

    let (prepared_points, class) =
        RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Standard);
    assert!(
        prepared_points.len() > raw_points.len(),
        "standard road preparation should densify long alignment spans"
    );
    let first_terrain_y = terrain.sample_height_world(prepared_points[0].x, prepared_points[0].z)
        * crate::config::HEIGHT_SCALE;
    let last = prepared_points.last().unwrap();
    let last_terrain_y = terrain.sample_height_world(last.x, last.z) * crate::config::HEIGHT_SCALE;
    assert!((prepared_points[0].y - first_terrain_y).abs() <= 0.001);
    assert!((last.y - last_terrain_y).abs() <= 0.001);
}

#[test]
fn two_point_standard_input_densifies_against_open_terrain() {
    let terrain = planar_world_terrain(97, 33, 1.0, 0.0, 0.04, 0.0);
    let raw_points = vec![
        Vector3::new(
            -24.0,
            terrain.sample_height_world(-24.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2,
            0.0,
        ),
        Vector3::new(
            24.0,
            terrain.sample_height_world(24.0, 0.0) * crate::config::HEIGHT_SCALE + 0.2,
            0.0,
        ),
    ];

    let (prepared_points, class) =
        RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Standard);
    assert!(
        prepared_points.len() > 2,
        "long two-point road strokes must become dense physical geometry"
    );
    for point in prepared_points {
        let terrain_y = terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
        assert!(
            (point.y - terrain_y).abs() <= 0.05,
            "solved road profile should stay on a gentle source-terrain slope: point={:.3} terrain={terrain_y:.3}",
            point.y
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

    let (_points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);
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

    let (_points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);
    assert_eq!(class, EdgeClass::Bridge);
}

#[test]
fn terrain_to_raised_input_becomes_bridge_ramp() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 2.5, 0.0)];

    let (points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Bridge);
    assert_eq!(points.len(), raw_points.len());
    assert!((points[0].y - raw_points[0].y).abs() <= 0.001);
    assert!((points[1].y - raw_points[1].y).abs() <= 0.001);
}

#[test]
fn bridge_ramp_height_survives_terrain_sync() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 2.5, 0.0)];
    let (points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);
    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
    let end = graph.add_node(points[1], NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        start,
        end,
        points,
        7.0,
        class,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    graph.sync_to_terrain(&terrain);

    assert_eq!(graph.edge(edge_idx).class, EdgeClass::Bridge);
    assert!((graph.node(end).pos.y - raw_points[1].y).abs() <= 0.001);
    assert!((graph.edge(edge_idx).geometry.last().unwrap().y - raw_points[1].y).abs() <= 0.001);
    assert!(
        (graph.edge(edge_idx).physical_geometry.last().unwrap().y - raw_points[1].y).abs() <= 0.001
    );
}

#[test]
fn terrain_to_lowered_input_becomes_tunnel_ramp() {
    let terrain = flat_terrain(65, 33);
    let raw_points = vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, -2.5, 0.0)];

    let (points, class) = RoadSurfaceSystem::prepare_road_input_points(&raw_points, &terrain);

    assert_eq!(class, EdgeClass::Tunnel);
    assert_eq!(points.len(), raw_points.len());
    assert!((points[0].y - raw_points[0].y).abs() <= 0.001);
    assert!((points[1].y - raw_points[1].y).abs() <= 0.001);
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
fn point_query_index_excludes_distant_road_in_same_terrain_chunk() {
    let mut graph = RegionGraph::new();
    let near_a = graph.add_node(Vector3::new(4.0, 0.0, 8.0), NodeType::Junction);
    let near_b = graph.add_node(Vector3::new(24.0, 0.0, 8.0), NodeType::Junction);
    let far_a = graph.add_node(Vector3::new(196.0, 0.0, 8.0), NodeType::Junction);
    let far_b = graph.add_node(Vector3::new(216.0, 0.0, 8.0), NodeType::Junction);
    let near_edge = graph.add_edge(test_edge(
        near_a,
        near_b,
        vec![Vector3::new(4.0, 0.0, 8.0), Vector3::new(24.0, 0.0, 8.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let far_edge = graph.add_edge(test_edge(
        far_a,
        far_b,
        vec![Vector3::new(196.0, 0.0, 8.0), Vector3::new(216.0, 0.0, 8.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let terrain = flat_terrain(256, 64);
    let mut surface = RoadSurfaceSystem::new(RegionGraph::CHUNK_SIZE);
    surface.compile_dirty(&graph, &terrain);

    let query_chunk = RoadSurfaceSystem::query_chunk_coords_for_world(12.0, 8.0);
    let contributors = surface
        .query_chunk_spans
        .get(&query_chunk)
        .expect("near road should own its fine query chunk");

    assert!(contributors.contains(&near_edge));
    assert!(!contributors.contains(&far_edge));
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

#[test]
fn failed_span_recompile_removes_stale_visual_piece_and_chunk_coverage() {
    let edge_idx = 7;
    let surface_chunk = (1, 2);
    let terrain_chunk = (3, 4);
    let mut surface = RoadSurfaceSystem::new(16.0);

    surface
        .compiled_visual_span_pieces
        .insert(edge_idx, empty_visual_span_piece(edge_idx));
    surface
        .surface_span_chunks
        .insert(edge_idx, vec![surface_chunk]);
    surface
        .earthwork_span_chunks
        .insert(edge_idx, vec![terrain_chunk]);
    surface
        .surface_chunk_spans
        .entry(surface_chunk)
        .or_default()
        .insert(edge_idx);
    surface
        .earthwork_chunk_spans
        .entry(terrain_chunk)
        .or_default()
        .insert(edge_idx);

    surface.apply_span_compile_result(edge_idx, None);

    assert!(!surface.compiled_visual_span_pieces.contains_key(&edge_idx));
    assert!(!surface.surface_span_chunks.contains_key(&edge_idx));
    assert!(!surface.earthwork_span_chunks.contains_key(&edge_idx));
    assert!(!surface.surface_chunk_spans.contains_key(&surface_chunk));
    assert!(!surface.earthwork_chunk_spans.contains_key(&terrain_chunk));
    assert!(surface.dirty_surface_chunks().contains(&surface_chunk));
    assert!(surface.dirty_terrain_chunks().contains(&terrain_chunk));
}

#[test]
fn failed_node_recompile_removes_stale_visual_piece_input_and_chunk_coverage() {
    let node_id = 11;
    let surface_chunk = (-2, 5);
    let terrain_chunk = (-3, 6);
    let input = crate::simulation::network::surface::RoadSurfaceVisualNodeCompileInput {
        kind: RoadSurfaceVisualNodePieceKind::Terminal,
        mouths: Vec::new(),
    };
    let mut surface = RoadSurfaceSystem::new(16.0);

    surface
        .compiled_visual_node_pieces
        .insert(node_id, empty_visual_node_piece(node_id));
    surface
        .compiled_visual_node_inputs
        .insert(node_id, input.clone());
    surface
        .surface_node_chunks
        .insert(node_id, vec![surface_chunk]);
    surface
        .earthwork_node_chunks
        .insert(node_id, vec![terrain_chunk]);
    surface
        .surface_chunk_nodes
        .entry(surface_chunk)
        .or_default()
        .insert(node_id);
    surface
        .earthwork_chunk_nodes
        .entry(terrain_chunk)
        .or_default()
        .insert(node_id);

    surface.apply_node_compile_result(node_id, input, None);

    assert!(!surface.compiled_visual_node_pieces.contains_key(&node_id));
    assert!(!surface.compiled_visual_node_inputs.contains_key(&node_id));
    assert!(!surface.surface_node_chunks.contains_key(&node_id));
    assert!(!surface.earthwork_node_chunks.contains_key(&node_id));
    assert!(!surface.surface_chunk_nodes.contains_key(&surface_chunk));
    assert!(!surface.earthwork_chunk_nodes.contains_key(&terrain_chunk));
    assert!(surface.dirty_surface_chunks().contains(&surface_chunk));
    assert!(surface.dirty_terrain_chunks().contains(&terrain_chunk));
}

fn empty_visual_span_piece(edge_idx: usize) -> RoadSurfaceVisualSpanPiece {
    RoadSurfaceVisualSpanPiece {
        edge_idx,
        outer_boundary_loops: Vec::new(),
        terrain_clip_boundary_loops: Vec::new(),
        road_surface_polygons: Vec::new(),
        curb_surface_polygons: Vec::new(),
        raised_step_face_polygons: Vec::new(),
        span_raised_step_sources: Vec::new(),
        sidewalk_surface_polygons: Vec::new(),
        span_owned_regions: Vec::new(),
        edge_class: EdgeClass::Standard,
        start_mouth_profile: None,
        end_mouth_profile: None,
        start_terrain_clip_node: false,
        end_terrain_clip_node: false,
        span_earthwork_support_regions: Vec::new(),
        earthwork_surface_polygons: Vec::new(),
        earthwork_outer_boundary_loops: Vec::new(),
        render_earthwork_faces: Vec::new(),
    }
}

fn empty_visual_node_piece(node_id: u32) -> RoadSurfaceVisualNodePiece {
    RoadSurfaceVisualNodePiece {
        node_id,
        kind: RoadSurfaceVisualNodePieceKind::Terminal,
        outer_boundary_loops: Vec::new(),
        terrain_clip_boundary_loops: Vec::new(),
        road_surface_polygons: Vec::new(),
        curb_surface_polygons: Vec::new(),
        raised_step_face_polygons: Vec::new(),
        raised_step_face_sources: Vec::new(),
        sidewalk_surface_polygons: Vec::new(),
        explicit_vertical_step_segments: Vec::new(),
        node_grade_authorities: Vec::new(),
        node_top_surface_sources: Vec::new(),
        owned_regions: Vec::new(),
        boolean_debug: None,
        earthwork_owner_sources: Vec::new(),
        earthwork_surface_polygons: Vec::new(),
        earthwork_outer_boundary_loops: Vec::new(),
        render_earthwork_faces: Vec::new(),
    }
}
