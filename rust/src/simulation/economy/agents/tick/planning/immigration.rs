//! Initial access plan construction for immigrating cars.

use super::super::super::{
    ACCESS_IMMIGRATION_ORIGIN, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR,
};
use super::super::access::{
    frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::lane_origin_node;
use super::candidate::{NODE_RANKS, PlannedTripCandidate, candidate_better, candidate_lane_id};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
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
    if target_building >= allocator.buildings.len() || target_building >= allocator.entrances.len()
    {
        return None;
    }
    let destination_entrance = &allocator.entrances[target_building];
    if destination_entrance.edge_idx >= graph.edge_count()
        || graph.edge(destination_entrance.edge_idx).deleted
    {
        return None;
    }
    if destination_entrance.car_lane_fwd == usize::MAX
        && destination_entrance.car_lane_bkw == usize::MAX
    {
        return None;
    }

    let mut best_candidate: Option<PlannedTripCandidate> = None;
    for destination_rank in NODE_RANKS {
        let planned_detach_node = if destination_rank == 0 {
            graph.edge(destination_entrance.edge_idx).start_node
        } else {
            graph.edge(destination_entrance.edge_idx).end_node
        };
        let planned_detach_lane_id =
            candidate_lane_id(MODE_CAR, destination_entrance, destination_rank == 0, false);
        if planned_detach_lane_id == usize::MAX {
            continue;
        }
        if lane_origin_node(planned_detach_lane_id, transit_network, graph)? != planned_detach_node
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
                MODE_CAR,
                destination_entrance,
                planned_detach_lane_id,
                planned_detach_lane_d,
                transit_network,
                graph,
            )?,
            MODE_CAR,
        );
        let destination_frontage_time_s = frontage_time_s(
            MODE_CAR,
            planned_detach_lane_id,
            planned_detach_lane_d,
            false,
            transit_network,
            graph,
        )?;
        let mut network_path = None;
        let network_path_time_s = if border_node == planned_detach_node {
            0.0
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (travel_seconds, _, path) = transit_network.cch_graph.find_path(
                border_node,
                planned_detach_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            )?;
            network_path = Some(path);
            travel_seconds
        };
        let candidate = PlannedTripCandidate {
            total_cost_s: network_path_time_s + destination_frontage_time_s + ingress_local_time_s,
            origin_rank: 0,
            destination_rank,
            mode: MODE_CAR,
            planned_attach_node: border_node,
            planned_detach_node,
            planned_attach_lane_id: usize::MAX,
            planned_detach_lane_id,
            planned_attach_lane_d: 0.0,
            planned_detach_lane_d,
            network_path,
        };
        if best_candidate
            .as_ref()
            .is_none_or(|best| candidate_better(&candidate, best))
        {
            best_candidate = Some(candidate);
        }
    }
    let mut chosen = best_candidate?;
    let (current_path, mut access_flags) =
        if chosen.planned_attach_node == chosen.planned_detach_node {
            (Vec::new(), ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH)
        } else if let Some(path) = chosen.network_path.take() {
            if path.len() < 2 {
                return None;
            }
            (path, ACCESS_PLAN_VALID)
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let path = transit_network
                .cch_graph
                .find_path(
                    chosen.planned_attach_node,
                    chosen.planned_detach_node,
                    usize::MAX,
                    graph,
                    TransitFlags::CAR,
                )
                .map(|(_, _, path)| path)?;
            if path.len() < 2 {
                return None;
            }
            (path, ACCESS_PLAN_VALID)
        };
    access_flags |= ACCESS_IMMIGRATION_ORIGIN;

    Some(BuiltTripPlan {
        mode: MODE_CAR,
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
}
