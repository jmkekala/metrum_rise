#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{RegionGraph, Edge};
    use crate::simulation::network::types::{TransitType, TransitFlags, NodeType};
    use godot::prelude::Vector3;

    #[test]
    fn test_t_junction_clipping() {
        let mut g = RegionGraph::new();
        // T intersection at (0,0) resulting from a split
        let n_center = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        
        let n_left = g.add_node(Vector3::new(-10.0, 0.0, 0.0), NodeType::Junction);
        let n_right = g.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let n_bot = g.add_node(Vector3::new(0.0, 0.0, 10.0), NodeType::Junction);

        // Edge 0: Left to Center (-X to 0) (Horizontal Main Road Part 1)
        g.add_edge(Edge {
            start_node: n_left, end_node: n_center,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            width: 2.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            current_congestion: 0.0,
            start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(-5.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            physical_geometry: vec![],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        // Edge 1: Center to Right (0 to +X) (Horizontal Main Road Part 2)
        g.add_edge(Edge {
            start_node: n_center, end_node: n_right,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            width: 2.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            current_congestion: 0.0,
            start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(5.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)],
            physical_geometry: vec![],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });
        // Edge 2: Bot to Center (+Z to 0) (Vertical Road connecting)
        g.add_edge(Edge {
            start_node: n_bot, end_node: n_center,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            width: 2.0, fwd_lanes: 1, bkw_lanes: 1, speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 0.0,
            current_congestion: 0.0,
            start_clip: 0.0, end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 10.0), Vector3::new(0.0, 0.0, 5.0), Vector3::new(0.0, 0.0, 0.0)],
            physical_geometry: vec![],
            zoning_left: false,
            zoning_right: false,
            deleted: false,
        });

        g.rebuild_intersection_clips();
    }
}
