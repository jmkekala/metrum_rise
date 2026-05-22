//! Junction and arbitrary-node canonical pipeline regression tests.

use super::*;

#[test]
fn visual_node_rejection_is_deterministic_for_multi_arm_nodes() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 10.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(64, 64);
    let mut surface_a = RoadSurfaceSystem::new(16.0);
    let mut surface_b = RoadSurfaceSystem::new(16.0);
    surface_a.compile_dirty(&graph, &terrain);
    surface_b.compile_dirty(&graph, &terrain);

    assert_eq!(
        surface_a
            .compiled_visual_node_pieces()
            .get(&center)
            .expect("flat multi-arm node should compile through raw corridor ownership")
            .kind,
        RoadSurfaceVisualNodePieceKind::JunctionN
    );
    assert_eq!(
        surface_a.compiled_visual_node_pieces().get(&center),
        surface_b.compiled_visual_node_pieces().get(&center)
    );
}

#[test]
fn oblique_t_junction_compiles_with_canonical_side_join_ownership() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(12.0, 0.0, 20.784609), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(12.0, 0.0, 20.784609),
        ],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn editor_sized_60_degree_t_junction_width_7_compiles_side_join_ownership() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-87.843, 0.0, -11.753), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-50.197, 0.0, -11.753), NodeType::Junction);
    let right = graph.add_node(Vector3::new(32.157, 0.0, -11.753), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(-20.197, 0.0, 40.209), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![
            Vector3::new(-87.843, 0.0, -11.753),
            Vector3::new(-50.197, 0.0, -11.753),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![
            Vector3::new(-50.197, 0.0, -11.753),
            Vector3::new(32.157, 0.0, -11.753),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(-50.197, 0.0, -11.753),
            Vector3::new(-20.197, 0.0, 40.209),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);

    let raw_clip_sources = surface
        .compiled_visual_span_pieces()
        .values()
        .flat_map(|piece| piece.terrain_clip_boundary_loops.iter().cloned())
        .chain(
            surface
                .compiled_visual_node_pieces()
                .values()
                .flat_map(|piece| piece.terrain_clip_boundary_loops.iter().cloned()),
        )
        .collect::<Vec<_>>();
    assert!(
        !raw_clip_sources.is_empty(),
        "editor-sized 60-degree T junction must have raw terrain clip source loops"
    );
    let unioned_clip_sources =
        RoadSurfaceSystem::union_terrain_clip_boundary_export(&raw_clip_sources)
            .expect("editor-sized 60-degree T junction clip union should be source-complete");
    assert!(
        !unioned_clip_sources.loops.is_empty(),
        "editor-sized 60-degree T junction raw clip loops must survive deterministic union"
    );

    let (road_loops, _) = surface
        .terrain_cdt_road_loops_for_world_bounds(&graph, -128.0, -32.0, 64.0, 64.0)
        .expect("editor-sized 60-degree T junction clip export should be source-complete");
    assert!(
        !road_loops.is_empty(),
        "editor-sized 60-degree T junction must export terrain clip loops"
    );
    let mesh = build_road_touched_terrain_patch(TerrainCdtInput::new(
        TerrainCdtPatch::new(-128.0, -32.0, 64.0, 64.0, [0.0; 4]),
        road_loops,
        Vec::new(),
    ))
    .expect("editor-sized 60-degree T terrain cutters must be accepted by terrain CDT");
    assert_eq!(mesh.stats.invalid_constraint_edges, 0);
}

#[test]
fn logged_flat_three_way_oblique_junction_compiles_side_join_ownership() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-60.311, 0.0, -3.324), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-12.773, 0.0, -3.324), NodeType::Junction);
    let east = graph.add_node(Vector3::new(79.689, 0.0, -3.324), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(22.227, 0.0, 57.298), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-60.311, 0.0, -3.324),
            Vector3::new(-12.773, 0.0, -3.324),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(-12.773, 0.0, -3.324),
            Vector3::new(22.227, 0.0, 57.298),
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
            Vector3::new(-12.773, 0.0, -3.324),
            Vector3::new(79.689, 0.0, -3.324),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "logged flat three-way oblique JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn logged_current_flat_three_way_oblique_junction_compiles_side_join_ownership() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-82.716, 0.0, -14.881), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-25.618, 0.0, -14.881), NodeType::Junction);
    let east = graph.add_node(Vector3::new(57.284, 0.0, -14.881), NodeType::Junction);
    let oblique = graph.add_node(Vector3::new(30.950, 0.0, 41.687), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-82.716, 0.0, -14.881),
            Vector3::new(-25.618, 0.0, -14.881),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        oblique,
        vec![
            Vector3::new(-25.618, 0.0, -14.881),
            Vector3::new(30.950, 0.0, 41.687),
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
            Vector3::new(-25.618, 0.0, -14.881),
            Vector3::new(57.284, 0.0, -14.881),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn logged_flat_three_way_right_angle_junction_compiles_explicit_raised_steps() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-102.807, 0.0, -14.721), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-35.427, 0.0, -14.721), NodeType::Junction);
    let east = graph.add_node(Vector3::new(37.193, 0.0, -14.721), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-35.427, 0.0, 35.279), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-102.807, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, -14.721),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(-35.427, 0.0, 35.279),
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
            Vector3::new(-35.427, 0.0, -14.721),
            Vector3::new(37.193, 0.0, -14.721),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn logged_flat_three_way_oblique_variant_compiles_with_explicit_vertical_steps() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-74.754, 0.0, -4.117), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-20.950, 0.0, -6.649), NodeType::Junction);
    let east = graph.add_node(Vector3::new(40.079, 0.0, -9.522), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(25.060, 0.0, 55.624), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-74.754, 0.0, -4.117),
            Vector3::new(-20.950, 0.0, -6.649),
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
            Vector3::new(-20.950, 0.0, -6.649),
            Vector3::new(25.060, 0.0, 55.624),
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
            Vector3::new(-20.950, 0.0, -6.649),
            Vector3::new(40.079, 0.0, -9.522),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

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

#[test]
fn logged_current_elevated_oblique_three_way_compiles_with_endpoint_profile_solve() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-6.578, 141.206, -5.989), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-43.834, 158.291, -122.338), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-23.211, 150.463, -57.933), NodeType::Junction);
    let branch = graph.add_node(Vector3::new(8.837, 153.266, -120.160), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-6.578, 141.206, -5.989),
            Vector3::new(-6.816, 141.251, -6.732),
            Vector3::new(-6.996, 141.296, -7.296),
            Vector3::new(-7.224, 141.339, -8.008),
            Vector3::new(-7.499, 141.380, -8.865),
            Vector3::new(-7.653, 141.423, -9.346),
            Vector3::new(-7.817, 141.469, -9.860),
            Vector3::new(-7.993, 141.523, -10.408),
            Vector3::new(-8.179, 141.586, -10.988),
            Vector3::new(-8.375, 141.659, -11.601),
            Vector3::new(-8.581, 141.740, -12.244),
            Vector3::new(-8.797, 141.830, -12.919),
            Vector3::new(-9.022, 141.925, -13.623),
            Vector3::new(-9.257, 142.028, -14.356),
            Vector3::new(-9.501, 142.138, -15.119),
            Vector3::new(-9.754, 142.255, -15.909),
            Vector3::new(-10.016, 142.377, -16.726),
            Vector3::new(-10.286, 142.500, -17.570),
            Vector3::new(-10.565, 142.617, -18.440),
            Vector3::new(-10.851, 142.718, -19.336),
            Vector3::new(-11.146, 142.798, -20.256),
            Vector3::new(-11.372, 142.854, -20.961),
            Vector3::new(-11.525, 142.890, -21.439),
            Vector3::new(-11.680, 142.913, -21.923),
            Vector3::new(-11.837, 142.930, -22.412),
            Vector3::new(-11.995, 142.948, -22.907),
            Vector3::new(-12.155, 142.970, -23.408),
            Vector3::new(-12.317, 142.996, -23.913),
            Vector3::new(-12.481, 143.025, -24.425),
            Vector3::new(-12.646, 143.055, -24.941),
            Vector3::new(-12.814, 143.086, -25.463),
            Vector3::new(-12.982, 143.117, -25.990),
            Vector3::new(-13.153, 143.149, -26.522),
            Vector3::new(-13.325, 143.180, -27.059),
            Vector3::new(-13.498, 143.211, -27.601),
            Vector3::new(-13.673, 143.242, -28.147),
            Vector3::new(-13.850, 143.277, -28.698),
            Vector3::new(-14.028, 143.320, -29.254),
            Vector3::new(-14.207, 143.376, -29.815),
            Vector3::new(-14.388, 143.447, -30.380),
            Vector3::new(-14.570, 143.534, -30.949),
            Vector3::new(-14.754, 143.632, -31.522),
            Vector3::new(-14.939, 143.737, -32.100),
            Vector3::new(-15.125, 143.845, -32.682),
            Vector3::new(-15.313, 143.954, -33.268),
            Vector3::new(-15.502, 144.062, -33.857),
            Vector3::new(-15.692, 144.170, -34.451),
            Vector3::new(-15.883, 144.279, -35.049),
            Vector3::new(-16.075, 144.390, -35.650),
            Vector3::new(-16.269, 144.502, -36.255),
            Vector3::new(-16.464, 144.614, -36.863),
            Vector3::new(-16.660, 144.726, -37.475),
            Vector3::new(-16.857, 144.839, -38.090),
            Vector3::new(-17.055, 144.957, -38.708),
            Vector3::new(-17.254, 145.083, -39.330),
            Vector3::new(-17.454, 145.221, -39.955),
            Vector3::new(-17.655, 145.372, -40.583),
            Vector3::new(-17.857, 145.535, -41.213),
            Vector3::new(-18.060, 145.706, -41.847),
            Vector3::new(-18.264, 145.880, -42.483),
            Vector3::new(-18.468, 146.056, -43.122),
            Vector3::new(-18.674, 146.231, -43.764),
            Vector3::new(-18.880, 146.405, -44.408),
            Vector3::new(-19.087, 146.579, -45.055),
            Vector3::new(-19.295, 146.753, -45.704),
            Vector3::new(-19.504, 146.926, -46.356),
            Vector3::new(-19.713, 147.097, -47.009),
            Vector3::new(-19.923, 147.266, -47.665),
            Vector3::new(-20.133, 147.434, -48.323),
            Vector3::new(-20.345, 147.606, -48.983),
            Vector3::new(-20.557, 147.786, -49.644),
            Vector3::new(-20.769, 147.976, -50.307),
            Vector3::new(-20.982, 148.177, -50.973),
            Vector3::new(-21.195, 148.386, -51.639),
            Vector3::new(-21.409, 148.602, -52.308),
            Vector3::new(-21.624, 148.822, -52.977),
            Vector3::new(-21.839, 149.046, -53.648),
            Vector3::new(-22.054, 149.275, -54.321),
            Vector3::new(-22.270, 149.506, -54.994),
            Vector3::new(-22.486, 149.732, -55.669),
            Vector3::new(-22.702, 149.946, -56.345),
            Vector3::new(-22.919, 150.138, -57.021),
            Vector3::new(-23.136, 150.308, -57.699),
            Vector3::new(-23.211, 150.463, -57.933),
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
            Vector3::new(-23.211, 150.463, -57.933),
            Vector3::new(-22.851, 150.678, -58.632),
            Vector3::new(-22.541, 150.881, -59.233),
            Vector3::new(-22.286, 151.062, -59.728),
            Vector3::new(-21.994, 151.223, -60.296),
            Vector3::new(-21.665, 151.370, -60.934),
            Vector3::new(-21.302, 151.508, -61.639),
            Vector3::new(-20.906, 151.642, -62.408),
            Vector3::new(-20.478, 151.768, -63.239),
            Vector3::new(-20.138, 151.883, -63.900),
            Vector3::new(-19.902, 151.983, -64.358),
            Vector3::new(-19.659, 152.069, -64.830),
            Vector3::new(-19.409, 152.146, -65.316),
            Vector3::new(-19.152, 152.220, -65.814),
            Vector3::new(-18.889, 152.297, -66.326),
            Vector3::new(-18.619, 152.380, -66.849),
            Vector3::new(-18.343, 152.470, -67.384),
            Vector3::new(-18.062, 152.565, -67.931),
            Vector3::new(-17.774, 152.660, -68.489),
            Vector3::new(-17.481, 152.749, -69.058),
            Vector3::new(-17.183, 152.825, -69.638),
            Vector3::new(-16.879, 152.883, -70.227),
            Vector3::new(-16.571, 152.924, -70.827),
            Vector3::new(-16.257, 152.950, -71.436),
            Vector3::new(-15.939, 152.969, -72.054),
            Vector3::new(-15.616, 152.986, -72.680),
            Vector3::new(-15.289, 153.006, -73.315),
            Vector3::new(-14.958, 153.030, -73.958),
            Vector3::new(-14.623, 153.058, -74.609),
            Vector3::new(-14.284, 153.089, -75.267),
            Vector3::new(-13.941, 153.122, -75.932),
            Vector3::new(-13.595, 153.159, -76.603),
            Vector3::new(-13.246, 153.198, -77.281),
            Vector3::new(-12.894, 153.239, -77.965),
            Vector3::new(-12.539, 153.280, -78.654),
            Vector3::new(-12.182, 153.318, -79.348),
            Vector3::new(-11.822, 153.351, -80.047),
            Vector3::new(-11.459, 153.377, -80.751),
            Vector3::new(-11.095, 153.396, -81.458),
            Vector3::new(-10.729, 153.405, -82.170),
            Vector3::new(-10.360, 153.405, -82.885),
            Vector3::new(-9.991, 153.396, -83.602),
            Vector3::new(-9.620, 153.376, -84.323),
            Vector3::new(-9.247, 153.348, -85.046),
            Vector3::new(-8.874, 153.314, -85.770),
            Vector3::new(-8.500, 153.275, -86.497),
            Vector3::new(-8.125, 153.235, -87.224),
            Vector3::new(-7.750, 153.195, -87.953),
            Vector3::new(-7.375, 153.158, -88.682),
            Vector3::new(-6.999, 153.124, -89.411),
            Vector3::new(-6.624, 153.095, -90.140),
            Vector3::new(-6.248, 153.071, -90.868),
            Vector3::new(-5.874, 153.053, -91.596),
            Vector3::new(-5.500, 153.038, -92.322),
            Vector3::new(-5.126, 153.025, -93.047),
            Vector3::new(-4.754, 153.012, -93.770),
            Vector3::new(-4.383, 152.999, -94.490),
            Vector3::new(-4.013, 152.986, -95.208),
            Vector3::new(-3.645, 152.972, -95.923),
            Vector3::new(-3.279, 152.958, -96.634),
            Vector3::new(-2.914, 152.943, -97.342),
            Vector3::new(-2.552, 152.930, -98.045),
            Vector3::new(-2.192, 152.919, -98.744),
            Vector3::new(-1.834, 152.913, -99.439),
            Vector3::new(-1.479, 152.915, -100.128),
            Vector3::new(-1.127, 152.926, -100.811),
            Vector3::new(-0.778, 152.944, -101.489),
            Vector3::new(-0.432, 152.968, -102.161),
            Vector3::new(-0.090, 152.994, -102.826),
            Vector3::new(0.249, 153.021, -103.484),
            Vector3::new(0.584, 153.047, -104.134),
            Vector3::new(0.915, 153.072, -104.777),
            Vector3::new(1.242, 153.096, -105.412),
            Vector3::new(1.565, 153.119, -106.039),
            Vector3::new(1.883, 153.142, -106.657),
            Vector3::new(2.197, 153.164, -107.266),
            Vector3::new(2.506, 153.186, -107.865),
            Vector3::new(2.809, 153.208, -108.455),
            Vector3::new(3.108, 153.228, -109.034),
            Vector3::new(3.401, 153.245, -109.603),
            Vector3::new(3.688, 153.258, -110.161),
            Vector3::new(3.970, 153.268, -110.708),
            Vector3::new(4.246, 153.275, -111.244),
            Vector3::new(4.515, 153.279, -111.767),
            Vector3::new(4.778, 153.282, -112.278),
            Vector3::new(5.035, 153.284, -112.777),
            Vector3::new(5.285, 153.287, -113.262),
            Vector3::new(5.528, 153.289, -113.734),
            Vector3::new(5.764, 153.291, -114.193),
            Vector3::new(6.105, 153.292, -114.854),
            Vector3::new(6.532, 153.292, -115.684),
            Vector3::new(6.928, 153.292, -116.453),
            Vector3::new(7.291, 153.291, -117.158),
            Vector3::new(7.620, 153.288, -117.796),
            Vector3::new(7.912, 153.285, -118.364),
            Vector3::new(8.168, 153.279, -118.860),
            Vector3::new(8.477, 153.273, -119.461),
            Vector3::new(8.837, 153.266, -120.160),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        vec![
            Vector3::new(-23.211, 150.463, -57.933),
            Vector3::new(-23.353, 150.663, -58.377),
            Vector3::new(-23.570, 150.859, -59.056),
            Vector3::new(-23.788, 151.047, -59.736),
            Vector3::new(-24.006, 151.223, -60.416),
            Vector3::new(-24.224, 151.384, -61.097),
            Vector3::new(-24.442, 151.532, -61.778),
            Vector3::new(-24.660, 151.670, -62.459),
            Vector3::new(-24.878, 151.805, -63.141),
            Vector3::new(-25.097, 151.940, -63.823),
            Vector3::new(-25.315, 152.078, -64.504),
            Vector3::new(-25.533, 152.219, -65.186),
            Vector3::new(-25.751, 152.364, -65.868),
            Vector3::new(-25.970, 152.514, -66.549),
            Vector3::new(-26.188, 152.667, -67.230),
            Vector3::new(-26.406, 152.821, -67.911),
            Vector3::new(-26.624, 152.968, -68.591),
            Vector3::new(-26.841, 153.101, -69.271),
            Vector3::new(-27.059, 153.210, -69.950),
            Vector3::new(-27.276, 153.292, -70.628),
            Vector3::new(-27.493, 153.351, -71.306),
            Vector3::new(-27.709, 153.392, -71.982),
            Vector3::new(-27.926, 153.425, -72.658),
            Vector3::new(-28.142, 153.458, -73.333),
            Vector3::new(-28.358, 153.494, -74.006),
            Vector3::new(-28.573, 153.534, -74.679),
            Vector3::new(-28.788, 153.576, -75.350),
            Vector3::new(-29.002, 153.618, -76.020),
            Vector3::new(-29.216, 153.660, -76.688),
            Vector3::new(-29.430, 153.702, -77.355),
            Vector3::new(-29.643, 153.743, -78.020),
            Vector3::new(-29.855, 153.784, -78.683),
            Vector3::new(-30.067, 153.827, -79.345),
            Vector3::new(-30.278, 153.871, -80.004),
            Vector3::new(-30.489, 153.918, -80.662),
            Vector3::new(-30.699, 153.966, -81.318),
            Vector3::new(-30.908, 154.015, -81.971),
            Vector3::new(-31.117, 154.064, -82.623),
            Vector3::new(-31.324, 154.113, -83.272),
            Vector3::new(-31.532, 154.162, -83.919),
            Vector3::new(-31.738, 154.211, -84.563),
            Vector3::new(-31.943, 154.259, -85.205),
            Vector3::new(-32.148, 154.307, -85.844),
            Vector3::new(-32.352, 154.355, -86.480),
            Vector3::new(-32.555, 154.403, -87.114),
            Vector3::new(-32.757, 154.452, -87.745),
            Vector3::new(-32.958, 154.500, -88.372),
            Vector3::new(-33.158, 154.547, -88.997),
            Vector3::new(-33.357, 154.593, -89.619),
            Vector3::new(-33.555, 154.637, -90.237),
            Vector3::new(-33.752, 154.680, -90.852),
            Vector3::new(-33.948, 154.721, -91.464),
            Vector3::new(-34.143, 154.761, -92.073),
            Vector3::new(-34.336, 154.800, -92.677),
            Vector3::new(-34.529, 154.838, -93.279),
            Vector3::new(-34.720, 154.875, -93.876),
            Vector3::new(-34.910, 154.912, -94.470),
            Vector3::new(-35.099, 154.949, -95.059),
            Vector3::new(-35.287, 154.984, -95.645),
            Vector3::new(-35.473, 155.019, -96.227),
            Vector3::new(-35.658, 155.052, -96.805),
            Vector3::new(-35.841, 155.082, -97.378),
            Vector3::new(-36.024, 155.110, -97.948),
            Vector3::new(-36.205, 155.139, -98.512),
            Vector3::new(-36.384, 155.172, -99.073),
            Vector3::new(-36.562, 155.214, -99.629),
            Vector3::new(-36.739, 155.267, -100.180),
            Vector3::new(-36.914, 155.333, -100.726),
            Vector3::new(-37.087, 155.409, -101.268),
            Vector3::new(-37.259, 155.491, -101.805),
            Vector3::new(-37.429, 155.575, -102.337),
            Vector3::new(-37.598, 155.658, -102.864),
            Vector3::new(-37.765, 155.739, -103.386),
            Vector3::new(-37.931, 155.818, -103.902),
            Vector3::new(-38.094, 155.895, -104.414),
            Vector3::new(-38.256, 155.969, -104.920),
            Vector3::new(-38.417, 156.041, -105.420),
            Vector3::new(-38.575, 156.111, -105.915),
            Vector3::new(-38.732, 156.184, -106.404),
            Vector3::new(-38.887, 156.264, -106.888),
            Vector3::new(-39.040, 156.358, -107.366),
            Vector3::new(-39.266, 156.467, -108.072),
            Vector3::new(-39.560, 156.591, -108.991),
            Vector3::new(-39.847, 156.724, -109.887),
            Vector3::new(-40.125, 156.860, -110.757),
            Vector3::new(-40.396, 156.992, -111.601),
            Vector3::new(-40.657, 157.117, -112.418),
            Vector3::new(-40.910, 157.233, -113.208),
            Vector3::new(-41.155, 157.341, -113.971),
            Vector3::new(-41.389, 157.442, -114.704),
            Vector3::new(-41.615, 157.537, -115.408),
            Vector3::new(-41.831, 157.627, -116.083),
            Vector3::new(-42.037, 157.710, -116.726),
            Vector3::new(-42.233, 157.789, -117.339),
            Vector3::new(-42.419, 157.863, -117.919),
            Vector3::new(-42.594, 157.934, -118.467),
            Vector3::new(-42.759, 158.002, -118.982),
            Vector3::new(-42.913, 158.067, -119.462),
            Vector3::new(-43.187, 158.129, -120.319),
            Vector3::new(-43.415, 158.187, -121.032),
            Vector3::new(-43.596, 158.240, -121.595),
            Vector3::new(-43.834, 158.291, -122.338),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    assert!((graph.edge(0).end_clip - 12.071).abs() <= 0.01);
    assert!((graph.edge(1).start_clip - 12.071).abs() <= 0.01);
    assert!((graph.edge(2).start_clip - 12.071).abs() <= 0.01);

    let mut main_geometry = graph.edge(0).geometry.clone();
    main_geometry.extend(graph.edge(2).geometry.iter().skip(1).copied());
    let mut stale_graph = RegionGraph::new();
    let stale_west = stale_graph.add_node(graph.node(west).pos, NodeType::Junction);
    let stale_south = stale_graph.add_node(graph.node(south).pos, NodeType::Junction);
    stale_graph.add_edge(test_edge(
        stale_west,
        stale_south,
        main_geometry,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    stale_graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&stale_graph, &terrain);
    for edge_idx in 0..graph.edge_count() {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    for node_id in [west, south, center, branch] {
        surface.mark_node_dirty(&graph, node_id);
    }
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "current elevated oblique 3-way JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
}

#[test]
fn logged_latest_elevated_oblique_three_way_compiles_with_endpoint_profile_solve() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-29.527,139.925,4.210],[-29.585,139.927,3.491],[-29.629,139.928,2.946],[-29.685,139.930,2.256],[-29.752,139.931,1.428],[-29.809,139.933,0.718],[-29.851,139.936,0.204],[-29.895,139.940,-0.342],[-29.941,139.944,-0.919],[-29.991,139.950,-1.526],[-30.042,139.956,-2.164],[-30.096,139.962,-2.831],[-30.152,139.969,-3.526],[-30.211,139.975,-4.250],[-30.271,139.981,-5.000],[-30.334,139.984,-5.778],[-30.399,139.986,-6.582],[-30.466,139.989,-7.411],[-30.535,139.996,-8.265],[-30.606,140.014,-9.143],[-30.679,140.046,-10.044],[-30.754,140.091,-10.969],[-30.830,140.146,-11.915],[-30.909,140.204,-12.884],[-30.969,140.258,-13.623],[-31.009,140.304,-14.123],[-31.050,140.342,-14.628],[-31.091,140.375,-15.138],[-31.132,140.406,-15.652],[-31.174,140.438,-16.171],[-31.217,140.470,-16.696],[-31.260,140.502,-17.224],[-31.303,140.533,-17.757],[-31.346,140.564,-18.295],[-31.390,140.598,-18.837],[-31.434,140.636,-19.384],[-31.479,140.684,-19.934],[-31.523,140.741,-20.489],[-31.569,140.808,-21.048],[-31.614,140.881,-21.611],[-31.660,140.958,-22.177],[-31.706,141.036,-22.748],[-31.753,141.113,-23.322],[-31.799,141.190,-23.900],[-31.846,141.266,-24.482],[-31.894,141.342,-25.067],[-31.941,141.419,-25.655],[-31.989,141.496,-26.247],[-32.037,141.571,-26.842],[-32.085,141.645,-27.440],[-32.134,141.719,-28.041],[-32.183,141.796,-28.646],[-32.232,141.881,-29.253],[-32.281,141.977,-29.863],[-32.331,142.088,-30.476],[-32.381,142.212,-31.092],[-32.431,142.347,-31.710],[-32.481,142.486,-32.331],[-32.531,142.628,-32.954],[-32.582,142.768,-33.579],[-32.632,142.908,-34.207],[-32.683,143.047,-34.837],[-32.734,143.187,-35.470],[-32.786,143.326,-36.104],[-32.837,143.464,-36.740],[-32.889,143.601,-37.378],[-32.940,143.739,-38.018],[-32.992,143.880,-38.660],[-33.044,144.030,-39.303],[-33.096,144.192,-39.948],[-33.149,144.369,-40.595],[-33.201,144.558,-41.243],[-33.254,144.756,-41.892],[-33.306,144.959,-42.542],[-33.359,145.162,-43.194],[-33.412,145.365,-43.846],[-33.464,145.566,-44.500],[-33.517,145.766,-45.155],[-33.570,145.965,-45.810],[-33.623,146.164,-46.466],[-33.676,146.362,-47.123],[-33.730,146.562,-47.780],[-33.783,146.765,-48.438],[-33.836,146.974,-49.097],[-33.889,147.190,-49.756],[-33.943,147.407,-50.415],[-33.996,147.618,-51.074],[-34.049,147.816,-51.733],[-34.102,147.998,-52.393],[-34.129,148.170,-52.715]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-34.129,148.170,-52.715],[-33.619,148.388,-53.314],[-33.181,148.608,-53.829],[-32.820,148.832,-54.254],[-32.406,149.068,-54.741],[-31.941,149.318,-55.287],[-31.428,149.580,-55.891],[-30.867,149.841,-56.550],[-30.262,150.085,-57.262],[-29.944,150.299,-57.636],[-29.615,150.481,-58.023],[-29.275,150.637,-58.422],[-28.926,150.779,-58.832],[-28.568,150.914,-59.254],[-28.200,151.045,-59.687],[-27.823,151.169,-60.130],[-27.437,151.284,-60.584],[-27.043,151.389,-61.047],[-26.640,151.486,-61.521],[-26.229,151.580,-62.004],[-25.811,151.675,-62.496],[-25.385,151.772,-62.997],[-24.952,151.872,-63.507],[-24.511,151.973,-64.024],[-24.064,152.074,-64.550],[-23.611,152.175,-65.083],[-23.151,152.275,-65.624],[-22.685,152.377,-66.172],[-22.214,152.481,-66.726],[-21.737,152.587,-67.287],[-21.255,152.694,-67.854],[-20.768,152.797,-68.427],[-20.276,152.889,-69.005],[-19.780,152.963,-69.588],[-19.280,153.014,-70.176],[-18.776,153.042,-70.769],[-18.268,153.053,-71.366],[-17.757,153.056,-71.968],[-17.242,153.057,-72.572],[-16.725,153.062,-73.180],[-16.206,153.072,-73.792],[-15.684,153.088,-74.405],[-15.160,153.106,-75.022],[-14.634,153.126,-75.640],[-14.106,153.150,-76.261],[-13.577,153.176,-76.882],[-13.047,153.208,-77.505],[-12.517,153.244,-78.129],[-11.986,153.281,-78.754],[-11.454,153.315,-79.379],[-10.923,153.337,-80.004],[-10.392,153.342,-80.628],[-9.861,153.327,-81.252],[-9.331,153.291,-81.875],[-8.803,153.241,-82.497],[-8.275,153.182,-83.118],[-7.749,153.121,-83.736],[-7.225,153.060,-84.352],[-6.703,153.001,-84.966],[-6.183,152.943,-85.577],[-5.666,152.884,-86.186],[-5.152,152.824,-86.790],[-4.641,152.763,-87.391],[-4.133,152.700,-87.988],[-3.629,152.637,-88.581],[-3.129,152.577,-89.170],[-2.633,152.521,-89.753],[-2.141,152.471,-90.331],[-1.654,152.427,-90.904],[-1.172,152.388,-91.471],[-0.695,152.353,-92.032],[-0.224,152.321,-92.586],[0.242,152.291,-93.134],[0.702,152.262,-93.674],[1.155,152.234,-94.208],[1.603,152.207,-94.733],[2.043,152.182,-95.251],[2.476,152.157,-95.761],[2.902,152.134,-96.262],[3.321,152.111,-96.754],[3.731,152.089,-97.237],[4.134,152.067,-97.710],[4.528,152.046,-98.174],[4.914,152.025,-98.628],[5.291,152.007,-99.071],[5.659,151.991,-99.504],[6.018,151.979,-99.925],[6.367,151.970,-100.336],[6.706,151.965,-100.734],[7.035,151.962,-101.121],[7.661,151.960,-101.858],[8.244,151.957,-102.544],[8.782,151.954,-103.175],[9.271,151.950,-103.751],[9.711,151.944,-104.268],[10.099,151.936,-104.724],[10.433,151.926,-105.117],[11.220,151.915,-106.042]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-34.129,148.170,-52.715],[-34.156,148.341,-53.052],[-34.209,148.523,-53.712],[-34.262,148.722,-54.371],[-34.316,148.937,-55.029],[-34.369,149.163,-55.688],[-34.422,149.394,-56.346],[-34.475,149.626,-57.003],[-34.528,149.856,-57.660],[-34.581,150.080,-58.316],[-34.634,150.296,-58.972],[-34.687,150.498,-59.626],[-34.740,150.683,-60.280],[-34.793,150.851,-60.933],[-34.845,151.004,-61.584],[-34.898,151.149,-62.235],[-34.950,151.291,-62.884],[-35.003,151.434,-63.532],[-35.055,151.579,-64.178],[-35.107,151.726,-64.823],[-35.159,151.873,-65.466],[-35.211,152.022,-66.108],[-35.263,152.172,-66.748],[-35.314,152.325,-67.386],[-35.366,152.477,-68.022],[-35.417,152.625,-68.657],[-35.468,152.761,-69.289],[-35.519,152.880,-69.919],[-35.570,152.978,-70.547],[-35.621,153.059,-71.172],[-35.671,153.126,-71.796],[-35.721,153.188,-72.416],[-35.771,153.249,-73.035],[-35.821,153.313,-73.650],[-35.870,153.380,-74.263],[-35.920,153.448,-74.873],[-35.969,153.518,-75.481],[-36.018,153.587,-76.085],[-36.066,153.658,-76.686],[-36.115,153.729,-77.285],[-36.163,153.801,-77.880],[-36.211,153.873,-78.471],[-36.258,153.941,-79.060],[-36.305,154.005,-79.645],[-36.352,154.061,-80.226],[-36.399,154.109,-80.804],[-36.446,154.151,-81.379],[-36.492,154.189,-81.949],[-36.537,154.226,-82.516],[-36.583,154.263,-83.079],[-36.628,154.300,-83.637],[-36.673,154.338,-84.192],[-36.717,154.376,-84.743],[-36.762,154.414,-85.289],[-36.805,154.451,-85.831],[-36.849,154.489,-86.369],[-36.892,154.526,-86.902],[-36.935,154.562,-87.431],[-36.977,154.598,-87.955],[-37.019,154.633,-88.474],[-37.061,154.667,-88.989],[-37.102,154.700,-89.498],[-37.143,154.733,-90.003],[-37.183,154.769,-90.503],[-37.243,154.809,-91.243],[-37.321,154.852,-92.211],[-37.398,154.899,-93.158],[-37.472,154.947,-94.082],[-37.545,154.994,-94.984],[-37.616,155.036,-95.862],[-37.685,155.074,-96.716],[-37.752,155.110,-97.545],[-37.817,155.149,-98.348],[-37.880,155.196,-99.126],[-37.941,155.257,-99.877],[-37.999,155.334,-100.600],[-38.056,155.423,-101.296],[-38.109,155.519,-101.963],[-38.161,155.616,-102.600],[-38.210,155.708,-103.208],[-38.257,155.798,-103.785],[-38.301,155.886,-104.331],[-38.342,155.978,-104.844],[-38.400,156.074,-105.554],[-38.467,156.175,-106.383],[-38.522,156.277,-107.072],[-38.567,156.379,-107.617],[-38.625,156.481,-108.336]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let south = graph.add_node(edge2_points.last().copied().unwrap(), NodeType::Junction);
    let center = graph.add_node(edge0_points.last().copied().unwrap(), NodeType::Junction);
    let branch = graph.add_node(edge1_points.last().copied().unwrap(), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        center,
        edge0_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        branch,
        edge1_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        edge2_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    assert!(
        graph.edge(0).end_clip > 0.0,
        "latest elevated edge 0 must be clipped into the junction; clip={:.3}",
        graph.edge(0).end_clip
    );
    assert!(
        graph.edge(1).start_clip > 0.0,
        "latest elevated edge 1 must be clipped into the junction; clip={:.3}",
        graph.edge(1).start_clip
    );
    assert!(
        graph.edge(2).start_clip > 0.0,
        "latest elevated edge 2 must be clipped into the junction; clip={:.3}",
        graph.edge(2).start_clip
    );

    let mut edit_path_main_geometry = edge0_points.clone();
    edit_path_main_geometry.extend(edge2_points.iter().skip(1).copied());

    let mut stale_main_geometry = edge0_points;
    stale_main_geometry.extend(edge2_points.iter().skip(1).copied());
    let mut stale_graph = RegionGraph::new();
    let stale_west = stale_graph.add_node(graph.node(west).pos, NodeType::Junction);
    let stale_south = stale_graph.add_node(graph.node(south).pos, NodeType::Junction);
    stale_graph.add_edge(test_edge(
        stale_west,
        stale_south,
        stale_main_geometry,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    stale_graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&stale_graph, &terrain);
    for edge_idx in 0..graph.edge_count() {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    for node_id in [west, south, center, branch] {
        surface.mark_node_dirty(&graph, node_id);
    }
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "latest elevated oblique 3-way JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }

    let mut edit_graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let config = crate::simulation::core::config::WorldConfig::default();
    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&config);
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    network.add_road(
        &mut edit_graph,
        edit_path_main_geometry,
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&edit_graph, &terrain);
    network.add_road(
        &mut edit_graph,
        edge1_points,
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&edit_graph, &terrain);

    let edit_center = (0..edit_graph.node_count() as u32)
        .find(|&node_id| {
            edit_graph
                .node_adjacency(node_id)
                .iter()
                .filter(|&&edge_idx| !edit_graph.edge(edge_idx).deleted)
                .count()
                == 3
        })
        .expect("add_road edit path must create a 3-way junction node");
    if !network
        .road_surface
        .compiled_visual_node_pieces()
        .contains_key(&edit_center)
    {
        panic!(
            "add_road elevated oblique JunctionN did not compile after endpoint profile solve: {}",
            canonical_junction_pipeline_report(&network.road_surface, &edit_graph, edit_center)
        );
    }
}

#[test]
fn logged_regenerated_elevated_three_way_rejects_same_material_height_conflict() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-11.903,142.295,-17.011],[-12.021,142.386,-17.571],[-12.165,142.477,-18.25],[-12.29,142.566,-18.841],[-12.438,142.65,-19.539],[-12.607,142.724,-20.34],[-12.797,142.785,-21.238],[-12.953,142.832,-21.974],[-13.063,142.87,-22.493],[-13.177,142.902,-23.035],[-13.296,142.933,-23.598],[-13.42,142.967,-24.183],[-13.548,143.005,-24.788],[-13.68,143.046,-25.414],[-13.817,143.087,-26.06],[-13.958,143.129,-26.725],[-14.102,143.17,-27.409],[-14.251,143.216,-28.111],[-14.403,143.272,-28.83],[-14.559,143.344,-29.568],[-14.718,143.437,-30.322],[-14.881,143.553,-31.092],[-15.047,143.687,-31.878],[-15.217,143.835,-32.679],[-15.389,143.989,-33.495],[-15.565,144.145,-34.326],[-15.744,144.299,-35.17],[-15.925,144.452,-36.027],[-16.109,144.607,-36.898],[-16.296,144.77,-37.78],[-16.485,144.95,-38.675],[-16.676,145.151,-39.58],[-16.87,145.372,-40.497],[-17.066,145.606,-41.424],[-17.264,145.835,-42.36],[-17.464,146.045,-43.306],[-17.565,146.226,-43.782],[-17.666,146.377,-44.26],[-17.768,146.507,-44.741],[-17.87,146.627,-45.223],[-17.972,146.747,-45.707],[-18.075,146.871,-46.194],[-18.178,147.0,-46.682],[-18.282,147.131,-47.172],[-18.386,147.262,-47.663],[-18.49,147.393,-48.156],[-18.595,147.523,-48.651],[-18.7,147.656,-49.147],[-18.805,147.794,-49.645],[-18.911,147.939,-50.144],[-19.016,148.092,-50.644],[-19.122,148.251,-51.146],[-19.229,148.415,-51.648],[-19.335,148.581,-52.152],[-19.442,148.748,-52.657],[-19.549,148.915,-53.163],[-19.656,149.083,-53.67],[-19.764,149.251,-54.178],[-19.871,149.419,-54.687],[-19.979,149.586,-55.196],[-20.087,149.752,-55.706],[-20.195,149.918,-56.217],[-20.303,150.085,-56.728],[-20.411,150.255,-57.24],[-20.52,150.429,-57.752],[-20.628,150.605,-58.264],[-20.737,150.775,-58.777],[-20.845,150.93,-59.29],[-20.954,151.065,-59.804],[-21.062,151.178,-60.317],[-21.126,151.278,-60.618]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-21.126,151.278,-60.618],[-20.467,151.293,-60.757],[-19.675,151.303,-60.925],[-19.173,151.305,-61.031],[-18.603,151.298,-61.151],[-17.97,151.285,-61.285],[-17.274,151.268,-61.432],[-16.52,151.249,-61.592],[-15.708,151.23,-61.763],[-14.844,151.212,-61.946],[-13.928,151.196,-62.139],[-12.963,151.181,-62.343],[-12.464,151.169,-62.449],[-11.953,151.159,-62.556],[-11.432,151.151,-62.666],[-10.9,151.14,-62.779],[-10.359,151.126,-62.893],[-9.807,151.106,-63.009],[-9.246,151.08,-63.128],[-8.676,151.049,-63.248],[-8.097,151.013,-63.37],[-7.51,150.976,-63.494],[-6.915,150.937,-63.619],[-6.312,150.898,-63.747],[-5.702,150.858,-63.875],[-5.084,150.817,-64.006],[-4.46,150.775,-64.137],[-3.83,150.732,-64.27],[-3.193,150.688,-64.404],[-2.551,150.643,-64.54],[-1.903,150.596,-64.676],[-1.251,150.547,-64.814],[-0.593,150.496,-64.953],[0.068,150.442,-65.092],[0.734,150.384,-65.233],[1.404,150.323,-65.374],[2.076,150.259,-65.516],[2.752,150.195,-65.658],[3.431,150.129,-65.801],[4.112,150.064,-65.945],[4.795,149.997,-66.089],[5.479,149.93,-66.233],[6.165,149.862,-66.378],[6.852,149.792,-66.523],[7.54,149.722,-66.668],[8.228,149.651,-66.813],[8.916,149.582,-66.958],[9.603,149.514,-67.103],[10.29,149.45,-67.247],[10.976,149.39,-67.392],[11.661,149.332,-67.536],[12.344,149.276,-67.68],[13.024,149.22,-67.824],[13.703,149.164,-67.967],[14.379,149.108,-68.11],[15.052,149.052,-68.251],[15.721,148.995,-68.393],[16.387,148.937,-68.533],[17.049,148.879,-68.672],[17.706,148.82,-68.811],[18.359,148.762,-68.949],[19.006,148.708,-69.085],[19.649,148.659,-69.221],[20.285,148.618,-69.355],[20.916,148.583,-69.488],[21.54,148.554,-69.62],[22.157,148.528,-69.75],[22.767,148.502,-69.879],[23.37,148.476,-70.006],[23.966,148.447,-70.131],[24.553,148.418,-70.255],[25.131,148.388,-70.377],[25.701,148.359,-70.498],[26.262,148.331,-70.616],[26.814,148.303,-70.732],[27.356,148.276,-70.847],[27.887,148.25,-70.959],[28.409,148.223,-71.069],[28.919,148.194,-71.177],[29.419,148.164,-71.282],[30.383,148.132,-71.486],[31.299,148.1,-71.679],[32.164,148.068,-71.862],[32.975,148.039,-72.033],[33.729,148.013,-72.193],[34.425,147.989,-72.34],[35.059,147.966,-72.474],[35.628,147.943,-72.594],[36.13,147.92,-72.7],[36.922,147.895,-72.868],[37.581,147.869,-73.007]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-21.126,151.278,-60.618],[-21.171,151.349,-60.831],[-21.279,151.427,-61.344],[-21.388,151.514,-61.858],[-21.497,151.61,-62.371],[-21.605,151.712,-62.884],[-21.714,151.817,-63.397],[-21.822,151.921,-63.91],[-21.93,152.024,-64.422],[-22.039,152.125,-64.934],[-22.147,152.226,-65.445],[-22.255,152.327,-65.955],[-22.363,152.429,-66.465],[-22.47,152.532,-66.975],[-22.578,152.638,-67.483],[-22.685,152.746,-67.991],[-22.792,152.854,-68.498],[-22.899,152.957,-69.004],[-23.006,153.049,-69.509],[-23.113,153.123,-70.013],[-23.219,153.178,-70.516],[-23.325,153.216,-71.017],[-23.431,153.243,-71.518],[-23.537,153.264,-72.017],[-23.642,153.285,-72.514],[-23.747,153.308,-73.011],[-23.851,153.333,-73.505],[-23.956,153.36,-73.998],[-24.06,153.388,-74.49],[-24.163,153.421,-74.98],[-24.318,153.458,-75.711],[-24.472,153.501,-76.438],[-24.675,153.549,-77.401],[-24.877,153.602,-78.356],[-25.077,153.659,-79.302],[-25.275,153.718,-80.238],[-25.471,153.778,-81.165],[-25.665,153.84,-82.081],[-25.857,153.902,-82.987],[-26.046,153.964,-83.881],[-26.233,154.025,-84.764],[-26.417,154.086,-85.634],[-26.598,154.145,-86.492],[-26.777,154.204,-87.336],[-26.952,154.262,-88.166],[-27.125,154.321,-88.982],[-27.294,154.38,-89.784],[-27.461,154.44,-90.57],[-27.623,154.5,-91.34],[-27.783,154.559,-92.094],[-27.939,154.617,-92.831],[-28.091,154.674,-93.551],[-28.24,154.729,-94.253],[-28.384,154.782,-94.937],[-28.525,154.833,-95.602],[-28.661,154.882,-96.247],[-28.794,154.928,-96.873],[-28.922,154.969,-97.479],[-29.045,155.007,-98.063],[-29.165,155.043,-98.627],[-29.279,155.084,-99.168],[-29.389,155.136,-99.687],[-29.494,155.202,-100.184],[-29.642,155.284,-100.885],[-29.822,155.38,-101.735],[-29.98,155.484,-102.484],[-30.117,155.593,-103.129],[-30.23,155.703,-103.666],[-30.439,155.813,-104.65]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let center = graph.add_node(*edge0_points.last().unwrap(), NodeType::Junction);
    let east = graph.add_node(*edge1_points.last().unwrap(), NodeType::Junction);
    let south = graph.add_node(*edge2_points.last().unwrap(), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        edge0_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        edge1_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        edge2_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();
    assert!(
        graph.edge(0).end_clip > 0.0,
        "regenerated elevated edge 0 must clip into the JunctionN; clip={:.3}",
        graph.edge(0).end_clip
    );
    assert!(
        graph.edge(1).start_clip > 0.0,
        "regenerated elevated edge 1 must clip into the JunctionN; clip={:.3}",
        graph.edge(1).start_clip
    );
    assert!(
        graph.edge(2).start_clip > 0.0,
        "regenerated elevated edge 2 must clip into the JunctionN; clip={:.3}",
        graph.edge(2).start_clip
    );

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "regenerated elevated JunctionN",
    );
}

#[test]
fn logged_current_elevated_three_way_rejects_same_material_height_conflict() {
    let terrain = TerrainSystem::with_chunking(1025, 1025, 1.0, 512, 0.0);
    let edge0_points = road_points_from_json(
        r#"[[-36.17,139.833,-5.769],[-36.277,139.832,-6.399],[-36.406,139.83,-7.164],[-36.518,139.829,-7.83],[-36.651,139.83,-8.615],[-36.803,139.834,-9.516],[-36.93,139.842,-10.264],[-37.02,139.854,-10.796],[-37.114,139.87,-11.355],[-37.213,139.888,-11.94],[-37.316,139.907,-12.549],[-37.423,139.926,-13.183],[-37.534,139.945,-13.841],[-37.649,139.963,-14.523],[-37.768,139.979,-15.227],[-37.891,139.992,-15.954],[-38.017,140.002,-16.702],[-38.147,140.01,-17.472],[-38.28,140.023,-18.262],[-38.417,140.046,-19.072],[-38.557,140.088,-19.902],[-38.701,140.15,-20.751],[-38.847,140.233,-21.617],[-38.997,140.331,-22.502],[-39.149,140.44,-23.404],[-39.304,140.552,-24.323],[-39.462,140.661,-25.257],[-39.622,140.762,-26.208],[-39.785,140.851,-27.173],[-39.909,140.926,-27.906],[-39.992,140.989,-28.399],[-40.076,141.046,-28.896],[-40.161,141.107,-29.396],[-40.246,141.179,-29.899],[-40.331,141.266,-30.406],[-40.417,141.37,-30.915],[-40.504,141.485,-31.428],[-40.591,141.606,-31.944],[-40.679,141.728,-32.463],[-40.767,141.849,-32.984],[-40.855,141.969,-33.508],[-40.944,142.088,-34.035],[-41.034,142.207,-34.565],[-41.124,142.326,-35.097],[-41.214,142.447,-35.632],[-41.305,142.568,-36.169],[-41.396,142.689,-36.709],[-41.487,142.809,-37.251],[-41.579,142.927,-37.795],[-41.672,143.046,-38.341],[-41.764,143.168,-38.889],[-41.857,143.297,-39.44],[-41.95,143.435,-39.992],[-42.044,143.584,-40.546],[-42.138,143.744,-41.102],[-42.232,143.91,-41.66],[-42.326,144.079,-42.219],[-42.421,144.249,-42.78],[-42.516,144.419,-43.342],[-42.611,144.587,-43.906],[-42.707,144.755,-44.471],[-42.803,144.923,-45.038],[-42.898,145.091,-45.605],[-42.995,145.26,-46.174],[-43.091,145.429,-46.744],[-43.187,145.598,-47.316],[-43.284,145.767,-47.888],[-43.381,145.936,-48.46],[-43.478,146.106,-49.034],[-43.575,146.279,-49.609],[-43.672,146.456,-50.184],[-43.769,146.638,-50.759],[-43.866,146.825,-51.336],[-43.964,147.016,-51.912],[-44.061,147.204,-52.489],[-44.159,147.386,-53.067],[-44.256,147.555,-53.644],[-44.354,147.713,-54.222],[-44.411,147.862,-54.564]]"#,
    );
    let edge1_points = road_points_from_json(
        r#"[[-44.411,147.862,-54.564],[-43.727,147.963,-54.68],[-43.099,148.062,-54.786],[-42.291,148.157,-54.922],[-41.575,148.248,-55.043],[-41.049,148.335,-55.132],[-40.486,148.421,-55.227],[-39.887,148.509,-55.328],[-39.255,148.602,-55.435],[-38.592,148.701,-55.547],[-37.899,148.809,-55.664],[-37.177,148.922,-55.786],[-36.43,149.041,-55.912],[-35.658,149.164,-56.043],[-34.864,149.294,-56.177],[-34.049,149.429,-56.314],[-33.215,149.571,-56.455],[-32.363,149.714,-56.599],[-31.497,149.852,-56.746],[-30.617,149.978,-56.894],[-29.725,150.086,-57.045],[-28.823,150.174,-57.197],[-27.913,150.244,-57.351],[-26.997,150.303,-57.506],[-26.076,150.358,-57.661],[-25.153,150.414,-57.817],[-24.229,150.473,-57.973],[-23.305,150.535,-58.129],[-22.385,150.595,-58.285],[-21.468,150.649,-58.44],[-20.558,150.692,-58.593],[-19.657,150.721,-58.746],[-18.765,150.735,-58.896],[-17.885,150.738,-59.045],[-17.018,150.732,-59.191],[-16.167,150.724,-59.335],[-15.333,150.715,-59.476],[-14.518,150.707,-59.614],[-13.724,150.698,-59.748],[-12.952,150.688,-59.878],[-12.204,150.676,-60.005],[-11.483,150.659,-60.126],[-10.79,150.64,-60.243],[-10.126,150.616,-60.356],[-9.495,150.59,-60.462],[-8.896,150.56,-60.563],[-8.333,150.527,-60.658],[-7.807,150.491,-60.747],[-7.091,150.453,-60.868],[-6.283,150.413,-61.005],[-5.655,150.371,-61.111],[-4.97,150.329,-61.226]]"#,
    );
    let edge2_points = road_points_from_json(
        r#"[[-44.411,147.862,-54.564],[-44.451,147.995,-54.8],[-44.549,148.139,-55.378],[-44.647,148.3,-55.956],[-44.744,148.48,-56.534],[-44.842,148.673,-57.112],[-44.939,148.87,-57.689],[-45.037,149.067,-58.266],[-45.134,149.257,-58.843],[-45.231,149.436,-59.419],[-45.329,149.602,-59.995],[-45.426,149.755,-60.57],[-45.523,149.896,-61.144],[-45.62,150.028,-61.718],[-45.716,150.157,-62.291],[-45.813,150.285,-62.863],[-45.91,150.414,-63.434],[-46.006,150.545,-64.004],[-46.102,150.676,-64.573],[-46.198,150.807,-65.141],[-46.293,150.937,-65.707],[-46.389,151.067,-66.272],[-46.484,151.196,-66.836],[-46.579,151.324,-67.399],[-46.674,151.453,-67.959],[-46.768,151.58,-68.519],[-46.862,151.707,-69.076],[-46.956,151.831,-69.632],[-47.05,151.954,-70.186],[-47.143,152.075,-70.739],[-47.236,152.195,-71.289],[-47.329,152.314,-71.837],[-47.421,152.434,-72.383],[-47.513,152.555,-72.928],[-47.604,152.676,-73.469],[-47.696,152.799,-74.009],[-47.786,152.922,-74.546],[-47.877,153.044,-75.081],[-47.966,153.167,-75.613],[-48.056,153.29,-76.143],[-48.145,153.415,-76.67],[-48.233,153.542,-77.194],[-48.322,153.672,-77.716],[-48.409,153.803,-78.234],[-48.496,153.932,-78.75],[-48.583,154.051,-79.263],[-48.669,154.154,-79.772],[-48.755,154.237,-80.279],[-48.84,154.299,-80.782],[-48.924,154.349,-81.282],[-49.008,154.395,-81.779],[-49.091,154.447,-82.272],[-49.215,154.512,-83.006],[-49.378,154.589,-83.971],[-49.538,154.677,-84.921],[-49.696,154.769,-85.856],[-49.851,154.863,-86.774],[-50.004,154.955,-87.676],[-50.153,155.046,-88.561],[-50.3,155.137,-89.428],[-50.443,155.229,-90.276],[-50.583,155.322,-91.106],[-50.72,155.415,-91.916],[-50.853,155.509,-92.706],[-50.983,155.6,-93.476],[-51.11,155.69,-94.224],[-51.232,155.777,-94.951],[-51.351,155.862,-95.655],[-51.467,155.945,-96.337],[-51.578,156.026,-96.995],[-51.685,156.104,-97.629],[-51.788,156.179,-98.239],[-51.886,156.25,-98.823],[-51.981,156.317,-99.382],[-52.071,156.381,-99.914],[-52.156,156.443,-100.42],[-52.276,156.507,-101.127],[-52.418,156.573,-101.971],[-52.541,156.643,-102.697],[-52.643,156.717,-103.301],[-52.83,156.793,-104.409]]"#,
    );

    let mut graph = RegionGraph::new();
    let west = graph.add_node(edge0_points[0], NodeType::Junction);
    let center = graph.add_node(*edge0_points.last().unwrap(), NodeType::Junction);
    let east = graph.add_node(*edge1_points.last().unwrap(), NodeType::Junction);
    let south = graph.add_node(*edge2_points.last().unwrap(), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        edge0_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        edge1_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        south,
        edge2_points.clone(),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.solve_junction_endpoint_profiles_for_edges(
        &HashSet::from([center]),
        &HashSet::from([0, 1, 2]),
    );
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert_junction_mouth_section_profile_matches_endpoint_plane(&surface, &graph, 0, false);
    assert_junction_mouth_section_profile_matches_endpoint_plane(&surface, &graph, 1, true);
    assert_junction_mouth_section_profile_matches_endpoint_plane(&surface, &graph, 2, true);
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "current elevated JunctionN",
    );

    let mut edit_graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let config = crate::simulation::core::config::WorldConfig::default();
    let mut zoning = crate::simulation::grid::zoning::ZoningSystem::new(&config);
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    for points in [edge0_points, edge1_points, edge2_points] {
        network.add_road(
            &mut edit_graph,
            points,
            1,
            1,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        network.road_surface.compile_dirty(&edit_graph, &terrain);
    }
    let edit_center = (0..edit_graph.node_count() as u32)
        .find(|&node_id| {
            edit_graph
                .node_adjacency(node_id)
                .iter()
                .filter(|&&edge_idx| !edit_graph.edge(edge_idx).deleted)
                .count()
                == 3
        })
        .expect("add_road edit path must create the elevated 3-way junction node");
    assert_junction_rejected_with_canonical_height_diagnostic(
        &network.road_surface,
        &edit_graph,
        edit_center,
        "add_road current elevated JunctionN",
    );
}

#[test]
fn logged_flat_oblique_t_junction_compiles_with_explicit_curb_sidewalk_endpoint_authority() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-140.162, 0.0, -60.230), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-75.827, 0.0, 89.838), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-57.710, 0.0, 22.223), NodeType::Junction);
    let east = graph.add_node(Vector3::new(50.757, 0.0, 130.689), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-140.162, 0.0, -60.230),
            Vector3::new(-57.710, 0.0, 22.223),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-57.710, 0.0, 22.223),
            Vector3::new(-75.827, 0.0, 89.838),
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
            Vector3::new(-57.710, 0.0, 22.223),
            Vector3::new(50.757, 0.0, 130.689),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(128, 128);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = surface
        .compiled_visual_node_pieces()
        .get(&center)
        .unwrap_or_else(|| {
            panic!(
                "logged flat oblique T must compile with explicit curb/sidewalk endpoint path: {}",
                canonical_junction_pipeline_report(&surface, &graph, center)
            )
        });
    assert_top_raised_step_owner_boundaries_have_vertical_faces(piece);
    assert_canonical_explicit_vertical_steps_have_faces(piece);
}

#[test]
fn logged_flat_oblique_four_way_compiles_with_explicit_height_carriers() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-168.693, 0.0, 22.598), NodeType::Junction);
    let east = graph.add_node(Vector3::new(-9.454, 0.0, 18.003), NodeType::Junction);
    let center = graph.add_node(Vector3::new(-125.850, 0.0, 21.362), NodeType::Junction);
    let north = graph.add_node(Vector3::new(-83.868, 0.0, 89.461), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-143.870, 0.0, -84.460), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-168.693, 0.0, 22.598),
            Vector3::new(-125.850, 0.0, 21.362),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        north,
        vec![
            Vector3::new(-125.850, 0.0, 21.362),
            Vector3::new(-83.868, 0.0, 89.461),
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
            Vector3::new(-125.850, 0.0, 21.362),
            Vector3::new(-9.454, 0.0, 18.003),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        south,
        center,
        vec![
            Vector3::new(-143.870, 0.0, -84.460),
            Vector3::new(-125.850, 0.0, 21.362),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(512, 512);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn arbitrary_six_way_junction_compiles_with_explicit_height_carriers() {
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [0.0_f32, 23.0, 61.0, 137.0, 211.0, 304.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 96.0, 0.0, angle.sin() * 96.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(192, 192);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "arbitrary six-way JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn arbitrary_five_way_junction_compiles_with_explicit_height_carriers() {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(2.668, 0.0, 10.799);
    let center = graph.add_node(center_pos, NodeType::Junction);
    for endpoint in [
        Vector3::new(-58.540, 0.0, 6.220),
        Vector3::new(115.507, 0.0, 19.240),
        Vector3::new(96.186, 0.0, 60.070),
        Vector3::new(35.647, 0.0, -130.899),
        Vector3::new(-27.212, 0.0, 50.632),
    ] {
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![center_pos, endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn dirty_node_recompile_refreshes_incident_span_sections_for_new_junction() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let left_edge = graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    let up_edge = graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, up);
    surface.mark_edge_dirty(&graph, up_edge);
    surface.compile_dirty(&graph, &terrain);

    let edge = graph.edge(left_edge);
    let total_length: f32 = edge
        .geometry
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum();
    let start_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.start_node),
    );
    let end_kind = surface.classify_surface_node_kind_from_graph_geometry(
        &graph,
        graph.get_valid_node(edge.end_node),
    );
    let (_, expected_handoff_s) = surface
        .visual_surface_handoff_range_for_edge(
            &graph,
            left_edge,
            edge,
            total_length,
            start_kind,
            end_kind,
        )
        .expect("left edge should have a visible span range after pairwise handoff");
    let local_handoff_s = RoadSurfaceSystem::visual_end_handoff_s_m(edge, total_length);
    assert!(
        expected_handoff_s < local_handoff_s - SAMPLE_EPSILON_M,
        "pairwise node ownership must extend the visual handoff before the old local limit"
    );
    let sections = surface.compiled_sections().get(&left_edge).unwrap();
    assert!(
        sections
            .iter()
            .any(|section| (section.s_m - expected_handoff_s).abs() <= SAMPLE_EPSILON_M),
        "dirty node recompilation must refresh incident span sections at the new visual handoff; expected_s={expected_handoff_s:.3} sections={:?}",
        sections
            .iter()
            .map(|section| section.s_m)
            .collect::<Vec<_>>()
    );
}

#[test]
fn dirty_recompile_expanded_arbitrary_node_piece_compiles_with_explicit_height_carriers() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    for angle_degrees in [35.0_f32, 158.0, 276.0] {
        let angle = angle_degrees.to_radians();
        let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
        let node = graph.add_node(endpoint, NodeType::Junction);
        graph.add_edge(test_edge(
            center,
            node,
            vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
            7.0,
            EdgeClass::Standard,
            TransitType::Road,
            TransitFlags::CAR | TransitFlags::FOOT,
        ));
    }
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(4.0);
    surface.compile_dirty(&graph, &terrain);

    let angle = 318.0_f32.to_radians();
    let endpoint = Vector3::new(angle.cos() * 88.0, 0.0, angle.sin() * 88.0);
    let new_node = graph.add_node(endpoint, NodeType::Junction);
    let new_edge = graph.add_edge(test_edge(
        center,
        new_node,
        vec![Vector3::new(0.0, 0.0, 0.0), endpoint],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    surface.mark_node_dirty(&graph, center);
    surface.mark_node_dirty(&graph, new_node);
    for &edge_idx in graph.node_adjacency(center) {
        surface.mark_edge_dirty(&graph, edge_idx);
    }
    surface.mark_edge_dirty(&graph, new_edge);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "expanded arbitrary JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn dirty_recompile_removes_node_from_old_chunks_after_topology_shrink() {
    let terrain = flat_terrain(192, 192);
    let mut graph = RegionGraph::new();
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let west = graph.add_node(Vector3::new(-64.0, 0.0, 0.0), NodeType::Junction);
    let east = graph.add_node(Vector3::new(64.0, 0.0, 0.0), NodeType::Junction);
    let north = graph.add_node(Vector3::new(0.0, 0.0, 64.0), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![Vector3::new(-64.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(64.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    let removed_edge = graph.add_edge(test_edge(
        center,
        north,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 64.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(2.0);
    surface.compile_dirty(&graph, &terrain);
    let old_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .expect("three-way node must own chunks before shrink")
        .clone();
    assert!(
        old_node_chunks.len() > 1,
        "test requires node coverage wide enough to prove stale chunk removal"
    );

    graph.edges[removed_edge].deleted = true;
    graph.rebuild_adjacency_list();
    graph.rebuild_intersection_clips();
    surface.mark_edge_dirty(&graph, removed_edge);
    surface.mark_node_dirty(&graph, center);
    surface.compile_dirty(&graph, &terrain);

    let new_node_chunks = surface
        .surface_node_chunks
        .get(&center)
        .cloned()
        .unwrap_or_default();
    let removed_chunks: Vec<SurfaceChunkKey> = old_node_chunks
        .into_iter()
        .filter(|chunk| !new_node_chunks.contains(chunk))
        .collect();
    assert!(
        !removed_chunks.is_empty(),
        "topology shrink must remove at least one old node-owned chunk"
    );
    for chunk in removed_chunks {
        if let Some(entry) = surface.surface_chunk_cache.get(&chunk) {
            assert!(
                !entry.node_ids.contains(&center),
                "stale node contributor remained in removed chunk {chunk:?}"
            );
        }
    }
}

#[test]
fn junction_node_non_road_surface_is_footprint_minus_asphalt() {
    let mut graph = RegionGraph::new();
    let left = graph.add_node(Vector3::new(-24.0, 0.0, 0.0), NodeType::Junction);
    let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let right = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let up = graph.add_node(Vector3::new(0.0, 0.0, 24.0), NodeType::Junction);
    graph.add_edge(test_edge(
        left,
        center,
        vec![Vector3::new(-24.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        right,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        up,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 24.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_adjacency_list();

    let terrain = flat_terrain(96, 96);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    assert_compiled_junction_piece(&surface, &graph, center);
}

#[test]
fn elevated_four_way_junction_rejects_same_material_height_conflict_after_endpoint_profile_solve() {
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
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "elevated 4-way JunctionN after endpoint profile solve",
    );
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
    surface.compile_dirty(&graph, &terrain);

    let mut max_mouth_abs_y = 0.0_f32;
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
            "steep JunctionN may compile only when same-XZ side vertices are resolved without hidden height conflicts: {dump}"
        );
    }
}

#[test]
fn elevated_three_way_junction_rejects_same_material_height_conflict_after_endpoint_profile_solve()
{
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
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "elevated 3-way JunctionN after endpoint profile solve",
    );
}

#[test]
fn skewed_elevated_four_way_junction_rejects_same_material_height_conflict() {
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
    assert_junction_rejected_with_canonical_height_diagnostic(
        &surface,
        &graph,
        center,
        "skewed elevated 4-way JunctionN",
    );
}

#[test]
fn node_overlay_preserves_skinny_closure_shapes() {
    let shapes = RoadSurfaceSystem::overlay_union_contours(&[vec![
        [0.0, 0.0],
        [2.0, 0.0],
        [2.0, 0.0005],
        [0.0, 0.0005],
    ]])
    .unwrap();

    assert_eq!(
        shapes.len(),
        1,
        "millimetre-scale deterministic closure slivers must not be filtered before rendering"
    );
}
