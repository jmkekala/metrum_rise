// SPDX-License-Identifier: GPL-2.0-only

//! Border-node discovery for outside-world freight.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;

/// Reuses gateway-list storage while detecting role changes which do not rebuild CCH.
/// Discovery is O(nodes + border adjacency). The small ordered result is filled serially
/// to reuse its capacity without allocating one filter buffer per parallel worker.
pub(crate) fn collect_connected_border_nodes(graph: &RegionGraph, borders: &mut Vec<u32>) {
    borders.clear();
    borders.extend(graph.nodes().iter().enumerate().filter_map(|(idx, node)| {
        if node.node_type != NodeType::Border {
            return None;
        }
        let connected = graph
            .node_adjacency(idx as u32)
            .iter()
            .any(|&edge| !graph.edge(edge).deleted);
        connected.then_some(idx as u32)
    }));
}
