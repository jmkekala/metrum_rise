// SPDX-License-Identifier: GPL-2.0-only

//! Pedestrian junction connection-count regressions.

#[cfg(test)]
mod tests {
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::lanes::{LaneSystem, LaneType};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    use crate::simulation::LANE_CONFIGS;

    fn build_4way_junction(fwd: u8, bkw: u8) -> (RegionGraph, LaneSystem, u32) {
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
                width: (fwd as f32 + bkw as f32) * 3.5,
                fwd_lanes: fwd,
                bkw_lanes: bkw,
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
            });
        }
        graph.rebuild_adjacency_list();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes, n1)
    }

    fn count_sidewalk_connections_at(graph: &RegionGraph, lanes: &LaneSystem, node: u32) -> usize {
        let mut total = 0;
        for &e_idx in &graph.adjacency[node as usize] {
            if let Some(lane_ids) = lanes.edge_lanes.get(&(e_idx as usize)) {
                for &l_id in lane_ids {
                    let l = &lanes.lanes[l_id];
                    if l.lane_type != LaneType::Foot {
                        continue;
                    }
                    if l.is_fwd == (graph.edges[e_idx].end_node == node) {
                        for &next_id in &l.next_lanes {
                            if lanes.lanes[next_id].edge_id == usize::MAX {
                                total += 1;
                            }
                        }
                    }
                }
            }
        }
        total
    }

    // The connection count at a 4-way junction is invariant to vehicle lane count:
    //
    //   4 edges × 2 sides = 8 sidewalk mouths
    //   Each mouth has one legal connector route to every other sidewalk mouth = 7 connections
    //   Each mouth also has one stationary same-side reversal = 1 connection
    //   Total = 8 × 8 = 64
    //
    // This is because sidewalks are always exactly ONE lane per side per edge (lane_idx ±100),
    // regardless of how many vehicle lanes the edge carries. Adding more vehicle lanes widens
    // the asphalt but does not add extra sidewalk mouths at the junction.
    #[test]
    fn test_pedestrian_junction_diagonal_teleport_forbidden() {
        for &(fwd, bkw, label) in LANE_CONFIGS {
            let (graph, lanes, n1) = build_4way_junction(fwd, bkw);
            let total = count_sidewalk_connections_at(&graph, &lanes, n1);
            assert_eq!(
                total, 64,
                "[{label}] every 4-way sidewalk mouth must reach both sidewalks of every road arm and reverse in place without a diagonal shortcut."
            );
        }
    }
}
