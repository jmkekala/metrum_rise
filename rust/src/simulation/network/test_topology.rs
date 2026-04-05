#[cfg(test)]
mod tests {
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    #[test]
    fn test_topology_split_near_end() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        // long road with many segments (250m)
        let pts = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(250.0, 0.0, 0.0)];
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        net.add_road(
            &mut graph,
            pts,
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // side road connecting near the end (segment 8 of 9)
        net.add_road(
            &mut graph,
            vec![Vector3::new(8.0, 0.0, 10.0), Vector3::new(8.0, 0.0, 0.0)].into(),
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert!(graph.edges.len() > 0);
    }

    #[test]
    fn test_shallow_angle_intersection() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        // straight road
        net.add_road(
            &mut graph,
            vec![
                Vector3::new(-100.0, 0.0, 0.0),
                Vector3::new(100.0, 0.0, 0.0),
            ],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // super shallow angle road (approx 3 degrees)
        net.add_road(
            &mut graph,
            vec![Vector3::new(100.0, 0.0, 5.0), Vector3::new(0.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert!(graph.edges.len() > 0);
    }

    #[test]
    fn test_4_way_intersection() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let _center = graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);

        // North, East, South, West roads connecting to center
        let dirs = [
            Vector3::new(0.0, 0.0, -100.0),
            Vector3::new(100.0, 0.0, 0.0),
            Vector3::new(0.0, 0.0, 100.0),
            Vector3::new(-100.0, 0.0, 0.0),
        ];

        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        for dir in dirs {
            net.add_road(
                &mut graph,
                vec![dir, Vector3::new(0.0, 0.0, 0.0)],
                1,
                1,
                crate::simulation::network::types::EdgeClass::Standard,
                &mut zoning,
                &mut allocator,
            );
        }

        assert!(graph.edges.len() >= 4);
    }

    #[test]
    fn test_extreme_angles() {
        use std::f32::consts::PI;

        let way_counts = [2, 3, 4, 8];

        for ways in way_counts {
            for deg in 10..=170 {
                // Test a subset to be faster
                let mut net = TransitNetwork::new();
                let mut graph = RegionGraph::new();
                let _center =
                    graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);

                let config = MapConfig::default();
                let mut zoning = ZoningSystem::new(&config);
                let mut allocator = BuildingAllocator::new();

                // Standard evenly spaced roads
                for w in 0..ways - 1 {
                    let standard_angle = (w as f32) * (PI * 2.0) / (ways as f32);
                    let dir = Vector3::new(
                        standard_angle.cos() * 100.0,
                        0.0,
                        standard_angle.sin() * 100.0,
                    );
                    net.add_road(
                        &mut graph,
                        vec![dir, Vector3::new(0.0, 0.0, 0.0)],
                        1,
                        1,
                        crate::simulation::network::types::EdgeClass::Standard,
                        &mut zoning,
                        &mut allocator,
                    );
                }

                // One extreme sweeper road relative to the 0-degree East road
                let rad = deg as f32 * PI / 180.0;
                let dir_extreme = Vector3::new(rad.cos() * 100.0, 0.0, rad.sin() * 100.0);
                net.add_road(
                    &mut graph,
                    vec![dir_extreme, Vector3::new(0.0, 0.0, 0.0)],
                    1,
                    1,
                    crate::simulation::network::types::EdgeClass::Standard,
                    &mut zoning,
                    &mut allocator,
                );

                // Verify clips are mathematically bounded securely below the runaway thresholds
                for edge in graph.edges.iter() {
                    assert!(edge.end_clip <= 8.01, "Burst! Found: {}", edge.end_clip);
                }
            }
        }
    }

    #[test]
    fn test_transit_graph_add_road() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // Add a 250m straight road
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(250.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // 250m road: no intermediate subdivision nodes, single edge with two endpoint nodes.
        assert_eq!(graph.edges.len(), 1, "Should have 1 edge for 250m road (no 100m subdivision)");
        assert_eq!(graph.nodes.len(), 2, "Should have 2 nodes for 250m road");
    }

    #[test]
    fn test_transit_graph_split_edge() {
        use crate::simulation::network::topology::split_edge;
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        // 1. Add a 100m road.
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        assert_eq!(graph.edges.len(), 1);
        let old_edge_id = 0;
        let _old_length = graph.edges[old_edge_id].physical_length;

        // 2. Add a building
        allocator
            .buildings
            .push(crate::simulation::buildings::allocator::Building {
                center_x: 75.0,
                center_y: 10.0,
                width_cells: 2,
                depth_cells: 2,
                zone_type: crate::simulation::grid::zoning::ZoneType::Residential,
                facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
                frontage_t: 0.75,
                frontage_node: 0,
                side_offset: 5.0,
                abandoned_timer: 0,
                edge_idx: old_edge_id,
                side: 1,
                cell_x: 7,
                cell_y: 0,
                occupancy: 0,
                asset_id: String::new(),
            });

        // 3. Create a split node at 40m
        let mid_pos = Vector3::new(40.0, 0.0, 0.0);
        let mid_id = graph.add_node(mid_pos, NodeType::Junction);

        // 4. Perform split
        split_edge(
            &mut net,
            &mut graph,
            old_edge_id,
            0,
            0.4,
            mid_id,
            &mut zoning,
            &mut allocator,
        );

        assert_eq!(graph.edges.iter().filter(|e| !e.deleted).count(), 2);
    }
}
