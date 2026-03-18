#[cfg(test)]
mod tests {
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::types::{TransitType, NodeType};
    use crate::simulation::network::topology::process_intersections;
    use godot::prelude::Vector3;

    #[test]
    fn test_topology_t_junction() {
        let mut net = TransitNetwork::new();
        // straight road
        net.add_road(vec![
            Vector3::new(-10.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
        ].into(), 1, 1);
        
        // side road connecting to the middle
        net.add_road(vec![
            Vector3::new(0.0, 0.0, 10.0),
            Vector3::new(0.0, 0.0, 0.0),
        ].into(), 1, 1);
        
        println!("Graph has {} edges", net.graph.edges.len());
        for (i, edge) in net.graph.edges.iter().enumerate() {
            println!("Edge {}: start node {}, end node {}, geometry len {}", i, edge.start_node, edge.end_node, edge.geometry.len());
        }
        
    }
}
