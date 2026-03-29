#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{RegionGraph, Edge};
    use crate::simulation::network::types::{NodeType, TransitType, TransitFlags, EdgeClass};
    use crate::simulation::network::lanes::{LaneSystem, LaneType};
    use godot::prelude::Vector3;

    #[test]
    fn test_pedestrian_junction_diagonal_teleport_forbidden() {
        let mut graph = RegionGraph::new();
        //      n2
        //      |
        // n0--n1--n3
        //      |
        //      n4
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(0.0, 0.0, 50.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n4 = graph.add_node(Vector3::new(0.0, 0.0, -50.0), NodeType::Junction);

        let edges = [(n0, n1), (n2, n1), (n3, n1), (n4, n1)];
        for (s, e) in edges {
            graph.add_edge(Edge {
                start_node: s,
                end_node: e,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                class: EdgeClass::Standard,
                width: 7.0,
                fwd_lanes: 1,
                bkw_lanes: 1,
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: 50.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                zoning_left: false,
                zoning_right: false,
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // A 4-way junction should have:
        // 8 mouths (4 edges * 2 sides)
        // For each mouth, exactly TWO connections (one CW, one CCW)
        // If it allows "diagonal teleport", it would have more.
        
        // Find all inbound foot lanes to node n1
        let mut total_junction_connections = 0;
        
        for &e_idx in &graph.adjacency[n1 as usize] {
            if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    if l.lane_type != LaneType::Foot { continue; }
                    
                    // Enters node n1?
                    if l.is_fwd == (graph.edges[e_idx].end_node == n1) {
                        for &next_id in &l.next_lanes {
                            if lanes.lanes[next_id].edge_id == usize::MAX {
                                total_junction_connections += 1;
                            }
                        }
                    }
                }
            }
        }

        // 8 mouths * 2 connections each = 16 connection lanes total
        // Currently, it's likely much higher (8 * 7 = 56).
        assert_eq!(total_junction_connections, 16, "4-way junction n1 should have exactly 16 sidewalk connections, but found {}", total_junction_connections);
    }
}
