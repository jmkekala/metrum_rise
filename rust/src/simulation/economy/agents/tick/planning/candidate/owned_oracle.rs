// SPDX-License-Identifier: GPL-2.0-only

//! Allocation-owning reference planner retained only for exact route regression oracles.

#![allow(dead_code)]

use super::*;
use std::collections::HashMap;

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
pub(super) fn combine_trip_ends(
    mode: u8,
    origin_rank: u8,
    destination_rank: u8,
    origin: &TripEnd,
    destination: &TripEnd,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Option<PlannedTripCandidate> {
    if shares_direct_frontage(origin, destination) {
        return direct_frontage_candidate(
            mode,
            origin_rank,
            destination_rank,
            origin,
            destination,
            transit_network,
            graph,
        );
    }
    let origin_frontage_time_s = origin.frontage_time_s?;
    let destination_frontage_time_s = destination.frontage_time_s?;

    let mut network_path = None;
    let network_path_time_s = if origin.node == destination.node {
        if mode == MODE_CAR
            || connector_to_lane_exists(origin.lane_id, destination.lane_id, transit_network)
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
            .get(origin.lane_id)?
            .edge_id;
        let (mut travel_seconds, _, mut path) = transit_network.cch_graph.find_path(
            origin.node,
            destination.node,
            planned_attach_edge,
            graph,
            transit_flags_for_mode(mode),
        )?;
        if mode != MODE_CAR
            && !pedestrian_path_has_lane_connectors(
                &path,
                origin.lane_id,
                destination.lane_id,
                transit_network,
                graph,
            )
        {
            let fallback = pedestrian_lane_connector_path(
                origin.node,
                destination.node,
                origin.lane_id,
                destination.lane_id,
                transit_network,
                graph,
            )?;
            travel_seconds = fallback.0;
            path = fallback.1;
        }
        network_path = Some(path);
        travel_seconds
    };

    let mut candidate = finish_candidate(
        mode,
        origin_rank,
        destination_rank,
        origin,
        destination,
        origin.local_time_s
            + origin_frontage_time_s
            + network_path_time_s
            + destination_frontage_time_s
            + destination.local_time_s,
    )?;
    candidate.network_path = network_path;
    Some(candidate)
}

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

/// Evaluates the pre-scratch path algorithm for exact regression comparison.
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

        visit_pedestrian_successors(&state, transit_network, graph, |next| {
            let next_key = (next.node, next.incoming_lane);
            if next.cost < *best.get(&next_key).unwrap_or(&f32::INFINITY) {
                best.insert(next_key, next.cost);
                previous.insert(next_key, state_key);
                heap.push(next);
            }
        });
    }

    None
}

fn visit_pedestrian_successors(
    state: &PedestrianSearchState,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    mut visit: impl FnMut(PedestrianSearchState),
) {
    let Some(from_lane) = transit_network.lane_system.lanes.get(state.incoming_lane) else {
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
        visit(PedestrianSearchState {
            cost: state.cost + travel_cost.max(0.001),
            node: next_node,
            incoming_lane: target_lane_id,
        });
    }
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
