// SPDX-License-Identifier: GPL-2.0-only

//! Modular query sub-modules for spatial and simulation state inspection (Item R13).

pub mod lanes;
pub mod network;
pub mod terrain;

use crate::simulation::network::graph::RegionGraph;
use godot::prelude::*;

/// Checks if a node is canonical (not an alias/merged node).
pub fn is_canonical_node(graph: &RegionGraph, node_id: u32) -> bool {
    graph.get_valid_node(node_id) == node_id
}

/// Checks if a node is canonical and still connected to a non-deleted edge.
pub fn is_live_canonical_node(graph: &RegionGraph, node_id: u32) -> bool {
    is_canonical_node(graph, node_id) && graph.node_has_live_incident_edge(node_id)
}

/// Finds the closest live canonical node to a given world position within a maximum distance.
pub fn get_closest_canonical_node(graph: &RegionGraph, world_pos: Vector3, max_dist: f32) -> i32 {
    let mut best_id = -1;
    let mut min_d_sq = max_dist * max_dist;
    for (i, node) in graph.nodes().iter().enumerate() {
        let node_id = i as u32;
        if !is_live_canonical_node(graph, node_id) {
            continue;
        }
        let dx = node.pos.x - world_pos.x;
        let dz = node.pos.z - world_pos.z;
        let d_sq = dx * dx + dz * dz;
        if d_sq < min_d_sq {
            min_d_sq = d_sq;
            best_id = node_id as i32;
        }
    }
    best_id
}

#[cfg(test)]
mod tests {
    use super::{get_closest_canonical_node, is_canonical_node, is_live_canonical_node};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    #[test]
    fn merged_alias_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let remove = graph.add_node(Vector3::new(10.2, 0.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(40.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(keep, far));
        graph.unite_nodes(keep, remove);

        assert!(is_canonical_node(&graph, keep));
        assert!(is_live_canonical_node(&graph, keep));
        assert!(!is_canonical_node(&graph, remove));
        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(10.1, 0.0, 0.0), 2.0),
            keep as i32
        );
    }

    #[test]
    fn deleted_edge_only_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let a = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let b = graph.add_node(Vector3::new(20.0, 0.0, 0.0), NodeType::Junction);
        let edge_idx = graph.add_edge(test_edge(a, b));

        assert!(is_live_canonical_node(&graph, a));
        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            a as i32
        );

        graph.edge_mut(edge_idx).deleted = true;

        assert!(!is_live_canonical_node(&graph, a));
        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            -1
        );
    }

    #[test]
    fn canonical_node_query_uses_xz_distance_for_editor_hits() {
        let mut graph = RegionGraph::new();
        let node = graph.add_node(Vector3::new(0.0, 50.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(20.0, 50.0, 0.0), NodeType::Junction);
        graph.add_edge(test_edge(node, far));

        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(0.5, 0.0, 0.0), 3.0),
            node as i32
        );
    }

    fn test_edge(start_node: u32, end_node: u32) -> crate::simulation::network::graph::Edge {
        crate::simulation::network::graph::Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: 20.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }
}
