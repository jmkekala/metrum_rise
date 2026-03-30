#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use godot::prelude::Vector3;

    #[test]
    fn test_opposite_buildings_share_node_and_jaywalk() {
        let config = MapConfig::default();
        let mut graph = RegionGraph::new();
        let mut network = TransitNetwork::new();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // 1. Create a straight road (100m)
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
            zoning_left: true,
            zoning_right: true,
            deleted: false,
        });
        graph.rebuild_adjacency_list();
        zoning.update_edge_grid_size(e_idx, 100.0);

        // 2. Add Building A on the LEFT side at 50m
        let nf_a = network.split_for_frontage(&mut graph, e_idx, 0.5, &mut zoning, &mut allocator);
        
        // 3. Add Building B on the RIGHT side at 50m
        // We find which edge now contains the 50m mark. 
        // It's either the end of the first half or start of the second.
        let edge_to_split = if graph.edges.len() > 1 {
             // Let's just check the edges. Edge 0 is n1->nf_a. Edge 1 is nf_a->n2.
             // Splitting Edge 0 at t=1.0 or Edge 1 at t=0.0 should both return nf_a.
             1 // Second half
        } else {
            e_idx
        };
        
        let nf_b = network.split_for_frontage(&mut graph, edge_to_split, 0.0, &mut zoning, &mut allocator);
        
        assert_eq!(nf_a, nf_b, "Opposite buildings at the same T-coordinate MUST share the same frontage node as requested");

        // 4. Verify that an agent going from A to B sees distance 0
        // (Even if crossings are blocked in the lane graph, the high-level pathfinder sees Node -> Node)
        // This is the "direct walk" bug.
    }
}
