//! Logged terminal regression tests.

use super::*;

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
        .unwrap_or_else(|| {
            panic!(
                "logged curved terminal should compile: {}",
                canonical_node_pipeline_report(
                    &surface,
                    &graph,
                    end,
                    RoadSurfaceVisualNodePieceKind::Terminal,
                )
            )
        });
    assert_eq!(
        terminal_piece.kind,
        RoadSurfaceVisualNodePieceKind::Terminal
    );
    assert_outer_boundary_vertices_match_visible_top(terminal_piece);
    assert_outer_boundary_vertices_use_visible_top_boundary_support(terminal_piece);
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
        "tiny boundary dust must be absent from final top-owned footprint export, not supplied by boundary interpolation; dump={dump}"
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
