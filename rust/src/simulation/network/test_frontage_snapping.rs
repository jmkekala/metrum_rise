#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::core::config::MapConfig;
    use godot::prelude::Vector3;

    #[test]
    fn test_frontage_centered_fallback() {
        let config = MapConfig::default();
        let mut graph = RegionGraph::new();
        let mut network = TransitNetwork::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let e_idx = graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: crate::simulation::network::types::TransitType::Road,
            allowed_types: crate::simulation::network::types::TransitFlags::CAR | crate::simulation::network::types::TransitFlags::FOOT,
            class: crate::simulation::network::types::EdgeClass::Standard,
            width: 10.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.8,
            base_cost: 1.0,
            physical_length: 10.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        // MIN_FRONTAGE_DISTANCE is 8.0.
        // A split at 5.0m on a 10m edge is exactly centered.
        // Tiebreaker should snap to Start Node (n1).
        let node = network.split_for_frontage(&mut graph, e_idx, 0.5, &mut zoning, &mut allocator);
        
        assert_eq!(node, n1, "Exactly centered split on short edge must snap to Start Node");
        assert_eq!(graph.edges.len(), 1, "Should not have created a new edge for snapped frontage");
    }

    #[test]
    fn test_frontage_agent_remap_order() {
        let config = MapConfig::default();
        let mut graph = RegionGraph::new();
        let mut network = TransitNetwork::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        let mut agents = AgentSystem::new();

        // 1. Create a 100m road with a split in the middle (50m)
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let e_idx = graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n1,
            end_node: n2,
            primary_type: crate::simulation::network::types::TransitType::Road,
            allowed_types: crate::simulation::network::types::TransitFlags::CAR | crate::simulation::network::types::TransitFlags::FOOT,
            class: crate::simulation::network::types::EdgeClass::Standard,
            width: 10.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.8,
            base_cost: 1.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
        });
        graph.rebuild_adjacency_list();
        zoning.update_edge_grid_size(e_idx, 100.0);

        let f_node = network.split_for_frontage(&mut graph, e_idx, 0.5, &mut zoning, &mut allocator);
        assert_eq!(graph.edges.len(), 2);
        let second_half_idx = graph.edges.len() - 1;

        // 2. Put an agent on the second half-edge
        let a_idx = agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.agents.current_edge[a_idx] = second_half_idx;
        agents.agents.transit[a_idx] = crate::simulation::economy::agents::TRANSIT_ON_ROAD;
        agents.agents.current_path[a_idx] = vec![f_node, n2];

        // 3. Remove the frontage node (merging edges)
        network.remove_frontage(&mut graph, f_node, &mut zoning, &mut allocator, &mut agents);

        // 4. Verify agent was moved to the surviving first-half edge and path was cleared
        assert_eq!(agents.agents.current_edge[a_idx], 0, "Agent must be migrated to the surviving edge");
        assert!(agents.agents.current_path[a_idx].is_empty(), "Agent path must be cleared to force re-routing after topology change");
    }
}
