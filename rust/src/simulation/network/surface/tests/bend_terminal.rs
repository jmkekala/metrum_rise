//! Bend and terminal node-surface regression tests.

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
fn logged_bend_with_fragmented_asphalt_curb_step_compiles() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-107.559, 0.0, -28.209), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-54.287, 0.0, -22.547), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-16.205, 0.0, 23.182), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-107.559, 0.0, -28.209),
            Vector3::new(-97.788, 0.0, -27.170),
            Vector3::new(-82.795, 0.0, -25.577),
            Vector3::new(-69.410, 0.0, -24.155),
            Vector3::new(-58.119, 0.0, -22.954),
            Vector3::new(-54.287, 0.0, -22.547),
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
            Vector3::new(-54.287, 0.0, -22.547),
            Vector3::new(-53.860, 0.0, -22.034),
            Vector3::new(-52.240, 0.0, -20.089),
            Vector3::new(-49.618, 0.0, -16.940),
            Vector3::new(-45.836, 0.0, -12.398),
            Vector3::new(-40.968, 0.0, -6.553),
            Vector3::new(-35.693, 0.0, -0.218),
            Vector3::new(-30.386, 0.0, 6.154),
            Vector3::new(-25.038, 0.0, 12.576),
            Vector3::new(-20.875, 0.0, 17.575),
            Vector3::new(-16.205, 0.0, 23.182),
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
fn logged_outer_bend_skips_one_sided_curb_step_slivers() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-116.890, 0.0, -31.104), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-53.167, 0.0, -27.526), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-17.253, 0.0, 19.023), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        road_points_from_json(
            "[[-116.89,0.0,-31.104],[-116.174,0.0,-31.064],[-115.314,0.0,-31.015],[-114.769,0.0,-30.985],[-114.152,0.0,-30.95],[-113.464,0.0,-30.912],[-112.709,0.0,-30.869],[-111.889,0.0,-30.823],[-111.009,0.0,-30.774],[-110.07,0.0,-30.721],[-109.33,0.0,-30.679],[-108.819,0.0,-30.651],[-108.296,0.0,-30.621],[-107.76,0.0,-30.591],[-107.211,0.0,-30.561],[-106.651,0.0,-30.529],[-106.08,0.0,-30.497],[-105.497,0.0,-30.464],[-104.904,0.0,-30.431],[-104.3,0.0,-30.397],[-103.686,0.0,-30.363],[-103.063,0.0,-30.328],[-102.43,0.0,-30.292],[-101.788,0.0,-30.256],[-101.138,0.0,-30.22],[-100.479,0.0,-30.183],[-99.813,0.0,-30.145],[-99.139,0.0,-30.107],[-98.458,0.0,-30.069],[-97.771,0.0,-30.03],[-97.077,0.0,-29.991],[-96.377,0.0,-29.952],[-95.671,0.0,-29.913],[-94.96,0.0,-29.873],[-94.244,0.0,-29.832],[-93.523,0.0,-29.792],[-92.799,0.0,-29.751],[-92.07,0.0,-29.71],[-91.338,0.0,-29.669],[-90.603,0.0,-29.628],[-89.865,0.0,-29.587],[-89.125,0.0,-29.545],[-88.383,0.0,-29.503],[-87.639,0.0,-29.462],[-86.894,0.0,-29.42],[-86.148,0.0,-29.378],[-85.402,0.0,-29.336],[-84.655,0.0,-29.294],[-83.908,0.0,-29.252],[-83.162,0.0,-29.21],[-82.417,0.0,-29.168],[-81.673,0.0,-29.127],[-80.931,0.0,-29.085],[-80.191,0.0,-29.043],[-79.453,0.0,-29.002],[-78.718,0.0,-28.961],[-77.986,0.0,-28.92],[-77.258,0.0,-28.879],[-76.533,0.0,-28.838],[-75.813,0.0,-28.798],[-75.097,0.0,-28.757],[-74.386,0.0,-28.718],[-73.68,0.0,-28.678],[-72.98,0.0,-28.639],[-72.286,0.0,-28.6],[-71.598,0.0,-28.561],[-70.917,0.0,-28.523],[-70.243,0.0,-28.485],[-69.577,0.0,-28.448],[-68.919,0.0,-28.411],[-68.268,0.0,-28.374],[-67.627,0.0,-28.338],[-66.994,0.0,-28.302],[-66.37,0.0,-28.267],[-65.756,0.0,-28.233],[-65.153,0.0,-28.199],[-64.559,0.0,-28.166],[-63.977,0.0,-28.133],[-63.405,0.0,-28.101],[-62.845,0.0,-28.07],[-62.297,0.0,-28.039],[-61.761,0.0,-28.009],[-61.237,0.0,-27.979],[-60.727,0.0,-27.951],[-59.986,0.0,-27.909],[-59.047,0.0,-27.856],[-58.167,0.0,-27.807],[-57.348,0.0,-27.761],[-56.593,0.0,-27.719],[-55.905,0.0,-27.68],[-55.287,0.0,-27.645],[-54.742,0.0,-27.615],[-53.882,0.0,-27.566],[-53.167,0.0,-27.526]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        road_points_from_json(
            "[[-53.167,0.0,-27.526],[-52.763,0.0,-27.003],[-52.279,0.0,-26.376],[-51.972,0.0,-25.977],[-51.624,0.0,-25.526],[-51.236,0.0,-25.023],[-50.81,0.0,-24.472],[-50.349,0.0,-23.874],[-49.853,0.0,-23.23],[-49.323,0.0,-22.545],[-48.763,0.0,-21.818],[-48.173,0.0,-21.054],[-47.868,0.0,-20.658],[-47.555,0.0,-20.253],[-47.236,0.0,-19.839],[-46.911,0.0,-19.418],[-46.58,0.0,-18.988],[-46.242,0.0,-18.551],[-45.899,0.0,-18.106],[-45.55,0.0,-17.654],[-45.196,0.0,-17.195],[-44.837,0.0,-16.73],[-44.473,0.0,-16.258],[-44.104,0.0,-15.78],[-43.731,0.0,-15.296],[-43.353,0.0,-14.806],[-42.971,0.0,-14.311],[-42.586,0.0,-13.812],[-42.196,0.0,-13.307],[-41.803,0.0,-12.798],[-41.407,0.0,-12.284],[-41.008,0.0,-11.767],[-40.606,0.0,-11.245],[-40.201,0.0,-10.721],[-39.794,0.0,-10.193],[-39.384,0.0,-9.662],[-38.973,0.0,-9.129],[-38.559,0.0,-8.593],[-38.144,0.0,-8.055],[-37.728,0.0,-7.515],[-37.31,0.0,-6.973],[-36.891,0.0,-6.431],[-36.472,0.0,-5.887],[-36.051,0.0,-5.342],[-35.631,0.0,-4.797],[-35.21,0.0,-4.251],[-34.789,0.0,-3.706],[-34.368,0.0,-3.161],[-33.948,0.0,-2.616],[-33.529,0.0,-2.072],[-33.11,0.0,-1.529],[-32.692,0.0,-0.988],[-32.276,0.0,-0.448],[-31.861,0.0,0.09],[-31.447,0.0,0.626],[-31.036,0.0,1.159],[-30.626,0.0,1.69],[-30.219,0.0,2.218],[-29.814,0.0,2.743],[-29.412,0.0,3.264],[-29.013,0.0,3.781],[-28.616,0.0,4.295],[-28.223,0.0,4.804],[-27.834,0.0,5.309],[-27.448,0.0,5.809],[-27.067,0.0,6.303],[-26.689,0.0,6.793],[-26.316,0.0,7.277],[-25.947,0.0,7.755],[-25.583,0.0,8.227],[-25.223,0.0,8.693],[-24.869,0.0,9.151],[-24.521,0.0,9.603],[-24.178,0.0,10.048],[-23.84,0.0,10.485],[-23.509,0.0,10.915],[-23.183,0.0,11.337],[-22.865,0.0,11.75],[-22.552,0.0,12.155],[-22.247,0.0,12.551],[-21.657,0.0,13.315],[-21.096,0.0,14.042],[-20.567,0.0,14.728],[-20.071,0.0,15.371],[-19.609,0.0,15.969],[-19.184,0.0,16.521],[-18.796,0.0,17.023],[-18.448,0.0,17.475],[-18.141,0.0,17.873],[-17.656,0.0,18.501],[-17.253,0.0,19.023]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("bend should compile through canonical owned regions");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert!(
        visual_polygon_boundary_contains_xz(
            &bend_piece.outer_boundary_loops,
            Vector2::new(-53.814, -20.179),
        ),
        "outer bend terrain cutter must preserve sampled outer span rail points; outer_loops={:?}",
        bend_piece.outer_boundary_loops
    );
}

#[test]
fn logged_curved_terminal_exports_outer_boundary_from_visible_top_support() {
    let terrain = flat_terrain(384, 384);
    let points = road_points_from_json(
        "[[-26.262,0.000,-35.164],[-25.870,0.000,-34.826],[-25.195,0.000,-34.246],[-24.743,0.000,-33.856],[-24.217,0.000,-33.404],[-23.622,0.000,-32.890],[-22.958,0.000,-32.319],[-22.230,0.000,-31.692],[-21.843,0.000,-31.359],[-21.440,0.000,-31.012],[-21.023,0.000,-30.653],[-20.591,0.000,-30.281],[-20.145,0.000,-29.897],[-19.686,0.000,-29.501],[-19.213,0.000,-29.094],[-18.727,0.000,-28.676],[-18.229,0.000,-28.246],[-17.718,0.000,-27.806],[-17.195,0.000,-27.356],[-16.661,0.000,-26.896],[-16.115,0.000,-26.426],[-15.558,0.000,-25.947],[-14.991,0.000,-25.458],[-14.414,0.000,-24.961],[-13.827,0.000,-24.456],[-13.230,0.000,-23.942],[-12.624,0.000,-23.420],[-12.010,0.000,-22.891],[-11.387,0.000,-22.354],[-10.756,0.000,-21.811],[-10.117,0.000,-21.261],[-9.471,0.000,-20.704],[-8.818,0.000,-20.142],[-8.158,0.000,-19.574],[-7.491,0.000,-19.000],[-6.819,0.000,-18.421],[-6.141,0.000,-17.837],[-5.458,0.000,-17.249],[-4.770,0.000,-16.656],[-4.077,0.000,-16.060],[-3.381,0.000,-15.460],[-2.680,0.000,-14.856],[-1.976,0.000,-14.250],[-1.268,0.000,-13.641],[-0.558,0.000,-13.029],[0.155,0.000,-12.416],[0.869,0.000,-11.800],[1.586,0.000,-11.183],[2.304,0.000,-10.565],[3.023,0.000,-9.946],[3.743,0.000,-9.326],[4.463,0.000,-8.706],[5.183,0.000,-8.086],[5.902,0.000,-7.466],[6.621,0.000,-6.847],[7.339,0.000,-6.228],[8.056,0.000,-5.611],[8.771,0.000,-4.996],[9.483,0.000,-4.382],[10.193,0.000,-3.771],[10.901,0.000,-3.161],[11.605,0.000,-2.555],[12.306,0.000,-1.952],[13.003,0.000,-1.351],[13.695,0.000,-0.755],[14.383,0.000,-0.162],[15.066,0.000,0.426],[15.744,0.000,1.010],[16.416,0.000,1.588],[17.083,0.000,2.162],[17.743,0.000,2.730],[18.396,0.000,3.293],[19.042,0.000,3.849],[19.681,0.000,4.400],[20.312,0.000,4.943],[20.935,0.000,5.480],[21.550,0.000,6.009],[22.155,0.000,6.530],[22.752,0.000,7.044],[23.339,0.000,7.550],[23.916,0.000,8.047],[24.483,0.000,8.535],[25.040,0.000,9.015],[25.586,0.000,9.485],[26.120,0.000,9.945],[26.643,0.000,10.395],[27.154,0.000,10.835],[27.652,0.000,11.264],[28.138,0.000,11.683],[28.611,0.000,12.090],[29.070,0.000,12.485],[29.516,0.000,12.869],[29.948,0.000,13.241],[30.365,0.000,13.601],[30.768,0.000,13.947],[31.155,0.000,14.281],[31.883,0.000,14.908],[32.547,0.000,15.479],[33.143,0.000,15.992],[33.668,0.000,16.445],[34.121,0.000,16.834],[34.795,0.000,17.415],[35.187,0.000,17.753]]",
    );
    let mut graph = RegionGraph::new();
    let start = graph.add_node(points[0], NodeType::Junction);
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
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let terminal_piece = surface
        .compiled_visual_node_pieces()
        .get(&end)
        .expect("logged curved terminal should compile");
    assert_eq!(
        terminal_piece.kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
    assert_outer_boundary_vertices_match_visible_top(terminal_piece);
    assert_outer_boundary_vertices_use_visible_top_boundary_support(terminal_piece);
}

#[test]
fn logged_current_bend_keeps_curved_inner_asphalt_curb_steps() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-191.431, 0.0, -105.786), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-118.080, 0.0, -99.065), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-70.293, 0.0, -45.373), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        road_points_from_json(
            "[[-191.431,0.0,-105.786],[-190.608,0.0,-105.711],[-189.899,0.0,-105.646],[-189.315,0.0,-105.592],[-188.646,0.0,-105.531],[-187.894,0.0,-105.462],[-187.063,0.0,-105.386],[-186.156,0.0,-105.303],[-185.429,0.0,-105.236],[-184.922,0.0,-105.190],[-184.398,0.0,-105.142],[-183.858,0.0,-105.092],[-183.301,0.0,-105.041],[-182.729,0.0,-104.989],[-182.141,0.0,-104.935],[-181.539,0.0,-104.880],[-180.922,0.0,-104.823],[-180.291,0.0,-104.766],[-179.646,0.0,-104.706],[-178.988,0.0,-104.646],[-178.318,0.0,-104.585],[-177.635,0.0,-104.522],[-176.940,0.0,-104.458],[-176.233,0.0,-104.394],[-175.515,0.0,-104.328],[-174.787,0.0,-104.261],[-174.048,0.0,-104.194],[-173.300,0.0,-104.125],[-172.542,0.0,-104.055],[-171.775,0.0,-103.985],[-170.999,0.0,-103.914],[-170.215,0.0,-103.842],[-169.424,0.0,-103.770],[-168.625,0.0,-103.697],[-167.819,0.0,-103.623],[-167.007,0.0,-103.548],[-166.188,0.0,-103.473],[-165.364,0.0,-103.398],[-164.535,0.0,-103.322],[-163.701,0.0,-103.245],[-162.862,0.0,-103.169],[-162.019,0.0,-103.091],[-161.173,0.0,-103.014],[-160.324,0.0,-102.936],[-159.472,0.0,-102.858],[-158.618,0.0,-102.780],[-157.761,0.0,-102.701],[-156.904,0.0,-102.623],[-156.045,0.0,-102.544],[-155.186,0.0,-102.465],[-154.326,0.0,-102.386],[-153.467,0.0,-102.308],[-152.608,0.0,-102.229],[-151.750,0.0,-102.150],[-150.894,0.0,-102.072],[-150.040,0.0,-101.994],[-149.188,0.0,-101.916],[-148.339,0.0,-101.838],[-147.492,0.0,-101.760],[-146.650,0.0,-101.683],[-145.811,0.0,-101.606],[-144.977,0.0,-101.530],[-144.148,0.0,-101.454],[-143.324,0.0,-101.378],[-142.505,0.0,-101.303],[-141.693,0.0,-101.229],[-140.887,0.0,-101.155],[-140.088,0.0,-101.082],[-139.297,0.0,-101.009],[-138.513,0.0,-100.937],[-137.737,0.0,-100.866],[-136.970,0.0,-100.796],[-136.212,0.0,-100.727],[-135.464,0.0,-100.658],[-134.725,0.0,-100.590],[-133.996,0.0,-100.524],[-133.279,0.0,-100.458],[-132.572,0.0,-100.393],[-131.877,0.0,-100.329],[-131.194,0.0,-100.267],[-130.523,0.0,-100.205],[-129.865,0.0,-100.145],[-129.221,0.0,-100.086],[-128.590,0.0,-100.028],[-127.973,0.0,-99.972],[-127.370,0.0,-99.917],[-126.783,0.0,-99.863],[-126.210,0.0,-99.810],[-125.654,0.0,-99.759],[-125.114,0.0,-99.710],[-124.590,0.0,-99.662],[-124.083,0.0,-99.615],[-123.356,0.0,-99.549],[-122.449,0.0,-99.466],[-121.618,0.0,-99.389],[-120.866,0.0,-99.321],[-120.197,0.0,-99.259],[-119.612,0.0,-99.206],[-118.904,0.0,-99.141],[-118.080,0.0,-99.065]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        northeast,
        road_points_from_json(
            "[[-118.080,0.0,-99.065],[-117.544,0.0,-98.462],[-117.082,0.0,-97.944],[-116.702,0.0,-97.516],[-116.265,0.0,-97.026],[-115.775,0.0,-96.476],[-115.234,0.0,-95.867],[-114.644,0.0,-95.204],[-114.006,0.0,-94.487],[-113.670,0.0,-94.110],[-113.324,0.0,-93.721],[-112.966,0.0,-93.319],[-112.599,0.0,-92.906],[-112.221,0.0,-92.482],[-111.833,0.0,-92.046],[-111.436,0.0,-91.600],[-111.029,0.0,-91.143],[-110.614,0.0,-90.676],[-110.189,0.0,-90.199],[-109.756,0.0,-89.713],[-109.315,0.0,-89.217],[-108.866,0.0,-88.713],[-108.410,0.0,-88.200],[-107.946,0.0,-87.678],[-107.475,0.0,-87.149],[-106.997,0.0,-86.612],[-106.512,0.0,-86.068],[-106.022,0.0,-85.516],[-105.525,0.0,-84.958],[-105.022,0.0,-84.394],[-104.514,0.0,-83.823],[-104.001,0.0,-83.246],[-103.483,0.0,-82.664],[-102.960,0.0,-82.077],[-102.433,0.0,-81.484],[-101.902,0.0,-80.887],[-101.367,0.0,-80.286],[-100.828,0.0,-79.681],[-100.286,0.0,-79.072],[-99.741,0.0,-78.460],[-99.193,0.0,-77.845],[-98.643,0.0,-77.226],[-98.091,0.0,-76.606],[-97.537,0.0,-75.983],[-96.981,0.0,-75.359],[-96.424,0.0,-74.733],[-95.865,0.0,-74.105],[-95.306,0.0,-73.477],[-94.746,0.0,-72.848],[-94.187,0.0,-72.219],[-93.627,0.0,-71.590],[-93.067,0.0,-70.961],[-92.508,0.0,-70.333],[-91.949,0.0,-69.705],[-91.392,0.0,-69.079],[-90.836,0.0,-68.455],[-90.282,0.0,-67.832],[-89.730,0.0,-67.211],[-89.180,0.0,-66.593],[-88.632,0.0,-65.978],[-88.087,0.0,-65.366],[-87.545,0.0,-64.757],[-87.006,0.0,-64.152],[-86.471,0.0,-63.551],[-85.940,0.0,-62.954],[-85.413,0.0,-62.361],[-84.890,0.0,-61.774],[-84.372,0.0,-61.192],[-83.859,0.0,-60.615],[-83.351,0.0,-60.044],[-82.848,0.0,-59.480],[-82.352,0.0,-58.922],[-81.861,0.0,-58.370],[-81.376,0.0,-57.826],[-80.898,0.0,-57.289],[-80.427,0.0,-56.759],[-79.963,0.0,-56.238],[-79.507,0.0,-55.725],[-79.058,0.0,-55.221],[-78.617,0.0,-54.725],[-78.184,0.0,-54.239],[-77.759,0.0,-53.762],[-77.344,0.0,-53.295],[-76.937,0.0,-52.838],[-76.540,0.0,-52.392],[-76.152,0.0,-51.956],[-75.775,0.0,-51.532],[-75.407,0.0,-51.119],[-75.049,0.0,-50.717],[-74.703,0.0,-50.328],[-74.367,0.0,-49.950],[-73.729,0.0,-49.234],[-73.139,0.0,-48.571],[-72.598,0.0,-47.962],[-72.108,0.0,-47.412],[-71.671,0.0,-46.922],[-71.291,0.0,-46.494],[-70.829,0.0,-45.976],[-70.293,0.0,-45.373]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = surface
        .compiled_visual_node_pieces()
        .get(&bend)
        .expect("bend should compile through canonical owned regions");
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_top_covers_footprint(bend_piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(bend_piece);
    assert_canonical_explicit_vertical_steps_have_faces(bend_piece);
    assert_earthwork_faces_stay_outside_top_footprint(bend_piece);
}

#[test]
fn logged_inside_bend_compiles_with_explicit_point_contact_curb_ownership() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-82.047, 0.0, -9.463), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(28.584, 0.0, -15.027), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(71.960, 0.0, 47.832), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-82.047, 0.0, -9.463),
            Vector3::new(28.584, 0.0, -15.027),
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
            Vector3::new(28.584, 0.0, -15.027),
            Vector3::new(71.960, 0.0, 47.832),
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
fn logged_loop_bend_does_not_assign_sidewalk_join_outside_height_field() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let northwest = graph.add_node(Vector3::new(-76.169, 0.0, 80.632), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-118.592, 0.0, 36.658), NodeType::Junction);
    let south = graph.add_node(Vector3::new(-125.370, 0.0, -4.912), NodeType::Junction);

    graph.add_edge(test_edge(
        northwest,
        bend,
        road_points_from_json(
            "[[-76.169,0.0,80.632],[-76.646,0.0,80.138],[-77.218,0.0,79.545],[-77.581,0.0,79.169],[-77.992,0.0,78.742],[-78.450,0.0,78.267],[-78.953,0.0,77.746],[-79.498,0.0,77.181],[-80.084,0.0,76.574],[-80.709,0.0,75.926],[-81.371,0.0,75.240],[-81.890,0.0,74.701],[-82.247,0.0,74.331],[-82.612,0.0,73.953],[-82.985,0.0,73.567],[-83.366,0.0,73.172],[-83.754,0.0,72.770],[-84.149,0.0,72.361],[-84.551,0.0,71.944],[-84.959,0.0,71.520],[-85.374,0.0,71.090],[-85.796,0.0,70.653],[-86.223,0.0,70.210],[-86.656,0.0,69.762],[-87.094,0.0,69.307],[-87.538,0.0,68.847],[-87.986,0.0,68.382],[-88.440,0.0,67.913],[-88.897,0.0,67.438],[-89.360,0.0,66.959],[-89.826,0.0,66.476],[-90.295,0.0,65.989],[-90.769,0.0,65.498],[-91.245,0.0,65.004],[-91.725,0.0,64.507],[-92.208,0.0,64.007],[-92.693,0.0,63.504],[-93.180,0.0,62.999],[-93.669,0.0,62.492],[-94.160,0.0,61.983],[-94.653,0.0,61.472],[-95.147,0.0,60.960],[-95.642,0.0,60.447],[-96.139,0.0,59.932],[-96.635,0.0,59.418],[-97.132,0.0,58.902],[-97.629,0.0,58.387],[-98.126,0.0,57.872],[-98.623,0.0,57.357],[-99.119,0.0,56.843],[-99.614,0.0,56.330],[-100.108,0.0,55.817],[-100.601,0.0,55.307],[-101.092,0.0,54.798],[-101.582,0.0,54.290],[-102.069,0.0,53.785],[-102.554,0.0,53.282],[-103.036,0.0,52.782],[-103.516,0.0,52.285],[-103.993,0.0,51.791],[-104.466,0.0,51.300],[-104.936,0.0,50.813],[-105.402,0.0,50.330],[-105.864,0.0,49.851],[-106.322,0.0,49.377],[-106.775,0.0,48.907],[-107.224,0.0,48.442],[-107.667,0.0,47.982],[-108.106,0.0,47.528],[-108.539,0.0,47.079],[-108.966,0.0,46.636],[-109.387,0.0,46.199],[-109.802,0.0,45.769],[-110.211,0.0,45.346],[-110.613,0.0,44.929],[-111.008,0.0,44.519],[-111.396,0.0,44.117],[-111.776,0.0,43.723],[-112.149,0.0,43.336],[-112.514,0.0,42.958],[-112.871,0.0,42.588],[-113.391,0.0,42.050],[-114.052,0.0,41.364],[-114.677,0.0,40.716],[-115.264,0.0,40.108],[-115.809,0.0,39.543],[-116.312,0.0,39.022],[-116.770,0.0,38.547],[-117.181,0.0,38.121],[-117.544,0.0,37.745],[-118.116,0.0,37.152],[-118.592,0.0,36.658]]",
        ),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        south,
        road_points_from_json(
            "[[-118.592,0.0,36.658],[-118.710,0.0,35.936],[-118.818,0.0,35.275],[-118.957,0.0,34.423],[-119.080,0.0,33.668],[-119.170,0.0,33.114],[-119.267,0.0,32.520],[-119.370,0.0,31.889],[-119.479,0.0,31.223],[-119.593,0.0,30.524],[-119.712,0.0,29.793],[-119.836,0.0,29.033],[-119.964,0.0,28.246],[-120.097,0.0,27.432],[-120.233,0.0,26.595],[-120.373,0.0,25.736],[-120.517,0.0,24.857],[-120.663,0.0,23.960],[-120.812,0.0,23.046],[-120.963,0.0,22.119],[-121.116,0.0,21.179],[-121.271,0.0,20.228],[-121.428,0.0,19.269],[-121.585,0.0,18.304],[-121.743,0.0,17.333],[-121.902,0.0,16.360],[-122.061,0.0,15.386],[-122.220,0.0,14.413],[-122.378,0.0,13.442],[-122.535,0.0,12.477],[-122.692,0.0,11.518],[-122.847,0.0,10.567],[-123.000,0.0,9.627],[-123.151,0.0,8.700],[-123.300,0.0,7.786],[-123.446,0.0,6.889],[-123.590,0.0,6.010],[-123.730,0.0,5.151],[-123.866,0.0,4.314],[-123.999,0.0,3.501],[-124.127,0.0,2.713],[-124.251,0.0,1.953],[-124.370,0.0,1.222],[-124.484,0.0,0.523],[-124.593,0.0,-0.143],[-124.696,0.0,-0.774],[-124.792,0.0,-1.367],[-124.883,0.0,-1.922],[-125.006,0.0,-2.677],[-125.145,0.0,-3.529],[-125.253,0.0,-4.190],[-125.370,0.0,-4.912]]",
        ),
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
fn logged_elevated_bend_rejects_implicit_cross_owner_cdt_height_edge() {
    let terrain = flat_terrain(1024, 1024);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(362.721, 212.172, -543.419), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(354.920, 197.879, -455.205), NodeType::Junction);
    let c = graph.add_node(Vector3::new(389.920, 181.789, -394.583), NodeType::Junction);

    graph.add_edge(test_edge(
        a,
        bend,
        vec![
            Vector3::new(362.721, 212.172, -543.419),
            Vector3::new(354.920, 197.879, -455.205),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        c,
        vec![
            Vector3::new(354.920, 197.879, -455.205),
            Vector3::new(389.920, 181.789, -394.583),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        !surface.compiled_visual_node_pieces().contains_key(&bend),
        "elevated bend must reject implicit cross-owner CDT height sharing until join ownership is legal"
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

    let travel = Vector2::new(40.0, 5.0).normalized();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = Vector2::new(0.0, 0.0);
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

    let end_travel = Vector2::new(-40.0, -5.0).normalized();
    let end_lateral = RoadSurfaceSystem::left_normal_xz(end_travel);
    let end_center = Vector2::new(40.0, 5.0);
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

#[test]
fn logged_curved_terminal_top_surfaces_cover_footprint() {
    let terrain = flat_terrain(384, 384);
    let points = road_points_from_json(
        "[[-52.080,0.0,25.947],[-52.858,0.0,26.111],[-53.527,0.0,26.253],\
        [-54.079,0.0,26.370],[-54.711,0.0,26.503],[-55.422,0.0,26.654],\
        [-56.206,0.0,26.820],[-57.063,0.0,27.001],[-57.987,0.0,27.197],\
        [-58.723,0.0,27.352],[-59.233,0.0,27.460],[-59.759,0.0,27.572],\
        [-60.299,0.0,27.686],[-60.854,0.0,27.803],[-61.424,0.0,27.924],\
        [-62.006,0.0,28.047],[-62.602,0.0,28.173],[-63.211,0.0,28.302],\
        [-63.833,0.0,28.434],[-64.466,0.0,28.568],[-65.111,0.0,28.704],\
        [-65.768,0.0,28.843],[-66.435,0.0,28.984],[-67.113,0.0,29.128],\
        [-67.801,0.0,29.273],[-68.499,0.0,29.421],[-69.206,0.0,29.571],\
        [-69.922,0.0,29.722],[-70.646,0.0,29.875],[-71.379,0.0,30.030],\
        [-72.119,0.0,30.187],[-72.867,0.0,30.345],[-73.621,0.0,30.505],\
        [-74.382,0.0,30.666],[-75.150,0.0,30.828],[-75.923,0.0,30.992],\
        [-76.701,0.0,31.157],[-77.484,0.0,31.323],[-78.272,0.0,31.489],\
        [-79.064,0.0,31.657],[-79.860,0.0,31.825],[-80.659,0.0,31.994],\
        [-81.461,0.0,32.164],[-82.266,0.0,32.334],[-83.073,0.0,32.505],\
        [-83.882,0.0,32.676],[-84.692,0.0,32.848],[-85.503,0.0,33.019],\
        [-86.315,0.0,33.191],[-87.126,0.0,33.363],[-87.938,0.0,33.535],\
        [-88.749,0.0,33.706],[-89.559,0.0,33.878],[-90.368,0.0,34.049],\
        [-91.175,0.0,34.220],[-91.980,0.0,34.390],[-92.782,0.0,34.560],\
        [-93.581,0.0,34.729],[-94.377,0.0,34.897],[-95.169,0.0,35.065],\
        [-95.957,0.0,35.232],[-96.740,0.0,35.397],[-97.518,0.0,35.562],\
        [-98.292,0.0,35.726],[-99.059,0.0,35.888],[-99.820,0.0,36.049],\
        [-100.575,0.0,36.209],[-101.322,0.0,36.367],[-102.062,0.0,36.524],\
        [-102.795,0.0,36.679],[-103.520,0.0,36.832],[-104.235,0.0,36.983],\
        [-104.942,0.0,37.133],[-105.640,0.0,37.281],[-106.328,0.0,37.426],\
        [-107.006,0.0,37.570],[-107.673,0.0,37.711],[-108.330,0.0,37.850],\
        [-108.975,0.0,37.986],[-109.609,0.0,38.120],[-110.230,0.0,38.252],\
        [-110.839,0.0,38.381],[-111.435,0.0,38.507],[-112.018,0.0,38.630],\
        [-112.587,0.0,38.751],[-113.142,0.0,38.868],[-113.682,0.0,38.982],\
        [-114.208,0.0,39.094],[-114.718,0.0,39.202],[-115.454,0.0,39.357],\
        [-116.379,0.0,39.553],[-117.235,0.0,39.734],[-118.020,0.0,39.900],\
        [-118.730,0.0,40.051],[-119.362,0.0,40.184],[-119.914,0.0,40.301],\
        [-120.583,0.0,40.443],[-121.361,0.0,40.607]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        14.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 2;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    for node_id in [start, end] {
        let terminal_piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .unwrap_or_else(|| panic!("logged curved road endpoint should compile a terminal piece; node_id={node_id} dump={dump}"));
        assert_eq!(
            terminal_piece.kind,
            RoadSurfaceVisualNodePieceKind::Terminal
        );
        assert_node_top_covers_footprint(terminal_piece);
        assert_material_triangles_do_not_overlap(terminal_piece);
    }
}

#[test]
fn logged_terminal_with_tiny_boundary_dust_exports_final_top_footprint() {
    let terrain = flat_terrain(256, 256);
    let points = road_points_from_json(
        r#"[[98.445,0.0,22.22],[98.058,0.0,22.613],[97.589,0.0,23.089],[97.18,0.0,23.504],[96.698,0.0,23.994],[96.145,0.0,24.556],[95.524,0.0,25.186],[95.015,0.0,25.703],[94.656,0.0,26.067],[94.282,0.0,26.447],[93.892,0.0,26.843],[93.488,0.0,27.253],[93.07,0.0,27.678],[92.637,0.0,28.117],[92.191,0.0,28.57],[91.731,0.0,29.037],[91.259,0.0,29.517],[90.774,0.0,30.009],[90.276,0.0,30.514],[89.767,0.0,31.032],[89.246,0.0,31.561],[88.713,0.0,32.101],[88.17,0.0,32.653],[87.616,0.0,33.215],[87.052,0.0,33.788],[86.478,0.0,34.371],[85.895,0.0,34.963],[85.302,0.0,35.565],[84.7,0.0,36.176],[84.09,0.0,36.795],[83.472,0.0,37.423],[82.846,0.0,38.059],[82.213,0.0,38.702],[81.572,0.0,39.352],[80.925,0.0,40.009],[80.271,0.0,40.673],[79.612,0.0,41.343],[78.946,0.0,42.018],[78.275,0.0,42.7],[77.599,0.0,43.386],[76.919,0.0,44.077],[76.234,0.0,44.772],[75.545,0.0,45.472],[74.853,0.0,46.175],[74.157,0.0,46.881],[73.458,0.0,47.591],[72.932,0.0,48.125],[72.581,0.0,48.481],[72.229,0.0,48.839],[71.877,0.0,49.196],[71.524,0.0,49.554],[71.171,0.0,49.913],[70.818,0.0,50.272],[70.464,0.0,50.631],[70.11,0.0,50.991],[69.755,0.0,51.351],[69.401,0.0,51.711],[69.046,0.0,52.071],[68.691,0.0,52.431],[68.336,0.0,52.791],[67.981,0.0,53.152],[67.626,0.0,53.512],[67.272,0.0,53.872],[66.917,0.0,54.233],[66.562,0.0,54.593],[66.208,0.0,54.953],[65.854,0.0,55.312],[65.5,0.0,55.671],[65.146,0.0,56.03],[64.793,0.0,56.389],[64.44,0.0,56.747],[64.088,0.0,57.105],[63.736,0.0,57.462],[63.385,0.0,57.819],[62.859,0.0,58.353],[62.161,0.0,59.062],[61.465,0.0,59.768],[60.772,0.0,60.472],[60.083,0.0,61.171],[59.399,0.0,61.866],[58.718,0.0,62.557],[58.042,0.0,63.244],[57.371,0.0,63.925],[56.706,0.0,64.601],[56.046,0.0,65.27],[55.392,0.0,65.934],[54.745,0.0,66.591],[54.105,0.0,67.242],[53.471,0.0,67.885],[52.845,0.0,68.52],[52.227,0.0,69.148],[51.617,0.0,69.767],[51.016,0.0,70.378],[50.423,0.0,70.98],[49.84,0.0,71.572],[49.266,0.0,72.155],[48.702,0.0,72.728],[48.148,0.0,73.29],[47.604,0.0,73.842],[47.072,0.0,74.382],[46.551,0.0,74.912],[46.041,0.0,75.429],[45.544,0.0,75.934],[45.059,0.0,76.427],[44.586,0.0,76.907],[44.126,0.0,77.373],[43.68,0.0,77.826],[43.248,0.0,78.266],[42.829,0.0,78.69],[42.425,0.0,79.101],[42.036,0.0,79.496],[41.661,0.0,79.876],[41.302,0.0,80.241],[40.794,0.0,80.757],[40.173,0.0,81.388],[39.62,0.0,81.949],[39.137,0.0,82.439],[38.729,0.0,82.854],[38.259,0.0,83.331],[37.872,0.0,83.724]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        3.5,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.bkw_lanes = 0;
    graph.add_edge(edge);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0]);
    for node_id in [start, end] {
        let piece = surface
            .compiled_visual_node_pieces()
            .get(&node_id)
            .unwrap_or_else(|| {
                panic!(
                    "tiny boundary dust should not survive into final top footprint; node_id={node_id} report={} dump={dump}",
                    canonical_node_pipeline_report(
                        &surface,
                        &graph,
                        node_id,
                        RoadSurfaceVisualNodePieceKind::Terminal,
                    )
                )
            });
        assert_node_top_covers_footprint(piece);
    }
    assert!(
        dump.contains("\"missing_source_count\":0")
            && dump.contains("\"boundary_interpolation_source_count\":0"),
        "tiny boundary dust must be absent from final top-owned footprint export, not repaired by boundary interpolation; dump={dump}"
    );
}

#[test]
fn logged_terminal_handoff_keeps_both_sidewalk_edges_owned() {
    let terrain = flat_terrain(128, 128);
    let points = road_points_from_json(
        "[[-67.97,0.0,12.333],[-67.147,0.0,12.502],[-66.439,0.0,12.648],\
        [-65.855,0.0,12.769],[-65.186,0.0,12.907],[-64.435,0.0,13.061],\
        [-63.605,0.0,13.232],[-62.699,0.0,13.419],[-61.972,0.0,13.569],\
        [-61.466,0.0,13.673],[-60.942,0.0,13.781],[-60.402,0.0,13.892],\
        [-59.846,0.0,14.007],[-59.274,0.0,14.125],[-58.687,0.0,14.246],\
        [-58.085,0.0,14.37],[-57.469,0.0,14.497],[-56.838,0.0,14.627],\
        [-56.194,0.0,14.76],[-55.537,0.0,14.895],[-54.867,0.0,15.033],\
        [-54.184,0.0,15.174],[-53.49,0.0,15.317],[-52.783,0.0,15.463],\
        [-52.066,0.0,15.61],[-51.339,0.0,15.76],[-50.6,0.0,15.912],\
        [-49.852,0.0,16.067],[-49.095,0.0,16.223],[-48.329,0.0,16.381],\
        [-47.554,0.0,16.54],[-46.77,0.0,16.702],[-45.979,0.0,16.865],\
        [-45.181,0.0,17.029],[-44.376,0.0,17.195],[-43.564,0.0,17.362],\
        [-42.746,0.0,17.531],[-41.923,0.0,17.701],[-41.094,0.0,17.871],\
        [-40.261,0.0,18.043],[-39.423,0.0,18.216],[-38.581,0.0,18.389],\
        [-37.736,0.0,18.564],[-36.887,0.0,18.739],[-36.036,0.0,18.914],\
        [-35.182,0.0,19.09],[-34.326,0.0,19.266],[-33.469,0.0,19.443],\
        [-32.611,0.0,19.62],[-31.753,0.0,19.797],[-30.894,0.0,19.974],\
        [-30.035,0.0,20.151],[-29.177,0.0,20.327],[-28.32,0.0,20.504],\
        [-27.465,0.0,20.68],[-26.611,0.0,20.856],[-25.76,0.0,21.032],\
        [-24.911,0.0,21.207],[-24.065,0.0,21.381],[-23.224,0.0,21.554],\
        [-22.386,0.0,21.727],[-21.552,0.0,21.899],[-20.723,0.0,22.07],\
        [-19.9,0.0,22.239],[-19.082,0.0,22.408],[-18.27,0.0,22.575],\
        [-17.465,0.0,22.741],[-16.667,0.0,22.906],[-15.876,0.0,23.069],\
        [-15.093,0.0,23.23],[-14.318,0.0,23.39],[-13.551,0.0,23.548],\
        [-12.794,0.0,23.704],[-12.046,0.0,23.858],[-11.308,0.0,24.01],\
        [-10.58,0.0,24.16],[-9.863,0.0,24.308],[-9.157,0.0,24.453],\
        [-8.462,0.0,24.596],[-7.78,0.0,24.737],[-7.11,0.0,24.875],\
        [-6.452,0.0,25.011],[-5.808,0.0,25.143],[-5.178,0.0,25.273],\
        [-4.561,0.0,25.4],[-3.959,0.0,25.524],[-3.372,0.0,25.645],\
        [-2.8,0.0,25.763],[-2.244,0.0,25.878],[-1.704,0.0,25.989],\
        [-1.181,0.0,26.097],[-0.674,0.0,26.201],[0.052,0.0,26.351],\
        [0.958,0.0,26.538],[1.788,0.0,26.709],[2.54,0.0,26.864],\
        [3.209,0.0,27.002],[3.793,0.0,27.122],[4.5,0.0,27.268],\
        [5.323,0.0,27.437]]",
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();

    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 0;
    let edge_idx = graph.add_edge(edge);

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .expect("logged terminal road should keep a visible span after terminal handoff");
    let start_terminal = surface
        .compiled_visual_node_pieces()
        .get(&start)
        .expect("logged terminal road start should compile a terminal piece");
    let start_mouth = span_piece
        .start_mouth_profile
        .as_ref()
        .expect("logged terminal span should expose a start mouth profile");
    let start_endpoint = RoadSurfaceSystem::build_mouth_profile_from_section(
        surface
            .compiled_sections()
            .get(&edge_idx)
            .and_then(|sections| sections.first())
            .expect("logged terminal road should compile endpoint sections"),
        super::IncidentEdgeSide::Start,
    )
    .expect("logged terminal endpoint section should expose a profile");

    assert_terminal_mouth_handoff_surface_is_owned(
        start_terminal,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb at logged terminal handoff",
    );
    assert_terminal_mouth_handoff_surface_is_owned(
        start_terminal,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk at logged terminal handoff",
    );
    assert_terminal_band_interval_grid_is_owned(
        start_terminal,
        &start_endpoint,
        start_mouth,
        RoadSurfaceBandKind::CurbOrShoulder,
        4,
        5,
        "right curb interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_owned(
        start_terminal,
        &start_endpoint,
        start_mouth,
        RoadSurfaceBandKind::Sidewalk,
        5,
        6,
        "right sidewalk interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_not_duplicated_by_span(
        span_piece,
        &start_endpoint,
        start_mouth,
        4,
        5,
        "right curb interval at logged terminal start",
    );
    assert_terminal_band_interval_grid_is_not_duplicated_by_span(
        span_piece,
        &start_endpoint,
        start_mouth,
        5,
        6,
        "right sidewalk interval at logged terminal start",
    );
    assert_raised_step_face_lower_edge_covers(
        &start_terminal.raised_step_face_polygons,
        start_endpoint.boundary_points_world[2],
        start_mouth.boundary_points_world[2],
        "left longitudinal raised-step face at logged terminal handoff",
    );
    assert_raised_step_face_lower_edge_covers(
        &start_terminal.raised_step_face_polygons,
        start_endpoint.boundary_points_world[4],
        start_mouth.boundary_points_world[4],
        "right longitudinal raised-step face at logged terminal handoff",
    );
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
        (left_curb_upper.y - left_road_lower.y - CURB_STEP_HEIGHT_M).abs() <= 0.004,
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
        (right_curb_upper.y - right_road_lower.y - CURB_STEP_HEIGHT_M).abs() <= 0.004,
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

    let travel = Vector2::new(40.0, 0.0).normalized();
    let lateral = RoadSurfaceSystem::left_normal_xz(travel);
    let center = Vector2::new(0.0, 0.0);
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

#[test]
fn logged_one_way_elevated_terminal_compiles_mouth_vertical_step() {
    let terrain = coarse_hillside_world_terrain(1025, 1025, 1.0);
    let points = road_points_from_json(
        r#"[[-155.772,156.540,-191.943],[-156.491,156.482,-192.388],[-157.109,156.423,-192.771],[-157.619,156.362,-193.087],[-158.203,156.296,-193.449],[-158.860,156.222,-193.855],[-159.585,156.137,-194.304],[-160.376,156.043,-194.794],[-161.011,155.947,-195.188],[-161.453,155.853,-195.462],[-161.910,155.764,-195.745],[-162.382,155.680,-196.037],[-162.867,155.601,-196.338],[-163.367,155.523,-196.647],[-163.880,155.444,-196.965],[-164.405,155.365,-197.290],[-164.944,155.284,-197.624],[-165.494,155.204,-197.965],[-166.057,155.125,-198.313],[-166.631,155.051,-198.669],[-167.216,154.982,-199.031],[-167.813,154.918,-199.401],[-168.419,154.855,-199.776],[-169.036,154.783,-200.158],[-169.662,154.694,-200.546],[-170.298,154.581,-200.940],[-170.942,154.441,-201.339],[-171.596,154.277,-201.744],[-172.257,154.096,-202.154],[-172.927,153.906,-202.568],[-173.603,153.712,-202.987],[-174.288,153.515,-203.411],[-174.978,153.316,-203.839],[-175.676,153.116,-204.271],[-176.379,152.915,-204.706],[-177.088,152.712,-205.145],[-177.802,152.504,-205.588],[-178.521,152.284,-206.033],[-179.245,152.041,-206.482],[-179.973,151.768,-206.933],[-180.705,151.462,-207.386],[-181.440,151.127,-207.841],[-182.178,150.771,-208.299],[-182.920,150.408,-208.758],[-183.663,150.046,-209.218],[-184.409,149.692,-209.680],[-185.156,149.347,-210.143],[-185.904,149.010,-210.606],[-186.654,148.678,-211.071],[-187.404,148.354,-211.535],[-188.154,148.039,-212.000],[-188.904,147.740,-212.464],[-189.653,147.462,-212.928],[-190.402,147.207,-213.392],[-191.149,146.976,-213.855],[-191.895,146.764,-214.317],[-192.638,146.563,-214.777],[-193.379,146.368,-215.236],[-194.118,146.175,-215.694],[-194.853,145.982,-216.149],[-195.585,145.789,-216.602],[-196.313,145.597,-217.053],[-197.037,145.405,-217.501],[-197.756,145.216,-217.947],[-198.470,145.037,-218.389],[-199.179,144.873,-218.828],[-199.882,144.733,-219.264],[-200.579,144.619,-219.696],[-201.270,144.528,-220.124],[-201.954,144.453,-220.547],[-202.631,144.384,-220.967],[-203.301,144.311,-221.381],[-203.962,144.230,-221.791],[-204.615,144.139,-222.196],[-205.260,144.039,-222.595],[-205.896,143.931,-222.989],[-206.522,143.815,-223.376],[-207.139,143.690,-223.758],[-207.745,143.558,-224.134],[-208.341,143.422,-224.503],[-208.927,143.292,-224.866],[-209.501,143.176,-225.221],[-210.063,143.082,-225.570],[-210.614,143.013,-225.911],[-211.152,142.962,-226.245],[-211.678,142.923,-226.570],[-212.191,142.885,-226.888],[-212.690,142.844,-227.197],[-213.176,142.797,-227.498],[-213.648,142.742,-227.790],[-214.105,142.677,-228.073],[-214.547,142.600,-228.347],[-214.974,142.509,-228.612],[-215.585,142.404,-228.990],[-216.344,142.290,-229.460],[-217.035,142.169,-229.888],[-217.656,142.046,-230.273],[-218.203,141.921,-230.612],[-218.675,141.794,-230.904],[-219.786,141.665,-231.592]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();
    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 2;
    edge.bkw_lanes = 0;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "start terminal should compile with explicit mouth asphalt-curb height ownership"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "end terminal should compile with explicit mouth asphalt-curb height ownership"
    );
}

#[test]
fn logged_two_lane_elevated_terminal_compiles_endpoint_vertical_step() {
    let terrain = coarse_hillside_world_terrain(1025, 1025, 1.0);
    let points = road_points_from_json(
        r#"[[-92.509,156.967,-122.123],[-92.307,156.897,-121.439],[-92.065,156.823,-120.618],[-91.912,156.738,-120.097],[-91.738,156.638,-119.507],[-91.544,156.520,-118.849],[-91.331,156.383,-118.128],[-91.101,156.233,-117.345],[-90.853,156.080,-116.504],[-90.588,155.934,-115.607],[-90.380,155.802,-114.899],[-90.236,155.687,-114.411],[-90.088,155.585,-113.911],[-89.937,155.491,-113.399],[-89.783,155.399,-112.875],[-89.625,155.304,-112.340],[-89.464,155.206,-111.794],[-89.300,155.108,-111.237],[-89.133,155.012,-110.670],[-88.963,154.919,-110.093],[-88.790,154.831,-109.506],[-88.615,154.747,-108.911],[-88.436,154.664,-108.306],[-88.256,154.581,-107.693],[-88.072,154.497,-107.071],[-87.887,154.412,-106.442],[-87.699,154.325,-105.805],[-87.510,154.237,-105.162],[-87.318,154.148,-104.511],[-87.124,154.057,-103.854],[-86.929,153.965,-103.191],[-86.731,153.872,-102.522],[-86.533,153.779,-101.847],[-86.332,153.687,-101.168],[-86.131,153.599,-100.484],[-85.928,153.516,-99.795],[-85.724,153.438,-99.103],[-85.519,153.362,-98.407],[-85.312,153.288,-97.707],[-85.105,153.213,-97.005],[-84.898,153.135,-96.300],[-84.689,153.053,-95.592],[-84.480,152.969,-94.883],[-84.271,152.882,-94.172],[-84.061,152.792,-93.461],[-83.851,152.700,-92.748],[-83.640,152.605,-92.034],[-83.430,152.510,-91.321],[-83.220,152.414,-90.607],[-83.010,152.319,-89.894],[-82.800,152.225,-89.182],[-82.590,152.134,-88.472],[-82.381,152.045,-87.763],[-82.173,151.956,-87.055],[-81.965,151.868,-86.350],[-81.758,151.780,-85.648],[-81.552,151.693,-84.948],[-81.347,151.606,-84.252],[-81.143,151.520,-83.560],[-80.940,151.435,-82.871],[-80.738,151.352,-82.187],[-80.538,151.268,-81.508],[-80.339,151.182,-80.833],[-80.142,151.093,-80.164],[-79.946,151.000,-79.501],[-79.753,150.903,-78.844],[-79.561,150.803,-78.193],[-79.371,150.703,-77.550],[-79.183,150.604,-76.913],[-78.998,150.506,-76.284],[-78.815,150.411,-75.662],[-78.634,150.316,-75.049],[-78.456,150.223,-74.444],[-78.280,150.132,-73.849],[-78.107,150.044,-73.262],[-77.937,149.959,-72.685],[-77.770,149.878,-72.118],[-77.606,149.800,-71.561],[-77.445,149.721,-71.015],[-77.287,149.633,-70.480],[-77.133,149.530,-69.956],[-76.982,149.400,-69.444],[-76.835,149.239,-68.944],[-76.691,149.043,-68.456],[-76.482,148.816,-67.748],[-76.218,148.570,-66.851],[-75.970,148.320,-66.010],[-75.739,148.081,-65.227],[-75.527,147.860,-64.506],[-75.333,147.658,-63.848],[-75.159,147.468,-63.258],[-75.005,147.280,-62.737],[-74.763,147.089,-61.916],[-74.562,146.893,-61.232]]"#,
    );
    let start_point = points[0];
    let end_point = *points.last().unwrap();
    let mut graph = RegionGraph::new();
    let start = graph.add_node(start_point, NodeType::Junction);
    let end = graph.add_node(end_point, NodeType::Junction);
    let mut edge = test_edge(
        start,
        end,
        points,
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    );
    edge.fwd_lanes = 1;
    edge.bkw_lanes = 1;
    graph.add_edge(edge);
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&start),
        "start terminal should compile with explicit terminal endpoint asphalt-curb height ownership"
    );
    assert!(
        surface.compiled_visual_node_pieces().contains_key(&end),
        "end terminal should compile with explicit terminal endpoint asphalt-curb height ownership"
    );
}

#[test]
fn span_visual_pieces_compile_explicit_band_polygons() {
    let terrain = flat_terrain(64, 64);
    let mut graph = RegionGraph::new();
    let a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let b = graph.add_node(Vector3::new(24.0, 0.0, 0.0), NodeType::Junction);
    let edge_idx = graph.add_edge(test_edge(
        a,
        b,
        vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(24.0, 0.0, 0.0)],
        10.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let span_piece = surface
        .compiled_visual_span_pieces()
        .get(&edge_idx)
        .unwrap();
    assert!(!span_piece.outer_boundary_loops.is_empty());
    assert!(!span_piece.road_surface_polygons.is_empty());
    assert!(!span_piece.curb_surface_polygons.is_empty());
    assert!(!span_piece.raised_step_face_polygons.is_empty());
    assert!(!span_piece.sidewalk_surface_polygons.is_empty());
    assert!(!span_piece.span_owned_regions.is_empty());
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::Asphalt)
            .count(),
        span_piece.road_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::CurbOrShoulder)
            .count(),
        span_piece.curb_surface_polygons.len()
    );
    assert_eq!(
        span_piece
            .span_owned_regions
            .iter()
            .filter(|region| region.role == RoadSurfaceSpanRegionRole::NonRoad)
            .count(),
        span_piece.sidewalk_surface_polygons.len()
    );
    assert!(
        span_piece.span_owned_regions.iter().all(|region| {
            region.edge_idx == edge_idx
                && region.end_section_index == region.start_section_index + 1
                && region.end_s_m > region.start_s_m
        }),
        "span owned regions must preserve edge, section interval, and solved section authority"
    );
    assert!(!span_piece.span_earthwork_support_regions.is_empty());
    assert_eq!(
        span_piece.span_earthwork_support_regions.len(),
        span_piece.span_owned_regions.len(),
        "grounded standard span support regions should cover the same solved band-owned footprint as the visible span"
    );
    for role in [
        RoadSurfaceSpanRegionRole::Asphalt,
        RoadSurfaceSpanRegionRole::CurbOrShoulder,
        RoadSurfaceSpanRegionRole::NonRoad,
    ] {
        assert!(
            span_piece
                .span_earthwork_support_regions
                .iter()
                .any(|region| region.role == role),
            "span earthwork support regions must retain role/material provenance for {role:?}"
        );
    }
    assert!(
        span_piece
            .span_earthwork_support_regions
            .iter()
            .all(|region| {
                region.edge_idx == edge_idx
                    && region.end_section_index == region.start_section_index + 1
                    && region.end_s_m > region.start_s_m
                    && RoadSurfaceSystem::polygon_has_area_xz(&region.polygon.points_world)
            }),
        "span earthwork support regions must preserve edge, section interval, source band, and top-surface geometry"
    );
    assert_eq!(
        span_piece.span_raised_step_sources.len(),
        span_piece.raised_step_face_polygons.len()
    );
    assert!(
        span_piece.span_raised_step_sources.iter().all(|source| {
            source.lower_owner.kind != source.raised_owner.kind
                && source.end_section_index == source.start_section_index + 1
                && source.end_s_m > source.start_s_m
                && source.start_raised_world.y > source.start_lower_world.y
                && source.end_raised_world.y > source.end_lower_world.y
        }),
        "span raised-step faces must carry owner-pair and solved section provenance"
    );
    assert!(
        span_piece
            .road_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .curb_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece.curb_surface_polygons.iter().all(|polygon| {
            polygon.triangles_world.iter().all(|triangle| {
                let min_y = triangle[0].y.min(triangle[1].y).min(triangle[2].y);
                let max_y = triangle[0].y.max(triangle[1].y).max(triangle[2].y);
                max_y - min_y <= 0.001
            })
        }),
        "curb top surface must be flat; vertical drop belongs to explicit raised-step faces"
    );
    assert!(
        span_piece
            .raised_step_face_polygons
            .iter()
            .all(|polygon| !RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .sidewalk_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(!span_piece.earthwork_surface_polygons.is_empty());
    assert!(!span_piece.earthwork_outer_boundary_loops.is_empty());
    assert!(!span_piece.render_earthwork_faces.is_empty());
    assert_span_earthwork_faces_have_support_provenance(span_piece, edge_idx, EdgeClass::Standard);
    assert!(
        span_piece
            .earthwork_surface_polygons
            .iter()
            .all(|polygon| RoadSurfaceSystem::polygon_has_area_xz(&polygon.points_world))
    );
    assert!(
        span_piece
            .render_earthwork_faces
            .iter()
            .all(|face| RoadSurfaceSystem::polygon_has_area_xz(&face.polygon.points_world))
    );
    assert_ne!(
        span_piece.earthwork_outer_boundary_loops,
        span_piece.outer_boundary_loops
    );
}
