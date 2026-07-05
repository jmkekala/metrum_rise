//! Flat junction canonical pipeline tests.

use super::*;

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
    assert_flat_junction_raised_geometry_invariants(piece);
}

#[test]
fn logged_flat_split_road_branch_junction_compiles() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-111.102150, 0.0, -31.755333),
        NodeType::Junction,
    );
    let center = graph.add_node(Vector3::new(-63.626408, 0.0, 3.200905), NodeType::Junction);
    let east = graph.add_node(Vector3::new(-12.655884, 0.0, 40.730339), NodeType::Junction);
    let branch = graph.add_node(
        Vector3::new(-23.504681, 0.0, -39.691963),
        NodeType::Junction,
    );

    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-111.102150, 0.0, -31.755333),
            Vector3::new(-106.414230, 0.0, -28.303635),
            Vector3::new(-101.726318, 0.0, -24.851936),
            Vector3::new(-97.038399, 0.0, -21.400236),
            Vector3::new(-92.350479, 0.0, -17.948538),
            Vector3::new(-87.662567, 0.0, -14.496840),
            Vector3::new(-82.974640, 0.0, -11.045139),
            Vector3::new(-78.286728, 0.0, -7.593441),
            Vector3::new(-73.598808, 0.0, -4.141743),
            Vector3::new(-68.910889, 0.0, -0.690044),
            Vector3::new(-64.222977, 0.0, 2.761654),
            Vector3::new(-63.626408, 0.0, 3.200905),
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
            Vector3::new(-63.626408, 0.0, 3.200905),
            Vector3::new(-59.614235, 0.0, -1.088382),
            Vector3::new(-55.602062, 0.0, -5.377669),
            Vector3::new(-51.589890, 0.0, -9.666955),
            Vector3::new(-47.577717, 0.0, -13.956243),
            Vector3::new(-43.565544, 0.0, -18.245527),
            Vector3::new(-39.553368, 0.0, -22.534815),
            Vector3::new(-35.541199, 0.0, -26.824100),
            Vector3::new(-31.529026, 0.0, -31.113390),
            Vector3::new(-27.516853, 0.0, -35.402672),
            Vector3::new(-23.504681, 0.0, -39.691963),
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
            Vector3::new(-63.626408, 0.0, 3.200905),
            Vector3::new(-59.535057, 0.0, 6.213356),
            Vector3::new(-54.847137, 0.0, 9.665054),
            Vector3::new(-50.159222, 0.0, 13.116753),
            Vector3::new(-45.471306, 0.0, 16.568451),
            Vector3::new(-40.783386, 0.0, 20.020149),
            Vector3::new(-36.095467, 0.0, 23.471848),
            Vector3::new(-31.407555, 0.0, 26.923546),
            Vector3::new(-26.719635, 0.0, 30.375244),
            Vector3::new(-22.031715, 0.0, 33.826942),
            Vector3::new(-17.343803, 0.0, 37.278641),
            Vector3::new(-12.655884, 0.0, 40.730339),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "logged flat split-road branch JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
}

fn assert_flat_junction_raised_geometry_invariants(piece: &RoadSurfaceVisualNodePiece) {
    assert_flat_junction_raised_top_triangles_stay_at_curb_height(piece);
    assert_node_curb_and_sidewalk_drops_are_retaining_walls(piece);
}

fn assert_flat_junction_raised_top_triangles_stay_at_curb_height(
    piece: &RoadSurfaceVisualNodePiece,
) {
    assert_flat_polygons_stay_at_curb_height("curb", &piece.curb_surface_polygons);
    assert_flat_polygons_stay_at_curb_height("sidewalk", &piece.sidewalk_surface_polygons);
}

fn assert_flat_polygons_stay_at_curb_height(label: &str, polygons: &[RoadSurfaceVisualPolygon]) {
    let expected_y = f64::from(CURB_STEP_HEIGHT_M);
    let off_height_triangles = polygons
        .iter()
        .flat_map(|polygon| polygon.triangles_world.iter().copied())
        .filter(|triangle| {
            triangle
                .iter()
                .any(|point| (point.y - expected_y).abs() > f64::from(SAMPLE_EPSILON_M))
        })
        .collect::<Vec<_>>();
    assert!(
        off_height_triangles.is_empty(),
        "{label} JunctionN top triangles must stay flat at curb height; off_height_triangles={off_height_triangles:?}"
    );
}

fn assert_node_curb_and_sidewalk_drops_are_retaining_walls(piece: &RoadSurfaceVisualNodePiece) {
    let raised_drop_faces = piece
        .render_earthwork_faces
        .iter()
        .filter(|face| {
            node_earthwork_face_owner_is_raised_top(face.source)
                && earthwork_face_has_curb_height_drop(face)
        })
        .collect::<Vec<_>>();
    assert!(
        !raised_drop_faces.is_empty(),
        "flat JunctionN should expose curb/sidewalk drop earthwork faces"
    );
    let sloped_drop_faces = raised_drop_faces
        .iter()
        .copied()
        .filter(|face| face.kind == RoadSurfaceEarthworkFaceKind::Slope)
        .collect::<Vec<_>>();
    assert!(
        sloped_drop_faces.is_empty(),
        "node curb/sidewalk drops must export retaining-wall faces, not slopes; sloped_drop_faces={sloped_drop_faces:?}"
    );
}

fn node_earthwork_face_owner_is_raised_top(source: RoadSurfaceEarthworkFaceSource) -> bool {
    let owner_kind = match source {
        RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { owner_kind, .. }
        | RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff { owner_kind, .. } => {
            owner_kind
        }
        RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { .. } => return false,
    };
    matches!(
        owner_kind,
        RoadSurfaceBandKind::CurbOrShoulder | RoadSurfaceBandKind::Sidewalk
    )
}

fn earthwork_face_has_curb_height_drop(face: &RoadSurfaceEarthworkRenderFace) -> bool {
    let [_, _, outer_end, outer_start] = face.polygon.points_world.as_slice() else {
        return false;
    };
    let inner_min_y = face.inner_start.y.min(face.inner_end.y);
    let outer_max_y = outer_start.y.max(outer_end.y);
    inner_min_y - outer_max_y >= f64::from(CURB_STEP_HEIGHT_M) * 0.5
}

#[test]
fn logged_flat_three_way_junction_keeps_all_height_jumps_faced() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-102.528061, 0.0, -37.261856),
        NodeType::Junction,
    );
    let center = graph.add_node(
        Vector3::new(-46.854595, 0.0, -30.163692),
        NodeType::Junction,
    );
    let east = graph.add_node(Vector3::new(18.217957, 0.0, -21.867180), NodeType::Junction);
    let south = graph.add_node(
        Vector3::new(-39.266411, 0.0, -89.681923),
        NodeType::Junction,
    );
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-102.528061, 0.0, -37.261856),
            Vector3::new(-46.854595, 0.0, -30.163692),
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
            Vector3::new(-46.854595, 0.0, -30.163692),
            Vector3::new(-39.266411, 0.0, -89.681923),
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
            Vector3::new(-46.854595, 0.0, -30.163692),
            Vector3::new(18.217957, 0.0, -21.867180),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
}

#[test]
fn regenerated_flat_three_way_junction_keeps_all_height_jumps_faced() {
    let mut graph = RegionGraph::new();
    let west = graph.add_node(
        Vector3::new(-72.868027, 0.0, -49.521713),
        NodeType::Junction,
    );
    let center = graph.add_node(
        Vector3::new(-21.145235, 0.0, -24.091516),
        NodeType::Junction,
    );
    let north = graph.add_node(Vector3::new(-43.206322, 0.0, 20.778391), NodeType::Junction);
    let east = graph.add_node(Vector3::new(32.258186, 0.0, 2.164986), NodeType::Junction);
    graph.add_edge(test_edge(
        west,
        center,
        vec![
            Vector3::new(-72.868027, 0.0, -49.521713),
            Vector3::new(-21.145235, 0.0, -24.091516),
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
            Vector3::new(-21.145235, 0.0, -24.091516),
            Vector3::new(-43.206322, 0.0, 20.778391),
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
            Vector3::new(-21.145235, 0.0, -24.091516),
            Vector3::new(32.258186, 0.0, 2.164986),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
}

#[test]
fn logged_curved_flat_three_way_junction_does_not_emit_orphan_raised_step_caps() {
    let mut graph = RegionGraph::new();
    let northwest = graph.add_node(
        Vector3::new(-40.449200, 0.0, -22.386183),
        NodeType::Junction,
    );
    let center = graph.add_node(
        Vector3::new(-21.853373, 0.0, -49.000950),
        NodeType::Junction,
    );
    let south = graph.add_node(
        Vector3::new(-44.080544, 0.0, -111.136162),
        NodeType::Junction,
    );
    let southeast = graph.add_node(Vector3::new(-0.356785, 0.0, -79.767349), NodeType::Junction);
    graph.add_edge(test_edge(
        northwest,
        center,
        vec![
            Vector3::new(-40.449200, 0.0, -22.386183),
            Vector3::new(-22.045769, 0.0, -48.725590),
            Vector3::new(-21.853373, 0.0, -49.000950),
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
            Vector3::new(-21.853373, 0.0, -49.000950),
            Vector3::new(-22.102993, 0.0, -49.698757),
            Vector3::new(-44.080544, 0.0, -111.136162),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        southeast,
        vec![
            Vector3::new(-21.853373, 0.0, -49.000950),
            Vector3::new(-21.576977, 0.0, -49.396534),
            Vector3::new(-0.356785, 0.0, -79.767349),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
}

#[test]
fn regenerated_curved_flat_three_way_junction_filters_curb_width_orphan_caps() {
    let mut graph = RegionGraph::new();
    let southwest = graph.add_node(
        Vector3::new(-73.806625, 0.0, -50.709042),
        NodeType::Junction,
    );
    let center = graph.add_node(Vector3::new(3.799001, 0.0, -32.290504), NodeType::Junction);
    let southeast = graph.add_node(
        Vector3::new(19.964146, 0.0, -100.398422),
        NodeType::Junction,
    );
    let east = graph.add_node(Vector3::new(99.692421, 0.0, -9.531635), NodeType::Junction);
    graph.add_edge(test_edge(
        southwest,
        center,
        road_points_from_json(include_str!("../data/logged_flat_three_way_cap_edge0.json")),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        southeast,
        road_points_from_json(include_str!("../data/logged_flat_three_way_cap_edge1.json")),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        east,
        road_points_from_json(include_str!("../data/logged_flat_three_way_cap_edge2.json")),
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    let dump = surface.build_edge_geometry_debug_dump(&graph, &terrain, &[0, 1, 2]);
    let json = dump
        .trim_start_matches("ROAD_GEOMETRY_DUMP_BEGIN")
        .trim_end_matches("ROAD_GEOMETRY_DUMP_END")
        .trim();
    let parsed: serde_json::Value =
        serde_json::from_str(json).expect("road geometry debug dump must parse");
    let center_node = parsed["nodes"]
        .as_array()
        .and_then(|nodes| nodes.iter().find(|node| node["node_id"] == center))
        .expect("logged center JunctionN must be present in debug dump");
    let details = &center_node["raised_step_face_details"];
    assert_eq!(
        details["missing_required_face_count"], 0,
        "logged JunctionN must not miss required raised-step faces; samples={:#?}",
        details["expected_raised_steps"]
    );
    assert_eq!(
        details["required_gap_count"], 0,
        "logged JunctionN must not leave final raised-step coverage gaps; samples={:#?}",
        details["required_gap_samples"],
    );
    assert!(
        details["missing_length_m"]
            .as_f64()
            .unwrap_or(f64::INFINITY)
            <= f64::EPSILON
    );
    let problem_faces = details["faces"]
        .as_array()
        .expect("raised-step details must include faces")
        .iter()
        .filter(|face| {
            face["problem"].as_bool().unwrap_or(false)
                || face["status"].as_str().is_some_and(|status| status != "ok")
        })
        .take(5)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        problem_faces.is_empty(),
        "JunctionN raised-step debug must not emit orphan/problem faces; samples={problem_faces:#?}"
    );
    assert_eq!(details["face_problem_count"], 0);
    assert_eq!(details["problem_count"], 0);
    assert!(
        center_node["material_footprint_coverage"]["suspicious_missing_shape_count"] == 0,
        "logged JunctionN top-footprint holes must not leave visible boundary gaps; coverage={:#?}",
        center_node["material_footprint_coverage"]
    );
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
    assert_no_missing_top_footprint_shapes_touch_boundaries(piece);
    assert_surface_terrain_cdt_contract(
        "logged flat 3-way JunctionN",
        &surface,
        &graph,
        &terrain,
        (-16.0, -52.0, 24.0, -16.0),
        false,
    );
}

#[test]
fn logged_flat_four_way_junction_does_not_emit_curb_width_orphan_caps() {
    let mut graph = RegionGraph::new();
    let southwest = graph.add_node(
        Vector3::new(-94.740189, 0.0, -33.348091),
        NodeType::Junction,
    );
    let center = graph.add_node(Vector3::new(-39.109142, 0.0, 1.256486), NodeType::Junction);
    let southeast = graph.add_node(Vector3::new(-7.418081, 0.0, -49.691299), NodeType::Junction);
    let east = graph.add_node(Vector3::new(36.835907, 0.0, 48.497128), NodeType::Junction);
    let northwest = graph.add_node(Vector3::new(-81.363907, 0.0, 69.186852), NodeType::Junction);
    graph.add_edge(test_edge(
        southwest,
        center,
        vec![
            Vector3::new(-94.740189, 0.0, -33.348091),
            Vector3::new(-39.109142, 0.0, 1.256486),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        southeast,
        vec![
            Vector3::new(-39.109142, 0.0, 1.256486),
            Vector3::new(-7.418081, 0.0, -49.691299),
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
            Vector3::new(-39.109142, 0.0, 1.256486),
            Vector3::new(36.835907, 0.0, 48.497128),
        ],
        7.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.add_edge(test_edge(
        center,
        northwest,
        vec![
            Vector3::new(-39.109142, 0.0, 1.256486),
            Vector3::new(-81.363907, 0.0, 69.186852),
        ],
        14.0,
        EdgeClass::Standard,
        TransitType::Road,
        TransitFlags::CAR | TransitFlags::FOOT,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
}

#[test]
fn logged_mixed_width_junction_keeps_single_footprint_boundary_height() {
    let mut graph = RegionGraph::new();
    let node0 = graph.add_node(
        Vector3::new(-117.430656, 0.0, -13.277603),
        NodeType::Junction,
    );
    let node1 = graph.add_node(Vector3::new(26.671715, 0.0, 62.899704), NodeType::Junction);
    let node2 = graph.add_node(Vector3::new(-54.213356, 0.0, 20.141167), NodeType::Junction);
    let node3 = graph.add_node(Vector3::new(13.269596, 0.0, -34.675423), NodeType::Junction);
    let node4 = graph.add_node(Vector3::new(-46.970161, 0.0, 70.222626), NodeType::Junction);
    let _unused = graph.add_node(Vector3::new(-160.0, 0.0, -160.0), NodeType::Junction);
    let node6 = graph.add_node(Vector3::new(-85.487671, 0.0, 3.608521), NodeType::Junction);
    let node7 = graph.add_node(
        Vector3::new(-34.649117, 0.0, -119.015366),
        NodeType::Junction,
    );

    graph.add_edge(logged_lane_edge(
        node0,
        node6,
        vec![
            Vector3::new(-117.430656, 0.000000, -13.277603),
            Vector3::new(-112.284142, 0.000000, -10.556985),
            Vector3::new(-107.137627, 0.000000, -7.836367),
            Vector3::new(-101.991119, 0.000000, -5.115748),
            Vector3::new(-96.844604, 0.000000, -2.395130),
            Vector3::new(-91.698090, 0.000000, 0.325488),
            Vector3::new(-86.551575, 0.000000, 3.046106),
            Vector3::new(-85.487671, 0.000000, 3.608521),
        ],
        1,
        1,
    ));
    graph.add_edge(logged_lane_edge(
        node2,
        node3,
        vec![
            Vector3::new(-54.213356, 0.000000, 20.141167),
            Vector3::new(-49.714493, 0.000000, 16.486727),
            Vector3::new(-45.215630, 0.000000, 12.832288),
            Vector3::new(-40.716766, 0.000000, 9.177849),
            Vector3::new(-36.217903, 0.000000, 5.523409),
            Vector3::new(-31.719038, 0.000000, 1.868969),
            Vector3::new(-27.220175, 0.000000, -1.785469),
            Vector3::new(-22.721312, 0.000000, -5.439909),
            Vector3::new(-18.222448, 0.000000, -9.094349),
            Vector3::new(-13.723584, 0.000000, -12.748787),
            Vector3::new(-9.224721, 0.000000, -16.403229),
            Vector3::new(-4.725857, 0.000000, -20.057667),
            Vector3::new(-0.226994, 0.000000, -23.712105),
            Vector3::new(4.271870, 0.000000, -27.366543),
            Vector3::new(8.770733, 0.000000, -31.020985),
            Vector3::new(13.269596, 0.000000, -34.675423),
        ],
        1,
        1,
    ));
    graph.add_edge(logged_lane_edge(
        node2,
        node1,
        vec![
            Vector3::new(-54.213356, 0.000000, 20.141167),
            Vector3::new(-50.525986, 0.000000, 22.090431),
            Vector3::new(-45.379471, 0.000000, 24.811050),
            Vector3::new(-40.232964, 0.000000, 27.531666),
            Vector3::new(-35.086441, 0.000000, 30.252289),
            Vector3::new(-29.939926, 0.000000, 32.972904),
            Vector3::new(-24.793419, 0.000000, 35.693523),
            Vector3::new(-19.646912, 0.000000, 38.414139),
            Vector3::new(-14.500389, 0.000000, 41.134762),
            Vector3::new(-9.353874, 0.000000, 43.855377),
            Vector3::new(-4.207367, 0.000000, 46.575993),
            Vector3::new(0.939156, 0.000000, 49.296616),
            Vector3::new(6.085663, 0.000000, 52.017235),
            Vector3::new(11.232170, 0.000000, 54.737846),
            Vector3::new(16.378685, 0.000000, 57.458466),
            Vector3::new(21.525200, 0.000000, 60.179085),
            Vector3::new(26.671715, 0.000000, 62.899704),
        ],
        1,
        1,
    ));
    graph.add_edge(logged_lane_edge(
        node2,
        node4,
        vec![
            Vector3::new(-54.213356, 0.000000, 20.141167),
            Vector3::new(-53.408558, 0.120000, 25.705772),
            Vector3::new(-52.603756, 0.026635, 31.270380),
            Vector3::new(-51.798958, 0.005912, 36.834988),
            Vector3::new(-50.994160, 0.001312, 42.399593),
            Vector3::new(-50.189358, 0.000291, 47.964203),
            Vector3::new(-49.384560, 0.000065, 53.528809),
            Vector3::new(-48.579762, 0.000014, 59.093414),
            Vector3::new(-47.774960, 0.000003, 64.658020),
            Vector3::new(-46.970161, 0.000000, 70.222626),
        ],
        1,
        1,
    ));
    graph.add_edge(logged_lane_edge(
        node6,
        node7,
        vec![
            Vector3::new(-85.487671, 0.000000, 3.608521),
            Vector3::new(-83.277298, 0.000000, -1.722952),
            Vector3::new(-81.066925, 0.000000, -7.054426),
            Vector3::new(-78.856552, 0.000000, -12.385900),
            Vector3::new(-76.646179, 0.000000, -17.717373),
            Vector3::new(-74.435814, 0.000000, -23.048845),
            Vector3::new(-72.225441, 0.000000, -28.380320),
            Vector3::new(-70.015068, 0.000000, -33.711792),
            Vector3::new(-67.804695, 0.000000, -39.043266),
            Vector3::new(-65.594322, 0.000000, -44.374741),
            Vector3::new(-63.383953, 0.000000, -49.706211),
            Vector3::new(-61.173580, 0.000000, -55.037685),
            Vector3::new(-58.963207, 0.000000, -60.369160),
            Vector3::new(-56.752838, 0.000000, -65.700630),
            Vector3::new(-54.542465, 0.000000, -71.032104),
            Vector3::new(-52.332092, 0.000000, -76.363579),
            Vector3::new(-50.121719, 0.000000, -81.695053),
            Vector3::new(-47.911346, 0.000000, -87.026527),
            Vector3::new(-45.700977, 0.000000, -92.358002),
            Vector3::new(-43.490604, 0.000000, -97.689468),
            Vector3::new(-41.280235, 0.000000, -103.020943),
            Vector3::new(-39.069859, 0.000000, -108.352425),
            Vector3::new(-36.859489, 0.000000, -113.683891),
            Vector3::new(-34.649117, 0.000000, -119.015366),
        ],
        2,
        2,
    ));
    graph.add_edge(logged_lane_edge(
        node6,
        node2,
        vec![
            Vector3::new(-85.487671, 0.000000, 3.608521),
            Vector3::new(-81.405060, 0.000000, 5.766724),
            Vector3::new(-76.258545, 0.000000, 8.487343),
            Vector3::new(-71.112038, 0.000000, 11.207960),
            Vector3::new(-65.965523, 0.000000, 13.928579),
            Vector3::new(-60.819012, 0.000000, 16.649195),
            Vector3::new(-55.672497, 0.000000, 19.369816),
            Vector3::new(-54.213356, 0.000000, 20.141167),
        ],
        1,
        1,
    ));
    graph.rebuild_intersection_clips();

    let terrain = flat_terrain(256, 256);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&node6) {
        panic!(
            "logged mixed-width JunctionN did not compile: {}",
            canonical_junction_pipeline_report(&surface, &graph, node6)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, node6);
    assert_flat_junction_raised_geometry_invariants(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
}

fn logged_lane_edge(
    start_node: u32,
    end_node: u32,
    points: Vec<Vector3>,
    fwd_lanes: u8,
    bkw_lanes: u8,
) -> Edge {
    crate::simulation::network::build_surface_edge(
        start_node,
        end_node,
        points,
        fwd_lanes,
        bkw_lanes,
        EdgeClass::Standard,
    )
}

#[test]
fn flat_bend_angle_matrix_compiles_conflict_first_owned_regions() {
    run_generated_cases_in_parallel(
        &GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES,
        |&angle_degrees| {
            compile_generated_flat_bend(
                angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        },
    );
}

#[test]
fn flat_t_junction_angle_matrix_compiles_conflict_first_owned_regions() {
    run_generated_cases_in_parallel(
        &GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES,
        |&angle_degrees| {
            compile_generated_flat_t_junction(
                angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        },
    );
}

#[test]
fn flat_four_way_junction_matrix_compiles_conflict_first_owned_regions() {
    for endpoint_angle_degrees in [
        [0.0, 90.0, 180.0, 270.0],
        [0.0, 5.0, 96.0, 181.0],
        [0.0, 35.0, 140.0, 252.0],
        [0.0, 73.0, 180.0, 244.0],
    ] {
        compile_generated_flat_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_arbitrary_multiway_junction_matrix_compiles_conflict_first_owned_regions() {
    run_generated_cases_in_parallel(
        &[
            [0.0, 11.0, 95.0, 194.0, 278.0],
            [0.0, 37.0, 118.0, 203.0, 291.0],
        ],
        |endpoint_angle_degrees| {
            compile_generated_flat_multiway_junction(
                endpoint_angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        },
    );
    run_generated_cases_in_parallel(
        &[
            [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            [0.0, 23.0, 61.0, 137.0, 211.0, 304.0],
        ],
        |endpoint_angle_degrees| {
            compile_generated_flat_multiway_junction(
                endpoint_angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            );
        },
    );
}

#[test]
fn flat_bend_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
    assert_generated_node_canonical_signature_eq(
        "from_center",
        &from_center,
        "to_center",
        &to_center,
    );
}

#[test]
fn flat_t_junction_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
    assert_generated_node_canonical_signature_eq(
        "from_center",
        &from_center,
        "to_center",
        &to_center,
    );
}

#[test]
fn flat_bend_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_t_junction_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_t_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let forward = compile_generated_flat_t_junction_raw_identity(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_t_junction_raw_identity(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_four_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let forward = compile_generated_flat_four_way_junction_raw_identity(
        73.0,
        244.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_four_way_junction_raw_identity(
        73.0,
        244.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_four_way_junction_matrix_preserves_exact_raw_polygon_identity() {
    run_generated_cases_in_parallel(
        &[
            [0.0, 90.0, 180.0, 270.0],
            [0.0, 5.0, 96.0, 181.0],
            [0.0, 35.0, 140.0, 252.0],
            [0.0, 73.0, 180.0, 244.0],
        ],
        |endpoint_angle_degrees| {
            assert_generated_flat_multiway_raw_identity_edit_order(endpoint_angle_degrees);
        },
    );
}

#[test]
fn flat_five_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let endpoint_angle_degrees = [0.0, 37.0, 118.0, 203.0, 291.0];
    let forward = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_six_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let endpoint_angle_degrees = [0.0, 23.0, 61.0, 137.0, 211.0, 304.0];
    let forward = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_flat_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn flat_mixed_width_junction_matrix_preserves_exact_raw_polygon_identity() {
    run_generated_cases_in_parallel(
        &[
            ([0.0, 35.0, 140.0, 252.0], [7.0, 10.5, 5.5, 8.75]),
            ([0.0, 90.0, 180.0, 270.0], [6.0, 9.0, 7.5, 11.0]),
        ],
        |(endpoint_angle_degrees, edge_widths_m)| {
            assert_generated_flat_multiway_widths_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
            );
        },
    );
    run_generated_cases_in_parallel(
        &[([0.0, 11.0, 95.0, 194.0, 278.0], [7.0, 12.0, 5.5, 8.0, 10.0])],
        |(endpoint_angle_degrees, edge_widths_m)| {
            assert_generated_flat_multiway_widths_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
            );
        },
    );
    run_generated_cases_in_parallel(
        &[(
            [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            [6.5, 9.0, 5.0, 11.0, 8.0, 7.5],
        )],
        |(endpoint_angle_degrees, edge_widths_m)| {
            assert_generated_flat_multiway_widths_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
            );
        },
    );
}

#[test]
fn flat_mixed_profile_mode_junction_matrix_preserves_exact_raw_polygon_identity() {
    use GeneratedEdgeProfileMode::{Shoulder, SidewalkCurb};

    run_generated_cases_in_parallel(
        &[
            (
                [0.0, 35.0, 140.0, 252.0],
                [7.0, 10.5, 5.5, 8.75],
                [SidewalkCurb, Shoulder, SidewalkCurb, Shoulder],
            ),
            (
                [0.0, 90.0, 180.0, 270.0],
                [6.0, 9.0, 7.5, 11.0],
                [Shoulder, SidewalkCurb, SidewalkCurb, Shoulder],
            ),
        ],
        |(endpoint_angle_degrees, edge_widths_m, edge_profile_modes)| {
            assert_generated_flat_multiway_widths_profile_modes_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
                edge_profile_modes,
            );
        },
    );
    run_generated_cases_in_parallel(
        &[(
            [0.0, 11.0, 95.0, 194.0, 278.0],
            [7.0, 12.0, 5.5, 8.0, 10.0],
            [SidewalkCurb, Shoulder, SidewalkCurb, Shoulder, SidewalkCurb],
        )],
        |(endpoint_angle_degrees, edge_widths_m, edge_profile_modes)| {
            assert_generated_flat_multiway_widths_profile_modes_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
                edge_profile_modes,
            );
        },
    );
    run_generated_cases_in_parallel(
        &[(
            [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            [6.5, 9.0, 5.0, 11.0, 8.0, 7.5],
            [
                SidewalkCurb,
                Shoulder,
                SidewalkCurb,
                Shoulder,
                SidewalkCurb,
                Shoulder,
            ],
        )],
        |(endpoint_angle_degrees, edge_widths_m, edge_profile_modes)| {
            assert_generated_flat_multiway_widths_profile_modes_raw_identity_edit_order(
                endpoint_angle_degrees,
                edge_widths_m,
                edge_profile_modes,
            );
        },
    );
}

#[test]
fn flat_arbitrary_multiway_junction_matrix_preserves_exact_raw_polygon_identity() {
    run_generated_cases_in_parallel(
        &[
            [0.0, 11.0, 95.0, 194.0, 278.0],
            [0.0, 37.0, 118.0, 203.0, 291.0],
        ],
        |endpoint_angle_degrees| {
            assert_generated_flat_multiway_raw_identity_edit_order(endpoint_angle_degrees);
        },
    );
    run_generated_cases_in_parallel(
        &[
            [0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            [0.0, 23.0, 61.0, 137.0, 211.0, 304.0],
        ],
        |endpoint_angle_degrees| {
            assert_generated_flat_multiway_raw_identity_edit_order(endpoint_angle_degrees);
        },
    );
}

#[test]
fn flat_junctionn_canonical_raw_polygon_golden_checks_cover_generated_matrix() {
    assert_canonical_node_raw_polygon_golden(
        "flat_4way_cross",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 90.0, 180.0, 270.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 275,
            carrier_record_count: 326,
            source_segment_record_count: 19,
            polygon_key_set_digest: 15505972699732544511,
            top_owner_height_field_digest: 4578930640497027894,
            carrier_owner_source_height_field_digest: 5687354493498944674,
            source_segment_id_digest: 17651613363160715331,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2460269, z_key: 3500000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2928932, z_key: 2928932 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2928932, z_key: 2928932 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 4444298, z_key: 1685304 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 4444298, z_key: 1685304 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 6173166, z_key: 761205 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 6173166, z_key: 761205 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 8049097, z_key: 192147 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3500000, z_key: 2460269 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2928932, z_key: 2928932 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2928932, z_key: 2928932 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1685304, z_key: 4444297 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1685304, z_key: 4444297 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -761205, z_key: 6173165 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -761205, z_key: 6173165 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -192147, z_key: 8049097 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -10000000, z_key: -1 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -8049097, z_key: -192148 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -6173166, z_key: -761205 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4444298, z_key: -1685304 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -6173166, z_key: -761205 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4444298, z_key: -1685304 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -4444298, z_key: -1685304 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2928932, z_key: -2928933 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 0, z_key: -10000000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 192147, z_key: -8049097 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 761205, z_key: -6173166 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1685304, z_key: -4444298 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 761205, z_key: -6173166 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1685304, z_key: -4444298 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 5 }, source_kind: Sidewalk, source_mouth_order_index: 0, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 4720168, z_key: 6472129 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 5509872, z_key: 5509872 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -6472129, z_key: 4720168 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -5509872, z_key: 5509872 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -5509872, z_key: -5509873 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4720168, z_key: -6472130 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 5509872, z_key: -5509872 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 6472129, z_key: -4720168 } }".to_owned(),
            ],
        },
    );
    assert_canonical_node_raw_polygon_golden(
        "flat_5way_ugly",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 11.0, 95.0, 194.0, 278.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 481,
            carrier_record_count: 567,
            source_segment_record_count: 28,
            polygon_key_set_digest: 18433125960916516512,
            top_owner_height_field_digest: 3017765074126608807,
            carrier_owner_source_height_field_digest: 13238820941424359883,
            source_segment_id_digest: 2898317876464273514,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 3500000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 24999999, z_key: 3500000 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -220606, z_key: 2521531 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -129694, z_key: 2053830 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3182686, z_key: 5172507 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3215507, z_key: 5125633 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1505004, z_key: -186455 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1100333, z_key: 144912 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1100333, z_key: 144912 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -760217, z_key: 542258 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -760217, z_key: 542258 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -495250, z_key: 993206 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -495250, z_key: 993206 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -313685, z_key: 1483713 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 20 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 2, segment_start: NodeOwnedRegionArrangementKey { x_key: -7762366, z_key: -1935374 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 0 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 20 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 2, segment_start: NodeOwnedRegionArrangementKey { x_key: -7762366, z_key: -1935374 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 0 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -7762366, z_key: -1935374 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 0 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2449579, z_key: -610748 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1978989, z_key: -544611 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1978989, z_key: -544611 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1504933, z_key: -577761 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1504933, z_key: -577761 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1048128, z_key: -708747 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1048128, z_key: -708747 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -628539, z_key: -931847 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -628539, z_key: -931847 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -264503, z_key: -1237309 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 16 }, source_kind: CurbOrShoulder, source_mouth_order_index: 2, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -5478065, z_key: 2241313 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -5129674, z_key: 2361699 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 4098552, z_key: 10521902 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 4397382, z_key: 7106258 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 4488044, z_key: 4621125 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 4772648, z_key: 4646025 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25467002, z_key: -1196566 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -6773251, z_key: 3464308 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25467002, z_key: -1196566 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -6773251, z_key: 3464308 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25467002, z_key: -1196566 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -6773251, z_key: 3464308 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -19298457, z_key: -1049904 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -16407747, z_key: -329169 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -16407747, z_key: -329169 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -12368269, z_key: 677985 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -4124910, z_key: 3814290 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4060885, z_key: 4170572 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -23047784, z_key: -10899524 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -5281053, z_key: -6469782 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -23047784, z_key: -10899524 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -5281053, z_key: -6469782 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -4516555, z_key: -4887843 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4234874, z_key: -4848255 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -4020359, z_key: -7320131 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3545930, z_key: -10695867 } }".to_owned(),
            ],
        },
    );
    assert_canonical_node_raw_polygon_golden(
        "flat_6way_near_parallel",
        &compile_generated_flat_multiway_junction_raw_identity(
            &[0.0, 3.0, 47.0, 121.0, 202.0, 305.0],
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        ),
        CanonicalNodeRawPolygonGolden {
            kind: RoadSurfaceVisualNodePieceKind::JunctionN,
            top_polygon_count: 604,
            carrier_record_count: 706,
            source_segment_record_count: 32,
            polygon_key_set_digest: 9084159174272668121,
            top_owner_height_field_digest: 2586669956823001875,
            carrier_owner_source_height_field_digest: 13406976634543183515,
            source_segment_id_digest: 6020531977663627717,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1551547, z_key: 9377824 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1486876, z_key: 9270193 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1502151, z_key: 2500000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1192557, z_key: 2098347 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1192557, z_key: 2098347 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -799145, z_key: 1778344 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -799145, z_key: 1778344 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -342862, z_key: 1557031 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3285594, z_key: -1327467 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2704209, z_key: -1092572 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2704209, z_key: -1092572 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2230918, z_key: -838928 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2230918, z_key: -838928 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1822982, z_key: -489748 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1822982, z_key: -489748 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1499357, z_key: -61256 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1499357, z_key: -61256 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1275081, z_key: 426636 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1275081, z_key: 426636 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1160575, z_key: 951257 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1576495, z_key: -868313 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -933132, z_key: -891482 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -933132, z_key: -891482 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -302699, z_key: -1021895 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 28 }, source_kind: CurbOrShoulder, source_mouth_order_index: 4, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -6050157, z_key: -6381075 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1367314, z_key: -3384221 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 28 }, source_kind: CurbOrShoulder, source_mouth_order_index: 4, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -237070, z_key: -5836183 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 81124, z_key: -6217921 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 11066223, z_key: 5160264 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 11307300, z_key: 4796038 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 11307300, z_key: 4796038 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 11644332, z_key: 4518210 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 14021154, z_key: 7704450 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 19794230, z_key: 13895315 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 11 }, source_kind: Sidewalk, source_mouth_order_index: 1, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 14021154, z_key: 7704450 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 19794230, z_key: 13895315 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -1722768, z_key: 12575190 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 220641, z_key: 9340814 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -1722768, z_key: 12575190 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 220641, z_key: 9340814 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 1571283, z_key: 7087083 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1797218, z_key: 7279199 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25052630, z_key: -4729248 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -8648215, z_key: 1898568 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25052630, z_key: -4729248 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -8648215, z_key: 1898568 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -25052630, z_key: -4729248 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -8648215, z_key: 1898568 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -7248965, z_key: 4977450 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3128661, z_key: -1879889 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 23 }, source_kind: Sidewalk, source_mouth_order_index: 3, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -6138818, z_key: 3058647 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -6065533, z_key: 2726364 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -21306563, z_key: -14001086 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3722355, z_key: -6896603 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -2526177, z_key: -5133636 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2035824, z_key: -5151295 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -634329, z_key: -7811318 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2906313, z_key: -12867881 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -634329, z_key: -7811318 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2906313, z_key: -12867881 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 35 }, source_kind: Sidewalk, source_mouth_order_index: 5, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 7672684, z_key: -4550579 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 7756683, z_key: -4714111 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 35 }, source_kind: Sidewalk, source_mouth_order_index: 5, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: 9604909, z_key: -5000000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 11097834, z_key: -7132118 } }".to_owned(),
            ],
        },
    );
}

fn assert_generated_flat_multiway_raw_identity_edit_order(endpoint_angle_degrees: &[f32]) {
    let (forward, reverse) = rayon::join(
        || {
            compile_generated_flat_multiway_junction_raw_identity(
                endpoint_angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            )
        },
        || {
            compile_generated_flat_multiway_junction_raw_identity(
                endpoint_angle_degrees,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Reverse,
            )
        },
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

fn assert_generated_flat_multiway_widths_raw_identity_edit_order(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
) {
    let (forward, reverse) = rayon::join(
        || {
            compile_generated_flat_multiway_junction_with_widths_raw_identity(
                endpoint_angle_degrees,
                edge_widths_m,
                GeneratedEditOrder::Forward,
            )
        },
        || {
            compile_generated_flat_multiway_junction_with_widths_raw_identity(
                endpoint_angle_degrees,
                edge_widths_m,
                GeneratedEditOrder::Reverse,
            )
        },
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

fn assert_generated_flat_multiway_widths_profile_modes_raw_identity_edit_order(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edge_profile_modes: &[GeneratedEdgeProfileMode],
) {
    let (forward, reverse) = rayon::join(
        || {
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                endpoint_angle_degrees,
                edge_widths_m,
                edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Forward,
            )
        },
        || {
            compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
                endpoint_angle_degrees,
                edge_widths_m,
                edge_profile_modes,
                GeneratedEdgeDirection::FromCenter,
                GeneratedEditOrder::Reverse,
            )
        },
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

fn compile_generated_flat_bend(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_bend_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat Bend did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_node_pipeline_report(
                &surface,
                &graph,
                center,
                RoadSurfaceVisualNodePieceKind::Bend
            )
        );
    }
    let piece = assert_compiled_bend_piece(&surface, &graph, center);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_t_junction(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_t_junction_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat JunctionN did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_multiway_junction(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let (graph, center) = generated_multiway_junction_graph(
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat multiway JunctionN did not compile; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    generated_node_canonical_signature(piece)
}

fn compile_generated_flat_t_junction_raw_identity(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_t_junction_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat JunctionN did not compile for raw identity; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph(
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_with_widths_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph_with_edge_widths(
        GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
        endpoint_angle_degrees,
        edge_widths_m,
        GeneratedEdgeDirection::FromCenter,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat mixed-width multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_widths_m={edge_widths_m:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_multiway_junction_with_widths_and_profile_modes_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edge_profile_modes: &[GeneratedEdgeProfileMode],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_multiway_junction_graph_with_edge_widths_and_profile_modes(
        GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
        endpoint_angle_degrees,
        edge_widths_m,
        edge_profile_modes,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat mixed-width/profile multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_widths_m={edge_widths_m:?} edge_profile_modes={edge_profile_modes:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_flat_four_way_junction_raw_identity(
    first_branch_angle_degrees: f32,
    second_branch_angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let (graph, center) = generated_four_way_junction_graph(
        first_branch_angle_degrees,
        second_branch_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::UseAuthoredPoints,
        flat_generated_point_at_xz,
        flat_generated_edge_points,
    );
    let terrain = flat_terrain(2048, 2048);
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated flat 4-way JunctionN did not compile for raw identity; first_branch_angle_degrees={first_branch_angle_degrees} second_branch_angle_degrees={second_branch_angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_flat_junction_raised_geometry_invariants(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}
