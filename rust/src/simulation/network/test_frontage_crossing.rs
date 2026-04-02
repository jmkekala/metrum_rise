#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{RegionGraph, Edge};
    use crate::simulation::network::types::{NodeType, TransitType, TransitFlags, EdgeClass};
    use crate::simulation::network::lanes::LaneSystem;
    use godot::prelude::Vector3;

    #[test]
    fn test_frontage_node_forbids_crossing() {
        let mut graph = RegionGraph::new();
        // n0 ---- n1 (Frontage Node) ---- n2
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Frontage);
        let n2 = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);

        let edges = [(n0, n1), (n1, n2)];
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
                deleted: false,
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Check node n1 for any crossing connections
        let mut crossing_count = 0;
        
        // Iterate only through lanes belonging to node n1
        // We do this by checking all inbound lanes to node n1 and their next_lanes
        for &e_idx in &graph.adjacency[n1 as usize] {
            let edge = &graph.edges[e_idx];
            let is_end = edge.end_node == n1;
            
            if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    // If this lane enters node n1
                    if l.is_fwd == is_end {
                        for &next_id in &l.next_lanes {
                            let next_l = &lanes.lanes[next_id];
                            // If it's a connection lane (edge_id == MAX)
                            if next_l.edge_id == usize::MAX {
                                // Find where this connection leads
                                    if let Some(&target_id) = next_l.next_lanes.first() {
                                        let target_l = &lanes.lanes[target_id];
                                        // A connection at a Frontage node is only valid if it preserves the physical side (lane_idx)
                                        // Any connection that changes lane_idx is a crossing.
                                        if target_l.lane_idx != l.lane_idx {
                                            crossing_count += 1;
                                        }
                                    }
                            }
                        }
                    }
                }
            }
        }

        // Divide by 2 because connections are usually bidirectional in this logic? 
        // Actually, let's just assert it is 0.
        assert_eq!(crossing_count, 0, "Frontage node n1 should have ZERO pedestrian crossings between sides of the same edge, but found {}", crossing_count);
    }
}
