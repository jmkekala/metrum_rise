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
