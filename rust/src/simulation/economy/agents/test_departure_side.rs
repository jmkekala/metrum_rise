#[cfg(test)]
mod tests {
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::economy::agents::{
        AgentSystem, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_NETWORK,
    };
    use crate::simulation::grid::zoning::ZoneType;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::lanes::LaneType;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::{Vector2, Vector3};

    use crate::simulation::LANE_CONFIGS;

    fn create_test_edge(n0: u32, n1: u32, fwd: u8, bkw: u8) -> Edge {
        Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: (fwd as f32 + bkw as f32) * 3.5,
            fwd_lanes: fwd,
            bkw_lanes: bkw,
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    fn create_test_building(edge_idx: usize, side: i8) -> Building {
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 1,
            depth_cells: 1,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(1.0, 0.0),
            frontage_t: 0.5, // t=0.5 → depart node = end_node of the edge
            side_offset: 5.0,
            abandoned_timer: 0,
            edge_idx,
            side,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: String::new(),
            level: 1,
            broken: false,
            stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: false,
            shipment_cooldown_days: 0,
        }
    }

    // Shared departure-side check. Spawns an agent departing from a building on the given
    // `side` (1 = LEFT, -1 = RIGHT) and asserts it lands on the matching sidewalk lane.
    //
    // `lane_idx` for sidewalks is a fixed constant regardless of vehicle lane count:
    //   LEFT sidewalk  → lane_idx =  100
    //   RIGHT sidewalk → lane_idx = -100
    // (See lanes.rs `build_lane` calls at the sidewalk section.)
    fn check_departure_side(
        fwd: u8,
        bkw: u8,
        building_side: i8,
        expected_lane_idx: i8,
        label: &str,
    ) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(create_test_edge(n0, n1, fwd, bkw));
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);

        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(create_test_building(edge_idx, building_side));

        let mut agents = AgentSystem::new();
        let a_id = agents.spawn_agent(0, n0, 100.0, 0.0, n0, 100.0, 0.0);
        agents.transit[a_id] = TRANSIT_ACCESS_EGRESS;
        agents.transit_mode[a_id] = MODE_WALK;
        agents.current_node[a_id] = n1;
        agents.current_building[a_id] = 0;
        agents.current_path[a_id] = vec![n1, n0];
        agents.current_path_index[a_id] = 1;

        for _ in 0..100 {
            let mut test_agents = AgentSystem::new();
            test_agents.agents = agents.agents.clone();
            for _ in 0..50 {
                test_agents.tick(&mut allocator, &mut network, &mut graph, 0.1);
                if test_agents.transit[a_id] == TRANSIT_NETWORK {
                    break;
                }
            }

            assert_eq!(
                test_agents.transit[a_id], TRANSIT_NETWORK,
                "[{label}] agent never reached ON_ROAD"
            );
            // One extra tick to initialize the lane in the ON_ROAD state.
            test_agents.tick(&mut allocator, &mut network, &mut graph, 0.1);

            let lane_id = test_agents.current_lane_id[a_id];
            let lane = &network.lane_system.lanes[lane_id];

            assert_eq!(
                lane.lane_type,
                LaneType::Foot,
                "[{label}] side={building_side}: agent should be on a Foot lane, found {:?}",
                lane.lane_type
            );
            assert_eq!(
                lane.lane_idx, expected_lane_idx,
                "[{label}] side={building_side}: expected lane_idx={expected_lane_idx} but found {}",
                lane.lane_idx
            );
        }
    }

    #[test]
    fn test_agent_departure_uses_correct_side_sidewalk() {
        for &(fwd, bkw, label) in LANE_CONFIGS {
            // LEFT building (side=1) → LEFT sidewalk (lane_idx=100)
            check_departure_side(fwd, bkw, 1, 100, label);
            // RIGHT building (side=-1) → RIGHT sidewalk (lane_idx=-100)
            check_departure_side(fwd, bkw, -1, -100, label);
        }
    }
}
