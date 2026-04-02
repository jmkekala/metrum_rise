#[cfg(test)]
mod tests {
    use crate::simulation::economy::agents::{AgentSystem, TRANSIT_DEPARTING, TRANSIT_ON_ROAD, MODE_WALK};
    use crate::simulation::network::graph::{RegionGraph, Edge};
    use crate::simulation::network::types::{NodeType, TransitType, TransitFlags, EdgeClass};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::grid::zoning::ZoneType;
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::{Vector2, Vector3};

    fn create_test_edge(n0: u32, n1: u32) -> Edge {
        Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
        }
    }

    fn create_test_building(edge_idx: usize, side: i8, frontage_node: u32) -> Building {
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(1.0, 0.0),
            frontage_t: 0.5,
            frontage_node,
            side_offset: 5.0,
            abandoned_timer: 0,
            edge_idx,
            side,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            variant: 0,
        }
    }

    #[test]
    fn test_agent_departure_uses_correct_side_sidewalk() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(create_test_edge(n0, n1));
        graph.rebuild_adjacency_list();
        
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        
        let mut allocator = BuildingAllocator::new();
        // Building on side 1 (LEFT)
        allocator.buildings.push(create_test_building(edge_idx, 1, n1));
        
        let mut agents = AgentSystem::new();
        let a_id = agents.spawn_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
        
        agents.transit[a_id] = TRANSIT_DEPARTING;
        agents.transit_mode[a_id] = MODE_WALK;
        agents.current_node[a_id] = n1;
        agents.current_building[a_id] = 0;
        agents.current_path[a_id] = vec![n1, n0];
        agents.current_path_index[a_id] = 1;

        for _ in 0..100 {
            let mut test_agents = AgentSystem::new();
            test_agents.agents = agents.agents.clone();
            for _ in 0..50 {
                test_agents.tick(&mut allocator, &network, &mut graph, 0.1);
                if test_agents.transit[a_id] == TRANSIT_ON_ROAD { break; }
            }
            
            assert_eq!(test_agents.transit[a_id], TRANSIT_ON_ROAD);
            // One extra tick to initialize the lane in the ON_ROAD state
            test_agents.tick(&mut allocator, &network, &mut graph, 0.1);
            
            let lane_id = test_agents.current_lane_id[a_id];
            let lane = &network.lane_system.lanes[lane_id];
            
            // Side 1 (Left) should match lane_idx 100
            assert_eq!(lane.lane_idx, 100, "Agent from LEFT building (side=1) should use LEFT sidewalk (lane_idx=100), but used {}", lane.lane_idx);
        }
    }
}
