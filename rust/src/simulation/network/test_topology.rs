#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::{TransitType, NodeType};
    // use crate::simulation::network::topology::process_intersections; // UNUSED
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use godot::prelude::Vector3;

    #[test]
    fn test_topology_split_near_end() {
        let mut net = TransitNetwork::new();
        // long road with many segments (250m)
        let pts = vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(250.0, 0.0, 0.0),
        ];
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        net.add_road(pts, 1, 1, false, false, &mut zoning, &mut allocator);
        
        // side road connecting near the end (segment 8 of 9)
        net.add_road(vec![
            Vector3::new(8.0, 0.0, 10.0),
            Vector3::new(8.0, 0.0, 0.0),
        ].into(), 1, 1, false, false, &mut zoning, &mut allocator);
        
        println!("Near-End Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start node {}, end node {}, geometry: {:?}", i, edge.start_node, edge.end_node, edge.geometry);
            assert!(edge.geometry.len() >= 2, "Edge {} must have at least 2 points", i);
        }
    }

    #[test]
    fn test_shallow_angle_intersection() {
        let mut net = TransitNetwork::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        // straight road
        net.add_road(vec![
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 1, 1, false, false, &mut zoning, &mut allocator);
        
        // super shallow angle road (approx 3 degrees)
        net.add_road(vec![
            Vector3::new(100.0, 0.0, 5.0),
            Vector3::new(0.0, 0.0, 0.0),
        ], 1, 1, false, false, &mut zoning, &mut allocator);
        
        println!("Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start node {}, end node {}, start_clip: {}, end_clip: {}", i, edge.start_node, edge.end_node, edge.start_clip, edge.end_clip);
        }
        println!("Junction Polygons generated: {}", net.graph.junction_polygons.len());
        if let Some(mesh) = net.graph.junction_polygons.get(&3) {
            println!("Vertices in junction: {}", mesh.vertices.len());
            for (i, v) in mesh.vertices.iter().enumerate() {
                println!(" v[{}]: {:?}", i, v);
            }
        }
    }

    #[test]
    fn test_4_way_intersection() {
        let mut net = TransitNetwork::new();
        let center = net.graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);
        
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
            net.add_road(vec![dir, Vector3::new(0.0, 0.0, 0.0)], 1, 1, false, false, &mut zoning, &mut allocator);
        }
        
        println!("4-way Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: nodes {}->{}, start_clip: {}, end_clip: {}", i, edge.start_node, edge.end_node, edge.start_clip, edge.end_clip);
            assert!(edge.end_clip >= 1.0 && edge.end_clip <= 10.0, "Clip distance should be reasonable for 90 degree angles");
        }
        
        assert!(net.graph.edges.len() >= 4);
    }

    #[test]
    fn test_extreme_angles() {
        use std::f32::consts::PI;
        println!("--- Sweeping all angles from 2 deg to 178 deg across N-way junctions ---");
        
        let way_counts = [2, 3, 4, 8];
        
        for ways in way_counts {
            for deg in 2..=178 {
                let mut net = TransitNetwork::new();
                let center = net.graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);
                
                let config = MapConfig::default();
                let mut zoning = ZoningSystem::new(&config);
                let mut allocator = BuildingAllocator::new();
                
                // Standard evenly spaced roads
                for w in 0..ways-1 {
                    let standard_angle = (w as f32) * (PI * 2.0) / (ways as f32);
                    let dir = Vector3::new(standard_angle.cos() * 100.0, 0.0, standard_angle.sin() * 100.0);
                    net.add_road(vec![dir, Vector3::new(0.0, 0.0, 0.0)], 1, 1, false, false, &mut zoning, &mut allocator);
                }
                
                // One extreme sweeper road relative to the 0-degree East road
                let rad = deg as f32 * PI / 180.0;
                let dir_extreme = Vector3::new(rad.cos() * 100.0, 0.0, rad.sin() * 100.0);
                net.add_road(vec![dir_extreme, Vector3::new(0.0, 0.0, 0.0)], 1, 1, false, false, &mut zoning, &mut allocator);
                
                // Verify clips are mathematically bounded securely below the runaway thresholds
                for edge in net.graph.edges.iter() {
                    assert!(edge.end_clip <= 8.0, "Angle {} in {}-way junction burst the 8.0m clip cap! Found: {}", deg, ways, edge.end_clip);
                    assert!(edge.end_clip >= 0.0, "Angle {} in {}-way junction has negative clip! Found: {}", deg, ways, edge.end_clip);
                }
            }
        }
        println!("All extreme angles across N-way junctions gracefully resolved within bounds!");
    }

    #[test]
    fn test_transit_graph_add_road() {
        let mut net = TransitNetwork::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        
        // Add a 250m straight road
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(250.0, 0.0, 0.0),
        ], 1, 1, false, false, &mut zoning, &mut allocator);
        
        // 250m should be split into 100m, 100m, 50m -> 3 edges
        assert_eq!(net.graph.edges.len(), 3, "Should have 3 edges for 250m road");
        assert_eq!(net.graph.nodes.len(), 4, "Should have 4 nodes for 250m road");
        
        // Find midpoint nodes by position
        let mid1 = net.graph.nodes.iter().position(|n| n.pos.distance_to(Vector3::new(100.0, 0.0, 0.0)) < 0.1).expect("Midpoint 1 should exist");
        let mid2 = net.graph.nodes.iter().position(|n| n.pos.distance_to(Vector3::new(200.0, 0.0, 0.0)) < 0.1).expect("Midpoint 2 should exist");

        // Middle nodes should have 2 edges each
        assert_eq!(net.graph.adjacency[mid1].len(), 2, "Midpoint 1 should be connected to 2 edges");
        assert_eq!(net.graph.adjacency[mid2].len(), 2, "Midpoint 2 should be connected to 2 edges");
    }

    #[test]
    fn test_transit_graph_split_edge() {
        use crate::simulation::network::topology::split_edge;
        let mut net = TransitNetwork::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        
        // 1. Add a 100m road. Physical cell size is 10.0m.
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 1, 1, true, true, &mut zoning, &mut allocator);
        
        assert_eq!(net.graph.edges.len(), 1);
        let old_edge_id = 0;
        let old_length = net.graph.edges[old_edge_id].physical_length;
        
        // 2. Add a building on the second half (at 70m = cell 7)
        allocator.buildings.push(crate::simulation::buildings::allocator::Building {
            center_x: 75.0, center_y: 10.0, width: 20, depth: 20,
            zone_type: crate::simulation::grid::zoning::ZoneType::Residential,
            facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
            frontage_t: 0.75, side_offset: 5.0, abandoned_timer: 0,
            edge_idx: old_edge_id, side: 1, cell_x: 7, cell_y: 0, occupancy: 0,
        });

        // 3. Create a split node at 40m
        let mid_pos = Vector3::new(40.0, 0.0, 0.0);
        let mid_id = net.graph.add_node(mid_pos, NodeType::Junction);
        
        // 4. Perform split (at 40m = cell 4)
        split_edge(&mut net, old_edge_id, 0, 0.4, mid_id, &mut zoning, &mut allocator);
        
        assert_eq!(net.graph.edges.iter().filter(|e| !e.deleted).count(), 2, "Should have 2 non-deleted edges after split");
        let new_edge_id = net.graph.edges.len() - 1; 
        
        // 5. Verification
        let e1 = &net.graph.edges[old_edge_id];
        let e2 = &net.graph.edges[new_edge_id];
        
        assert!((e1.physical_length + e2.physical_length - old_length).abs() < 0.1);
        assert_eq!(e1.end_node, mid_id);
        assert_eq!(e2.start_node, mid_id);
        
        // Verify building migration: split at 40m = cell 4. Building at cell 7 should move to new edge.
        // new cell_x = 7 - 4 = 3.
        assert_eq!(allocator.buildings[0].edge_idx, new_edge_id, "Building should have migrated to new edge");
        assert_eq!(allocator.buildings[0].cell_x, 3, "Building cell_x should have been adjusted");
    }

    #[test]
    fn test_transit_graph_compact_edges() {
        let mut net = TransitNetwork::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        
        // 1. Add two 100m roads
        net.add_road(vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)], 1, 1, true, true, &mut zoning, &mut allocator);
        net.add_road(vec![Vector3::new(0.0, 0.0, 50.0), Vector3::new(100.0, 0.0, 50.0)], 1, 1, true, true, &mut zoning, &mut allocator);
        
        assert_eq!(net.graph.edges.len(), 2);
        
        // 2. Add building on Edge 1
        allocator.buildings.push(crate::simulation::buildings::allocator::Building {
            center_x: 50.0, center_y: 60.0, width: 20, depth: 20,
            zone_type: crate::simulation::grid::zoning::ZoneType::Residential,
            facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
            frontage_t: 0.5, side_offset: 5.0, abandoned_timer: 0,
            edge_idx: 1, side: 1, cell_x: 5, cell_y: 0, occupancy: 0,
        });

        // 3. Mark Edge 0 as deleted
        net.graph.edges[0].deleted = true;
        
        // 4. Compact
        let mapping = net.graph.compact_edges();
        assert_eq!(mapping.get(&1), Some(&0), "Edge 1 should remap to index 0");
        
        // 5. Apply mapping to allocator
        allocator.update_edge_indices(&mapping);
        
        assert_eq!(net.graph.edges.len(), 1);
        assert_eq!(allocator.buildings[0].edge_idx, 0, "Building should now point to remapped Edge 0");
    }
}
