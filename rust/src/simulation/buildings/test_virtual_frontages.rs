#[cfg(test)]
mod tests {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::agents::{
        MODE_CAR, MODE_WALK, TRANSIT_ARRIVING, TRANSIT_ON_ROAD,
    };
    use crate::simulation::economy::demand::DemandSystem;
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::EdgeClass;
    use godot::prelude::Vector3;

    #[test]
    fn test_virtual_frontage_placement() {
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        // Create a single 100m road with zoning enabled
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            true,
            true,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // Set zoning for residential on the left side (3x3 area)
        let edge_idx = 0;
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(edge_idx, 1, 5 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0; // High demand to force spawning

        // Initial graph state
        let initial_nodes = graph.nodes.len();
        let initial_edges = graph.edges.len();

        // Tick allocator to spawn building
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut net,
            &mut graph,
            &config,
        );

        // Verify building was spawned
        assert_eq!(allocator.buildings.len(), 1, "Building should have spawned");
        let b = &allocator.buildings[0];

        // Verify T-coordinate calculation
        assert!(
            (b.frontage_t - 0.65).abs() < 0.01,
            "frontage_t should be 0.65, found {}",
            b.frontage_t
        );

        // CRITICAL: Verify graph hasn't been physically split
        assert_eq!(
            graph.nodes.len(),
            initial_nodes,
            "Node count should NOT change"
        );
        assert_eq!(
            graph.edges.len(),
            initial_edges,
            "Edge count should NOT change"
        );
    }

    #[test]
    fn test_virtual_frontage_routing_targets() {
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        // Create a road from (0,0,0) to (100,0,0) with zoning
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            true,
            true,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // Spawn building at t=0.15 (near start_node)
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 0 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut net,
            &mut graph,
            &config,
        );

        // Spawn building at t=0.85 (near end_node)
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 7 + dx, dy, ZoneType::Residential);
            }
        }
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut net,
            &mut graph,
            &config,
        );

        assert_eq!(allocator.buildings.len(), 2);

        let start_node = graph.edges[0].start_node;
        let end_node = graph.edges[0].end_node;

        // Verify routing target derivation
        let b_near_start = &allocator.buildings[0]; // t=0.15
        let target_near_start = if b_near_start.frontage_t < 0.5 {
            start_node
        } else {
            end_node
        };
        assert_eq!(target_near_start, start_node);

        let b_near_end = &allocator.buildings[1]; // t=0.85
        let target_near_end = if b_near_end.frontage_t < 0.5 {
            start_node
        } else {
            end_node
        };
        assert_eq!(target_near_end, end_node);
    }

    #[test]
    fn test_wide_road_arrival() {
        let config = MapConfig::default();
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();

        let mut desirability = DesirabilitySystem::new(&config);
        let (env_w, env_h) = config.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(&config);

        // 4-lane road (14m wide)
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            2,
            2,
            true,
            true,
            EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // Spawn building at t=0.5
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 3 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut net,
            &mut graph,
            &config,
        );

        let b_idx = 0;
        let b = &allocator.buildings[b_idx];

        // Setup agent on the road part, in a far lane (offset > 2m)
        let agent_idx = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.target_building[agent_idx] = b_idx;
        agents.current_edge[agent_idx] = 0;
        agents.current_node[agent_idx] = graph.edges[0].start_node;
        agents.transit[agent_idx] = TRANSIT_ON_ROAD;
        agents.current_lane_id[agent_idx] = 1; // Outer fwd lane
        agents.current_path[agent_idx] = vec![graph.edges[0].start_node, graph.edges[0].end_node];
        agents.current_path_index[agent_idx] = 1;
        agents.target_node[agent_idx] = graph.edges[0].end_node;

        // Road width 14m, lane width 3.5m. Lane 1 center is 5.25m from centerline.
        let physical_len = graph.edges[0].physical_length;
        let frontage_dist = b.frontage_t * physical_len;

        // Move agent exactly behind the frontage so a single tick (0.32m) triggers arrival
        agents.lane_distance[agent_idx] = frontage_dist - 0.2;

        net.lane_system.rebuild(&mut graph);
        net.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
        agents.tick(&mut allocator, &net, &mut graph, 0.016);

        println!(
            "physical_len={}, lane_dist={}, expected_target={}, is_fwd={}, lane_len={}",
            physical_len,
            agents.lane_distance[agent_idx],
            frontage_dist,
            net.lane_system.lanes[3].is_fwd,
            net.lane_system.lanes[3].length
        );

        assert_eq!(
            agents.transit[agent_idx], TRANSIT_ARRIVING,
            "Agent should have arrived via projected distance check"
        );
    }
}
