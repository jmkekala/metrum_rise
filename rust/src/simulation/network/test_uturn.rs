// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: test_uturn.rs
//  script_path: rust/src/simulation/network/test_uturn.rs
//  module_name: test_uturn
//  version: 0.1.0
//  description: Tests that U-turn lane connections are not generated where
//           another way through exists, and are allowed at a dead end.
//  kind: test
//  spec: none
//  internal_dependencies: [graph, lanes]
//  external_dependencies: [godot]
//  features: [u-turn, turn-restrictions, lane-connections]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

// ========================================================================
// U-TURN TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::lanes::{LaneSystem, LaneType};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    #[test]
    fn test_uturn_forbidden_at_two_way_junction() {
        let mut graph = RegionGraph::new();
        // n0 ---- n1 (Two-way Junction) ---- n2
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
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
                lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: 50.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                deleted: false,
                no_building_spawn: false,
                vehicle_frontage_access:
                    crate::simulation::network::types::VehicleFrontageAccess::BothSides,
                frontage_class: Default::default(),
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Check node n1 for any U-turn connections
        let mut uturn_count = 0;
        for &e_idx in &graph.adjacency[n1 as usize] {
            let edge = &graph.edges[e_idx];
            let is_end = edge.end_node == n1;

            if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    // Skip footpaths
                    if l.lane_type == LaneType::Foot {
                        continue;
                    }

                    // If this lane enters node n1
                    if l.is_fwd == is_end {
                        for &next_id in &l.next_lanes {
                            let next_l = &lanes.lanes[next_id];
                            // If it leads to a target lane
                            if let Some(&target_id) = next_l.next_lanes.first() {
                                let target_l = &lanes.lanes[target_id];
                                // If target is on the SAME edge as l, it's a U-turn
                                if target_l.edge_id == l.edge_id && l.edge_id != usize::MAX {
                                    uturn_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            uturn_count, 0,
            "Two-way junction n1 should have ZERO U-turns, but found {}",
            uturn_count
        );
    }

    #[test]
    fn test_uturn_forbidden_at_three_way_junction() {
        let mut graph = RegionGraph::new();
        //      n2
        //      |
        // n0--n1--n3
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(0.0, 0.0, 50.0), NodeType::Junction);
        let n3 = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);

        let edges = [(n0, n1), (n2, n1), (n3, n1)];
        for (s, e) in edges {
            graph.add_edge(Edge {
                start_node: s,
                end_node: e,
                primary_type: TransitType::Road,
                allowed_types: TransitFlags::CAR,
                class: EdgeClass::Standard,
                width: 7.0,
                lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
                speed_limit: 50.0,
                base_cost: 1.0,
                physical_length: 50.0,
                current_congestion: 0.0,
                start_clip: 0.0,
                end_clip: 0.0,
                geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                physical_geometry: vec![graph.nodes[s as usize].pos, graph.nodes[e as usize].pos],
                deleted: false,
                no_building_spawn: false,
                vehicle_frontage_access:
                    crate::simulation::network::types::VehicleFrontageAccess::BothSides,
                frontage_class: Default::default(),
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Check node n1 for any U-turn connections
        let mut uturn_count = 0;
        for &e_idx in &graph.adjacency[n1 as usize] {
            if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    // If inbound to n1
                    if l.is_fwd == (graph.edges[e_idx].end_node == n1) {
                        for &next_id in &l.next_lanes {
                            if let Some(&target_id) = lanes.lanes[next_id].next_lanes.first() {
                                if lanes.lanes[target_id].edge_id == l.edge_id {
                                    uturn_count += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        assert_eq!(
            uturn_count, 0,
            "Three-way junction n1 should have ZERO U-turns, but found {}",
            uturn_count
        );
    }

    #[test]
    fn test_uturn_allowed_at_dead_end() {
        let mut graph = RegionGraph::new();
        // n0 ---- n1 (Dead end)
        let n0 = graph.add_node(Vector3::new(-50.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 50.0,
            base_cost: 1.0,
            physical_length: 50.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![graph.nodes[n0 as usize].pos, graph.nodes[n1 as usize].pos],
            physical_geometry: vec![graph.nodes[n0 as usize].pos, graph.nodes[n1 as usize].pos],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        });
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);

        // Check node n1 for U-turn connections
        let mut uturn_count = 0;
        let e_idx = 0;
        if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
            for &l_id in lane_ids {
                let l = &lanes.lanes[l_id];
                if l.lane_type == LaneType::Foot {
                    continue;
                }

                // Entrance to n1 (end node)
                if l.is_fwd == true {
                    for &next_id in &l.next_lanes {
                        let next_l = &lanes.lanes[next_id];
                        if let Some(&target_id) = next_l.next_lanes.first() {
                            let target_l = &lanes.lanes[target_id];
                            if target_l.edge_id == l.edge_id {
                                uturn_count += 1;
                            }
                        }
                    }
                }
            }
        }

        assert!(
            uturn_count > 0,
            "Dead end n1 should have at least one U-turn connection for cars to turn back"
        );
    }
}
