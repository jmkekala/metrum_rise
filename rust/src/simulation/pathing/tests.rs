use super::*;
use crate::simulation::network::graph::{Edge, TransitGraph};
use crate::simulation::network::types::{TransitType, TransitFlags};
use godot::prelude::Vector3;

#[test]
fn test_cost_calculation_slope_penalty() {
    let mut edge = Edge {
        start_node: 0,
        end_node: 1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 4.0,
        speed_limit: 50.0,
        base_cost: 0.0,
        current_congestion: 0.0,
        geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ],
    };

    let flat_cost = cost::CostCalculator::calculate_base_cost(&edge);
    
    // Add a hill: 100m length, 50m height (50% slope)
    edge.geometry[1] = Vector3::new(100.0, 50.0, 0.0);
    let steep_cost = cost::CostCalculator::calculate_base_cost(&edge);

    assert!(steep_cost > flat_cost * 2.0, "Steep route cost ({}) should heavily penalize over flat route cost ({})", steep_cost, flat_cost);
}

#[test]
fn test_flow_field_avoids_congestion() {
    let mut graph = TransitGraph::new();
    // Create a square network
    // n0 --- n1
    // |      |
    // n2 --- n3
    
    // Node 3 is target. 
    // Edge 1->3 is very expensive (high congestion)
    // Edge 2->3 is cheap
    
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    let n2 = graph.add_node(Vector3::new(0.0, 0.0, 100.0), crate::simulation::network::types::NodeType::Junction);
    let n3 = graph.add_node(Vector3::new(100.0, 0.0, 100.0), crate::simulation::network::types::NodeType::Junction);
    
    graph.add_edge(Edge { start_node: n0, end_node: n1, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 0.0, geometry: vec![] });
    graph.add_edge(Edge { start_node: n0, end_node: n2, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 0.0, geometry: vec![] });
    // Expensive segment!
    graph.add_edge(Edge { start_node: n1, end_node: n3, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 5.0, geometry: vec![] });
    // Cheap segment!
    graph.add_edge(Edge { start_node: n2, end_node: n3, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 0.0, geometry: vec![] });
    
    let flow = flow::FlowField::generate(&graph, n3);
    
    // Agent at n0 should step 'downhill' toward n2, not n1, because n1->n3 is highly congested.
    let next = flow.get_next_node(&graph, n0);
    assert_eq!(next, Some(n2));
}

#[test]
fn test_highway_vs_dirt_road_cost() {
    let highway = Edge {
        start_node: 0,
        end_node: 1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 10.0,
        speed_limit: 100.0, // Highway speed
        base_cost: 0.0,
        current_congestion: 0.0,
        geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10000.0, 0.0, 0.0), // 10km
        ],
    };

    let dirt_road = Edge {
        start_node: 0,
        end_node: 1,
        primary_type: TransitType::Road,
        allowed_types: TransitFlags::CAR,
        width: 4.0,
        speed_limit: 20.0, // Slow dirt road
        base_cost: 0.0,
        current_congestion: 0.0,
        geometry: vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(5000.0, 0.0, 0.0), // 5km
        ],
    };

    let highway_cost = cost::CostCalculator::calculate_base_cost(&highway);
    let dirt_road_cost = cost::CostCalculator::calculate_base_cost(&dirt_road);

    assert!(highway_cost < dirt_road_cost, "10km highway ({}) should be cheaper than 5km dirt road ({})", highway_cost, dirt_road_cost);
}

#[test]
fn test_flow_field_slope_avoidance() {
    let mut graph = TransitGraph::new();
    
    // n0 (Start)
    // | \
    // n1  n2 (Hill Peak vs Flat Bypass)
    // | / 
    // n3 (End)
    
    let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);
    let n1 = graph.add_node(Vector3::new(100.0, 41.0, 0.0), crate::simulation::network::types::NodeType::Junction); // 41% grade!
    let n2 = graph.add_node(Vector3::new(0.0, 0.0, 300.0), crate::simulation::network::types::NodeType::Junction); // Flat bypass
    let n3 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), crate::simulation::network::types::NodeType::Junction);

    let mut hill_up = Edge { start_node: n0, end_node: n1, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 0.0, current_congestion: 0.0, geometry: vec![graph.nodes[n0 as usize].pos, graph.nodes[n1 as usize].pos] };
    let mut hill_down = Edge { start_node: n1, end_node: n3, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 0.0, current_congestion: 0.0, geometry: vec![graph.nodes[n1 as usize].pos, graph.nodes[n3 as usize].pos] };
    
    let mut bypass_1 = Edge { start_node: n0, end_node: n2, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 0.0, current_congestion: 0.0, geometry: vec![graph.nodes[n0 as usize].pos, graph.nodes[n2 as usize].pos] };
    let mut bypass_2 = Edge { start_node: n2, end_node: n3, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 0.0, current_congestion: 0.0, geometry: vec![graph.nodes[n2 as usize].pos, graph.nodes[n3 as usize].pos] };

    // Set base costs dynamically
    hill_up.base_cost = cost::CostCalculator::calculate_base_cost(&hill_up);
    hill_down.base_cost = cost::CostCalculator::calculate_base_cost(&hill_down);
    bypass_1.base_cost = cost::CostCalculator::calculate_base_cost(&bypass_1);
    bypass_2.base_cost = cost::CostCalculator::calculate_base_cost(&bypass_2);

    graph.add_edge(hill_up);
    graph.add_edge(hill_down);
    graph.add_edge(bypass_1);
    graph.add_edge(bypass_2);

    let flow = flow::FlowField::generate(&graph, n3);
    
    // Agent at n0 should avoid the steep hill (n1) and take the long bypass (n2)
    let next = flow.get_next_node(&graph, n0);
    assert_eq!(next, Some(n2));
}

#[test]
fn test_flow_field_timing_benchmark() {
    let mut graph = TransitGraph::new();
    let grid_size = 32; // 32x32 = 1024 nodes
    
    // Create 1024 nodes
    for z in 0..grid_size {
        for x in 0..grid_size {
            graph.add_node(Vector3::new(x as f32 * 10.0, 0.0, z as f32 * 10.0), crate::simulation::network::types::NodeType::Junction);
        }
    }
    
    // Connect them in a grid
    for z in 0..grid_size {
        for x in 0..grid_size {
            let n = z * grid_size + x;
            if x < grid_size - 1 {
                let right = n + 1;
                graph.add_edge(Edge { start_node: n, end_node: right, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 0.0, geometry: vec![] });
            }
            if z < grid_size - 1 {
                let down = n + grid_size;
                graph.add_edge(Edge { start_node: n, end_node: down, primary_type: TransitType::Road, allowed_types: 0, width: 4.0, speed_limit: 50.0, base_cost: 10.0, current_congestion: 0.0, geometry: vec![] });
            }
        }
    }
    
    let start = std::time::Instant::now();
    let _flow = flow::FlowField::generate(&graph, 0); // Generate flow to corner node
    let duration = start.elapsed();
    
    assert!(duration.as_millis() < 15, "Flow field generation for 1000 nodes took {} ms (limit: < 15ms)", duration.as_millis());
}
