#[cfg(test)]
mod tests {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::graph::{Edge, Node};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::*;
    use std::collections::HashMap;

    #[test]
    fn test_edge_compaction_remapping() {
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let mut agents = AgentSystem::new();
        let config = MapConfig::default();
        let mut allocator = BuildingAllocator::new();
        let mut zoning = ZoningSystem::new(&config);

        // 1. Add Road A (Index 0)
        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            2,
            2,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // 2. Add Road B (Index 1)
        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 50.0), Vector3::new(100.0, 0.0, 50.0)],
            2,
            2,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert_eq!(graph.edges.len(), 2);

        // 3. Mark Road A as deleted
        graph.edges[0].deleted = true;

        // 4. Place a building on Road B (Index 1)
        allocator
            .buildings
            .push(crate::simulation::buildings::allocator::Building {
                center_x: 50.0,
                center_y: 50.0,
                width_cells: 3,
                depth_cells: 3,
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
                worker_count: 0,
                asset_id: String::new(), level: 1,
                broken: false,
                stock: 0.0,
                revenue: 0.0,
                operating_budget: 500.0,
                utility_service_available: false,
            });

        // 5. Spawn an agent on Road B (Index 1)
        agents.spawn_agent(0, 0, 0.0, 0.0, 0, 0.0, 0.0);
        agents.current_edge[0] = 1; // Agent on Road B

        // 6. Perform Compaction
        let mapping = graph.compact_edges();
        assert!(!mapping.is_empty(), "Compaction should return a mapping");
        assert_eq!(mapping.get(&1), Some(&0), "Road B (1) should remap to 0");

        // 7. Apply Mapping to other systems
        agents.update_edge_indices(&mapping);
        allocator.update_edge_indices(&mapping);
        zoning.update_edge_indices(&mapping);

        // 8. Verification
        assert_eq!(
            graph.edges.len(),
            1,
            "Graph should have only 1 edge after compaction"
        );
        assert_eq!(
            allocator.buildings[0].edge_idx, 0,
            "Building should now point to Road B at index 0"
        );
        assert_eq!(
            agents.current_edge[0], 0,
            "Agent should now point to Road B at index 0"
        );
    }

    #[test]
    fn test_rebuild_pathing_skips_deleted_edges_without_compacting() {
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();

        graph.nodes = vec![
            Node {
                pos: Vector3::new(0.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(100.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
        ];

        let edge_template = Edge {
            start_node: 0,
            end_node: 1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            class: EdgeClass::Standard,
            width: 8.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 20.0,
            base_cost: 0.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: Vec::new(),
            physical_geometry: Vec::new(),
            deleted: false, no_building_spawn: false,
        };

        graph.edges = vec![
            Edge {
                base_cost: 1.0,
                deleted: true,
                ..edge_template.clone()
            },
            Edge {
                base_cost: 7.0,
                ..edge_template
            },
        ];
        graph.rebuild_adjacency_list();

        network.cch_dirty_chunks.insert((0, 0));
        network.rebuild_pathing(&mut graph);

        assert_eq!(
            graph.edges.len(),
            2,
            "Pathing rebuild should not compact the soft-deleted edge array"
        );
        assert!(
            graph.edges[0].deleted,
            "Deleted edge slot should remain in place"
        );

        let (cost, _dist, nodes) = network
            .cch_graph
            .find_path(0, 1, usize::MAX, &graph, TransitFlags::CAR)
            .expect("live edge should still be routable after rebuild");
        assert_eq!(cost, 7.0, "CCH should ignore the deleted lower-cost edge");
        assert_eq!(nodes, vec![0, 1]);
    }
}
