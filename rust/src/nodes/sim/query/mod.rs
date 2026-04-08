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

/// Finds the closest canonical node to a given world position within a maximum distance.
pub fn get_closest_canonical_node(graph: &RegionGraph, world_pos: Vector3, max_dist: f32) -> i32 {
    let mut best_id = -1;
    let mut min_d = max_dist;
    for (i, node) in graph.nodes().iter().enumerate() {
        let node_id = i as u32;
        if !is_canonical_node(graph, node_id) {
            continue;
        }
        let d = node.pos.distance_to(world_pos);
        if d < min_d {
            min_d = d;
            best_id = node_id as i32;
        }
    }
    best_id
}

#[cfg(test)]
mod tests {
    use super::{get_closest_canonical_node, is_canonical_node};
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    #[test]
    fn merged_alias_nodes_are_not_treated_as_live_query_nodes() {
        let mut graph = RegionGraph::new();
        let keep = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
        let remove = graph.add_node(Vector3::new(10.2, 0.0, 0.0), NodeType::Junction);
        graph.unite_nodes(keep, remove);

        assert!(is_canonical_node(&graph, keep));
        assert!(!is_canonical_node(&graph, remove));
        assert_eq!(
            get_closest_canonical_node(&graph, Vector3::new(10.1, 0.0, 0.0), 2.0),
            keep as i32
        );
    }
}
