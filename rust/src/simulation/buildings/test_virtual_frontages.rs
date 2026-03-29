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
        // Option C: each building placement calls split_for_frontage, inserting a real graph
        // node at the exact frontage position and splitting the edge into two half-edges.
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

        // Zone cells 5-7 (frontage_t ≈ 0.65 on the original 100m edge).
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 5 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;

        let initial_nodes = graph.nodes.len();
        let initial_edges = graph.edges.len();

        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );

        assert_eq!(allocator.buildings.len(), 1, "Building should have spawned");
        let b = &allocator.buildings[0];

        // Option C: a real node was inserted at the frontage position.
        assert_eq!(
            graph.nodes.len(), initial_nodes + 1,
            "Exactly one frontage node should have been inserted"
        );
        assert!(
            graph.edges.len() > initial_edges,
            "Edge should have been split into two half-edges"
        );

        // The building references the new node directly.
        let expected_frontage_node = initial_nodes as u32;
        assert_eq!(
            b.frontage_node, expected_frontage_node,
            "Building should reference the new frontage node"
        );

        // After the split the building sits at the end of the first half-edge,
        // so frontage_t within that half-edge is near 1.0.
        assert!(
            b.frontage_t > 0.9,
            "frontage_t within first half-edge should be close to 1.0, got {}",
            b.frontage_t
        );
    }

    #[test]
    fn test_virtual_frontage_routing_targets() {
        // Option C: two sequential ticks each insert a frontage node. The second building lands
        // on the second half-edge produced by the first split. Both buildings get distinct nodes.
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

        let orig_start = graph.edges[0].start_node;
        let orig_end = graph.edges[0].end_node;

        // Fill all cells so both ticks find something to build.
        for dx in 0..10 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;

        // First tick: spawns the building closest to the start.
        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );
        assert_eq!(allocator.buildings.len(), 1, "First building should have spawned");
        let b1_fn = allocator.buildings[0].frontage_node;

        // Second tick: spawns on the second half-edge created by the first split.
        allocator.tick(
            &mut demand, &mut zoning, &desirability, &noise,
            &mut agents, &mut net, &mut graph, &config,
        );
        assert_eq!(allocator.buildings.len(), 2, "Second building should have spawned");
        let b2_fn = allocator.buildings[1].frontage_node;

        // Each building must have its own distinct frontage node.
        assert_ne!(b1_fn, b2_fn, "Buildings should have different frontage nodes");

        // Neither frontage node is the original road endpoint — they are real mid-edge nodes.
        assert_ne!(b1_fn, orig_start, "b1 frontage should not be orig start_node");
        assert_ne!(b1_fn, orig_end,   "b1 frontage should not be orig end_node");
        assert_ne!(b2_fn, orig_start, "b2 frontage should not be orig start_node");
        assert_ne!(b2_fn, orig_end,   "b2 frontage should not be orig end_node");

        // Verify the nodes are reachable from each other via CCH.
        let cch = &net.cch_graph;
        assert!(
            cch.find_path(b1_fn, b2_fn, usize::MAX, &graph,
                crate::simulation::network::types::TransitFlags::FOOT).is_some(),
            "frontage nodes should be connected via road network"
        );
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
