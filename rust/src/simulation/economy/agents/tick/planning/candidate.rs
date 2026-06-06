//! Candidate scoring and exact path construction for building-origin trips.

use super::super::super::{
    ACCESS_PATH_FROM_FLOW_FIELD, ACCESS_PLAN_VALID, ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR,
};
use super::super::access::{
    direct_frontage_segment_time_s, frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::super::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use std::sync::atomic::{AtomicU32, Ordering};

pub(super) const NODE_RANKS: [u8; 2] = [0, 1];
const NODE_RANK_PAIRS: [(u8, u8); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];

#[derive(Clone)]
pub(super) struct PlannedTripCandidate {
    pub(super) total_cost_s: f32,
    pub(super) origin_rank: u8,
    pub(super) destination_rank: u8,
    pub(super) mode: u8,
    pub(super) planned_attach_node: u32,
    pub(super) planned_detach_node: u32,
    pub(super) planned_attach_lane_id: usize,
    pub(super) planned_detach_lane_id: usize,
    pub(super) planned_attach_lane_d: f32,
    pub(super) planned_detach_lane_d: f32,
    pub(super) network_path: Option<Vec<u32>>,
}

pub(super) fn transit_flags_for_mode(mode: u8) -> u8 {
    if mode == MODE_CAR {
        TransitFlags::CAR
    } else {
        TransitFlags::FOOT
    }
}

pub(super) fn candidate_better(
    new_candidate: &PlannedTripCandidate,
    best: &PlannedTripCandidate,
) -> bool {
    new_candidate.total_cost_s < best.total_cost_s
        || (new_candidate.total_cost_s == best.total_cost_s
            && (
                new_candidate.origin_rank,
                new_candidate.destination_rank,
                new_candidate.planned_attach_lane_id,
                new_candidate.planned_detach_lane_id,
                new_candidate.planned_attach_lane_d.to_bits(),
                new_candidate.planned_detach_lane_d.to_bits(),
            ) < (
                best.origin_rank,
                best.destination_rank,
                best.planned_attach_lane_id,
                best.planned_detach_lane_id,
                best.planned_attach_lane_d.to_bits(),
                best.planned_detach_lane_d.to_bits(),
            ))
}

pub(super) fn candidate_lane_id(
    mode: u8,
    entrance: &BuildingEntrance,
    toward_start: bool,
    origin: bool,
) -> usize {
    match (mode == MODE_CAR, toward_start, origin) {
        (false, true, true) => entrance.foot_lane_bkw,
        (false, false, true) => entrance.foot_lane_fwd,
        (false, true, false) => entrance.foot_lane_fwd,
        (false, false, false) => entrance.foot_lane_bkw,
        (true, true, true) => entrance.car_lane_bkw,
        (true, false, true) => entrance.car_lane_fwd,
        (true, true, false) => entrance.car_lane_fwd,
        (true, false, false) => entrance.car_lane_bkw,
    }
}

fn evaluate_planned_trip_candidate(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    if origin_entrance.edge_idx >= graph.edge_count()
        || destination_entrance.edge_idx >= graph.edge_count()
    {
        return None;
    }
    let origin_edge = graph.edge(origin_entrance.edge_idx);
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if origin_edge.deleted || destination_edge.deleted {
        return None;
    }

    let planned_attach_node = if origin_rank == 0 {
        origin_edge.start_node
    } else {
        origin_edge.end_node
    };
    let planned_detach_node = if destination_rank == 0 {
        destination_edge.start_node
    } else {
        destination_edge.end_node
    };

    let planned_attach_lane_id = candidate_lane_id(mode, origin_entrance, origin_rank == 0, true);
    let planned_detach_lane_id =
        candidate_lane_id(mode, destination_entrance, destination_rank == 0, false);
    if planned_attach_lane_id == usize::MAX || planned_detach_lane_id == usize::MAX {
        return None;
    }
    if lane_terminal_node(planned_attach_lane_id, transit_network, graph)? != planned_attach_node {
        return None;
    }
    if lane_origin_node(planned_detach_lane_id, transit_network, graph)? != planned_detach_node {
        return None;
    }

    let planned_attach_lane_d = projected_lane_distance_for_entrance(
        origin_entrance,
        planned_attach_lane_id,
        transit_network,
        graph,
    )?;
    let planned_detach_lane_d = projected_lane_distance_for_entrance(
        destination_entrance,
        planned_detach_lane_id,
        transit_network,
        graph,
    )?;

    let egress_local_time_s = local_access_time_s(
        local_access_distance(
            mode,
            origin_entrance,
            planned_attach_lane_id,
            planned_attach_lane_d,
            transit_network,
            graph,
        )?,
        mode,
    );
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
    let same_lane_direct_frontage = mode == MODE_CAR
        && origin_entrance.edge_idx == destination_entrance.edge_idx
        && planned_attach_lane_id == planned_detach_lane_id
        && planned_attach_lane_d <= planned_detach_lane_d + 1e-6;

    let mut network_path = None;
    let total_cost_s = if same_lane_direct_frontage {
        let direct_frontage_time_s = direct_frontage_segment_time_s(
            mode,
            planned_attach_lane_id,
            planned_attach_lane_d,
            planned_detach_lane_d,
            transit_network,
            graph,
        )?;
        egress_local_time_s + direct_frontage_time_s + ingress_local_time_s
    } else {
        let origin_frontage_time_s = frontage_time_s(
            mode,
            planned_attach_lane_id,
            planned_attach_lane_d,
            true,
            transit_network,
            graph,
        )?;
        let destination_frontage_time_s = frontage_time_s(
            mode,
            planned_detach_lane_id,
            planned_detach_lane_d,
            false,
            transit_network,
            graph,
        )?;

        let network_path_time_s = if planned_attach_node == planned_detach_node {
            0.0
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (travel_seconds, _, path) = transit_network.cch_graph.find_path(
                planned_attach_node,
                planned_detach_node,
                usize::MAX,
                graph,
                transit_flags_for_mode(mode),
            )?;
            network_path = Some(path);
            travel_seconds
        };

        egress_local_time_s
            + origin_frontage_time_s
            + network_path_time_s
            + destination_frontage_time_s
            + ingress_local_time_s
    };
    if !total_cost_s.is_finite() {
        return None;
    }

    Some(PlannedTripCandidate {
        total_cost_s,
        origin_rank,
        destination_rank,
        mode,
        planned_attach_node,
        planned_detach_node,
        planned_attach_lane_id,
        planned_detach_lane_id,
        planned_attach_lane_d,
        planned_detach_lane_d,
        network_path,
    })
}

pub(super) fn build_exact_path_for_candidate(
    candidate: &mut PlannedTripCandidate,
    target_building: usize,
    target_zone: ZoneType,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<(Vec<u32>, u8)> {
    let direct_same_lane_frontage = candidate.mode == MODE_CAR
        && candidate.planned_attach_lane_id == candidate.planned_detach_lane_id
        && candidate.planned_attach_lane_id != usize::MAX
        && candidate.planned_attach_lane_d <= candidate.planned_detach_lane_d + 1e-6;
    if direct_same_lane_frontage {
        return Some((Vec::new(), ACCESS_PLAN_VALID));
    }

    if candidate.planned_attach_node == candidate.planned_detach_node {
        return Some((Vec::new(), ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH));
    }

    let mut access_flags = ACCESS_PLAN_VALID;
    let flow_field = if candidate.mode == MODE_CAR {
        transit_network.flow_fields.car(target_zone)
    } else {
        transit_network.flow_fields.foot(target_zone)
    };
    if let Some(ff) = flow_field {
        let attach_idx = candidate.planned_attach_node as usize;
        if attach_idx < ff.nearest_building.len()
            && ff.nearest_building[attach_idx] == target_building
        {
            if let Some(path) = ff.build_path(candidate.planned_attach_node, graph.node_count() + 1)
            {
                if path.last().copied() == Some(candidate.planned_detach_node)
                    && crate::simulation::pathing::cch::CchGraph::path_has_valid_turns(&path, graph)
                {
                    access_flags |= ACCESS_PATH_FROM_FLOW_FIELD;
                    return Some((path, access_flags));
                }
            }
        }
    }

    if let Some(path) = candidate.network_path.take() {
        if path.len() < 2 {
            return None;
        }
        return Some((path, access_flags));
    }

    pathfind_count.fetch_add(1, Ordering::Relaxed);
    let path = transit_network
        .cch_graph
        .find_path(
            candidate.planned_attach_node,
            candidate.planned_detach_node,
            usize::MAX,
            graph,
            transit_flags_for_mode(candidate.mode),
        )
        .map(|(_, _, path)| path)?;
    if path.len() < 2 {
        return None;
    }
    Some((path, access_flags))
}

pub(super) fn entrance_pair_supports_mode(
    mode: u8,
    has_car: bool,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
) -> bool {
    if mode == MODE_CAR {
        has_car
            && (origin_entrance.car_lane_fwd != usize::MAX
                || origin_entrance.car_lane_bkw != usize::MAX)
            && (destination_entrance.car_lane_fwd != usize::MAX
                || destination_entrance.car_lane_bkw != usize::MAX)
    } else {
        (origin_entrance.foot_lane_fwd != usize::MAX || origin_entrance.foot_lane_bkw != usize::MAX)
            && (destination_entrance.foot_lane_fwd != usize::MAX
                || destination_entrance.foot_lane_bkw != usize::MAX)
    }
}

pub(super) fn best_trip_candidate_for_mode(
    mode: u8,
    origin_entrance: &BuildingEntrance,
    destination_entrance: &BuildingEntrance,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    let mut best: Option<PlannedTripCandidate> = None;
    for (origin_rank, destination_rank) in NODE_RANK_PAIRS {
        if let Some(candidate) = evaluate_planned_trip_candidate(
            mode,
            origin_rank,
            destination_rank,
            origin_entrance,
            destination_entrance,
            transit_network,
            graph,
            pathfind_count,
        ) {
            if best
                .as_ref()
                .is_none_or(|best| candidate_better(&candidate, best))
            {
                best = Some(candidate);
            }
        }
    }
    best
}
