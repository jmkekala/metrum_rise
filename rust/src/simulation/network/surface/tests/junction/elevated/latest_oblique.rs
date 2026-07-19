//! Latest logged elevated oblique junction compile tests.

use super::*;

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
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&config);
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

fn compile_remote_crossing_case(
    branch_angle_degrees: f32,
    crossing_angle_offset_degrees: f32,
    gradient_x: f32,
    gradient_z: f32,
    phase: f32,
) -> (TransitNetwork, RegionGraph, u32, u32) {
    let terrain = TerrainSystem::with_chunking(257, 257, 1.0, 128, 140.0);
    let height = |x: f32, z: f32| {
        140.0
            + x * gradient_x
            + z * gradient_z
            + 0.22 * (x * 0.07 + phase).sin()
            + 0.17 * (z * 0.055 - phase).cos()
            + 0.08 * ((x + z) * 0.11).sin()
    };
    let dense_segment = |start: Vector2, end: Vector2| {
        let steps = (start.distance_to(end) / 1.5).ceil() as usize;
        (0..=steps)
            .map(|step| {
                let xz = start.lerp(end, step as f32 / steps as f32);
                Vector3::new(xz.x, height(xz.x, xz.y), xz.y)
            })
            .collect::<Vec<_>>()
    };
    let center = Vector2::new(-49.109577, 34.990925);
    let main_angle = 19.25_f32.to_radians();
    let main_direction = Vector2::new(main_angle.cos(), main_angle.sin());
    let branch_angle = main_angle + branch_angle_degrees.to_radians();
    let branch_direction = Vector2::new(branch_angle.cos(), branch_angle.sin());
    let branch_end = center + branch_direction * 100.0;

    let mut graph = RegionGraph::new();
    let mut network = TransitNetwork::new();
    let config = crate::simulation::core::config::WorldConfig::default();
    let mut zoning = crate::simulation::zoning::ZoningSystem::new(&config);
    let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
    network.add_road(
        &mut graph,
        dense_segment(
            center - main_direction * 100.0,
            center + main_direction * 100.0,
        ),
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    // Keep the old junction at the end of this authored edge. A remote split then replaces the
    // incident half-edge and must force the existing JunctionN through the same dirty path as play.
    network.add_road(
        &mut graph,
        dense_segment(branch_end, center),
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&graph, &terrain);

    let old_junction = (0..graph.node_count() as u32)
        .find(|&node_id| {
            graph.get_valid_node(node_id) == node_id
                && graph.live_node_connection_count(node_id) == 3
        })
        .expect("branch endpoint must split the main road into a three-way junction");
    assert!(
        network
            .road_surface
            .compiled_visual_node_pieces()
            .contains_key(&old_junction),
        "the pre-edit three-way JunctionN must compile: {}",
        canonical_junction_pipeline_report(&network.road_surface, &graph, old_junction)
    );

    let crossing_center = center + branch_direction * 50.0;
    let crossing_angle = branch_angle + crossing_angle_offset_degrees.to_radians();
    let crossing_direction = Vector2::new(crossing_angle.cos(), crossing_angle.sin());
    network.add_road(
        &mut graph,
        dense_segment(
            crossing_center - crossing_direction * 18.0,
            crossing_center + crossing_direction * 18.0,
        ),
        1,
        1,
        EdgeClass::Standard,
        &mut zoning,
        &mut allocator,
    );
    network.road_surface.compile_dirty(&graph, &terrain);

    let new_junction = (0..graph.node_count() as u32)
        .filter(|&node_id| node_id != old_junction)
        .find(|&node_id| {
            graph.get_valid_node(node_id) == node_id
                && graph.live_node_connection_count(node_id) == 4
        })
        .expect("remote crossing must create a four-way junction");

    (network, graph, old_junction, new_junction)
}

#[test]
fn remote_crossing_keeps_existing_and_new_sloped_junctions_compilable() {
    let (network, graph, old_junction, new_junction) =
        compile_remote_crossing_case(35.0, 80.0, 0.0074, -0.0168, 0.74);
    for node_id in [old_junction, new_junction] {
        assert!(
            network
                .road_surface
                .compiled_visual_node_pieces()
                .contains_key(&node_id),
            "remote crossing must retain JunctionN node={node_id}: {}",
            canonical_junction_pipeline_report(&network.road_surface, &graph, node_id)
        );
    }
}

#[test]
fn remote_crossing_compiles_new_junction_with_dust_near_source_segment_candidates() {
    let (network, graph, old_junction, new_junction) =
        compile_remote_crossing_case(50.0, 95.0, 0.0095, -0.0135, 1.85);
    for node_id in [old_junction, new_junction] {
        if network
            .road_surface
            .compiled_visual_node_pieces()
            .contains_key(&node_id)
        {
            continue;
        }
        // A failed atomic incremental publication deliberately retains the prior span generation,
        // so rebuild from the current graph to report the actual affected-node diagnostic.
        let terrain = TerrainSystem::with_chunking(257, 257, 1.0, 128, 140.0);
        let mut diagnostic_surface = RoadSurfaceSystem::new(16.0);
        diagnostic_surface.compile_dirty(&graph, &terrain);
        panic!(
            "the remote crossing must preserve both JunctionN pieces while resolving dust-near source candidates, failed node={node_id}: {}",
            canonical_junction_pipeline_report(&diagnostic_surface, &graph, node_id)
        );
    }
}
