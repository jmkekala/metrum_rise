// SPDX-License-Identifier: GPL-2.0-only

//! Positive walking-route proofs over the actual directed pedestrian lane connectors.

use super::super::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::network::types::TransitFlags;
use crate::simulation::network::{TransitNetwork, graph::RegionGraph, lanes::LaneType};
use rayon::prelude::*;

/// Cached strongly connected pedestrian lane groups. A shared cyclic group proves a
/// legal directed connector path; other pairs still require the ordinary trip query.
#[derive(Clone, Debug, Default)]
pub(crate) struct PedestrianReachability {
    groups: Vec<usize>,
    cyclic: Vec<bool>,
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
        Self { groups, cyclic }
    }

    pub(super) fn proves(&self, from_lane: usize, to_lane: usize) -> bool {
        match (self.groups.get(from_lane), self.groups.get(to_lane)) {
            (Some(&from), Some(&to)) => from == to && self.cyclic[from],
            _ => false,
        }
    }

    pub(super) fn cyclic_group(&self, lane: usize) -> usize {
        self.groups.get(lane).copied().filter(|&group| self.cyclic[group]).unwrap_or(usize::MAX)
    }
}
