// SPDX-License-Identifier: GPL-2.0-only

//! Border-node discovery for outside-world freight.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;

pub(super) fn connected_border_nodes(graph: &RegionGraph) -> Vec<u32> {
    graph
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(idx, node)| {
            if node.node_type != NodeType::Border {
                return None;
            }
            let connected = graph
                .node_adjacency(idx as u32)
                .iter()
                .any(|&edge_idx| !graph.edge(edge_idx).deleted);
            if connected { Some(idx as u32) } else { None }
        })
        .collect()
}

/// Returns whether ordinary `OWA` freight has at least one physical city gateway.
pub(crate) fn has_connected_border_node(graph: &RegionGraph) -> bool {
    graph.nodes().iter().enumerate().any(|(idx, node)| {
        node.node_type == NodeType::Border
            && graph
                .node_adjacency(idx as u32)
                .iter()
                .any(|&edge_idx| !graph.edge(edge_idx).deleted)
    })
}
