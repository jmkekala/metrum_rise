// SPDX-License-Identifier: GPL-2.0-only

//! Logged bend regression tests.

use super::*;

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
fn logged_bend_with_sidewalk_side_join_residual_compiles() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-109.987, 0.0, 21.730), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-48.637, 0.0, 61.543), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(21.046, 0.0, 37.484), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-109.987, 0.0, 21.730),
            Vector3::new(-48.637, 0.0, 61.543),
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
            Vector3::new(-48.637, 0.0, 61.543),
            Vector3::new(21.046, 0.0, 37.484),
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
fn logged_elevated_bend_with_curb_sidewalk_shared_height_seam_compiles() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-128.495667, 137.482468, 50.014038),
        NodeType::Junction,
    );
    let bend = graph.add_node(
        Vector3::new(-62.814247, 138.091599, 47.805443),
        NodeType::Junction,
    );
    let northeast = graph.add_node(
        Vector3::new(-28.634865, 137.346497, 98.009171),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-128.495667, 137.482468, 50.014038),
            Vector3::new(-62.814247, 138.091599, 47.805443),
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
            Vector3::new(-62.814247, 138.091599, 47.805443),
            Vector3::new(-28.634865, 137.346497, 98.009171),
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
fn logged_elevated_bend_with_conflicting_footprint_boundary_height_compiles() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-109.914040, 137.423492, 61.868073),
        NodeType::Junction,
    );
    let bend = graph.add_node(
        Vector3::new(-52.119457, 137.511230, 78.142029),
        NodeType::Junction,
    );
    let northeast = graph.add_node(
        Vector3::new(-28.145103, 137.416855, 126.829033),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-109.914040, 137.423492, 61.868073),
            Vector3::new(-52.119457, 137.511230, 78.142029),
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
            Vector3::new(-52.119457, 137.511230, 78.142029),
            Vector3::new(-28.145103, 137.416855, 126.829033),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);
    assert_bend_material_partition_keeps_sidewalk_visible(bend_piece);
}

#[test]
fn logged_current_elevated_bend_with_curb_sidewalk_boundary_height_conflict_compiles() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-124.141922, 137.527832, 58.071350),
        NodeType::Junction,
    );
    let bend = graph.add_node(
        Vector3::new(-47.562737, 137.724960, 71.863647),
        NodeType::Junction,
    );
    let northeast = graph.add_node(
        Vector3::new(-5.922226, 137.472488, 144.964676),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-124.141922, 137.527832, 58.071350),
            Vector3::new(-47.562737, 137.724960, 71.863647),
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
            Vector3::new(-47.562737, 137.724960, 71.863647),
            Vector3::new(-5.922226, 137.472488, 144.964676),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);
    assert_bend_material_partition_keeps_sidewalk_visible(bend_piece);
}

#[test]
fn logged_elevated_bend_with_fragmented_curb_sidewalk_side_join_compiles() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-154.372910, 138.110077, -2.822815),
        NodeType::Junction,
    );
    let bend = graph.add_node(
        Vector3::new(-85.724274, 137.737137, 49.256790),
        NodeType::Junction,
    );
    let northeast = graph.add_node(
        Vector3::new(-52.975925, 137.248505, 114.189369),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-154.372910, 138.110077, -2.822815),
            Vector3::new(-85.724274, 137.737137, 49.256790),
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
            Vector3::new(-85.724274, 137.737137, 49.256790),
            Vector3::new(-52.975925, 137.248505, 114.189369),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);
    assert_bend_material_partition_keeps_sidewalk_visible(bend_piece);
}

#[test]
fn logged_elevated_bend_does_not_leave_detached_asphalt_mouth_band_islands() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-65.679413, 137.331345, 74.066315),
        NodeType::Junction,
    );
    let bend = graph.add_node(
        Vector3::new(68.557236, 138.349731, 139.894348),
        NodeType::Junction,
    );
    let northeast = graph.add_node(
        Vector3::new(76.133392, 137.273651, 219.235001),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-65.679413, 137.331345, 74.066315),
            Vector3::new(68.557236, 138.349731, 139.894348),
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
            Vector3::new(68.557236, 138.349731, 139.894348),
            Vector3::new(76.133392, 137.273651, 219.235001),
        ],
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
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend,
                )
            )
        });
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_top_covers_footprint(bend_piece);
    assert_bend_material_partition_keeps_sidewalk_visible(bend_piece);
    assert_bend_asphalt_has_no_detached_islands(bend_piece);
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
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend,
                )
            )
        });
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert!(
        visual_polygon_boundary_passes_near_xz(
            &bend_piece.outer_boundary_loops,
            Vector2::new(-53.814, -20.179),
            0.5,
        ),
        "outer bend terrain cutter must preserve the explicit outer span rail within projection tolerance; outer_loops={:?}",
        bend_piece.outer_boundary_loops
    );
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
        .unwrap_or_else(|| {
            panic!(
                "bend should compile through canonical owned regions: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    bend,
                    RoadSurfaceVisualNodePieceKind::Bend,
                )
            )
        });
    assert_eq!(bend_piece.kind, RoadSurfaceVisualNodePieceKind::Bend);
    assert_node_top_covers_footprint(bend_piece);
    assert_bend_material_partition_keeps_sidewalk_visible(bend_piece);
    assert_top_raised_step_owner_boundaries_have_vertical_faces(bend_piece);
    assert_canonical_explicit_vertical_steps_have_faces(bend_piece);
    assert_earthwork_faces_stay_outside_top_footprint(bend_piece);
}

#[test]
fn logged_current_bend_keeps_inner_sidewalk_side_join_inside_footprint() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-85.017464, 0.0, -22.491150),
        NodeType::Junction,
    );
    let bend = graph.add_node(Vector3::new(-20.176170, 0.0, 12.873726), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(-14.238701, 0.0, 65.644653), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-85.017464, 0.0, -22.491150),
            Vector3::new(-20.176170, 0.0, 12.873726),
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
            Vector3::new(-20.176170, 0.0, 12.873726),
            Vector3::new(-14.238701, 0.0, 65.644653),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);

    assert_bend_sidewalk_side_join_contours_survive_footprint(&surface, bend_piece, bend);
    assert_bend_non_road_side_join_contours_survive_footprint(&surface, bend_piece, bend);
}

#[test]
fn logged_current_bend_keeps_curved_curb_side_join_inside_footprint() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(Vector3::new(-95.642860, 0.0, -4.336258), NodeType::Junction);
    let bend = graph.add_node(Vector3::new(-23.908737, 0.0, 15.929688), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(9.426231, 0.0, 88.333519), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-95.642860, 0.0, -4.336258),
            Vector3::new(-23.908737, 0.0, 15.929688),
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
            Vector3::new(-23.908737, 0.0, 15.929688),
            Vector3::new(9.426231, 0.0, 88.333519),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);

    assert_bend_non_road_side_join_contours_survive_footprint(&surface, bend_piece, bend);
}

#[test]
fn logged_inner_bend_orients_fragmented_curb_step_faces_to_lower_asphalt() {
    let terrain = flat_terrain(512, 512);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-18.243053, 0.0, -35.618744),
        NodeType::Junction,
    );
    let bend = graph.add_node(Vector3::new(36.906178, 0.0, -21.168159), NodeType::Junction);
    let southeast = graph.add_node(Vector3::new(67.482918, 0.0, -51.944984), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-18.243053, 0.0, -35.618744),
            Vector3::new(36.906178, 0.0, -21.168159),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        bend,
        southeast,
        vec![
            Vector3::new(36.906178, 0.0, -21.168159),
            Vector3::new(67.482918, 0.0, -51.944984),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);
    assert_raised_step_faces_visible_from_lower_owner(bend_piece);
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
fn logged_outer_bend_closes_footprint_with_arc_boundary() {
    let terrain = flat_terrain(384, 384);
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-125.385117, 0.0, -31.426414),
        NodeType::Junction,
    );
    let bend = graph.add_node(Vector3::new(-6.673889, 0.0, -23.719093), NodeType::Junction);
    let northeast = graph.add_node(Vector3::new(34.735245, 0.0, 44.360130), NodeType::Junction);

    graph.add_edge(test_edge(
        west,
        bend,
        vec![
            Vector3::new(-125.385117, 0.0, -31.426414),
            Vector3::new(-6.673889, 0.0, -23.719093),
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
            Vector3::new(-6.673889, 0.0, -23.719093),
            Vector3::new(34.735245, 0.0, 44.360130),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);
    let bend_piece = assert_compiled_bend_piece(&surface, &graph, bend);

    assert_bend_outer_boundary_preserves_corner_trim_support(&surface, bend_piece, bend);
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

fn assert_bend_material_partition_keeps_sidewalk_visible(piece: &RoadSurfaceVisualNodePiece) {
    let footprint_area_m2 = visual_polygon_union_area_m2(&piece.outer_boundary_loops);
    let asphalt_area_m2 = visual_polygon_union_area_m2(&piece.road_surface_polygons);
    let sidewalk_area_m2 = visual_polygon_union_area_m2(&piece.sidewalk_surface_polygons);
    let asphalt_ratio = asphalt_area_m2 / footprint_area_m2;
    let sidewalk_ratio = sidewalk_area_m2 / footprint_area_m2;

    assert!(
        asphalt_ratio <= 0.78 && sidewalk_ratio >= 0.18,
        "logged bend material partition must not let asphalt consume the sidewalk bands; footprint_area_m2={footprint_area_m2:.3} asphalt_area_m2={asphalt_area_m2:.3} asphalt_ratio={asphalt_ratio:.3} sidewalk_area_m2={sidewalk_area_m2:.3} sidewalk_ratio={sidewalk_ratio:.3}"
    );
}

fn assert_bend_asphalt_has_no_detached_islands(piece: &RoadSurfaceVisualNodePiece) {
    let contours = overlay_contours_from_top_polygons(&piece.road_surface_polygons);
    let mut shape_areas_m2 = RoadSurfaceSystem::overlay_union_contours(&contours)
        .unwrap_or_default()
        .iter()
        .map(RoadSurfaceSystem::overlay_shape_area_m2)
        .filter(|area_m2| *area_m2 > 0.01)
        .collect::<Vec<_>>();
    shape_areas_m2.sort_by(|left, right| right.total_cmp(left));
    let detached_area_m2 = shape_areas_m2.iter().skip(1).sum::<f32>();

    assert!(
        detached_area_m2 <= 0.05,
        "logged bend asphalt must be one connected ownership region; detached_area_m2={detached_area_m2:.3} shape_areas_m2={shape_areas_m2:?}"
    );
}

fn assert_bend_sidewalk_side_join_contours_survive_footprint(
    surface: &RoadSurfaceSystem,
    piece: &RoadSurfaceVisualNodePiece,
    node_id: u32,
) {
    let input = surface
        .compiled_visual_node_inputs
        .get(&node_id)
        .expect("compiled bend input must be cached");
    let arrangement_input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        node_id,
        piece.kind,
        &input.mouths,
    )
    .expect("compiled bend input must rebuild arrangement input");
    let rails = RoadSurfaceSystem::build_node_rail_contours_from_input(&arrangement_input)
        .expect("compiled bend input must rebuild generated rail contours");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("compiled bend rails must rebuild boolean ownership");
    let debug_snapshot =
        node::NodeBooleanDebugSnapshot::from_rails_and_ownership(&rails, &ownership, true);
    assert!(
        debug_snapshot.corner_trims.iter().any(|trim| {
            trim.source_band_kind == RoadSurfaceBandKind::Sidewalk
                && trim.side_join_intersections.iter().any(|intersection| {
                    intersection.owner == Some(trim.source_owner)
                        && matches!(
                            intersection.kind,
                            rails::NodeGeneratedContourKind::Band {
                                kind: RoadSurfaceBandKind::Sidewalk,
                            }
                        )
                        && intersection.area_m2 > 1.0
                })
        }),
        "Bend side-join trim debug must expose raw trim/sidewalk intersections with source provenance: {:?}",
        debug_snapshot.corner_trims
    );
    let sidewalk_side_join_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == rails::NodeGeneratedContourPurpose::BendSideJoin
                && contour.contributes_to_non_road_band()
                && matches!(
                    contour.kind,
                    rails::NodeGeneratedContourKind::Band {
                        kind: RoadSurfaceBandKind::Sidewalk,
                    }
                )
        })
        .collect::<Vec<_>>();
    assert!(
        sidewalk_side_join_contours.len() >= 2,
        "logged bend must generate both sidewalk side-join bands: {:?}",
        rails.contours
    );

    for contour in sidewalk_side_join_contours {
        let raw_shapes =
            RoadSurfaceSystem::overlay_union_contours(&[overlay_contour_from_generated_contour(
                contour,
            )])
            .expect("generated sidewalk side-join contour must union");
        let raw_area_m2 = overlay_area_m2(&raw_shapes);
        let retained_shapes = RoadSurfaceSystem::overlay_binary_shapes(
            &raw_shapes,
            &ownership.footprint_shapes,
            OverlayRule::Intersect,
        )
        .expect("sidewalk side-join / footprint intersection must solve");
        let retained_area_m2 = overlay_area_m2(&retained_shapes);
        let lost_area_m2 = raw_area_m2 - retained_area_m2;

        assert!(
            lost_area_m2 <= 0.05,
            "Bend footprint trims must not delete the generated sidewalk side-join band; lost_area_m2={lost_area_m2:.3} raw_area_m2={raw_area_m2:.3} retained_area_m2={retained_area_m2:.3} contour={contour:?} corner_trims={:?} footprint={:?}",
            rails.corner_trims,
            ownership.footprint_shapes
        );
    }
}

fn assert_bend_non_road_side_join_contours_survive_footprint(
    surface: &RoadSurfaceSystem,
    piece: &RoadSurfaceVisualNodePiece,
    node_id: u32,
) {
    let input = surface
        .compiled_visual_node_inputs
        .get(&node_id)
        .expect("compiled bend input must be cached");
    let arrangement_input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        node_id,
        piece.kind,
        &input.mouths,
    )
    .expect("compiled bend input must rebuild arrangement input");
    let rails = RoadSurfaceSystem::build_node_rail_contours_from_input(&arrangement_input)
        .expect("compiled bend input must rebuild generated rail contours");
    let ownership = RoadSurfaceSystem::build_node_boolean_ownership_from_rails(&rails)
        .expect("compiled bend rails must rebuild boolean ownership");
    let side_join_contours = rails
        .contours
        .iter()
        .filter(|contour| {
            contour.purpose == rails::NodeGeneratedContourPurpose::BendSideJoin
                && contour.contributes_to_non_road_band()
        })
        .collect::<Vec<_>>();
    assert!(
        side_join_contours.iter().any(|contour| matches!(
            contour.kind,
            rails::NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::CurbOrShoulder,
            }
        )),
        "logged bend must generate curved curb side-join bands: {:?}",
        rails.contours
    );
    assert!(
        side_join_contours.iter().any(|contour| matches!(
            contour.kind,
            rails::NodeGeneratedContourKind::Band {
                kind: RoadSurfaceBandKind::Sidewalk,
            }
        )),
        "logged bend must generate curved sidewalk side-join bands: {:?}",
        rails.contours
    );

    for contour in side_join_contours {
        let raw_shapes =
            RoadSurfaceSystem::overlay_union_contours(&[overlay_contour_from_generated_contour(
                contour,
            )])
            .expect("generated non-road side-join contour must union");
        let raw_area_m2 = overlay_area_m2(&raw_shapes);
        let retained_shapes = RoadSurfaceSystem::overlay_binary_shapes(
            &raw_shapes,
            &ownership.footprint_shapes,
            OverlayRule::Intersect,
        )
        .expect("non-road side-join / footprint intersection must solve");
        let retained_area_m2 = overlay_area_m2(&retained_shapes);
        let lost_area_m2 = raw_area_m2 - retained_area_m2;

        assert!(
            lost_area_m2 <= 0.05,
            "Bend footprint trims must not delete generated non-road side-join bands; lost_area_m2={lost_area_m2:.3} raw_area_m2={raw_area_m2:.3} retained_area_m2={retained_area_m2:.3} contour={contour:?} corner_trims={:?} footprint={:?}",
            rails.corner_trims,
            ownership.footprint_shapes
        );
    }
}

fn assert_bend_outer_boundary_preserves_corner_trim_support(
    surface: &RoadSurfaceSystem,
    piece: &RoadSurfaceVisualNodePiece,
    node_id: u32,
) {
    let input = surface
        .compiled_visual_node_inputs
        .get(&node_id)
        .expect("compiled bend input must be cached");
    let arrangement_input = RoadSurfaceSystem::build_node_arrangement_input_from_mouths(
        node_id,
        piece.kind,
        &input.mouths,
    )
    .expect("compiled bend input must rebuild arrangement input");
    let rails = RoadSurfaceSystem::build_node_rail_contours_from_input(&arrangement_input)
        .expect("compiled bend input must rebuild generated rail contours");
    let preserved_trim_points = rails
        .corner_trims
        .iter()
        .flat_map(|trim| trim.points_xz.iter().copied())
        .filter(|point| visual_polygon_boundary_contains_xz(&piece.outer_boundary_loops, *point))
        .count();

    assert!(
        preserved_trim_points >= 3,
        "outer bend footprint must preserve generated corner-trim support points; preserved_trim_points={preserved_trim_points} corner_trims={:?} outer_loops={:?}",
        rails.corner_trims,
        piece.outer_boundary_loops
    );
}

fn overlay_contour_from_generated_contour(
    contour: &rails::NodeGeneratedContour,
) -> NodeOverlayContour {
    contour
        .points_xz
        .iter()
        .map(|point| [point.x, point.y])
        .collect()
}

fn visual_polygon_boundary_passes_near_xz(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector2,
    tolerance_m: f64,
) -> bool {
    visual_polygon_boundary_min_distance_xz(polygons, point) <= tolerance_m
}

fn visual_polygon_boundary_min_distance_xz(
    polygons: &[RoadSurfaceVisualPolygon],
    point: Vector2,
) -> f64 {
    let point = backend::RoadVec2::new(f64::from(point.x), f64::from(point.y));
    polygons
        .iter()
        .filter(|polygon| polygon.points_world.len() >= 2)
        .flat_map(|polygon| {
            polygon
                .points_world
                .iter()
                .zip(polygon.points_world.iter().cycle().skip(1))
                .take(polygon.points_world.len())
        })
        .map(|(start, end)| {
            let start = backend::RoadVec2::new(start.x, start.z);
            let end = backend::RoadVec2::new(end.x, end.z);
            point_to_segment_distance_xz(point, start, end)
        })
        .fold(f64::INFINITY, f64::min)
}

fn point_to_segment_distance_xz(
    point: backend::RoadVec2,
    start: backend::RoadVec2,
    end: backend::RoadVec2,
) -> f64 {
    let segment = end - start;
    let length_sq = segment.length_squared();
    if length_sq <= f64::EPSILON {
        return point.distance(start);
    }
    let t = ((point - start).dot(segment) / length_sq).clamp(0.0, 1.0);
    point.distance(start + segment * t)
}

fn visual_polygon_union_area_m2(polygons: &[RoadSurfaceVisualPolygon]) -> f32 {
    let contours = overlay_contours_from_top_polygons(polygons);
    RoadSurfaceSystem::overlay_union_contours(&contours)
        .map(|shapes| overlay_area_m2(&shapes))
        .unwrap_or(0.0)
}
