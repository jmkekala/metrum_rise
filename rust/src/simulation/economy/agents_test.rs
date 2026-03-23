#[cfg(test)]
mod tests {
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::network::graph::{TransitGraph, Edge};
    use crate::simulation::network::types::{TransitType, TransitFlags, NodeType};
    use crate::simulation::pathing::hpa::HpaGraph;
    use crate::simulation::economy::demand::DemandSystem;
    use godot::prelude::Vector3;

    #[test]
    fn test_car_avoids_walkway() {
        let mut g = TransitGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(5.0, 0.0, 10.0), NodeType::Junction);

        // A-B is a Walkway (0 lanes)
        g.add_edge(Edge {
            start_node: n0, end_node: n1,
            primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
            width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 5.0, base_cost: 10.0, physical_length: 10.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        // A-C-B is a Road (2 lanes)
        g.add_edge(Edge {
            start_node: n0, end_node: n2,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 5.0, physical_length: 5.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 10.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 10.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        g.add_edge(Edge {
            start_node: n2, end_node: n1,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 5.0, physical_length: 5.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::new(5.0, 0.0, 10.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(5.0, 0.0, 10.0), Vector3::new(10.0, 0.0, 0.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        let hpa = HpaGraph::build(&g);
        
        // Find path for CAR (pedestrian = false)
        let (_cost, _dist, p) = hpa.find_path(n0, n1, usize::MAX, &g, false).expect("Car should find a path");
        // Path should be A -> C -> B (nodes 0, 2, 1)
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], n2);
        assert_eq!(p[1], n1);
    }

    #[test]
    fn test_pedestrian_prefers_walkway() {
        let mut g = TransitGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(5.0, 0.0, 1.0), NodeType::Junction);

        // A-B is a Road (direct)
        g.add_edge(Edge {
            start_node: n0, end_node: n1,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 1.0, physical_length: 1.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        // A-C-B is a Walkway (loop)
        g.add_edge(Edge {
            start_node: n0, end_node: n2,
            primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
            width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 5.0, base_cost: 0.5, physical_length: 0.5,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 1.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 1.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        g.add_edge(Edge {
            start_node: n2, end_node: n1,
            primary_type: TransitType::Foot, allowed_types: TransitFlags::FOOT,
            width: 2.0, fwd_lanes: 0, bkw_lanes: 0, speed_limit: 5.0, base_cost: 0.5, physical_length: 0.5,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::new(5.0, 0.0, 1.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(5.0, 0.0, 1.0), Vector3::new(10.0, 0.0, 0.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        let hpa = HpaGraph::build(&g);
        
        // Find path for PEDESTRIAN (pedestrian = true)
        let (_cost, _dist, p) = hpa.find_path(n0, n1, usize::MAX, &g, true).unwrap();
        // Path should be A -> C -> B (nodes 0, 2, 1) because Road A-B is 1.0 * 10 = 10.0 cost for pedestrians.
        // Walkway A-C-B is 0.5 + 0.5 = 1.0 cost.
        assert_eq!(p.len(), 2);
        assert_eq!(p[0], n2);
        assert_eq!(p[1], n1);
    }

    #[test]
    fn test_parking_search_full() {
        let mut g = TransitGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);

        // E0: n0-n1, length 10m -> capacity (10/6)*2 = 2.
        g.add_edge(Edge {
            start_node: n0, end_node: n1,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 10.0, physical_length: 10.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            parking_occupied: 2, // FULL
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        // E1: n0-n2, length 10m -> capacity 2.
        g.add_edge(Edge {
            start_node: n0, end_node: n2,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 10.0, physical_length: 10.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 10.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 10.0)],
            parking_occupied: 0, // FREE
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        let agents = AgentSystem::new();
        let spot = agents.find_parking_spot(n0, &g);
        assert!(spot.is_some());
        let (edge_idx, _) = spot.unwrap();
        assert_eq!(edge_idx, 1, "Should skip full edge 0 and pick edge 1");
    }

    #[test]
    fn test_car_only_from_home_persistence() {
        let mut g = TransitGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n_far = g.add_node(Vector3::new(1000.0, 0.0, 0.0), NodeType::Junction);

        let mut agents = AgentSystem::new();
        let i = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        
        // Agent is at Work (B1), car is at Home (B0)
        agents.home_building[i] = 0;
        agents.current_building[i] = 1; // away from home
        agents.has_car[i] = true;
        agents.parked_edge[i] = usize::MAX; // Car is at home
        agents.pos_x[i] = 10.0;
        agents.pos_y[i] = 0.0;

        let hpa = HpaGraph::build(&g);
        let (target, driving) = agents.decide_transit_mode(i, n_far, &g, &hpa);
        
        assert_eq!(driving, false, "Should NOT be able to drive if car is at home and agent is at work");
        assert_eq!(target, n_far, "Should head to far target by foot");
    }

    #[test]
    fn test_multi_stop_walking_and_retrieval() {
        let mut g = TransitGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n_near = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n_far = g.add_node(Vector3::new(1000.0, 0.0, 0.0), NodeType::Junction);
        
        // Edge E1 for parking
        g.add_edge(Edge {
            start_node: n0, end_node: n_near,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0, base_cost: 1.0, physical_length: 100.0,
            current_congestion: 0.0, start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
            parking_occupied: 0,
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        let mut agents = AgentSystem::new();
        let i = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        
        // 1. Agent at Work (B1). Car parked at E1.
        agents.home_building[i] = 0;
        agents.current_building[i] = 1;
        agents.has_car[i] = true;
        agents.parked_edge[i] = 0; // E1
        agents.parked_progression[i] = 0; // At n0
        agents.pos_x[i] = 0.0;
        agents.pos_y[i] = 0.0;

        let hpa = HpaGraph::build(&g);
        // Trip to Near Shop (B2)
        let (target1, driving1) = agents.decide_transit_mode(i, n_near, &g, &hpa);
        assert_eq!(driving1, false, "Should walk to near shop");
        assert_eq!(target1, n_near);
        assert_eq!(agents.parked_edge[i], 0, "Car should stay parked at E1");

        // 2. Arrive at B2. Decide to go Home (Far).
        agents.current_building[i] = 2; // Arrived at shop
        agents.pos_x[i] = 100.0;
        agents.pos_y[i] = 0.0;
        
        let (target2, driving2) = agents.decide_transit_mode(i, n_far, &g, &hpa);
        assert_eq!(driving2, false, "Must walk to car first before driving far");
        assert_eq!(target2, n0, "Should head back to parking E1 (n0)");
    }
}
