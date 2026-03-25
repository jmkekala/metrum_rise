#[cfg(test)]
mod tests {
    use godot::prelude::*;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::grid::zoning::{ZoningSystem, ZoneType};
    use crate::simulation::core::config::MapConfig;

    #[test]
    fn test_edge_compaction_remapping() {
        let mut network = TransitNetwork::new();
        let mut agents = AgentSystem::new();
        let config = MapConfig::default();
        let mut allocator = BuildingAllocator::new();
        let mut zoning = ZoningSystem::new(&config);

        // 1. Add Road A (Index 0)
        network.add_road(
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            2, 2, true, true, &mut zoning, &mut allocator
        );

        // 2. Add Road B (Index 1)
        network.add_road(
            vec![Vector3::new(0.0, 0.0, 50.0), Vector3::new(100.0, 0.0, 50.0)],
            2, 2, true, true, &mut zoning, &mut allocator
        );

        assert_eq!(network.graph.edges.len(), 2);

        // 3. Mark Road A as deleted
        network.graph.edges[0].deleted = true;

        // 4. Place a building on Road B (Index 1)
        // Note: For unit tests, we can manually insert into the allocator.
        allocator.buildings.push(crate::simulation::buildings::allocator::Building {
            center_x: 50.0,
            center_y: 50.0,
            width: 30,
            depth: 30,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 5.0,
            abandoned_timer: 0,
            edge_idx: 1, // Points to Road B
            side: 1,
            cell_x: 5,
            cell_y: 0,
            occupancy: 0,
        });

        // 5. Spawn an agent on Road B (Index 1)
        agents.spawn_agent(0, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.current_edge[0] = 1; // Agent on Road B

        // 6. Perform Compaction
        let mapping = network.graph.compact_edges();
        assert!(!mapping.is_empty(), "Compaction should return a mapping");
        assert_eq!(mapping.get(&1), Some(&0), "Road B (1) should remap to 0");

        // 7. Apply Mapping to other systems
        agents.update_edge_indices(&mapping);
        allocator.update_edge_indices(&mapping);
        zoning.update_edge_indices(&mapping);

        // 8. Verification
        assert_eq!(network.graph.edges.len(), 1, "Graph should have only 1 edge after compaction");
        assert_eq!(allocator.buildings[0].edge_idx, 0, "Building should now point to Road B at index 0");
        assert_eq!(agents.current_edge[0], 0, "Agent should now point to Road B at index 0");
    }
}
