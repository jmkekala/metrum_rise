use super::*;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use godot::prelude::Vector3;

fn create_test_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3, width: f32) -> Edge {
    Edge {
        start_node: n1, end_node: n2, primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT, class: EdgeClass::Standard,
        width, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 13.0, physical_length: (p2 - p1).length(),
        geometry: vec![p1, p2], physical_geometry: vec![p1, p2], deleted: false, ..Default::default()
    }
}

fn create_test_walkway(n1: u32, n2: u32, p1: Vector3, p2: Vector3) -> Edge {
    Edge {
        start_node: n1, end_node: n2, primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT, class: EdgeClass::Standard,
        width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 1.5,
        physical_length: (p2 - p1).length(), geometry: vec![p1, p2],
        physical_geometry: vec![p1, p2], deleted: false, ..Default::default()
    }
}

#[test]
fn collinear_grade_change_keeps_constant_side_vector() {
    let points = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 5.0, 0.0), Vector3::new(20.0, 10.0, 0.0)];
    let side = road_strip::polyline_side_at(&points, 1);
    assert!((side.x - 0.0).abs() < 0.001);
    assert!((side.y - 1.0).abs() < 0.001);
}

#[test]
fn horizontal_bend_expands_outer_join() {
    let points = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0), Vector3::new(10.0, 5.0, 10.0)];
    let side = road_strip::polyline_side_at(&points, 1);
    assert!(side.length() > 1.3);
}

#[test]
fn straight_width_change_is_not_rendered_as_junction_disk() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    graph.add_edge(create_test_edge(n0, n1, Vector3::new(-25.0, 0.0, 0.0), Vector3::ZERO, 7.0));
    graph.add_edge(create_test_edge(n1, n2, Vector3::ZERO, Vector3::new(25.0, 0.0, 0.0), 14.0));
    let node_states = build_node_render_states(&graph);
    assert_eq!(node_states.get(&n1).map(|s| s.kind), Some(NodeRenderKind::WidthTransition));
}

#[test]
fn road_walkway_connection_keeps_sidewalk_pass_through() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(-10.0, 0.0, -10.0), NodeType::Junction);
    graph.add_edge(create_test_edge(n0, n1, Vector3::new(-25.0, 0.0, 0.0), Vector3::ZERO, 7.0));
    graph.add_edge(create_test_edge(n1, n2, Vector3::ZERO, Vector3::new(25.0, 0.0, 0.0), 7.0));
    graph.add_edge(create_test_walkway(n3, n1, Vector3::new(-10.0, 0.0, -10.0), Vector3::ZERO));
    let sidewalk_incs = build_sidewalk_node_incidents(&graph);
    let road_states = build_node_render_states(&graph);
    let sidewalk_states = build_sidewalk_node_render_states(&graph, &sidewalk_incs);
    assert_eq!(road_states.get(&n1).map(|s| s.kind), Some(NodeRenderKind::PassThrough));
    assert_eq!(sidewalk_states.get(&n1).map(|s| s.kind), Some(NodeRenderKind::PassThrough));
    assert!(!junction_fill::node_uses_polygon_fill(&graph, &sidewalk_states, &sidewalk_incs, n1));
}

#[test]
fn sidewalk_boundary_collapse_prefers_outer_points_on_shared_rays() {
    let center = Vector3::ZERO;
    let mut boundary = vec![
        BoundaryPoint { angle: -2.3561945, point: Vector3::new(-5.0, 0.0, -5.0) },
        BoundaryPoint { angle: -2.3561945, point: Vector3::new(-1.4, 0.0, -1.4) },
        BoundaryPoint { angle: -0.7853982, point: Vector3::new(1.4, 0.0, -1.4) },
        BoundaryPoint { angle: -0.7853982, point: Vector3::new(5.0, 0.0, -5.0) },
    ];
    junction_fill::collapse_boundary_rays(&mut boundary, center);
    assert_eq!(boundary.len(), 2);
    assert!(boundary.iter().any(|p| p.point.distance_squared_to(Vector3::new(-5.0, 0.0, -5.0)) < 0.001));
    assert!(boundary.iter().any(|p| p.point.distance_squared_to(Vector3::new(5.0, 0.0, -5.0)) < 0.001));
}

#[test]
fn sidewalk_pass_through_node_trims_walkway_but_not_road_shoulders() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(0.0, 0.0, -12.0), NodeType::Junction);
    graph.add_edge(create_test_edge(n0, n1, Vector3::new(-25.0, 0.0, 0.0), Vector3::ZERO, 7.0));
    graph.add_edge(create_test_edge(n1, n2, Vector3::ZERO, Vector3::new(25.0, 0.0, 0.0), 7.0));
    graph.add_edge(create_test_walkway(n3, n1, Vector3::new(0.0, 0.0, -12.0), Vector3::ZERO));
    let sidewalk_incs = build_sidewalk_node_incidents(&graph);
    let sidewalk_states = build_sidewalk_node_render_states(&graph, &sidewalk_incs);
    assert_eq!(junction_fill::edge_endpoint_trim_distance(&graph, &sidewalk_states, &sidewalk_incs, &graph.edges[0], false, sidewalk_surface_half_width(&graph.edges[0])), 0.0);
    assert!(junction_fill::edge_endpoint_trim_distance(&graph, &sidewalk_states, &sidewalk_incs, &graph.edges[2], false, sidewalk_surface_half_width(&graph.edges[2])) >= 4.9);
}

#[test]
fn sidewalk_pass_through_node_builds_apron_polygon_for_centered_walkway() {
    let mut graph = RegionGraph::new();
    let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(0.0, 0.0, -12.0), NodeType::Junction);
    graph.add_edge(create_test_edge(n0, n1, Vector3::new(-25.0, 0.0, 0.0), Vector3::ZERO, 7.0));
    graph.add_edge(create_test_edge(n1, n2, Vector3::ZERO, Vector3::new(25.0, 0.0, 0.0), 7.0));
    graph.add_edge(create_test_walkway(n3, n1, Vector3::new(0.0, 0.0, -12.0), Vector3::ZERO));
    let sidewalk_incs = build_sidewalk_node_incidents(&graph);
    let sidewalk_states = build_sidewalk_node_render_states(&graph, &sidewalk_incs);
    let incs = sidewalk_incs.get(&n1).unwrap();
    let (road_incs, foot_incs) = junction_fill::sidewalk_pass_through_components(&graph, incs).unwrap();
    let polygon = junction_fill::sidewalk_apron_polygon(&graph, n1, &road_incs, foot_incs[0], *sidewalk_states.get(&n1).unwrap()).unwrap();
    for expected in [Vector3::new(-5.0, 0.0, -3.5), Vector3::new(5.0, 0.0, -3.5), Vector3::new(-1.4, 0.0, -5.0), Vector3::new(1.4, 0.0, -5.0)] {
        assert!(polygon.iter().any(|p: &Vector3| p.distance_squared_to(expected) < 0.05));
    }
}
