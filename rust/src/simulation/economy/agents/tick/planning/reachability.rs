// SPDX-License-Identifier: GPL-2.0-only

//! Positive walking-route proofs over the actual directed pedestrian lane connectors.

use super::super::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, lanes::LaneType};
use rayon::prelude::*;

/// Compact positive walking proof at one door, scoped to its pedestrian network generation.
/// Unproven trips, including equal-node attachments, still need the ordinary route query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildingWalkingAccess {
    origins: [usize; 2],
    destinations: [usize; 2],
    nodes: [u32; 2],
}

impl Default for BuildingWalkingAccess {
    fn default() -> Self {
        Self {
            origins: [usize::MAX; 2],
            destinations: [usize::MAX; 2],
            nodes: [u32::MAX; 2],
        }
    }
}

impl BuildingWalkingAccess {
    /// Wraps already validated node-ranked groups from an existing access census.
    pub(crate) fn from_groups(
        origins: [usize; 2],
        destinations: [usize; 2],
        nodes: [u32; 2],
    ) -> Self {
        Self {
            origins,
            destinations,
            nodes,
        }
    }

    /// Uses exactly the ordinary planner's pedestrian geometry and finite-frontage checks.
    pub(crate) fn at_entrance(
        entrance: &crate::simulation::buildings::allocator::BuildingEntrance,
        network: &TransitNetwork,
        graph: &RegionGraph,
        walking: &PedestrianReachability,
    ) -> Self {
        let [origin, destination] =
            super::trip::BuildingTripEnds::walking_pair_at_entrance(entrance, network, graph);
        let nodes = graph
            .get_edge(entrance.edge_idx)
            .map_or([u32::MAX; 2], |edge| [edge.start_node, edge.end_node]);
        Self::from_groups(
            origin.walking_groups(walking),
            destination.walking_groups(walking),
            nodes,
        )
    }

    /// Proves one directed trip without accepting a cycle in place of a zero-hop connector.
    pub(crate) fn proves_to(self, destination: Self) -> bool {
        self.origins.into_iter().enumerate().any(|(from, group)| {
            group != usize::MAX
                && destination
                    .destinations
                    .into_iter()
                    .enumerate()
                    .any(|(to, target)| {
                        group == target && self.nodes[from] != destination.nodes[to]
                    })
        })
    }
}

/// Cached strongly connected pedestrian lane groups. A shared cyclic group proves a
/// legal directed connector path; other pairs still require the ordinary trip query.
#[derive(Clone, Debug, Default)]
pub(crate) struct PedestrianReachability {
    groups: Vec<usize>,
    cyclic: Vec<bool>,
    outgoing: Vec<Vec<usize>>,
    visited: Vec<u64>,
    visit: u64,
    pending: Vec<usize>,
}

impl PedestrianReachability {
    /// Builds in O(lanes + connectors) on access-topology preparation, never per buyer.
    /// Edges match the trip planner's pedestrian connector fallback, including direction.
    pub(crate) fn build(network: &TransitNetwork, graph: &RegionGraph) -> Self {
        let lanes = &network.lane_system.lanes;
        let valid = |index: usize| {
            lanes.get(index).is_some_and(|lane| {
                lane.lane_type == LaneType::Foot
                    && graph.get_edge(lane.edge_id).is_some_and(|edge| {
                        !edge.deleted
                            && edge.allowed_types & TransitFlags::FOOT != 0
                            && edge.base_cost.is_finite()
                            && lane.length.is_finite()
                    })
            })
        };
        let outgoing: Vec<Vec<usize>> = lanes
            .par_iter()
            .enumerate()
            .map(|(index, lane)| {
                if !valid(index) {
                    return Vec::new();
                }
                let Some(node) = lane_terminal_node(index, network, graph) else {
                    return Vec::new();
                };
                lane.next_lanes
                    .iter()
                    .filter_map(|&connector| {
                        let connector = lanes.get(connector)?;
                        if connector.edge_id != usize::MAX
                            || connector.lane_type != LaneType::Foot
                            || !connector.length.is_finite()
                        {
                            return None;
                        }
                        let &target = connector.next_lanes.first()?;
                        (valid(target)
                            && lane_origin_node(target, network, graph) == Some(node)
                            && lane_terminal_node(target, network, graph).is_some())
                        .then_some(target)
                    })
                    .collect()
            })
            .collect();
        let mut incoming = vec![Vec::new(); lanes.len()];
        for (source, targets) in outgoing.iter().enumerate() {
            for &target in targets {
                incoming[target].push(source);
            }
        }
        let (groups, count) =
            crate::simulation::pathing::components::strong_components(&outgoing, &incoming);
        let mut sizes = vec![0_usize; count];
        for &group in &groups {
            sizes[group] += 1;
        }
        let mut cyclic: Vec<bool> = sizes.into_iter().map(|size| size > 1).collect();
        for (lane, targets) in outgoing.iter().enumerate() {
            if targets.contains(&lane) {
                cyclic[groups[lane]] = true;
            }
        }
        let mut group_edges = vec![Vec::new(); count];
        for (source, targets) in outgoing.iter().enumerate() {
            for &target in targets {
                if groups[source] != groups[target] {
                    group_edges[groups[source]].push(groups[target]);
                }
            }
        }
        Self {
            groups,
            cyclic,
            outgoing: group_edges,
            visited: vec![0; count],
            visit: 0,
            pending: Vec::with_capacity(count),
        }
    }

    /// Preserves the graph-bounded traversal reserve after a construction-market fork.
    pub(crate) fn prepare_query(&mut self) {
        self.pending.clear();
        self.pending.reserve(self.outgoing.len());
    }

    /// Exact directed connector reachability. Shared cyclic groups are O(1); otherwise
    /// visits only the reachable condensed graph, O(groups + connectors) worst case.
    /// Mark-before-push bounds reusable stack storage by group count, without query allocations.
    pub(super) fn reaches(&mut self, from: usize, to: usize) -> bool {
        let (Some(&from), Some(&to)) = (self.groups.get(from), self.groups.get(to)) else {
            return false;
        };
        if from == to {
            return self.cyclic[from];
        }
        self.visit = self.visit.wrapping_add(1);
        if self.visit == 0 {
            self.visited.fill(0);
            self.visit = 1;
        }
        self.pending.clear();
        self.pending.push(from);
        self.visited[from] = self.visit;
        while let Some(group) = self.pending.pop() {
            for &target in &self.outgoing[group] {
                if target == to {
                    return true;
                }
                if self.visited[target] != self.visit {
                    self.visited[target] = self.visit;
                    self.pending.push(target);
                }
            }
        }
        false
    }

    pub(super) fn proves(&self, from_lane: usize, to_lane: usize) -> bool {
        match (self.groups.get(from_lane), self.groups.get(to_lane)) {
            (Some(&from), Some(&to)) => from == to && self.cyclic[from],
            _ => false,
        }
    }

    pub(super) fn cyclic_group(&self, lane: usize) -> usize {
        self.groups
            .get(lane)
            .copied()
            .filter(|&group| self.cyclic[group])
            .unwrap_or(usize::MAX)
    }
}
