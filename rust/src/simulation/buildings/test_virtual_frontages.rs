#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::economy::demand::DemandSystem;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::economy::agents::{TRANSIT_ON_ROAD, TRANSIT_ARRIVING, MODE_CAR, MODE_WALK};
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use godot::prelude::Vector3;

    #[test]
    fn test_virtual_frontage_placement() {
        let mut net = TransitNetwork::new();
        let mut zoning = ZoningSystem::new();
        let mut allocator = BuildingAllocator::new(100, 100);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();
        
        let mut desirability = DesirabilitySystem::new(500, 500); 
        for x in 0..500 {
            for y in 0..500 {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(100, 100);

        // Create a single 100m road with zoning enabled
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 1, 1, true, true, &mut zoning, &mut allocator);

        // Set zoning for residential on the left side (3x3 area)
        let edge_idx = 0;
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(edge_idx, 1, 5 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0; // High demand to force spawning

        // Initial graph state
        let initial_nodes = net.graph.nodes.len();
        let initial_edges = net.graph.edges.len();

        // Tick allocator to spawn building
        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut net);

        // Verify building was spawned
        assert_eq!(allocator.buildings.len(), 1, "Building should have spawned");
        let b = &allocator.buildings[0];
        
        // Verify T-coordinate calculation
        // Cell 5 is from 50m to 60m. Frontage point (x+1.5) is at 5+1.5 = 6.5 cells = 65m.
        // Edge is 100m. frontage_t should be 0.65.
        assert!((b.frontage_t - 0.65).abs() < 0.01, "frontage_t should be 0.65, found {}", b.frontage_t);

        // CRITICAL: Verify graph hasn't been physically split
        assert_eq!(net.graph.nodes.len(), initial_nodes, "Node count should NOT change");
        assert_eq!(net.graph.edges.len(), initial_edges, "Edge count should NOT change");
    }

    #[test]
    fn test_virtual_frontage_routing_targets() {
        let mut net = TransitNetwork::new();
        let mut zoning = ZoningSystem::new();
        let mut allocator = BuildingAllocator::new(100, 100);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();
        
        let mut desirability = DesirabilitySystem::new(500, 500);
        for x in 0..500 {
            for y in 0..500 {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(100, 100);

        // Create a road from (0,0,0) to (100,0,0) with zoning
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 1, 1, true, true, &mut zoning, &mut allocator);

        // Spawn building at t=0.15 (near start_node)
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 0 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;
        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut net);
        
        // Spawn building at t=0.85 (near end_node)
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 7 + dx, dy, ZoneType::Residential);
            }
        }
        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut net);

        assert_eq!(allocator.buildings.len(), 2);
        
        let start_node = net.graph.edges[0].start_node;
        let end_node = net.graph.edges[0].end_node;

        // Verify routing target derivation
        let b_near_start = &allocator.buildings[0]; // t=0.15
        let target_near_start = if b_near_start.frontage_t < 0.5 { start_node } else { end_node };
        assert_eq!(target_near_start, start_node);

        let b_near_end = &allocator.buildings[1]; // t=0.85
        let target_near_end = if b_near_end.frontage_t < 0.5 { start_node } else { end_node };
        assert_eq!(target_near_end, end_node);
    }

    #[test]
    fn test_wide_road_arrival() {
        let mut net = TransitNetwork::new();
        let mut zoning = ZoningSystem::new();
        let mut allocator = BuildingAllocator::new(100, 100);
        let mut agents = AgentSystem::new();
        let mut demand = DemandSystem::new();
        
        let mut desirability = DesirabilitySystem::new(500, 500);
        for x in 0..500 {
            for y in 0..500 {
                desirability.grid.set(x, y, 100.0);
            }
        }
        let noise = NoiseSystem::new(100, 100);

        // 4-lane road (14m wide)
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 2, 2, true, true, &mut zoning, &mut allocator);

        // Spawn building at t=0.5
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, 3 + dx, dy, ZoneType::Residential);
            }
        }
        demand.residential = 500.0;
        allocator.tick(&mut demand, &mut zoning, &desirability, &noise, &mut agents, &mut net);
        
        let b_idx = 0;
        let b = &allocator.buildings[b_idx];
        
        // Setup agent on the road part, in a far lane (offset > 2m)
        let agent_idx = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.target_building[agent_idx] = b_idx;
        agents.current_edge[agent_idx] = 0;
        agents.current_node[agent_idx] = net.graph.edges[0].start_node;
        agents.transit[agent_idx] = TRANSIT_ON_ROAD;
        agents.transit_mode[agent_idx] = MODE_CAR;
        agents.current_lane[agent_idx] = 1; // Outer fwd lane
        
        // Road width 14m, lane width 3.5m. Lane 1 center is 5.25m from centerline.
        // So agent will be at Z = 5.25 (approx).
        agents.pos_x[agent_idx] = 0.0;
        agents.pos_y[agent_idx] = 5.25; 

        let frontage_x = b.frontage_t * 100.0; // t * length
        
        // Move agent near the frontage (dist_along < 2m, but Euclidean distance > 5m)
        agents.pos_x[agent_idx] = frontage_x - 1.0; 
        
        // Euclidean distance: sqrt(1.0^2 + 5.25^2) = 5.34m (> 2.0m)
        // Projected distance: 1.0m (< 2.0m)
        
        let hpa = crate::simulation::pathing::hpa::HpaGraph::build(&net.graph);
        agents.tick(&allocator, &hpa, &mut net.graph, 0.016);
        
        assert_eq!(agents.transit[agent_idx], TRANSIT_ARRIVING, 
            "Agent should have arrived via projected distance check");
    }
}
