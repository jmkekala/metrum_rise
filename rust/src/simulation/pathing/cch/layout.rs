// SPDX-License-Identifier: GPL-2.0-only

//! Immutable node slices and exact turn tables for one hierarchy topology revision.

use super::CchGraph;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::ops::Index;

/// CSR storage preserving ascending shortcut IDs within a node, including equal-cost ties.
pub(super) struct NodeSlices<T> {
    /// Node slice boundaries, including the final sentinel.
    pub(super) offsets: Vec<usize>,
    /// Concatenated node payloads.
    pub(super) values: Vec<T>,
}

impl<T> NodeSlices<T> {
    /// Creates a layout with no entries at any node.
    pub(super) fn empty(nodes: usize) -> Self {
        Self {
            offsets: vec![0; nodes + 1],
            values: Vec::new(),
        }
    }

    /// Flattens ordered rows once during topology construction.
    pub(super) fn from_rows(rows: Vec<Vec<T>>) -> Self {
        let mut offsets = Vec::with_capacity(rows.len() + 1);
        offsets.push(0);
        for row in &rows {
            offsets.push(offsets.last().unwrap() + row.len());
        }
        let values = rows.into_iter().flatten().collect();
        Self { offsets, values }
    }

    /// Number of nodes, excluding the offset sentinel.
    pub(super) fn len(&self) -> usize {
        self.offsets.len() - 1
    }

    /// Node slices in node-ID order.
    pub(super) fn iter(&self) -> impl Iterator<Item = &[T]> {
        self.offsets
            .windows(2)
            .map(|ends| &self.values[ends[0]..ends[1]])
    }
}

impl<T> Index<usize> for NodeSlices<T> {
    type Output = [T];

    fn index(&self, node: usize) -> &Self::Output {
        &self.values[self.offsets[node]..self.offsets[node + 1]]
    }
}

/// Exact edge-pair projection of the vehicle connectors the lane builder creates.
pub(super) struct VehicleTurns {
    /// Nodes whose legal departures depend on the arrival edge: explicit whitelists, and every
    /// open node except a dead end, where the lane builder creates no U-turn connector.
    pub(super) restricted: Vec<bool>,
    /// Restricted nodes governed by an explicit whitelist; a whitelist may be empty.
    whitelisted: Vec<bool>,
    /// Sorted unique permitted edge pairs per whitelisted node.
    pub(super) allowed: NodeSlices<(usize, usize)>,
}

impl VehicleTurns {
    /// Creates unrestricted nodes for an empty hierarchy.
    pub(super) fn empty(nodes: usize) -> Self {
        Self {
            restricted: vec![false; nodes],
            whitelisted: vec![false; nodes],
            allowed: NodeSlices::empty(nodes),
        }
    }

    /// Compiles source-lane whitelists once per topology, independently per node with Rayon.
    pub(super) fn build(graph: &RegionGraph) -> Self {
        // Only source lane ±100 denotes a pedestrian entry. Preserve target lane semantics,
        // nonincident edge IDs, duplicate lane entries and empty vehicle whitelists exactly.
        let rows: Vec<_> = (0..graph.node_count())
            .into_par_iter()
            .map(|id| {
                let node = graph.node(id as u32);
                let whitelisted = CchGraph::node_has_vehicle_whitelist(node);
                let mut pairs: Vec<_> = node
                    .lane_connections
                    .iter()
                    .filter(|&(&(_, lane), _)| lane != 100 && lane != -100)
                    .flat_map(|(&(incoming, _), targets)| {
                        targets
                            .iter()
                            .map(move |&(outgoing, _)| (incoming, outgoing))
                    })
                    .collect();
                pairs.sort_unstable();
                pairs.dedup();
                let restricted = whitelisted || !graph.open_node_allows_u_turn(id as u32);
                (restricted, whitelisted, pairs)
            })
            .collect();
        let mut allowed = NodeSlices::empty(graph.node_count());
        for (node, (_, _, pairs)) in rows.iter().enumerate() {
            allowed.offsets[node + 1] = allowed.offsets[node] + pairs.len();
        }
        allowed.values.reserve(allowed.offsets[graph.node_count()]);
        let mut restricted = Vec::with_capacity(graph.node_count());
        let mut whitelisted = Vec::with_capacity(graph.node_count());
        for (flag, whitelist, pairs) in rows {
            restricted.push(flag);
            whitelisted.push(whitelist);
            allowed.values.extend(pairs);
        }
        Self {
            restricted,
            whitelisted,
            allowed,
        }
    }

    /// O(1) at open nodes, O(log T) for T explicit pairs at whitelisted junctions.
    /// Other restricted nodes permit every turn except the U-turn onto the arrival edge.
    pub(super) fn permits(&self, node: u32, incoming: usize, outgoing: usize) -> bool {
        if self.whitelisted[node as usize] {
            self.allowed[node as usize]
                .binary_search(&(incoming, outgoing))
                .is_ok()
        } else {
            !self.restricted[node as usize] || incoming != outgoing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::Node;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;
    use std::collections::HashMap;

    #[test]
    fn compiled_turns_match_lane_map_including_empty_and_pedestrian_entries() {
        let mut nodes = Vec::new();
        for connections in [
            vec![],
            vec![((0, 100), vec![(1, 0)]), ((2, -100), vec![(3, 0)])],
            vec![((0, 0), vec![])],
            vec![
                ((0, 0), vec![(1, 100), (1, 0)]),
                ((0, 1), vec![(1, 1), (2, -100)]),
                ((2, 100), vec![(3, 0)]),
                ((3, -100), vec![(0, 0)]),
                ((3, -1), vec![]),
            ],
        ] {
            nodes.push(Node {
                pos: Vector3::ZERO,
                node_type: NodeType::Junction,
                lane_connections: connections.into_iter().collect(),
                crosswalk_overrides: HashMap::new(),
            });
        }
        let mut graph = RegionGraph::new();
        graph.set_nodes_edges_for_test(nodes, Vec::new());
        graph.rebuild_adjacency_list();
        for phase in 0..2 {
            if phase == 1 {
                graph.remove_lane_connection(2, (0, 0));
                graph.add_lane_connection(1, 3, 0, 2, 0);
            }
            let turns = VehicleTurns::build(&graph);
            for node in 0..graph.node_count() as u32 {
                for incoming in 0..5 {
                    for outgoing in 0..5 {
                        assert_eq!(
                            turns.permits(node, incoming, outgoing),
                            CchGraph::vehicle_turn_allowed(&graph, node, incoming, outgoing),
                            "phase={phase} node={node} {incoming}->{outgoing}"
                        );
                    }
                }
            }
            assert_eq!(turns.allowed[3], [(0, 1), (0, 2)]);
        }
    }
}
