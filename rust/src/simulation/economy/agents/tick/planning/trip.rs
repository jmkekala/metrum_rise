//! Building-origin trip planning and commute estimates.

use super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, MODE_WALK,
};
use super::super::access::{
    frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::lane_terminal_node;
use super::candidate::{
    PlannedTripCandidate, best_trip_candidate_for_mode, build_exact_path_for_candidate,
    candidate_better, candidate_lane_id, entrance_pair_supports_mode, mode_choice_cost_for,
};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use std::sync::atomic::{AtomicU32, Ordering};

/// Builds a full plan for a trip that starts inside a building.
pub(crate) fn plan_building_origin_trip(
    current_building: usize,
    target_building: usize,
    activity: u8,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    if current_building >= allocator.buildings.len()
        || target_building >= allocator.buildings.len()
        || current_building >= allocator.entrances.len()
        || target_building >= allocator.entrances.len()
    {
        return None;
    }

    let origin_entrance = &allocator.entrances[current_building];
    let destination_entrance = &allocator.entrances[target_building];
    let best_walk =
        if entrance_pair_supports_mode(MODE_WALK, has_car, origin_entrance, destination_entrance) {
            best_trip_candidate_for_mode(
                MODE_WALK,
                origin_entrance,
                destination_entrance,
                transit_network,
                graph,
                pathfind_count,
            )
        } else {
            None
        };
    let best_car =
        if entrance_pair_supports_mode(MODE_CAR, has_car, origin_entrance, destination_entrance) {
            best_trip_candidate_for_mode(
                MODE_CAR,
                origin_entrance,
                destination_entrance,
                transit_network,
                graph,
                pathfind_count,
            )
        } else {
            None
        };

    let mut chosen = match (best_walk, best_car) {
        (None, None) => return None,
        (Some(walk), None) => walk,
        (None, Some(car)) => car,
        (Some(walk), Some(car)) => {
            if walk.mode_choice_cost_s <= car.mode_choice_cost_s {
                walk
            } else {
                car
            }
        }
    };

    let target_zone = allocator.buildings[target_building].zone_type;
    let (current_path, access_flags) = build_exact_path_for_candidate(
        &mut chosen,
        target_building,
        target_zone,
        transit_network,
        graph,
        pathfind_count,
    )?;

    Some(BuiltTripPlan {
        mode: chosen.mode,
        target_building,
        activity,
        planned_attach_node: chosen.planned_attach_node,
        planned_detach_node: chosen.planned_detach_node,
        planned_attach_lane_id: chosen.planned_attach_lane_id,
        planned_detach_lane_id: chosen.planned_detach_lane_id,
        planned_attach_lane_d: chosen.planned_attach_lane_d,
        planned_detach_lane_d: chosen.planned_detach_lane_d,
        current_path,
        access_flags,
    })
}

/// Builds a full car plan from a building door to an outside-world border node.
pub(crate) fn plan_building_to_border_trip(
    current_building: usize,
    border_node: u32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltTripPlan> {
    if current_building >= allocator.buildings.len()
        || current_building >= allocator.entrances.len()
        || border_node as usize >= graph.node_count()
    {
        return None;
    }

    let origin_entrance = &allocator.entrances[current_building];
    if origin_entrance.edge_idx >= graph.edge_count()
        || graph.edge(origin_entrance.edge_idx).deleted
    {
        return None;
    }

    let origin_edge = graph.edge(origin_entrance.edge_idx);
    let mut best_candidate: Option<PlannedTripCandidate> = None;
    for origin_rank in super::candidate::NODE_RANKS {
        let planned_attach_node = if origin_rank == 0 {
            origin_edge.start_node
        } else {
            origin_edge.end_node
        };
        let planned_attach_lane_id =
            candidate_lane_id(MODE_CAR, origin_entrance, origin_rank == 0, true);
        if planned_attach_lane_id == usize::MAX {
            continue;
        }
        if lane_terminal_node(planned_attach_lane_id, transit_network, graph)?
            != planned_attach_node
        {
            continue;
        }

        let planned_attach_lane_d = projected_lane_distance_for_entrance(
            origin_entrance,
            planned_attach_lane_id,
            transit_network,
            graph,
        )?;
        let egress_local_time_s = local_access_time_s(
            local_access_distance(
                MODE_CAR,
                origin_entrance,
                planned_attach_lane_id,
                planned_attach_lane_d,
                transit_network,
                graph,
            )?,
            MODE_CAR,
        );
        let origin_frontage_time_s = frontage_time_s(
            MODE_CAR,
            planned_attach_lane_id,
            planned_attach_lane_d,
            true,
            transit_network,
            graph,
        )?;

        let mut network_path = None;
        let network_path_time_s = if planned_attach_node == border_node {
            0.0
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (travel_seconds, _, path) = transit_network.cch_graph.find_path(
                planned_attach_node,
                border_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            )?;
            network_path = Some(path);
            travel_seconds
        };
        let total_cost_s = egress_local_time_s + origin_frontage_time_s + network_path_time_s;
        if !total_cost_s.is_finite() {
            continue;
        }

        let candidate = PlannedTripCandidate {
            total_cost_s,
            mode_choice_cost_s: mode_choice_cost_for(MODE_CAR, total_cost_s),
            origin_rank,
            destination_rank: 0,
            mode: MODE_CAR,
            planned_attach_node,
            planned_detach_node: border_node,
            planned_attach_lane_id,
            planned_detach_lane_id: usize::MAX,
            planned_attach_lane_d,
            planned_detach_lane_d: 0.0,
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
    let current_path = if chosen.planned_attach_node == border_node {
        Vec::new()
    } else if let Some(path) = chosen.network_path.take() {
        if path.len() < 2 {
            return None;
        }
        path
    } else {
        pathfind_count.fetch_add(1, Ordering::Relaxed);
        let path = transit_network
            .cch_graph
            .find_path(
                chosen.planned_attach_node,
                border_node,
                usize::MAX,
                graph,
                TransitFlags::CAR,
            )
            .map(|(_, _, path)| path)?;
        if path.len() < 2 {
            return None;
        }
        path
    };

    Some(BuiltTripPlan {
        mode: MODE_CAR,
        target_building: usize::MAX,
        activity: 2,
        planned_attach_node: chosen.planned_attach_node,
        planned_detach_node: border_node,
        planned_attach_lane_id: chosen.planned_attach_lane_id,
        planned_detach_lane_id: usize::MAX,
        planned_attach_lane_d: chosen.planned_attach_lane_d,
        planned_detach_lane_d: 0.0,
        current_path,
        access_flags: ACCESS_PLAN_VALID | ACCESS_FREIGHT_BORDER_DESTINATION,
    })
}

/// Returns whether the ordinary building-origin trip planner can build this trip.
pub(crate) fn building_origin_trip_is_feasible(
    current_building: usize,
    target_building: usize,
    activity: u8,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> bool {
    plan_building_origin_trip(
        current_building,
        target_building,
        activity,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    )
    .is_some()
}

/// Estimates a building-origin trip duration in whole simulation minutes.
pub(crate) fn estimate_building_origin_trip_minutes(
    current_building: usize,
    target_building: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<u16> {
    if current_building >= allocator.buildings.len()
        || target_building >= allocator.buildings.len()
        || current_building >= allocator.entrances.len()
        || target_building >= allocator.entrances.len()
    {
        return None;
    }

    let origin_entrance = &allocator.entrances[current_building];
    let destination_entrance = &allocator.entrances[target_building];

    let mut best_candidate: Option<PlannedTripCandidate> = None;
    for mode in [MODE_WALK, MODE_CAR] {
        if !entrance_pair_supports_mode(mode, has_car, origin_entrance, destination_entrance) {
            continue;
        }
        if let Some(candidate) = best_trip_candidate_for_mode(
            mode,
            origin_entrance,
            destination_entrance,
            transit_network,
            graph,
            pathfind_count,
        ) && best_candidate
            .as_ref()
            .is_none_or(|best| candidate_better(&candidate, best))
        {
            best_candidate = Some(candidate);
        }
    }

    best_candidate.map(|candidate| candidate.total_cost_s.ceil().clamp(1.0, u16::MAX as f32) as u16)
}
