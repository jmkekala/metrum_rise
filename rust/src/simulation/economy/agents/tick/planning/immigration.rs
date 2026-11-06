// SPDX-License-Identifier: GPL-2.0-only

//! Initial access plan construction for immigrating cars.

use super::super::super::{
    ACCESS_IMMIGRATION_ORIGIN, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR,
};
use super::super::access::{
    frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::lane_origin_node;
use super::candidate::TripQueryScratch;
use super::candidate::{
    NODE_RANKS, PlannedTripCandidate, candidate_better, candidate_lane_id, mode_choice_cost_for,
};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::{AtomicU32, Ordering};

/// Builds an initial exact access plan for an immigrating car entering from a border node.
pub(crate) fn plan_immigration_trip(
    border_node: u32,
    target_building: usize,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    let entrance = allocator.entrances.get(target_building)?;
    plan_border_arrival(border_node, target_building, MODE_CAR, entrance,
        transit_network, graph, pathfind_count)
}

/// Exact border-to-endpoint quote/materialization, O(Q) for two directed frontage choices.
/// Cars receive a parking entrance; pedestrians receive the home door. Freight retains its wrapper.
pub(crate) fn plan_border_arrival(
    border_node: u32,
    target_building: usize,
    mode: u8,
    destination_entrance: &crate::simulation::buildings::allocator::BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    if destination_entrance.edge_idx >= graph.edge_count()
        || graph.edge(destination_entrance.edge_idx).deleted
    {
        return None;
    }

    TripQueryScratch::with_routes(|scratch| {
        let mut best_candidate: Option<PlannedTripCandidate> = None;
        for destination_rank in NODE_RANKS {
            let planned_detach_node = if destination_rank == 0 {
                graph.edge(destination_entrance.edge_idx).start_node
            } else {
                graph.edge(destination_entrance.edge_idx).end_node
            };
            let planned_detach_lane_id =
                candidate_lane_id(mode, destination_entrance, destination_rank == 0, false);
            if planned_detach_lane_id == usize::MAX {
                continue;
            }
            if lane_origin_node(planned_detach_lane_id, transit_network, graph)?
                != planned_detach_node
            {
                continue;
            }
            let planned_detach_lane_d = projected_lane_distance_for_entrance(
                destination_entrance,
                planned_detach_lane_id,
                transit_network,
                graph,
            )?;
            let ingress_local_time_s = local_access_time_s(
                local_access_distance(
                    mode,
                    destination_entrance,
                    planned_detach_lane_id,
                    planned_detach_lane_d,
                    transit_network,
                    graph,
                )?,
                mode,
            );
            let destination_frontage_time_s = frontage_time_s(
                mode,
                planned_detach_lane_id,
                planned_detach_lane_d,
                false,
                transit_network,
                graph,
            )?;
            let network_path_time_s = if border_node == planned_detach_node {
                0.0
            } else {
                pathfind_count.fetch_add(1, Ordering::Relaxed);
                scratch.cch_cost(
                    border_node,
                    planned_detach_node,
                    usize::MAX,
                    destination_entrance.edge_idx,
                    mode,
                    transit_network,
                    graph,
                )?
            };
            let total_cost_s =
                network_path_time_s + destination_frontage_time_s + ingress_local_time_s;
            let candidate = PlannedTripCandidate {
                total_cost_s,
                mode_choice_cost_s: mode_choice_cost_for(mode, total_cost_s),
                origin_rank: 0,
                destination_rank,
                mode,
                planned_attach_node: border_node,
                planned_detach_node,
                planned_attach_lane_id: usize::MAX,
                planned_detach_lane_id,
                planned_attach_lane_d: 0.0,
                planned_detach_lane_d,
                #[cfg(test)]
                network_path: None,
            };
            if best_candidate
                .as_ref()
                .is_none_or(|best| candidate_better(&candidate, best))
            {
                scratch.retain_network_path(border_node == planned_detach_node);
                best_candidate = Some(candidate);
            }
        }
        let chosen = best_candidate?;
        let current_path = scratch.finish_path();
        let mut access_flags = ACCESS_PLAN_VALID | ACCESS_IMMIGRATION_ORIGIN;
        if chosen.planned_attach_node == chosen.planned_detach_node {
            access_flags |= ACCESS_ZERO_HOP_NODE_PATH;
        } else if current_path.len() < 2 {
            return None;
        }

        Some(BuiltTripPlan {
            mode,
            target_building,
            activity: 0,
            planned_attach_node: chosen.planned_attach_node,
            planned_detach_node: chosen.planned_detach_node,
            planned_attach_lane_id: usize::MAX,
            planned_detach_lane_id: chosen.planned_detach_lane_id,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d: chosen.planned_detach_lane_d,
            current_path,
            access_flags,
        })
    })
}
