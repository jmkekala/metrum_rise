//! Building-origin trip planning and commute estimates.

use super::super::super::{MODE_CAR, MODE_WALK};
use super::candidate::{
    best_trip_candidate_for_mode, build_exact_path_for_candidate, entrance_pair_supports_mode,
};
use super::types::BuiltTripPlan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::AtomicU32;

/// Builds a full plan for a trip that starts inside a building.
pub(in crate::simulation::economy::agents::tick) fn plan_building_origin_trip(
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
            if walk.total_cost_s <= car.total_cost_s {
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

    let mut best_cost_s: Option<f32> = None;
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
        ) && best_cost_s.is_none_or(|best| candidate.total_cost_s < best)
        {
            best_cost_s = Some(candidate.total_cost_s);
        }
    }

    best_cost_s.map(|seconds| seconds.ceil().clamp(1.0, u16::MAX as f32) as u16)
}
