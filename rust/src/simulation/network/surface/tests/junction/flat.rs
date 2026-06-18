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
    assert_no_unfaced_cross_material_height_boundaries(piece);
    assert_surface_no_unfaced_cross_material_height_boundaries(&surface);
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
            top_polygon_count: 159,
            carrier_record_count: 231,
            source_segment_record_count: 21,
            polygon_key_set_digest: 11037011028374031390,
            top_owner_height_field_digest: 621186394552791434,
            carrier_owner_source_height_field_digest: 913672194833674532,
            source_segment_id_digest: 6302080835280355820,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 3500000 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 682816, z_key: 3432748 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 682816, z_key: 3432748 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1339392, z_key: 3233578 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 1339392, z_key: 3233578 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1944496, z_key: 2910144 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 1944496, z_key: 2910144 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2474874, z_key: 2474874 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2474874, z_key: 2474874 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2910144, z_key: 1944496 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2910144, z_key: 1944496 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3233578, z_key: 1339392 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3233578, z_key: 1339392 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3432748, z_key: 682816 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3500000, z_key: 0 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3432748, z_key: 682816 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3432748, z_key: 682816 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3233578, z_key: 1339392 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3233578, z_key: 1339392 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2910144, z_key: 1944496 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2910144, z_key: 1944496 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2474874, z_key: 2474874 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2474874, z_key: 2474874 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1944496, z_key: 2910144 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1944496, z_key: 2910144 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1339392, z_key: 3233578 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1339392, z_key: 3233578 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -682816, z_key: 3432748 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3500000, z_key: 0 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3432749, z_key: -682816 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3432749, z_key: -682816 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3233578, z_key: -1339392 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3233578, z_key: -1339392 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2910144, z_key: -1944496 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2910144, z_key: -1944496 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2474874, z_key: -2474874 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2474874, z_key: -2474874 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1944496, z_key: -2910144 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1944496, z_key: -2910144 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1339392, z_key: -3233578 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1339392, z_key: -3233578 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -682816, z_key: -3432749 } }".to_owned(),
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
            top_polygon_count: 310,
            carrier_record_count: 412,
            source_segment_record_count: 23,
            polygon_key_set_digest: 9854829695880295285,
            top_owner_height_field_digest: 17442502495652634281,
            carrier_owner_source_height_field_digest: 5738464101469686050,
            source_segment_id_digest: 12389571840172255405,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 1149084, z_key: 3285733 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1261413, z_key: 3264787 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2271412, z_key: 2641303 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2353284, z_key: 2590763 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3098841, z_key: 1608083 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3128856, z_key: 1568522 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3469237, z_key: 386338 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3483886, z_key: 335460 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3469237, z_key: 386338 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3483886, z_key: 335460 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 9 }, source_kind: Carriageway, source_mouth_order_index: 1, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3416874, z_key: 664173 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3482910, z_key: 324444 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3486681, z_key: -305045 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3486007, z_key: 312651 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3486007, z_key: 312651 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3376755, z_key: 920609 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3376755, z_key: 920609 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3162328, z_key: 1499893 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3162328, z_key: 1499893 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2849404, z_key: 2032460 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2849404, z_key: 2032460 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2447730, z_key: 2501723 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2447730, z_key: 2501723 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1969817, z_key: 2893064 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1969817, z_key: 2893064 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1430550, z_key: 3194296 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1430550, z_key: 3194296 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -846726, z_key: 3396035 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3377431, z_key: -842088 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3288924, z_key: -1197070 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3288924, z_key: -1197070 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2968168, z_key: -1854717 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2968168, z_key: -1854717 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2517689, z_key: -2431304 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2517689, z_key: -2431304 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1957175, z_key: -2901631 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1957175, z_key: -2901631 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1311123, z_key: -3245143 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1311123, z_key: -3245143 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -607769, z_key: -3446827 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3465938, z_key: 487106 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3469237, z_key: 386338 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: CurbOrShoulder, owner_index: 22 }, source_kind: CurbOrShoulder, source_mouth_order_index: 3, source_band_index: 4, segment_start: NodeOwnedRegionArrangementKey { x_key: -6915640, z_key: -5331410 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 846726, z_key: -3396035 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 17 }, source_kind: Sidewalk, source_mouth_order_index: 2, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -4517612, z_key: 2142705 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -4070577, z_key: 2903515 } }".to_owned(),
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
            top_polygon_count: 384,
            carrier_record_count: 521,
            source_segment_record_count: 29,
            polygon_key_set_digest: 3666754348081216896,
            top_owner_height_field_digest: 5035324630783135359,
            carrier_owner_source_height_field_digest: 2664139000050933879,
            source_segment_id_digest: 13527816232291989855,
            source_segment_ids: vec![
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 1257250, z_key: 3254192 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 1318200, z_key: 3242275 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2241140, z_key: 2638608 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2442267, z_key: 2507057 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 2241140, z_key: 2638608 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2442267, z_key: 2507057 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3185840, z_key: 1432699 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3206658, z_key: 1402621 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 3 }, source_kind: Carriageway, source_mouth_order_index: 0, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 3483366, z_key: 0 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 3498801, z_key: 91620 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1958599, z_key: 2898151 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1944496, z_key: 2910143 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1225726, z_key: 3278352 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -441696, z_key: 3472017 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1225726, z_key: 3278352 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -441696, z_key: 3472017 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -441696, z_key: 3472017 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 6805, z_key: 3476910 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -441696, z_key: 3472017 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 6805, z_key: 3476910 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 15 }, source_kind: Carriageway, source_mouth_order_index: 2, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: 1880549, z_key: 2951870 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 2376488, z_key: 2548471 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3480020, z_key: 373443 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3479197, z_key: -381034 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3480020, z_key: 373443 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3479197, z_key: -381034 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3480020, z_key: 373443 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3319133, z_key: 1110566 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3479197, z_key: -381034 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3316702, z_key: -1117805 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3319133, z_key: 1110566 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -3004011, z_key: 1796084 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3004011, z_key: 1796084 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2554087, z_key: 2391799 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -3004011, z_key: 1796084 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2554087, z_key: 2391799 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1794293, z_key: 2986205 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1311123, z_key: 3245143 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 21 }, source_kind: Carriageway, source_mouth_order_index: 3, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1311123, z_key: 3245143 }, segment_end: NodeOwnedRegionArrangementKey { x_key: 0, z_key: 0 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2867032, z_key: -2007517 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2491019, z_key: -2458622 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2867032, z_key: -2007517 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2491019, z_key: -2458622 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2491019, z_key: -2458622 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -2044874, z_key: -2840508 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -2044874, z_key: -2840508 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1541158, z_key: -3142424 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -1541158, z_key: -3142424 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -994054, z_key: -3355869 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Carriageway, owner_index: 27 }, source_kind: Carriageway, source_mouth_order_index: 4, source_band_index: 3, segment_start: NodeOwnedRegionArrangementKey { x_key: -994054, z_key: -3355869 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -418963, z_key: -3474833 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -1777307, z_key: -4654731 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -1420077, z_key: -4794099 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -1420077, z_key: -4794099 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -598519, z_key: -4964048 } }".to_owned(),
                "NodeSourceCarrierSegmentId { owner: NodeBandOwner { kind: Sidewalk, owner_index: 29 }, source_kind: Sidewalk, source_mouth_order_index: 4, source_band_index: 5, segment_start: NodeOwnedRegionArrangementKey { x_key: -1420077, z_key: -4794099 }, segment_end: NodeOwnedRegionArrangementKey { x_key: -598519, z_key: -4964048 } }".to_owned(),
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
    assert_compiled_junction_piece(&surface, &graph, center);
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
    assert_compiled_junction_piece(&surface, &graph, center);
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
    assert_compiled_junction_piece(&surface, &graph, center);
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
    assert_compiled_junction_piece(&surface, &graph, center);
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
    assert_compiled_junction_piece(&surface, &graph, center);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}
