#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::{TransitType, NodeType};
    use crate::simulation::network::topology::process_intersections;
    use godot::prelude::Vector3;

    #[test]
    fn test_topology_split_near_end() {
        let mut net = TransitNetwork::new();
        // long road with many segments
        let mut pts = Vec::new();
        for i in 0..10 {
            pts.push(Vector3::new(i as f32, 0.0, 0.0));
        }
        net.add_road(pts.into(), 1, 1);
        
        // side road connecting near the end (segment 8 of 9)
        net.add_road(vec![
            Vector3::new(8.0, 0.0, 10.0),
            Vector3::new(8.0, 0.0, 0.0),
        ].into(), 1, 1);
        
        println!("Near-End Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start node {}, end node {}, geometry: {:?}", i, edge.start_node, edge.end_node, edge.geometry);
            assert!(edge.geometry.len() >= 2, "Edge {} must have at least 2 points", i);
        }
    }

    #[test]
    fn test_shallow_angle_intersection() {
        let mut net = TransitNetwork::new();
        // straight road
        net.add_road(vec![
            Vector3::new(-100.0, 0.0, 0.0),
            Vector3::new(100.0, 0.0, 0.0),
        ], 1, 1);
        
        // super shallow angle road (approx 3 degrees)
        net.add_road(vec![
            Vector3::new(100.0, 0.0, 5.0),
            Vector3::new(0.0, 0.0, 0.0),
        ], 1, 1);
        
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
        
        for dir in dirs {
            net.add_road(vec![dir, Vector3::new(0.0, 0.0, 0.0)], 1, 1);
        }
        
        println!("4-way Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start_clip: {}, end_clip: {}", i, edge.start_clip, edge.end_clip);
            assert!(edge.end_clip >= 1.0 && edge.end_clip <= 5.0, "Clip distance should be reasonable for 90 degree angles");
        }
        
        assert_eq!(net.graph.junction_polygons.len(), 1, "Should generate exactly 1 junction polygon");
    }

    #[test]
    fn test_8_way_intersection() {
        let mut net = TransitNetwork::new();
        // 8 points in a circle connecting to center
        use std::f32::consts::PI;
        for i in 0..8 {
            let angle = (i as f32) * PI / 4.0;
            let dir = Vector3::new(angle.cos() * 100.0, 0.0, angle.sin() * 100.0);
            net.add_road(vec![dir, Vector3::new(0.0, 0.0, 0.0)], 1, 1);
        }
        
        println!("8-way Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start_clip: {}, end_clip: {}", i, edge.start_clip, edge.end_clip);
            // 45 degree angle intersections require slightly more clip distance but should definitely stay under 8.0 meters (our cap)
            assert!(edge.end_clip <= 8.0, "Clip distance must not exceed extreme acute cap for 45 deg angles");
            assert!(edge.end_clip >= 2.0, "Clip distance must naturally be somewhat large for 45 deg angles");
        }
        
        assert_eq!(net.graph.junction_polygons.len(), 1, "Should generate exactly 1 junction polygon");
    }

    #[test]
    fn test_extreme_angles() {
        use std::f32::consts::PI;
        println!("--- Sweeping all angles from 2 deg to 178 deg across N-way junctions ---");
        
        let way_counts = [2, 3, 4, 8];
        
        for ways in way_counts {
            for deg in 2..=178 {
                let mut net = TransitNetwork::new();
                let _center = net.graph.find_or_add_node(Vector3::new(0.0, 0.0, 0.0), 0.1, NodeType::Junction);
                
                // Standard evenly spaced roads
                for w in 0..ways-1 {
                    let standard_angle = (w as f32) * (PI * 2.0) / (ways as f32);
                    let dir = Vector3::new(standard_angle.cos() * 100.0, 0.0, standard_angle.sin() * 100.0);
                    net.add_road(vec![dir, Vector3::new(0.0, 0.0, 0.0)], 1, 1);
                }
                
                // One extreme sweeper road relative to the 0-degree East road
                let rad = deg as f32 * PI / 180.0;
                let dir_extreme = Vector3::new(rad.cos() * 100.0, 0.0, rad.sin() * 100.0);
                net.add_road(vec![dir_extreme, Vector3::new(0.0, 0.0, 0.0)], 1, 1);
                
                // Verify clips are mathematically bounded securely below the runaway thresholds
                for edge in net.graph.edges.iter() {
                    assert!(edge.end_clip <= 8.0, "Angle {} in {}-way junction burst the 8.0m clip cap! Found: {}", deg, ways, edge.end_clip);
                    assert!(edge.end_clip >= 0.0, "Angle {} in {}-way junction has negative clip! Found: {}", deg, ways, edge.end_clip);
                }
            }
        }
        println!("All extreme angles across N-way junctions gracefully resolved within bounds!");
    }
}
