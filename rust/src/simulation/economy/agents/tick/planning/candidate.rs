// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: candidate.rs
//  script_path: rust/src/simulation/economy/agents/tick/planning/candidate.rs
//  module_name: candidate
//  version: 0.1.0
//  description: Scores candidate trip plans and builds the exact path for
//  kind: module
//  spec: none
//  internal_dependencies: [graph, lanes, allocator]
//  external_dependencies: []
//  features: [trip-candidates, mode-choice, path-construction]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

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
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};

// ========================================================================
// SEARCH CONSTANTS
// ========================================================================

pub(super) const NODE_RANKS: [u8; 2] = [0, 1];
const NODE_RANK_PAIRS: [(u8, u8); 4] = [(0, 0), (0, 1), (1, 0), (1, 1)];
const CAR_MODE_CHOICE_OVERHEAD_S: f32 = 180.0;
const WALK_CONNECTOR_COST_SPEED_MS: f32 = 1.4;

// ========================================================================
// WHAT A CANDIDATE HOLDS
// ========================================================================

#[derive(Clone)]
pub(super) struct PlannedTripCandidate {
    pub(super) total_cost_s: f32,
    pub(super) mode_choice_cost_s: f32,
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

// ========================================================================
// SCORING A CANDIDATE
// ========================================================================

pub(super) fn transit_flags_for_mode(mode: u8) -> u8 {
    if mode == MODE_CAR {
        TransitFlags::CAR
    } else {
        TransitFlags::FOOT
    }
}

pub(super) fn mode_choice_cost_for(mode: u8, travel_time_s: f32) -> f32 {
    if mode == MODE_CAR {
        travel_time_s + CAR_MODE_CHOICE_OVERHEAD_S
    } else {
        travel_time_s
    }
}

pub(super) fn candidate_better(
    new_candidate: &PlannedTripCandidate,
    best: &PlannedTripCandidate,
) -> bool {
    match new_candidate
        .mode_choice_cost_s
        .total_cmp(&best.mode_choice_cost_s)
    {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => {
            (
                new_candidate.total_cost_s.to_bits(),
                new_candidate.origin_rank,
                new_candidate.destination_rank,
                new_candidate.planned_attach_lane_id,
                new_candidate.planned_detach_lane_id,
                new_candidate.planned_attach_lane_d.to_bits(),
                new_candidate.planned_detach_lane_d.to_bits(),
            ) < (
                best.total_cost_s.to_bits(),
                best.origin_rank,
                best.destination_rank,
                best.planned_attach_lane_id,
                best.planned_detach_lane_id,
                best.planned_attach_lane_d.to_bits(),
                best.planned_detach_lane_d.to_bits(),
            )
        }
    }
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
    let same_lane_direct_frontage = origin_entrance.edge_idx == destination_entrance.edge_idx
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
            if mode == MODE_CAR
                || same_lane_direct_frontage
                || connector_to_lane_exists(
                    planned_attach_lane_id,
                    planned_detach_lane_id,
                    transit_network,
                )
            {
                0.0
            } else {
                return None;
            }
        } else {
            pathfind_count.fetch_add(1, Ordering::Relaxed);
            let planned_attach_edge = transit_network
                .lane_system
                .lanes
                .get(planned_attach_lane_id)?
                .edge_id;
            let (mut travel_seconds, _, mut path) = transit_network.cch_graph.find_path(
                planned_attach_node,
                planned_detach_node,
                planned_attach_edge,
                graph,
                transit_flags_for_mode(mode),
            )?;
            if mode != MODE_CAR
                && !pedestrian_path_has_lane_connectors(
                    &path,
                    planned_attach_lane_id,
                    planned_detach_lane_id,
                    transit_network,
                    graph,
                )
            {
                let fallback = pedestrian_lane_connector_path(
                    planned_attach_node,
                    planned_detach_node,
                    planned_attach_lane_id,
                    planned_detach_lane_id,
                    transit_network,
                    graph,
                )?;
                travel_seconds = fallback.0;
                path = fallback.1;
            }
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
        mode_choice_cost_s: mode_choice_cost_for(mode, total_cost_s),
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

// ========================================================================
// THE EXACT PATH
// ========================================================================

pub(super) fn build_exact_path_for_candidate(
    candidate: &mut PlannedTripCandidate,
    target_building: usize,
    target_zone: ZoneType,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<(Vec<u32>, u8)> {
    let direct_same_lane_frontage = candidate.planned_attach_lane_id
        == candidate.planned_detach_lane_id
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
                let turns_valid =
                    crate::simulation::pathing::cch::CchGraph::path_has_valid_turns(&path, graph);
                let lanes_valid = candidate.mode == MODE_CAR
                    || pedestrian_path_has_lane_connectors(
                        &path,
                        candidate.planned_attach_lane_id,
                        candidate.planned_detach_lane_id,
                        transit_network,
                        graph,
                    );
                if path.last().copied() == Some(candidate.planned_detach_node)
                    && turns_valid
                    && lanes_valid
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
        if candidate.mode != MODE_CAR
            && !pedestrian_path_has_lane_connectors(
                &path,
                candidate.planned_attach_lane_id,
                candidate.planned_detach_lane_id,
                transit_network,
                graph,
            )
        {
            return pedestrian_lane_connector_path(
                candidate.planned_attach_node,
                candidate.planned_detach_node,
                candidate.planned_attach_lane_id,
                candidate.planned_detach_lane_id,
                transit_network,
                graph,
            )
            .map(|(_, path)| (path, access_flags));
        }
        return Some((path, access_flags));
    }

    pathfind_count.fetch_add(1, Ordering::Relaxed);
    let planned_attach_edge = transit_network
        .lane_system
        .lanes
        .get(candidate.planned_attach_lane_id)?
        .edge_id;
    let path = transit_network
        .cch_graph
        .find_path(
            candidate.planned_attach_node,
            candidate.planned_detach_node,
            planned_attach_edge,
            graph,
            transit_flags_for_mode(candidate.mode),
        )
        .map(|(_, _, path)| path)?;
    if path.len() < 2 {
        return None;
    }
    if candidate.mode != MODE_CAR
        && !pedestrian_path_has_lane_connectors(
            &path,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            transit_network,
            graph,
        )
    {
        return pedestrian_lane_connector_path(
            candidate.planned_attach_node,
            candidate.planned_detach_node,
            candidate.planned_attach_lane_id,
            candidate.planned_detach_lane_id,
            transit_network,
            graph,
        )
        .map(|(_, path)| (path, access_flags));
    }
    Some((path, access_flags))
}

// ========================================================================
// PEDESTRIAN CONNECTORS
// ========================================================================

pub(super) fn pedestrian_path_has_lane_connectors(
    path: &[u32],
    attach_lane_id: usize,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if path.len() < 2 {
        return connector_to_lane_exists(attach_lane_id, detach_lane_id, transit_network);
    }

    let mut possible_lanes = vec![attach_lane_id];
    for node_pair in path.windows(2) {
        let from_node = node_pair[0];
        let to_node = node_pair[1];
        let Some(out_edge) = graph.get_edge_between_nodes(from_node, to_node) else {
            return false;
        };
        let mut next_lanes = Vec::with_capacity(4);
        for lane_id in possible_lanes {
            collect_connector_targets_to_edge(
                lane_id,
                from_node,
                out_edge,
                transit_network,
                graph,
                &mut next_lanes,
            );
        }
        if next_lanes.is_empty() {
            return false;
        }
        next_lanes.sort_unstable();
        next_lanes.dedup();
        possible_lanes = next_lanes;
    }

    possible_lanes.into_iter().any(|lane_id| {
        connector_to_lane_exists(lane_id, detach_lane_id, transit_network)
            || lane_id == detach_lane_id
    })
}

pub(super) fn pedestrian_path_has_lane_connectors_from_edge(
    path: &[u32],
    start_node: u32,
    incoming_edge: usize,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    let start_lanes =
        pedestrian_incoming_lanes_at_node(start_node, incoming_edge, transit_network, graph);
    pedestrian_path_has_lane_connectors_from_start_lanes(
        path,
        start_lanes,
        detach_lane_id,
        transit_network,
        graph,
    )
}

pub(super) fn pedestrian_path_has_lane_connectors_from_node(
    path: &[u32],
    start_node: u32,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if path.first().copied() != Some(start_node) {
        return false;
    }
    if path.len() < 2 {
        return lane_origin_node(detach_lane_id, transit_network, graph) == Some(start_node);
    }
    let start_lanes =
        pedestrian_start_lanes_from_node_path(path, start_node, transit_network, graph);
    pedestrian_path_has_lane_connectors_from_start_lanes(
        &path[1..],
        start_lanes,
        detach_lane_id,
        transit_network,
        graph,
    )
}

fn pedestrian_path_has_lane_connectors_from_start_lanes(
    path: &[u32],
    mut possible_lanes: Vec<usize>,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if possible_lanes.is_empty() {
        return false;
    }
    if path.len() < 2 {
        return possible_lanes.into_iter().any(|lane_id| {
            connector_to_lane_exists(lane_id, detach_lane_id, transit_network)
                || lane_id == detach_lane_id
        });
    }

    for node_pair in path.windows(2) {
        let from_node = node_pair[0];
        let to_node = node_pair[1];
        let Some(out_edge) = graph.get_edge_between_nodes(from_node, to_node) else {
            return false;
        };
        let mut next_lanes = Vec::with_capacity(4);
        for lane_id in possible_lanes {
            collect_connector_targets_to_edge(
                lane_id,
                from_node,
                out_edge,
                transit_network,
                graph,
                &mut next_lanes,
            );
        }
        if next_lanes.is_empty() {
            return false;
        }
        next_lanes.sort_unstable();
        next_lanes.dedup();
        possible_lanes = next_lanes;
    }

    possible_lanes.into_iter().any(|lane_id| {
        connector_to_lane_exists(lane_id, detach_lane_id, transit_network)
            || lane_id == detach_lane_id
    })
}

fn collect_connector_targets_to_edge(
    from_lane_id: usize,
    from_node: u32,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    out: &mut Vec<usize>,
) {
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return;
    };
    for &conn_lane_id in &from_lane.next_lanes {
        let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
            continue;
        };
        if conn_lane.edge_id != usize::MAX || conn_lane.lane_type != LaneType::Foot {
            continue;
        }
        let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
            continue;
        };
        let Some(target_lane) = transit_network.lane_system.lanes.get(target_lane_id) else {
            continue;
        };
        if target_lane.edge_id == target_edge_id
            && target_lane.lane_type == LaneType::Foot
            && lane_origin_node(target_lane_id, transit_network, graph) == Some(from_node)
        {
            out.push(target_lane_id);
        }
    }
}

fn connector_to_lane_exists(
    from_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> bool {
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return false;
    };
    from_lane.next_lanes.iter().copied().any(|conn_lane_id| {
        transit_network
            .lane_system
            .lanes
            .get(conn_lane_id)
            .is_some_and(|conn_lane| {
                conn_lane.edge_id == usize::MAX
                    && conn_lane.lane_type == LaneType::Foot
                    && conn_lane.next_lanes.first().copied() == Some(target_lane_id)
            })
    })
}

pub(super) fn pedestrian_lane_connector_path(
    start_node: u32,
    end_node: u32,
    attach_lane_id: usize,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<(f32, Vec<u32>)> {
    pedestrian_lane_connector_path_from_start_lanes(
        start_node,
        end_node,
        vec![attach_lane_id],
        detach_lane_id,
        transit_network,
        graph,
    )
}

pub(super) fn pedestrian_lane_connector_path_from_edge(
    start_node: u32,
    end_node: u32,
    incoming_edge: usize,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<(f32, Vec<u32>)> {
    let start_lanes =
        pedestrian_incoming_lanes_at_node(start_node, incoming_edge, transit_network, graph);
    pedestrian_lane_connector_path_from_start_lanes(
        start_node,
        end_node,
        start_lanes,
        detach_lane_id,
        transit_network,
        graph,
    )
}

pub(super) fn pedestrian_lane_connector_path_from_node(
    start_node: u32,
    end_node: u32,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<(f32, Vec<u32>)> {
    if start_node == end_node {
        return (lane_origin_node(detach_lane_id, transit_network, graph) == Some(start_node))
            .then_some((0.0, vec![start_node]));
    }

    let mut heap = BinaryHeap::new();
    let mut best: HashMap<(u32, usize), f32> = HashMap::new();
    let mut previous: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
    for start_lane_id in pedestrian_outgoing_lanes_at_node(start_node, transit_network, graph) {
        let Some(next_node) = lane_terminal_node(start_lane_id, transit_network, graph) else {
            continue;
        };
        let lane = &transit_network.lane_system.lanes[start_lane_id];
        let travel_cost = graph
            .edges()
            .get(lane.edge_id)
            .map(|edge| edge.base_cost)
            .unwrap_or(lane.length / WALK_CONNECTOR_COST_SPEED_MS);
        let key = (next_node, start_lane_id);
        let cost = travel_cost.max(0.001);
        if cost < *best.get(&key).unwrap_or(&f32::INFINITY) {
            best.insert(key, cost);
            previous.insert(key, (start_node, usize::MAX));
            heap.push(PedestrianSearchState {
                cost,
                node: next_node,
                incoming_lane: start_lane_id,
            });
        }
    }

    search_pedestrian_lane_connector_path(
        &mut heap,
        &mut best,
        &mut previous,
        end_node,
        detach_lane_id,
        transit_network,
        graph,
    )
}

fn pedestrian_lane_connector_path_from_start_lanes(
    start_node: u32,
    end_node: u32,
    start_lanes: Vec<usize>,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<(f32, Vec<u32>)> {
    if start_lanes.is_empty() {
        return None;
    }
    if start_node == end_node {
        return start_lanes
            .into_iter()
            .any(|lane_id| connector_to_lane_exists(lane_id, detach_lane_id, transit_network))
            .then_some((0.0, vec![start_node]));
    }

    let mut heap = BinaryHeap::new();
    let mut best: HashMap<(u32, usize), f32> = HashMap::new();
    let mut previous: HashMap<(u32, usize), (u32, usize)> = HashMap::new();
    for start_lane_id in start_lanes {
        let start_key = (start_node, start_lane_id);
        best.insert(start_key, 0.0);
        heap.push(PedestrianSearchState {
            cost: 0.0,
            node: start_node,
            incoming_lane: start_lane_id,
        });
    }

    search_pedestrian_lane_connector_path(
        &mut heap,
        &mut best,
        &mut previous,
        end_node,
        detach_lane_id,
        transit_network,
        graph,
    )
}

fn search_pedestrian_lane_connector_path(
    heap: &mut BinaryHeap<PedestrianSearchState>,
    best: &mut HashMap<(u32, usize), f32>,
    previous: &mut HashMap<(u32, usize), (u32, usize)>,
    end_node: u32,
    detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<(f32, Vec<u32>)> {
    while let Some(state) = heap.pop() {
        let state_key = (state.node, state.incoming_lane);
        if state.cost > *best.get(&state_key).unwrap_or(&f32::INFINITY) {
            continue;
        }
        if state.node == end_node
            && connector_to_lane_exists(state.incoming_lane, detach_lane_id, transit_network)
        {
            return Some((
                state.cost,
                reconstruct_pedestrian_nodes(state_key, &previous),
            ));
        }

        let Some(from_lane) = transit_network.lane_system.lanes.get(state.incoming_lane) else {
            continue;
        };
        for &conn_lane_id in &from_lane.next_lanes {
            let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
                continue;
            };
            if conn_lane.edge_id != usize::MAX || conn_lane.lane_type != LaneType::Foot {
                continue;
            }
            let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
                continue;
            };
            let Some(target_lane) = transit_network.lane_system.lanes.get(target_lane_id) else {
                continue;
            };
            if target_lane.lane_type != LaneType::Foot
                || lane_origin_node(target_lane_id, transit_network, graph) != Some(state.node)
            {
                continue;
            }
            let Some(next_node) = lane_terminal_node(target_lane_id, transit_network, graph) else {
                continue;
            };
            let travel_cost = graph
                .edges()
                .get(target_lane.edge_id)
                .map(|edge| edge.base_cost)
                .unwrap_or(target_lane.length / WALK_CONNECTOR_COST_SPEED_MS)
                + conn_lane.length / WALK_CONNECTOR_COST_SPEED_MS;
            let next_cost = state.cost + travel_cost.max(0.001);
            let next_key = (next_node, target_lane_id);
            if next_cost < *best.get(&next_key).unwrap_or(&f32::INFINITY) {
                best.insert(next_key, next_cost);
                previous.insert(next_key, state_key);
                heap.push(PedestrianSearchState {
                    cost: next_cost,
                    node: next_node,
                    incoming_lane: target_lane_id,
                });
            }
        }
    }

    None
}

fn pedestrian_start_lanes_from_node_path(
    path: &[u32],
    start_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Vec<usize> {
    let Some(&next_node) = path.get(1) else {
        return Vec::new();
    };
    let Some(first_edge) = graph.get_edge_between_nodes(start_node, next_node) else {
        return Vec::new();
    };
    pedestrian_outgoing_lanes_on_edge(start_node, first_edge, transit_network, graph)
}

fn pedestrian_outgoing_lanes_at_node(
    node_id: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Vec<usize> {
    if node_id as usize >= graph.node_adjacency_count() {
        return Vec::new();
    }
    let mut lanes = Vec::new();
    for &edge_id in graph.node_adjacency(node_id) {
        lanes.extend(pedestrian_outgoing_lanes_on_edge(
            node_id,
            edge_id,
            transit_network,
            graph,
        ));
    }
    lanes
}

fn pedestrian_outgoing_lanes_on_edge(
    node_id: u32,
    edge_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Vec<usize> {
    transit_network
        .lane_system
        .edge_lanes
        .get(&edge_id)
        .into_iter()
        .flat_map(|lane_ids| lane_ids.iter().copied())
        .filter(|&lane_id| {
            transit_network
                .lane_system
                .lanes
                .get(lane_id)
                .is_some_and(|lane| lane.lane_type == LaneType::Foot)
                && lane_origin_node(lane_id, transit_network, graph) == Some(node_id)
        })
        .collect()
}

fn pedestrian_incoming_lanes_at_node(
    node_id: u32,
    incoming_edge: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Vec<usize> {
    if incoming_edge == usize::MAX {
        return Vec::new();
    }
    transit_network
        .lane_system
        .edge_lanes
        .get(&incoming_edge)
        .into_iter()
        .flat_map(|lane_ids| lane_ids.iter().copied())
        .filter(|&lane_id| {
            transit_network
                .lane_system
                .lanes
                .get(lane_id)
                .is_some_and(|lane| lane.lane_type == LaneType::Foot)
                && lane_terminal_node(lane_id, transit_network, graph) == Some(node_id)
        })
        .collect()
}

fn reconstruct_pedestrian_nodes(
    mut key: (u32, usize),
    previous: &HashMap<(u32, usize), (u32, usize)>,
) -> Vec<u32> {
    let mut nodes = vec![key.0];
    while let Some(&prev_key) = previous.get(&key) {
        key = prev_key;
        nodes.push(key.0);
    }
    nodes.reverse();
    nodes
}

#[derive(Copy, Clone, PartialEq)]
struct PedestrianSearchState {
    cost: f32,
    node: u32,
    incoming_lane: usize,
}

impl Eq for PedestrianSearchState {}

impl Ord for PedestrianSearchState {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        other
            .cost
            .partial_cmp(&self.cost)
            .unwrap_or(CmpOrdering::Equal)
    }
}

impl PartialOrd for PedestrianSearchState {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    fn test_edge(start_node: u32, end_node: u32, start_x: f32, end_x: f32) -> Edge {
        Edge {
            start_node,
            end_node,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
            speed_limit: 14.0,
            base_cost: 1.0,
            physical_length: (end_x - start_x).abs(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![
                Vector3::new(start_x, 0.0, 0.0),
                Vector3::new(end_x, 0.0, 0.0),
            ],
            physical_geometry: vec![
                Vector3::new(start_x, 0.0, 0.0),
                Vector3::new(end_x, 0.0, 0.0),
            ],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
            frontage_class: Default::default(),
        }
    }

    #[test]
    fn pedestrian_path_validation_rejects_missing_attach_edge_backtrack_connector() {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(200.0, 0.0, 0.0), NodeType::Junction);
        let edge0 = graph.add_edge(test_edge(n0, n1, 0.0, 100.0));
        let edge1 = graph.add_edge(test_edge(n1, n2, 100.0, 200.0));
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);

        let attach_lane = network.lane_system.edge_lanes[&edge0]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &network.lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Foot
                    && lane_terminal_node(lane_id, &network, &graph) == Some(n0)
            })
            .expect("foot lane ending at n0");
        let detach_lane = network.lane_system.edge_lanes[&edge1]
            .iter()
            .copied()
            .find(|&lane_id| {
                let lane = &network.lane_system.lanes[lane_id];
                lane.lane_type == LaneType::Foot
                    && lane_origin_node(lane_id, &network, &graph) == Some(n2)
            })
            .expect("foot lane starting at n2");

        network.lane_system.lanes[attach_lane].next_lanes.clear();

        assert!(
            !pedestrian_path_has_lane_connectors(
                &[n0, n1, n2],
                attach_lane,
                detach_lane,
                &network,
                &graph,
            ),
            "planner must reject a node path that movement cannot realize with sidewalk connectors"
        );
    }
}

// ========================================================================
// CHOOSING THE BEST
// ========================================================================

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
