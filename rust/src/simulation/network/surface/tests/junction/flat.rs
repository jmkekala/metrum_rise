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
fn flat_bend_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_bend(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_t_junction_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_t_junction(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
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
