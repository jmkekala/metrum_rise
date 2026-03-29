#[cfg(test)]
mod tests {
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::{AgentSystem, MODE_WALK};
    use crate::simulation::economy::demand::DemandSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::{Vector2, Vector3};

    #[test]
    fn test_car_avoids_walkway() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(5.0, 0.0, 10.0), NodeType::Junction);

        // A-B is a Walkway (0 lanes)
        g.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Foot,
            allowed_types: TransitFlags::FOOT,
            width: 2.0,
            fwd_lanes: 0,
            bkw_lanes: 0,
            speed_limit: 5.0,
            base_cost: 10.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });

        // A-C-B is a Road (2 lanes)
        g.add_edge(Edge {
            start_node: n0,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 5.0,
            physical_length: 5.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 10.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 10.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });
        g.add_edge(Edge {
            start_node: n2,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 5.0,
            physical_length: 5.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(5.0, 0.0, 10.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(5.0, 0.0, 10.0), Vector3::new(10.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });

        let cch = CchGraph::build(&g);

        // Find path for CAR (pedestrian = false)
        let (_cost, _dist, p) = cch
            .find_path(n0, n1, usize::MAX, &g, TransitFlags::CAR)
            .expect("Car should find a path");
        // Path should be A -> C -> B (nodes 0, 2, 1)
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], n0);
        assert_eq!(p[1], n2);
        assert_eq!(p[2], n1);
    }

    #[test]
    fn test_pedestrian_prefers_walkway() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(5.0, 0.0, 1.0), NodeType::Junction);

        // A-B is a Road (direct)
        g.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 2.0,
            physical_length: 1.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(10.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });

        // A-C-B is a Walkway (loop)
        g.add_edge(Edge {
            start_node: n0,
            end_node: n2,
            primary_type: TransitType::Foot,
            allowed_types: TransitFlags::FOOT,
            width: 2.0,
            fwd_lanes: 0,
            bkw_lanes: 0,
            speed_limit: 5.0,
            base_cost: 0.5,
            physical_length: 0.5,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 1.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(5.0, 0.0, 1.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });
        g.add_edge(Edge {
            start_node: n2,
            end_node: n1,
            primary_type: TransitType::Foot,
            allowed_types: TransitFlags::FOOT,
            width: 2.0,
            fwd_lanes: 0,
            bkw_lanes: 0,
            speed_limit: 5.0,
            base_cost: 0.5,
            physical_length: 0.5,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(5.0, 0.0, 1.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(5.0, 0.0, 1.0), Vector3::new(10.0, 0.0, 0.0)],
            zoning_left: false,
            zoning_right: false,
            class: EdgeClass::Standard,
            deleted: false,
        });

        let cch = CchGraph::build(&g);

        // Find path for PEDESTRIAN (pedestrian = true)
        let (_cost, _dist, p) = cch
            .find_path(n0, n1, usize::MAX, &g, TransitFlags::FOOT)
            .unwrap();
        // Path should be A -> C -> B (nodes 0, 2, 1) because Road A-B costs 2.0 while Walkway A-C-B costs 0.5 + 0.5 = 1.0.
        assert_eq!(p.len(), 3);
        assert_eq!(p[0], n0);
        assert_eq!(p[1], n2);
        assert_eq!(p[2], n1);
    }

    #[test]
    fn test_car_only_from_home_persistence() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n_far = g.add_node(Vector3::new(1000.0, 0.0, 0.0), NodeType::Junction);

        let mut agents = AgentSystem::new();
        let i = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);

        // Agent is at Work (B1), car is at Home (B0)
        agents.home_building[i] = 0;
        agents.current_building[i] = 1; // away from home
        agents.has_car[i] = true;
        agents.pos_x[i] = 10.0;
        agents.pos_y[i] = 0.0;

        let cch = CchGraph::build(&g);
        let (target, driving) = agents.decide_transit_mode(i, n_far, &g, &cch);

        assert_eq!(
            driving, MODE_WALK,
            "Should NOT be able to drive if car is at home and agent is at work"
        );
        assert_eq!(target, n_far, "Should head to far target by foot");
    }

    #[test]
    fn test_agent_fsm_lifecycle() {
        use godot::prelude::Vector2;

        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);

        // n0 ----100m road---- n1
        g.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            width: 6.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(100.0, 0.0, 0.0)],
            zoning_left: true,
            zoning_right: true,
            class: EdgeClass::Standard,
            deleted: false,
        });

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut g); // rebuild before CCH
        network.cch_graph = CchGraph::build(&g);
        let config = MapConfig::default();
        let mut allocator = BuildingAllocator::new();
        let mut zoning = ZoningSystem::new(&config);
        // 1. Setup Buildings
        // Residential (Home) at 5m
        allocator.buildings.push(Building {
            center_x: 5.0,
            center_y: 10.0,
            width: 30,
            depth: 30,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.05,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
        });
        // Industrial (Work) at 50m
        allocator.buildings.push(Building {
            center_x: 50.0,
            center_y: 10.0,
            width: 30,
            depth: 30,
            zone_type: ZoneType::Industrial,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 5,
            cell_y: 0,
            occupancy: 0,
        });
        // Commercial (Shop) at 95m
        allocator.buildings.push(Building {
            center_x: 95.0,
            center_y: 10.0,
            width: 30,
            depth: 30,
            zone_type: ZoneType::Commercial,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.95,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 9,
            cell_y: 0,
            occupancy: 0,
        });
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        // 2. Spawn Agents
        for _ in 0..10 {
            let i = agents.spawn_agent(0, n0, 0.0, 0.0, n0, 5.0, 10.0);
            agents.home_building[i] = 0;
            agents.work_building[i] = 1;
            agents.has_car[i] = false;
            agents.transit_mode[i] = MODE_WALK;
            agents.money[i] = 100.0;
            agents.transit[i] = 0; // TRANSIT_IDLE
            agents.current_building[i] = 0;
            agents.pos_x[i] = 5.0;
            agents.pos_y[i] = 10.0;
        }

        let mut transitioned = false;

        // 3. Tick Simulation
        for _ in 0..1000 {
            agents.tick(&mut allocator, &network, &mut g, 1.0);

            for i in 0..agents.count {
                if agents.activity[i] != 0 || agents.transit[i] != 0 {
                    transitioned = true;
                    break;
                }
            }
            if transitioned {
                break;
            }
        }

        assert!(
            transitioned,
            "At least one agent should have started a journey during 1000 ticks"
        );
    }

    #[test]
    fn test_vehicle_type_persistence() {
        let mut agents = AgentSystem::new();

        // Spawn 3 agents
        let i0 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        let i1 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        let i2 = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);

        let type0 = agents.vehicle_type[i0];
        let type1 = agents.vehicle_type[i1];
        let type2 = agents.vehicle_type[i2];

        // Kill middle agent (i1). i2 should swap into index 1.
        let mut allocator = BuildingAllocator::new();
        agents.kill_agent(1, &mut allocator);

        assert_eq!(agents.count, 2);
        assert_eq!(agents.vehicle_type[0], type0);
        assert_eq!(agents.vehicle_type[1], type2);
    }

    #[test]
    fn test_border_spawn_movement() {
        use crate::simulation::network::types::{EdgeClass, TransitType};
        
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // Node 0 is the city center. Node 1 is the border node.
        let n0 = graph.add_node(godot::prelude::Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(godot::prelude::Vector3::new(100.0, 0.0, 0.0), NodeType::Border);
        
        // Edge goes from n0 to n1 (City to Border). 
        // Forward lane goes OUT of city. Backward lane comes INTO city.
        network.add_road(
            &mut graph,
            vec![godot::prelude::Vector3::new(0.0, 0.0, 0.0), godot::prelude::Vector3::new(100.0, 0.0, 0.0)],
            1, // fwd lanes (going to border)
            1, // bkw lanes (coming from border)
            false,
            false,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        
        // Make sure CCH is ready
        network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);

        let mut agents = AgentSystem::new();
        
        // Add a house at n0 so the agent has a destination
        allocator.buildings.push(crate::simulation::buildings::allocator::Building {
            center_x: 0.0,
            center_y: 0.0,
            width: 10,
            depth: 10,
            zone_type: crate::simulation::grid::zoning::ZoneType::Residential,
            facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
            frontage_t: 0.1,
            side_offset: 5.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
        });

        // Spawn agent at the border
        let agent_idx = agents.spawn_agent(
            0, // home bldg index
            n0, // target node (city)
            0.0, 0.0,
            n1, // highway node (start)
            100.0, 0.0 // Start position
        );

        // Tick to move
        agents.tick(&mut allocator, &network, &mut graph, 1.0); // 1 tick
        
        println!("Pos X after 1 tick: {}", agents.pos_x[agent_idx]);
        assert!(agents.pos_x[agent_idx] < 100.0, "Agent should have moved towards city");
    }
}
