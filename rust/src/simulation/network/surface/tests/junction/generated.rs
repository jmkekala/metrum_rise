//! Generated Bend and JunctionN conflict-matrix fixtures.

use super::*;
use crate::simulation::network::surface::NodeOverlayShape;

pub(in crate::simulation::network::surface::tests::junction) const GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M:
    f32 = 768.0;
pub(in crate::simulation::network::surface::tests::junction) const GENERATED_CONFLICT_MATRIX_ANGLES_DEGREES:
    [f32; 7] =
    [1.0, 15.0, 30.0, 60.0, 90.0, 120.0, 150.0];

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests::junction) enum GeneratedEdgeDirection {
    FromCenter,
    ToCenter,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests::junction) enum GeneratedEditOrder {
    Forward,
    Reverse,
}

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests::junction) enum GeneratedEndpointProfileMode {
    UseAuthoredPoints,
    SolveJunctionEndpointProfiles,
}

#[derive(Clone, Debug)]
pub(in crate::simulation::network::surface::tests::junction) struct GeneratedNodeCanonicalSignature
{
    kind: RoadSurfaceVisualNodePieceKind,
    coverage: Vec<GeneratedMaterialCoverage>,
}

#[derive(Clone, Debug)]
struct GeneratedMaterialCoverage {
    label: &'static str,
    shapes: NodeOverlayShapes,
}

pub(in crate::simulation::network::surface::tests::junction) fn assert_generated_node_canonical_signature_eq(
    left_label: &str,
    left: &GeneratedNodeCanonicalSignature,
    right_label: &str,
    right: &GeneratedNodeCanonicalSignature,
) {
    if left.kind != right.kind {
        panic!(
            "generated node kinds differ: {left_label}={:?} {right_label}={:?}",
            left.kind, right.kind
        );
    }

    let mut failures = Vec::new();
    for (left_coverage, right_coverage) in left.coverage.iter().zip(&right.coverage) {
        if left_coverage.label != right_coverage.label {
            failures.push(format!(
                "label_mismatch:{}:{}",
                left_coverage.label, right_coverage.label
            ));
            continue;
        }
        let left_minus_right = RoadSurfaceSystem::overlay_binary_shapes(
            &left_coverage.shapes,
            &right_coverage.shapes,
            OverlayRule::Difference,
        )
        .expect("generated coverage difference overlay should succeed");
        let right_minus_left = RoadSurfaceSystem::overlay_binary_shapes(
            &right_coverage.shapes,
            &left_coverage.shapes,
            OverlayRule::Difference,
        )
        .expect("generated coverage difference overlay should succeed");
        let left_only_m2 = overlay_area_m2(&left_minus_right);
        let right_only_m2 = overlay_area_m2(&right_minus_left);
        let budget_m2 =
            RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&left_coverage.shapes).max(
                RoadSurfaceSystem::overlay_numeric_area_budget_for_shapes(&right_coverage.shapes),
            );
        if left_only_m2 > budget_m2 || right_only_m2 > budget_m2 {
            failures.push(format!(
                "{}: left_only_m2={left_only_m2:.6} right_only_m2={right_only_m2:.6} budget_m2={budget_m2:.6} {left_label}_shapes={} {right_label}_shapes={}",
                left_coverage.label,
                generated_shapes_summary(&left_coverage.shapes),
                generated_shapes_summary(&right_coverage.shapes)
            ));
        }
    }
    if left.coverage.len() != right.coverage.len() {
        failures.push(format!(
            "coverage_len_mismatch:{}:{}",
            left.coverage.len(),
            right.coverage.len()
        ));
    }
    if !failures.is_empty() {
        panic!(
            "generated node canonical material coverage differs: {left_label} vs {right_label}: {}",
            failures.join(" | ")
        );
    }
}

pub(in crate::simulation::network::surface::tests::junction) fn generated_node_canonical_signature(
    piece: &RoadSurfaceVisualNodePiece,
) -> GeneratedNodeCanonicalSignature {
    GeneratedNodeCanonicalSignature {
        kind: piece.kind,
        coverage: vec![
            generated_material_coverage(
                "outer",
                overlay_contours_from_polygons(&piece.outer_boundary_loops),
            ),
            generated_material_coverage(
                "road",
                overlay_contours_from_top_polygons(&piece.road_surface_polygons),
            ),
            generated_material_coverage(
                "curb",
                overlay_contours_from_top_polygons(&piece.curb_surface_polygons),
            ),
            generated_material_coverage(
                "sidewalk",
                overlay_contours_from_top_polygons(&piece.sidewalk_surface_polygons),
            ),
        ],
    }
}

fn generated_material_coverage(
    label: &'static str,
    contours: Vec<NodeOverlayContour>,
) -> GeneratedMaterialCoverage {
    let mut shapes =
        RoadSurfaceSystem::overlay_union_contours(&contours).expect("generated coverage union");
    RoadSurfaceSystem::sort_overlay_shapes(&mut shapes);
    GeneratedMaterialCoverage { label, shapes }
}

fn generated_shapes_summary(shapes: &NodeOverlayShapes) -> String {
    shapes
        .iter()
        .enumerate()
        .map(|(shape_index, shape)| {
            format!(
                "shape={shape_index}:area_mm2={}:{}",
                (RoadSurfaceSystem::overlay_shape_area_m2(shape) * 1_000_000.0).round() as i64,
                generated_overlay_shape_signature(shape)
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn generated_overlay_shape_signature(shape: &NodeOverlayShape) -> String {
    shape
        .iter()
        .enumerate()
        .map(|(contour_index, contour)| {
            let points = contour
                .iter()
                .map(|point| {
                    (
                        (point[0] * 1000.0).round() as i64,
                        (point[1] * 1000.0).round() as i64,
                    )
                })
                .collect::<Vec<_>>();
            format!("contour={contour_index}:{points:?}")
        })
        .collect::<Vec<_>>()
        .join(";")
}

pub(in crate::simulation::network::surface::tests::junction) fn generated_bend_graph<P, E>(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    endpoint_profile_mode: GeneratedEndpointProfileMode,
    mut point_at_xz: P,
    mut edge_points: E,
) -> (RegionGraph, u32)
where
    P: FnMut(Vector2) -> Vector3,
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    let center_xz = Vector2::ZERO;
    let first_xz = point_at_angle_degrees(0.0);
    let second_xz = point_at_angle_degrees(angle_degrees);
    generated_center_graph(
        center_xz,
        &[first_xz, second_xz],
        edge_direction,
        edit_order,
        endpoint_profile_mode,
        &mut point_at_xz,
        &mut edge_points,
    )
}

pub(in crate::simulation::network::surface::tests::junction) fn generated_t_junction_graph<P, E>(
    angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    endpoint_profile_mode: GeneratedEndpointProfileMode,
    mut point_at_xz: P,
    mut edge_points: E,
) -> (RegionGraph, u32)
where
    P: FnMut(Vector2) -> Vector3,
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    let center_xz = Vector2::ZERO;
    let west_xz = point_at_angle_degrees(180.0);
    let east_xz = point_at_angle_degrees(0.0);
    let branch_xz = point_at_angle_degrees(angle_degrees);
    generated_center_graph(
        center_xz,
        &[west_xz, east_xz, branch_xz],
        edge_direction,
        edit_order,
        endpoint_profile_mode,
        &mut point_at_xz,
        &mut edge_points,
    )
}

pub(in crate::simulation::network::surface::tests::junction) fn generated_four_way_junction_graph<
    P,
    E,
>(
    first_branch_angle_degrees: f32,
    second_branch_angle_degrees: f32,
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    endpoint_profile_mode: GeneratedEndpointProfileMode,
    mut point_at_xz: P,
    mut edge_points: E,
) -> (RegionGraph, u32)
where
    P: FnMut(Vector2) -> Vector3,
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    let center_xz = Vector2::ZERO;
    let west_xz = point_at_angle_degrees(180.0);
    let east_xz = point_at_angle_degrees(0.0);
    let first_branch_xz = point_at_angle_degrees(first_branch_angle_degrees);
    let second_branch_xz = point_at_angle_degrees(second_branch_angle_degrees);
    generated_center_graph(
        center_xz,
        &[west_xz, east_xz, first_branch_xz, second_branch_xz],
        edge_direction,
        edit_order,
        endpoint_profile_mode,
        &mut point_at_xz,
        &mut edge_points,
    )
}

pub(in crate::simulation::network::surface::tests::junction) fn generated_multiway_junction_graph<
    P,
    E,
>(
    endpoint_angle_degrees: &[f32],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    endpoint_profile_mode: GeneratedEndpointProfileMode,
    mut point_at_xz: P,
    mut edge_points: E,
) -> (RegionGraph, u32)
where
    P: FnMut(Vector2) -> Vector3,
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    assert!(
        endpoint_angle_degrees.len() >= 3,
        "generated multiway JunctionN fixtures require at least three endpoint angles"
    );
    let center_xz = Vector2::ZERO;
    let endpoint_xzs = endpoint_angle_degrees
        .iter()
        .copied()
        .map(point_at_angle_degrees)
        .collect::<Vec<_>>();
    generated_center_graph(
        center_xz,
        &endpoint_xzs,
        edge_direction,
        edit_order,
        endpoint_profile_mode,
        &mut point_at_xz,
        &mut edge_points,
    )
}

fn generated_center_graph<P, E>(
    center_xz: Vector2,
    endpoint_xzs: &[Vector2],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    endpoint_profile_mode: GeneratedEndpointProfileMode,
    point_at_xz: &mut P,
    edge_points: &mut E,
) -> (RegionGraph, u32)
where
    P: FnMut(Vector2) -> Vector3,
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    let mut graph = RegionGraph::new();
    let center_pos = point_at_xz(center_xz);
    let center = graph.add_node(center_pos, NodeType::Junction);
    let endpoints = endpoint_xzs
        .iter()
        .copied()
        .map(|endpoint_xz| {
            let endpoint_pos = point_at_xz(endpoint_xz);
            let endpoint = graph.add_node(endpoint_pos, NodeType::Junction);
            (endpoint, endpoint_xz)
        })
        .collect::<Vec<_>>();

    add_generated_center_edges(
        &mut graph,
        center,
        center_xz,
        &endpoints,
        edge_direction,
        edit_order,
        edge_points,
    );
    if matches!(
        endpoint_profile_mode,
        GeneratedEndpointProfileMode::SolveJunctionEndpointProfiles
    ) {
        graph.rebuild_adjacency_list();
        let adaptable_edges = (0..graph.edge_count()).collect::<HashSet<_>>();
        graph
            .solve_junction_endpoint_profiles_for_edges(&HashSet::from([center]), &adaptable_edges);
    }
    graph.rebuild_intersection_clips();

    (graph, center)
}

fn point_at_angle_degrees(angle_degrees: f32) -> Vector2 {
    let angle_radians = angle_degrees.to_radians();
    Vector2::new(
        angle_radians.cos() * GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
        angle_radians.sin() * GENERATED_CONFLICT_MATRIX_EDGE_LENGTH_M,
    )
}

fn add_generated_center_edges<E>(
    graph: &mut RegionGraph,
    center: u32,
    center_xz: Vector2,
    endpoints: &[(u32, Vector2)],
    edge_direction: GeneratedEdgeDirection,
    edit_order: GeneratedEditOrder,
    edge_points: &mut E,
) where
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    match edit_order {
        GeneratedEditOrder::Forward => {
            for &(endpoint, endpoint_xz) in endpoints {
                add_generated_center_edge(
                    graph,
                    center,
                    center_xz,
                    endpoint,
                    endpoint_xz,
                    edge_direction,
                    edge_points,
                );
            }
        }
        GeneratedEditOrder::Reverse => {
            for &(endpoint, endpoint_xz) in endpoints.iter().rev() {
                add_generated_center_edge(
                    graph,
                    center,
                    center_xz,
                    endpoint,
                    endpoint_xz,
                    edge_direction,
                    edge_points,
                );
            }
        }
    }
}

fn add_generated_center_edge<E>(
    graph: &mut RegionGraph,
    center: u32,
    center_xz: Vector2,
    endpoint: u32,
    endpoint_xz: Vector2,
    edge_direction: GeneratedEdgeDirection,
    edge_points: &mut E,
) where
    E: FnMut(Vector2, Vector2) -> Vec<Vector3>,
{
    let center_to_endpoint = edge_points(center_xz, endpoint_xz);
    let (start, end, points) = match edge_direction {
        GeneratedEdgeDirection::FromCenter => (center, endpoint, center_to_endpoint),
        GeneratedEdgeDirection::ToCenter => {
            let mut endpoint_to_center = center_to_endpoint;
            endpoint_to_center.reverse();
            (endpoint, center, endpoint_to_center)
        }
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

pub(in crate::simulation::network::surface::tests::junction) fn flat_generated_point_at_xz(
    xz: Vector2,
) -> Vector3 {
    Vector3::new(xz.x, 0.0, xz.y)
}

pub(in crate::simulation::network::surface::tests::junction) fn flat_generated_edge_points(
    start_xz: Vector2,
    end_xz: Vector2,
) -> Vec<Vector3> {
    vec![
        flat_generated_point_at_xz(start_xz),
        flat_generated_point_at_xz(end_xz),
    ]
}
