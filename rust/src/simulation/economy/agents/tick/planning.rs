//! Trip planning and network replanning helpers for agent movement.

use super::super::{
    ACCESS_IMMIGRATION_ORIGIN, ACCESS_PATH_FROM_FLOW_FIELD, ACCESS_PLAN_VALID,
    ACCESS_ZERO_HOP_NODE_PATH, MODE_CAR, MODE_WALK,
};
use super::access::{
    direct_frontage_segment_time_s, frontage_time_s, local_access_distance, local_access_time_s,
    projected_lane_distance_for_entrance,
};
use super::lane_nav::{lane_origin_node, lane_terminal_node};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use std::sync::atomic::{AtomicU32, Ordering};

const NODE_RANKS: [u8; 2] = [0, 1];
const NODE_RANK_PAIRS: [(u8, u8); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];

#[derive(Clone)]
struct PlannedTripCandidate {
    total_cost_s: f32,
    origin_rank: u8,
    destination_rank: u8,
    mode: u8,
    planned_attach_node: u32,
    planned_detach_node: u32,
    planned_attach_lane_id: usize,
    planned_detach_lane_id: usize,
    planned_attach_lane_d: f32,
    planned_detach_lane_d: f32,
}

/// A fully built trip from a current building to a target building.
#[derive(Clone)]
pub(super) struct BuiltTripPlan {
    /// Travel mode chosen for the trip.
    pub(super) mode: u8,
    /// Destination building index.
    pub(super) target_building: usize,
    /// Activity to start after the trip completes.
    pub(super) activity: u8,
    /// Road node where the access-egress leg attaches to the network.
    pub(super) planned_attach_node: u32,
    /// Road node where the network leg detaches toward the destination.
    pub(super) planned_detach_node: u32,
    /// Lane used for the origin access handoff.
    pub(super) planned_attach_lane_id: usize,
    /// Lane used for the destination access handoff.
    pub(super) planned_detach_lane_id: usize,
    /// Distance along the attach lane for the origin handoff.
    pub(super) planned_attach_lane_d: f32,
    /// Distance along the detach lane for the destination handoff.
    pub(super) planned_detach_lane_d: f32,
    /// Planned network node path between attach and detach nodes.
    pub(super) current_path: Vec<u32>,
    /// Access-plan flags describing path provenance and special cases.
    pub(super) access_flags: u8,
}

/// A rebuilt destination-side network plan for an agent already outside a building.
#[derive(Clone)]
pub(super) struct BuiltNetworkReplan {
    /// Road node where the network leg detaches toward the destination.
    pub(super) planned_detach_node: u32,
    /// Lane used for the destination access handoff.
    pub(super) planned_detach_lane_id: usize,
    /// Distance along the detach lane for the destination handoff.
    pub(super) planned_detach_lane_d: f32,
    /// Planned network node path from current location to detach node.
    pub(super) current_path: Vec<u32>,
    /// Access-plan flags describing path provenance and special cases.
    pub(super) access_flags: u8,
}

fn transit_flags_for_mode(mode: u8) -> u8 {
    if mode == MODE_CAR {
        TransitFlags::CAR
    } else {
        TransitFlags::FOOT
    }
}

fn candidate_better(new_candidate: &PlannedTripCandidate, best: &PlannedTripCandidate) -> bool {
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

fn candidate_lane_id(
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
            transit_network
                .cch_graph
                .find_path(
                    planned_attach_node,
                    planned_detach_node,
                    usize::MAX,
                    graph,
                    transit_flags_for_mode(mode),
                )
                .map(|(travel_seconds, _, _)| travel_seconds)?
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
    })
}

fn build_exact_path_for_candidate(
    candidate: &PlannedTripCandidate,
    target_building: usize,
    target_zone: crate::simulation::zoning::ZoneType,
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

fn entrance_pair_supports_mode(
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

fn best_trip_candidate_for_mode(
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

/// Rebuilds a destination-side network plan for an agent already in transit.
pub(super) fn plan_network_replan(
    start_node: u32,
    incoming_edge: usize,
    target_building: usize,
    mode: u8,
    preserve_flags: u8,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<BuiltNetworkReplan> {
    if target_building >= allocator.buildings.len() || target_building >= allocator.entrances.len()
    {
        return None;
    }
    let destination_entrance = &allocator.entrances[target_building];
    if destination_entrance.edge_idx >= graph.edge_count() {
        return None;
    }
    let destination_edge = graph.edge(destination_entrance.edge_idx);
    if destination_edge.deleted {
        return None;
    }

    let mut best: Option<(f32, u8, usize, f32, u32, Vec<u32>)> = None;
    for destination_rank in NODE_RANKS {
        let planned_detach_node = if destination_rank == 0 {
            destination_edge.start_node
        } else {
            destination_edge.end_node
        };
        let planned_detach_lane_id =
            candidate_lane_id(mode, destination_entrance, destination_rank == 0, false);
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
        let (network_time_s, current_path) = if start_node == planned_detach_node {
            (0.0, Vec::new())
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let (travel_seconds, _, path) = transit_network.cch_graph.find_path(
                start_node,
                planned_detach_node,
                incoming_edge,
                graph,
                transit_flags_for_mode(mode),
            )?;
            if path.len() < 2 {
                continue;
            }
            (travel_seconds, path)
        };
        let total_cost_s = network_time_s + destination_frontage_time_s + ingress_local_time_s;
        let new_key = (
            total_cost_s.to_bits(),
            destination_rank,
            planned_detach_lane_id,
            planned_detach_lane_d.to_bits(),
        );
        let replace = match &best {
            None => true,
            Some((best_cost, best_rank, best_lane, best_d, _, _)) => {
                new_key
                    < (
                        best_cost.to_bits(),
                        *best_rank,
                        *best_lane,
                        best_d.to_bits(),
                    )
            }
        };
        if replace {
            best = Some((
                total_cost_s,
                destination_rank,
                planned_detach_lane_id,
                planned_detach_lane_d,
                planned_detach_node,
                current_path,
            ));
        }
    }

    let (_, _, planned_detach_lane_id, planned_detach_lane_d, planned_detach_node, current_path) =
        best?;
    let mut access_flags = ACCESS_PLAN_VALID | (preserve_flags & ACCESS_IMMIGRATION_ORIGIN);
    if current_path.is_empty() {
        access_flags |= ACCESS_ZERO_HOP_NODE_PATH;
    }

    Some(BuiltNetworkReplan {
        planned_detach_node,
        planned_detach_lane_id,
        planned_detach_lane_d,
        current_path,
        access_flags,
    })
}

/// Builds a full plan for a trip that starts inside a building.
pub(super) fn plan_building_origin_trip(
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

    let chosen = match (best_walk, best_car) {
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
        &chosen,
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
pub(super) fn estimate_building_origin_trip_minutes(
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

/// Builds an initial exact access plan for an immigrating car entering from a border node.
pub(super) fn plan_immigration_trip(
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
        let network_path_time_s = if border_node == planned_detach_node {
            0.0
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            transit_network
                .cch_graph
                .find_path(
                    border_node,
                    planned_detach_node,
                    usize::MAX,
                    graph,
                    TransitFlags::CAR,
                )
                .map(|(travel_seconds, _, _)| travel_seconds)?
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
        };
        if best_candidate
            .as_ref()
            .is_none_or(|best| candidate_better(&candidate, best))
        {
            best_candidate = Some(candidate);
        }
    }
    let chosen = best_candidate?;
    let (current_path, mut access_flags) =
        if chosen.planned_attach_node == chosen.planned_detach_node {
            (Vec::new(), ACCESS_PLAN_VALID | ACCESS_ZERO_HOP_NODE_PATH)
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
