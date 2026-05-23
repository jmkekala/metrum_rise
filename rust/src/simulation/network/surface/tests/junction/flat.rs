//! Flat junction canonical pipeline tests.

use super::*;

const FLAT_CONFLICT_MATRIX_EDGE_LENGTH_M: f32 = 768.0;
const FLAT_CONFLICT_MATRIX_ANGLES_DEGREES: [f32; 7] = [1.0, 15.0, 30.0, 60.0, 90.0, 120.0, 150.0];

#[derive(Clone, Copy, Debug)]
enum GeneratedEdgeDirection {
    FromCenter,
    ToCenter,
}

#[derive(Clone, Copy, Debug)]
enum GeneratedEditOrder {
    Forward,
    Reverse,
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
fn flat_bend_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in FLAT_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_bend(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_t_junction_angle_matrix_compiles_conflict_first_owned_regions() {
    for angle_degrees in FLAT_CONFLICT_MATRIX_ANGLES_DEGREES {
        compile_generated_flat_t_junction(
            angle_degrees,
            GeneratedEdgeDirection::FromCenter,
            GeneratedEditOrder::Forward,
        );
    }
}

#[test]
fn flat_bend_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    compile_generated_flat_bend(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
}

#[test]
fn flat_t_junction_reversed_edge_direction_compiles_conflict_first_owned_regions() {
    compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    compile_generated_flat_t_junction(
        30.0,
        GeneratedEdgeDirection::ToCenter,
        GeneratedEditOrder::Forward,
    );
}

#[test]
fn flat_bend_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    compile_generated_flat_bend(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
}

#[test]
fn flat_t_junction_equivalent_edit_order_compiles_conflict_first_owned_regions() {
    compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Forward,
    );
    compile_generated_flat_t_junction(
        60.0,
        GeneratedEdgeDirection::FromCenter,
        GeneratedEditOrder::Reverse,
    );
}

fn compile_generated_flat_bend(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> Vec<NodePolygonSignature> {
    let (graph, center) = generated_flat_bend_graph(angle_degrees, edge_direction, edit_order);
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
    node_piece_polygon_signature(piece)
}

fn compile_generated_flat_t_junction(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> Vec<NodePolygonSignature> {
    let (graph, center) =
        generated_flat_t_junction_graph(angle_degrees, edge_direction, edit_order);
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
    node_piece_polygon_signature(piece)
}

fn generated_flat_bend_graph(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> (RegionGraph, u32) {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let first_pos = point_at_angle_degrees(0.0);
    let second_pos = point_at_angle_degrees(angle_degrees);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let first = graph.add_node(first_pos, NodeType::Junction);
    let second = graph.add_node(second_pos, NodeType::Junction);

    add_generated_center_edges(
        &mut graph,
        center,
        center_pos,
        &[(first, first_pos), (second, second_pos)],
        edge_direction,
        edit_order,
    );
    graph.rebuild_intersection_clips();

    (graph, center)
}

fn generated_flat_t_junction_graph(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) -> (RegionGraph, u32) {
    let mut graph = RegionGraph::new();
    let center_pos = Vector3::new(0.0, 0.0, 0.0);
    let west_pos = point_at_angle_degrees(180.0);
    let east_pos = point_at_angle_degrees(0.0);
    let branch_pos = point_at_angle_degrees(angle_degrees);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let west = graph.add_node(west_pos, NodeType::Junction);
    let east = graph.add_node(east_pos, NodeType::Junction);
    let branch = graph.add_node(branch_pos, NodeType::Junction);

    add_generated_center_edges(
        &mut graph,
        center,
        center_pos,
        &[(west, west_pos), (east, east_pos), (branch, branch_pos)],
        edge_direction,
        edit_order,
    );
    graph.rebuild_intersection_clips();

    (graph, center)
}

fn point_at_angle_degrees(angle_degrees: f32) -> Vector3 {
    let angle_radians = angle_degrees.to_radians();
    Vector3::new(
        angle_radians.cos() * FLAT_CONFLICT_MATRIX_EDGE_LENGTH_M,
        0.0,
        angle_radians.sin() * FLAT_CONFLICT_MATRIX_EDGE_LENGTH_M,
    )
}

fn add_generated_center_edges(
    graph: &mut RegionGraph,
    center: u32,
    center_pos: Vector3,
    endpoints: &[(u32, Vector3)],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
) {
    match edit_order {
        GeneratedEditOrder::Forward => {
            for &(endpoint, endpoint_pos) in endpoints {
                add_generated_center_edge(
                    graph,
                    center,
                    center_pos,
                    endpoint,
                    endpoint_pos,
                    edge_direction,
                );
            }
        }
        GeneratedEditOrder::Reverse => {
            for &(endpoint, endpoint_pos) in endpoints.iter().rev() {
                add_generated_center_edge(
                    graph,
                    center,
                    center_pos,
                    endpoint,
                    endpoint_pos,
                    edge_direction,
                );
            }
        }
    }
}

fn add_generated_center_edge(
    graph: &mut RegionGraph,
    center: u32,
    center_pos: Vector3,
    endpoint: u32,
    endpoint_pos: Vector3,
    edge_direction: GeneratedEdgeDirection,
) {
    let (start, end, points) = match edge_direction {
        GeneratedEdgeDirection::FromCenter => (center, endpoint, vec![center_pos, endpoint_pos]),
        GeneratedEdgeDirection::ToCenter => (endpoint, center, vec![endpoint_pos, center_pos]),
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

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodePolygonSignature {
    layer: &'static str,
    points: Vec<(i64, i64, i64)>,
}

fn node_piece_polygon_signature(piece: &RoadSurfaceVisualNodePiece) -> Vec<NodePolygonSignature> {
    let mut signature = Vec::new();
    append_polygon_signatures(&mut signature, "outer", &piece.outer_boundary_loops);
    append_polygon_signatures(&mut signature, "asphalt", &piece.road_surface_polygons);
    append_polygon_signatures(&mut signature, "curb", &piece.curb_surface_polygons);
    append_polygon_signatures(&mut signature, "sidewalk", &piece.sidewalk_surface_polygons);
    append_polygon_signatures(
        &mut signature,
        "raised_step",
        &piece.raised_step_face_polygons,
    );
    signature.sort();
    signature
}

fn append_polygon_signatures(
    target: &mut Vec<NodePolygonSignature>,
    layer: &'static str,
    polygons: &[RoadSurfaceVisualPolygon],
) {
    target.extend(polygons.iter().map(|polygon| NodePolygonSignature {
        layer,
        points: canonical_polygon_point_keys(polygon),
    }));
}

fn canonical_polygon_point_keys(polygon: &RoadSurfaceVisualPolygon) -> Vec<(i64, i64, i64)> {
    let mut points = polygon
        .points_world
        .iter()
        .map(|&point| {
            let (x_key, z_key) = test_xz_key(point);
            let y_mm = (point.y * 1000.0).round() as i64;
            (x_key, y_mm, z_key)
        })
        .collect::<Vec<_>>();
    if points.len() >= 2 && points.first() == points.last() {
        points.pop();
    }

    let forward = rotate_polygon_key_start_to_min(points.clone());
    points.reverse();
    let reverse = rotate_polygon_key_start_to_min(points);
    forward.min(reverse)
}

fn rotate_polygon_key_start_to_min(mut points: Vec<(i64, i64, i64)>) -> Vec<(i64, i64, i64)> {
    if let Some((start_index, _)) = points.iter().enumerate().min_by(|(_, a), (_, b)| a.cmp(b)) {
        points.rotate_left(start_index);
    }
    points
}
