//! Elevated generated Bend and JunctionN conflict-matrix tests.

use super::*;

#[test]
fn elevated_bend_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_elevated_bend(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn elevated_t_junction_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_elevated_t_junction(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn elevated_four_way_junction_matrix_compiles_conflict_first_owned_regions() {
    for endpoint_angle_degrees in [
        [0.0, 90.0, 180.0, 270.0],
        [0.0, 15.0, 96.0, 181.0],
        [0.0, 73.0, 180.0, 244.0],
    ] {
        compile_generated_elevated_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn elevated_arbitrary_multiway_junction_matrix_compiles_conflict_first_owned_regions() {
    for endpoint_angle_degrees in [
        [0.0, 14.0, 95.0, 205.0, 300.0],
        [0.0, 45.0, 120.0, 205.0, 300.0],
        [0.0, 65.0, 145.0, 230.0, 310.0],
    ] {
        compile_generated_elevated_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
    for endpoint_angle_degrees in [
        [0.0, 12.0, 60.0, 145.0, 230.0, 310.0],
        [0.0, 40.0, 105.0, 170.0, 235.0, 310.0],
        [0.0, 60.0, 120.0, 180.0, 240.0, 300.0],
    ] {
        compile_generated_elevated_multiway_junction(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn elevated_t_junction_acute_angle_compiles_conflict_first_owned_regions() {
    compile_generated_elevated_t_junction(
        1.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
}

#[test]
fn elevated_t_junction_right_angle_compiles_conflict_first_owned_regions() {
    compile_generated_elevated_t_junction(
        90.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
}

#[test]
fn elevated_t_junction_obtuse_angle_compiles_conflict_first_owned_regions() {
    compile_generated_elevated_t_junction(
        150.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
}

#[test]
fn elevated_bend_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_elevated_bend(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_elevated_bend(
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
fn elevated_t_junction_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    let from_center = compile_generated_elevated_t_junction(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let to_center = compile_generated_elevated_t_junction(
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
fn elevated_bend_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_elevated_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_elevated_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn elevated_t_junction_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    let forward = compile_generated_elevated_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_elevated_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_generated_node_canonical_signature_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn elevated_four_way_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity() {
    let endpoint_angle_degrees = [0.0, 73.0, 180.0, 244.0];
    let forward = compile_generated_elevated_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    let reverse = compile_generated_elevated_multiway_junction_raw_identity(
        &endpoint_angle_degrees,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
    assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
}

#[test]
fn elevated_arbitrary_multiway_junction_equivalent_edit_order_preserves_exact_raw_polygon_identity()
{
    for endpoint_angle_degrees in [
        [0.0, 14.0, 95.0, 205.0, 300.0],
        [0.0, 45.0, 120.0, 205.0, 300.0],
        [0.0, 65.0, 145.0, 230.0, 310.0],
    ] {
        let forward = compile_generated_elevated_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_elevated_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for endpoint_angle_degrees in [
        [0.0, 12.0, 60.0, 145.0, 230.0, 310.0],
        [0.0, 40.0, 105.0, 170.0, 235.0, 310.0],
        [0.0, 60.0, 120.0, 180.0, 240.0, 300.0],
    ] {
        let forward = compile_generated_elevated_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_elevated_multiway_junction_raw_identity(
            &endpoint_angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

#[test]
fn elevated_mixed_width_junction_matrix_preserves_exact_raw_polygon_identity() {
    for (endpoint_angle_degrees, edge_widths_m) in [
        ([0.0, 90.0, 180.0, 270.0], [6.0, 9.0, 7.5, 11.0]),
        ([0.0, 73.0, 180.0, 244.0], [7.0, 10.5, 5.5, 8.75]),
    ] {
        let forward = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m) in
        [([0.0, 14.0, 95.0, 205.0, 300.0], [7.0, 12.0, 5.5, 8.0, 10.0])]
    {
        let forward = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
    for (endpoint_angle_degrees, edge_widths_m) in [(
        [0.0, 12.0, 60.0, 145.0, 230.0, 310.0],
        [6.5, 9.0, 5.0, 11.0, 8.0, 7.5],
    )] {
        let forward = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Forward,
        );
        let reverse = compile_generated_elevated_multiway_junction_with_widths_raw_identity(
            &endpoint_angle_degrees,
            &edge_widths_m,
            GeneratedEditOrder::Reverse,
        );
        assert_canonical_node_raw_polygon_identity_eq("forward", &forward, "reverse", &reverse);
    }
}

fn compile_generated_elevated_bend(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let terrain = generated_elevated_planar_terrain();
    let (graph, center) = generated_bend_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
        |xz| generated_elevated_point_at_xz(&terrain, xz),
        |start_xz, end_xz| generated_elevated_edge_points(&terrain, start_xz, end_xz),
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated elevated Bend did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_node_pipeline_report(
                &surface,
                &graph,
                center,
                RoadSurfaceVisualNodePieceKind::Bend
            )
        );
    }
    let piece = assert_compiled_bend_piece(&surface, &graph, center);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    generated_node_canonical_signature(piece)
}

fn compile_generated_elevated_t_junction(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let terrain = generated_elevated_planar_terrain();
    let (graph, center) = generated_t_junction_graph(
        angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
        |xz| generated_elevated_point_at_xz(&terrain, xz),
        |start_xz, end_xz| generated_elevated_edge_points(&terrain, start_xz, end_xz),
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated elevated JunctionN did not compile; angle_degrees={angle_degrees} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    generated_node_canonical_signature(piece)
}

fn compile_generated_elevated_multiway_junction(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> GeneratedNodeCanonicalSignature {
    let terrain = generated_elevated_planar_terrain();
    let (graph, center) = generated_multiway_junction_graph_with_edge_length(
        96.0,
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
        |xz| generated_elevated_point_at_xz(&terrain, xz),
        |start_xz, end_xz| generated_elevated_edge_points(&terrain, start_xz, end_xz),
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated elevated multiway JunctionN did not compile; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    generated_node_canonical_signature(piece)
}

fn compile_generated_elevated_multiway_junction_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let terrain = generated_elevated_planar_terrain();
    let (graph, center) = generated_multiway_junction_graph_with_edge_length(
        96.0,
        endpoint_angle_degrees,
        edge_direction,
        edit_order,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
        |xz| generated_elevated_point_at_xz(&terrain, xz),
        |start_xz, end_xz| generated_elevated_edge_points(&terrain, start_xz, end_xz),
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated elevated multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_direction={edge_direction:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn compile_generated_elevated_multiway_junction_with_widths_raw_identity(
    endpoint_angle_degrees: &[f32],
    edge_widths_m: &[f32],
    edit_order: GeneratedEditOrder,
) -> CanonicalNodeRawPolygonIdentity {
    let terrain = generated_elevated_planar_terrain();
    let (graph, center) = generated_multiway_junction_graph_with_edge_widths(
        96.0,
        endpoint_angle_degrees,
        edge_widths_m,
        GeneratedEdgeDirection::FromCenter,
        edit_order,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles,
        |xz| generated_elevated_point_at_xz(&terrain, xz),
        |start_xz, end_xz| generated_elevated_edge_points(&terrain, start_xz, end_xz),
    );
    let mut surface = RoadSurfaceSystem::new(16.0);
    surface.compile_dirty(&graph, &terrain);

    if !surface.compiled_visual_node_pieces().contains_key(&center) {
        panic!(
            "generated elevated mixed-width multiway JunctionN did not compile for raw identity; endpoint_angle_degrees={endpoint_angle_degrees:?} edge_widths_m={edge_widths_m:?} edit_order={edit_order:?}: {}",
            canonical_junction_pipeline_report(&surface, &graph, center)
        );
    }
    let piece = assert_compiled_junction_piece(&surface, &graph, center);
    assert_node_earthwork_faces_have_footprint_provenance(piece);
    canonical_node_raw_polygon_identity(&surface, &graph, center)
}

fn generated_elevated_planar_terrain() -> TerrainSystem {
    planar_world_terrain(1601, 1601, 1.0, 150.0, 0.035, -0.014)
}

fn generated_elevated_point_at_xz(terrain: &TerrainSystem, xz: Vector2) -> Vector3 {
    Vector3::new(
        xz.x,
        terrain.sample_height_world(xz.x, xz.y) * crate::config::HEIGHT_SCALE,
        xz.y,
    )
}

fn generated_elevated_edge_points(
    terrain: &TerrainSystem,
    start_xz: Vector2,
    end_xz: Vector2,
) -> Vec<Vector3> {
    grounded_polyline_points_from_terrain(terrain, start_xz, end_xz, 24)
}
