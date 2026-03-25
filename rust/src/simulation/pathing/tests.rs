use super::*;
use crate::simulation::network::graph::{TransitGraph, Edge};
use crate::simulation::network::types::{TransitType, TransitFlags, NodeType};
use crate::simulation::pathing::hpa::HpaGraph;
use godot::prelude::Vector3;

#[test]
fn test_slope_cost_calculation() {
    let mut edge = Edge {
        start_node: 0,
        end_node: 1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 4.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 0.0,
        physical_length: 100.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        physical_geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
        zoning_left: false,
        zoning_right: false,
        deleted: false,
    };

    let (flat_cost, _) = cost::CostCalculator::calculate_costs(&edge);
    
    // Add a hill: 100m length, 50m height (50% slope)
    edge.geometry[1] = Vector3::new(100.0, 50.0, 0.0);
    edge.physical_geometry[1] = Vector3::new(100.0, 50.0, 0.0);
    let (steep_cost, _) = cost::CostCalculator::calculate_costs(&edge);

    assert!(steep_cost > flat_cost * 2.0, "Steep route cost ({}) should heavily penalize over flat route cost ({})", steep_cost, flat_cost);
}

#[test]
fn test_pathing_avoids_steep_slope() {
    let mut graph = TransitGraph::new();
    // A (0,0,0)
    let n_a = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    // B (100,0,0) - Goal
    let n_b = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
    // C (50,-50,0) - Detour point
    let n_c = graph.add_node(Vector3::new(50.0, -50.0, 0.0), NodeType::Junction);

    // 1. Short but very steep road: A -> B
    // Slope = 50m height / 100m length = 50%.
    // Cost should be massive due to exponential penalty: 1 + (0.5 * 5)^2 = 1 + 2.5^2 = 7.25x multiplier.
    graph.add_edge(Edge {
        start_node: n_a, end_node: n_b,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
        width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 0.0,
        physical_length: 100.0, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 50.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 50.0, 0.0)],
        zoning_left: false, zoning_right: false, deleted: false,
    });

    // 2. Long but flat detour: A -> C -> B
    // A -> C (70.7m)
    graph.add_edge(Edge {
        start_node: n_a, end_node: n_c,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
        width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 0.0,
        physical_length: 70.7, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, -50.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, -50.0, 0.0)],
        zoning_left: false, zoning_right: false, deleted: false,
    });
    // C -> B (70.7m)
    graph.add_edge(Edge {
        start_node: n_c, end_node: n_b,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
        width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 0.0,
        physical_length: 70.7, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![Vector3::new(50.0, -50.0, 0.0), Vector3::new(100.0, 50.0, 0.0)], // Wait, B is at y=50? 
        // Let's re-eval coordinates.
        // A (0,0,0)
        // B (100, 0, 0)
        // Let's make B flat too.
        physical_geometry: vec![Vector3::new(50.0, -50.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
        zoning_left: false, zoning_right: false, deleted: false,
    });
    
    // Correction:
    // A = (0,0,0)
    // B = (100,0,0)
    // C = (50, -100, 0)
    // Edge A-B: geometry [ (0,0,0), (100, 50, 0) ] -> steep
    // Edge A-C: geometry [ (0,0,0), (50, -100, 0) ] -> flat
    // Edge C-B: geometry [ (50,-100,0), (100, 0, 0) ] -> flat
    
    // Total flat distance = dist(A,C) + dist(C,B) = sqrt(50^2 + 100^2) * 2 = 111.8 * 2 = 223.6m
    // Steep distance = 100m, but 50% slope.
    
    let hpa = HpaGraph::build(&graph);
    let path = hpa.find_path(n_a, n_b, usize::MAX, &graph, TransitFlags::CAR);
    
    assert!(path.is_some(), "Should find a path");
    let (_cost, _dist, nodes) = path.unwrap();
    
    assert!(nodes.contains(&n_c), "Router should have detoured through node C to avoid the steep slope on A-B. Path was: {:?}", nodes);
    assert_eq!(nodes, vec![n_a, n_c, n_b]);
}





#[test]
fn test_bidirectional_walkway_pathing() {
    let mut graph = TransitGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);

    // Create a bidirectional walkway (primary_type = Foot, allowed_types = 1, lanes = 0)
    graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Foot,
        allowed_types: TransitFlags::FOOT,
        width: 2.0,
        fwd_lanes: 0,
        bkw_lanes: 0,
        speed_limit: 5.0,
        base_cost: 0.0,
        physical_length: 10.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        zoning_left: false,
        zoning_right: false,
        deleted: false,
    });

    let hpa = HpaGraph::build(&graph);
    
    // 1. Pedestrian should be able to walk n0 -> n1
    let path_fwd = hpa.find_path(n0, n1, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(path_fwd.is_some(), "Pedestrian should find path n0 -> n1 on walkway");
    
    // 2. Pedestrian should be able to walk n1 -> n0
    let path_bkw = hpa.find_path(n1, n0, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(path_bkw.is_some(), "Pedestrian should find path n1 -> n0 on walkway");
    
    // 3. Car should NOT be able to use walkway
    let path_car = hpa.find_path(n0, n1, usize::MAX, &graph, TransitFlags::CAR);
    assert!(path_car.is_none(), "Car should NOT find path on walkway");
}

#[test]
fn test_car_uturn_allowed() {
    let mut graph = TransitGraph::new();
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);

    // Create a bidirectional road (idx 0)
    let edge_idx = graph.add_edge(Edge {
        start_node: n0,
        end_node: n1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0,
        fwd_lanes: 1,
        bkw_lanes: 1,
        speed_limit: 50.0,
        base_cost: 10.0,
        physical_length: 10.0,
        current_congestion: 0.0,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
        zoning_left: false,
        zoning_right: false,
        deleted: false,
    });

    let hpa = HpaGraph::build(&graph);
    
    // Agent is at n1, having come from n0 via edge_idx.
    // They want to go back to n0.
    // If U-turns are allowed on same edge, they should find path [n0] via edge_idx.
    let path = hpa.find_path(n1, n0, edge_idx, &graph, TransitFlags::CAR);
    assert!(path.is_some(), "Car should be allowed to U-turn on bidirectional road");
    let (_, _, nodes) = path.unwrap();
    assert_eq!(nodes, vec![n1, n0]);
}

#[test]
fn test_car_avoids_walkway_shortcut() {
    let mut graph = TransitGraph::new();
    // n0 --- (Road) --- n1 --- (Road) --- n2
    //  \--- (Walkway shortcut) ---/
    
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    
    // Road path
    graph.add_edge(Edge {
        start_node: n0, end_node: n1,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 10.0,
        physical_length: 100.0, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![], physical_geometry: vec![Vector3::ZERO, Vector3::RIGHT * 100.0],
        zoning_left: false, zoning_right: false, deleted: false,
    });
    graph.add_edge(Edge {
        start_node: n1, end_node: n2,
        primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
        width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 10.0,
        physical_length: 100.0, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![], physical_geometry: vec![Vector3::RIGHT * 100.0, Vector3::RIGHT * 200.0],
        zoning_left: false, zoning_right: false, deleted: false,
    });
    
    // Walkway shortcut n0 -> n2 directly
    graph.add_edge(Edge {
        start_node: n0, end_node: n2,
        primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
        width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 10.0, base_cost: 5.0, // Cheaper than road
        physical_length: 200.0, current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
        geometry: vec![], physical_geometry: vec![Vector3::ZERO, Vector3::RIGHT * 200.0],
        zoning_left: false, zoning_right: false, deleted: false,
    });
    
    let hpa = hpa::HpaGraph::build(&graph);
    
    // Car should take the road path (2 nodes) and ignore the shortcut
    let path_car = hpa.find_path(n0, n2, usize::MAX, &graph, TransitFlags::CAR);
    assert!(path_car.is_some());
    let (_cost, _dist, nodes) = path_car.unwrap();
    assert_eq!(nodes.len(), 3, "Car should take 3 nodes (n0, n1, n2)");
    // Wait, if n0->n1 and n1->n2 are separate edges, path should be [n1, n2].
    assert!(nodes.contains(&n1), "Car must travel through n1 to avoid walkway");
    
    // Pedestrian should take the shortcut (n2 only)
    let path_ped = hpa.find_path(n0, n2, usize::MAX, &graph, TransitFlags::FOOT);
    assert!(path_ped.is_some());
    let (_c, _d, nodes_ped) = path_ped.unwrap();
    assert_eq!(nodes_ped.len(), 2, "Pedestrian should take direct walkway shortcut [n0, n2]");
    assert_eq!(nodes_ped[0], n0);
    assert_eq!(nodes_ped[1], n2);
}
