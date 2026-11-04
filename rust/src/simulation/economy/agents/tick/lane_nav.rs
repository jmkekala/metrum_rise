// SPDX-License-Identifier: GPL-2.0-only

//! Lane graph navigation helpers used by agent movement and planning.

use crate::simulation::economy::agents::ACCESS_PLAN_VALID;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;

/// Returns the road node where a road lane begins for its travel direction.
pub(super) fn lane_origin_node(
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    Some(if lane.is_fwd {
        edge.start_node
    } else {
        edge.end_node
    })
}

/// Returns the origin node of a lane-attach hold's lane when the hold is still valid: the lane
/// exists on the car's live edge and `lane_d` lies on it. O(1).
pub(super) fn held_attach_origin(
    lane_id: usize,
    lane_d: f32,
    edge: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    let origin = lane_origin_node(lane_id, transit_network, graph)?;
    (lane.edge_id == edge && !graph.edge(edge).deleted && (0.0..=lane.length).contains(&lane_d))
        .then_some(origin)
}

/// Returns the road node where a road lane ends for its travel direction.
pub(super) fn lane_terminal_node(
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    Some(if lane.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    })
}

pub(super) fn connector_targets_edge(
    conn_lane_id: usize,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
) -> bool {
    let Some(conn_lane) = transit_network.lane_system.lanes.get(conn_lane_id) else {
        return false;
    };
    if conn_lane.edge_id != usize::MAX {
        return false;
    }
    let Some(&target_lane_id) = conn_lane.next_lanes.first() else {
        return false;
    };
    transit_network
        .lane_system
        .lanes
        .get(target_lane_id)
        .is_some_and(|target_lane| target_lane.edge_id == target_edge_id)
}

pub(super) fn connector_targets_lane(
    conn_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> bool {
    transit_network
        .lane_system
        .lanes
        .get(conn_lane_id)
        .is_some_and(|conn_lane| {
            conn_lane.edge_id == usize::MAX
                && conn_lane.next_lanes.first().copied() == Some(target_lane_id)
        })
}

/// Fills `out` with connector lanes that lead from `from_lane_id` to `target_edge_id`.
pub(super) fn collect_connector_lanes_to_edge(
    from_lane_id: usize,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
    out: &mut Vec<usize>,
) -> bool {
    out.clear();
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return false;
    };

    for &conn_lane_id in &from_lane.next_lanes {
        if connector_targets_edge(conn_lane_id, target_edge_id, transit_network) {
            out.push(conn_lane_id);
        }
    }
    if out.len() > 1 {
        let shortest = out
            .iter()
            .map(|&lane_id| transit_network.lane_system.lanes[lane_id].length)
            .min_by(f32::total_cmp)
            .unwrap_or(f32::INFINITY);
        out.retain(|&lane_id| {
            (transit_network.lane_system.lanes[lane_id].length - shortest).abs() <= 0.001
        });
    }
    !out.is_empty()
}

/// Fills `out` with connector lanes that lead from `from_lane_id` to `target_lane_id`.
pub(super) fn collect_connector_lanes_to_lane(
    from_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
    out: &mut Vec<usize>,
) -> bool {
    out.clear();
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return false;
    };

    for &conn_lane_id in &from_lane.next_lanes {
        if connector_targets_lane(conn_lane_id, target_lane_id, transit_network) {
            out.push(conn_lane_id);
        }
    }
    !out.is_empty()
}

/// Returns the vehicle lane of `target_edge_id` that `from_lane_id` runs straight into at a
/// pass-through node with no connectors.
pub(super) fn direct_vehicle_lane_to_edge(
    from_lane_id: usize,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let from_lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    if from_lane.lane_type != LaneType::Vehicle {
        return None;
    }

    from_lane.next_lanes.iter().copied().find(|&next_lane_id| {
        transit_network
            .lane_system
            .lanes
            .get(next_lane_id)
            .is_some_and(|lane| {
                lane.edge_id == target_edge_id && lane.lane_type == LaneType::Vehicle
            })
    })
}

/// Returns whether `from_lane_id` runs straight into the road lane `target_lane_id`.
pub(super) fn direct_vehicle_lane_to_lane(
    from_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> bool {
    let Some(from_lane) = transit_network.lane_system.lanes.get(from_lane_id) else {
        return false;
    };
    if from_lane.lane_type != LaneType::Vehicle {
        return false;
    }
    from_lane.next_lanes.contains(&target_lane_id)
        && transit_network
            .lane_system
            .lanes
            .get(target_lane_id)
            .is_some_and(|lane| lane.lane_type == LaneType::Vehicle && lane.edge_id != usize::MAX)
}

/// Seed of the stable choice among equally short connectors at a junction.
#[inline(always)]
pub(super) fn connector_choice_seed(
    agent_idx: usize,
    from_lane_id: usize,
    target_id: usize,
    node_id: u32,
    path_idx: usize,
) -> u64 {
    (agent_idx as u64)
        ^ ((from_lane_id as u64) << 11)
        ^ ((target_id as u64) << 23)
        ^ ((node_id as u64) << 37)
        ^ ((path_idx as u64) << 3)
}

fn connection_lane_to_edge(
    from_lane_id: usize,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let from_lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    from_lane
        .next_lanes
        .iter()
        .copied()
        .find(|&conn_lane_id| connector_targets_edge(conn_lane_id, target_edge_id, transit_network))
}

fn connection_lane_to_lane_unclaimed(
    from_lane_id: usize,
    target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let from_lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    from_lane
        .next_lanes
        .iter()
        .copied()
        .find(|&conn_lane_id| connector_targets_lane(conn_lane_id, target_lane_id, transit_network))
}

/// Where a car at the end of a road lane goes next under its route: the road edge of its next
/// path node, or its planned detach lane when that lane starts at the lane's terminal node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum NextTarget {
    /// The road edge from the lane's terminal node to the route's next node.
    Edge(usize),
    /// The planned detach lane, entered at its start (zero-hop access).
    Lane(usize),
}

/// Returns where the car on road lane `lane_id` goes after the lane's end, read from its route
/// as `planned_next_connector` reads it. O(node degree), no allocation.
#[allow(clippy::too_many_arguments)]
pub(super) fn planned_next_target(
    lane_id: usize,
    current_node: u32,
    path: &[u32],
    path_idx: usize,
    access_flags: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<NextTarget> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX {
        return None;
    }
    let terminal_node = lane_terminal_node(lane_id, transit_network, graph)?;
    if (access_flags & ACCESS_PLAN_VALID) != 0
        && terminal_node == planned_detach_node
        && planned_detach_lane_id != usize::MAX
        && lane_origin_node(planned_detach_lane_id, transit_network, graph)
            == Some(planned_detach_node)
    {
        return Some(NextTarget::Lane(planned_detach_lane_id));
    }

    let next_idx = if path.get(path_idx).copied() == Some(terminal_node) {
        path_idx + 1
    } else {
        path_idx
    };
    let next_node = *path.get(next_idx)?;
    let target_edge = graph.get_edge_between_nodes(terminal_node, next_node)?;
    let current_edge = lane.edge_id;
    if target_edge == current_edge && current_node != terminal_node {
        return None;
    }
    Some(NextTarget::Edge(target_edge))
}

/// Returns the connector lane that should be used after the current road lane.
#[allow(clippy::too_many_arguments)]
pub(super) fn planned_next_connector(
    lane_id: usize,
    current_node: u32,
    path: &[u32],
    path_idx: usize,
    access_flags: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<usize> {
    match planned_next_target(
        lane_id,
        current_node,
        path,
        path_idx,
        access_flags,
        planned_detach_node,
        planned_detach_lane_id,
        transit_network,
        graph,
    )? {
        // A lane whose lane use has no connector to the detach lane itself enters the detach
        // road on another lane and changes to it on the frontage, as the movement step does.
        NextTarget::Lane(detach_lane_id) => {
            connection_lane_to_lane_unclaimed(lane_id, detach_lane_id, transit_network).or_else(
                || {
                    let edge = transit_network.lane_system.lanes[detach_lane_id].edge_id;
                    connection_lane_to_edge(lane_id, edge, transit_network)
                },
            )
        }
        NextTarget::Edge(target_edge) => {
            connection_lane_to_edge(lane_id, target_edge, transit_network)
        }
    }
}

// Metres of approach a car needs per lane it may have to cross to reach its turn lane: the
// longest lane-change S-curve (`LANE_CHANGE_MAX_LENGTH_M`).
const TURN_PREPARATION_PER_LANE_M: f32 = 70.0;
// Metres of approach beyond the lane changes: the 12 m road-end buffer plus room to start.
const TURN_PREPARATION_SLACK_M: f32 = 30.0;

/// Length of the turn-preparation window at the end of an approach with `count` lanes in the
/// car's direction: `70 m x (count - 1) + 30 m`, fixed for every road speed (`ROAD-94`, D2).
#[inline(always)]
pub(super) fn turn_preparation_window_m(count: u8) -> f32 {
    f32::from(count.saturating_sub(1)) * TURN_PREPARATION_PER_LANE_M + TURN_PREPARATION_SLACK_M
}

/// Whether vehicle lane `lane_id` continues onto road `edge`: straight through a pass-through
/// node, or through one of its connectors. O(successors), no allocation.
pub(super) fn vehicle_lane_reaches_edge(
    lane_id: usize,
    edge: usize,
    network: &TransitNetwork,
) -> bool {
    let lanes = &network.lane_system.lanes;
    lanes.get(lane_id).is_some_and(|lane| {
        lane.next_lanes.iter().any(|&next| {
            let next = &lanes[next];
            next.edge_id == edge
                || (next.edge_id == usize::MAX
                    && next
                        .next_lanes
                        .first()
                        .is_some_and(|&out| lanes[out].edge_id == edge))
        })
    })
}

/// Whether any vehicle lane of `lane_id`'s carriageway (same edge, same direction, `lane_id`
/// included) continues onto road `edge`: a lane that cannot make a road turn its siblings make
/// has missed a lane-use turn, not a missing road turn. O(L x C) for L lanes of C successors.
pub(super) fn sibling_lane_reaches_edge(
    lane_id: usize,
    edge: usize,
    network: &TransitNetwork,
) -> bool {
    let lanes = &network.lane_system;
    let Some(source) = lanes.lanes.get(lane_id) else {
        return false;
    };
    lanes
        .edge_lanes
        .get(&source.edge_id)
        .is_some_and(|siblings| {
            siblings.iter().any(|&id| {
                let lane = &lanes.lanes[id];
                lane.lane_type == LaneType::Vehicle
                    && lane.is_fwd == source.is_fwd
                    && vehicle_lane_reaches_edge(id, edge, network)
            })
        })
}

/// Returns the node road lane `lane_id` approaches when a car at `lane_d` on it is inside the
/// node's turn-preparation window: the lane is flagged by the lane use
/// (`LaneSystem::needs_turn_preparation`) and at most `turn_preparation_window_m` of it remain.
/// The flag is read first, so a car on any other lane pays one byte read. O(1), no allocation.
#[inline]
pub(super) fn junction_approach_node(
    lane_id: usize,
    lane_d: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    if !network.lane_system.needs_turn_preparation(lane_id) {
        return None;
    }
    let lane = network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.lane_type != LaneType::Vehicle {
        return None;
    }
    let edge = graph.get_edge(lane.edge_id)?;
    let count = if lane.is_fwd {
        edge.fwd_lanes
    } else {
        edge.bkw_lanes
    };
    if count <= 1 || lane.length - lane_d > turn_preparation_window_m(count) {
        return None;
    }
    Some(if lane.is_fwd {
        edge.end_node
    } else {
        edge.start_node
    })
}

/// Chooses the lane of `lane_id`'s carriageway nearest to it that makes the route's next movement
/// at the lane's end, where `planned_next_target` says the car goes (the next path edge, or the
/// zero-hop detach lane's edge). Returns `lane_id` itself when it already makes the movement, so
/// a car on a good lane does not weave, and `None` when the route has no next movement there or
/// no lane makes it. Ties go to the smallest `(|lane index delta|, |lane index|, lane id)`, a pure
/// function of the lane snapshot and the car's columns. Called inside the window
/// (`junction_approach_node`) only. O(D + L x C): the target is O(node degree), then L sibling
/// lanes of C successors each; no allocation, no route query.
#[allow(clippy::too_many_arguments)]
pub(super) fn junction_approach_lane(
    lane_id: usize,
    current_node: u32,
    path: &[u32],
    path_idx: usize,
    access_flags: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<usize> {
    let lanes = &network.lane_system;
    let lane = lanes.lanes.get(lane_id)?;
    let target_edge = match planned_next_target(
        lane_id,
        current_node,
        path,
        path_idx,
        access_flags,
        planned_detach_node,
        planned_detach_lane_id,
        network,
        graph,
    )? {
        NextTarget::Edge(edge) => edge,
        NextTarget::Lane(detach_lane_id) => lanes.lanes.get(detach_lane_id)?.edge_id,
    };
    lanes
        .edge_lanes
        .get(&lane.edge_id)?
        .iter()
        .copied()
        .filter(|&candidate| {
            let sibling = &lanes.lanes[candidate];
            sibling.is_fwd == lane.is_fwd
                && sibling.lane_type == LaneType::Vehicle
                && vehicle_lane_reaches_edge(candidate, target_edge, network)
        })
        .min_by_key(|&candidate| {
            let index = lanes.lanes[candidate].lane_idx;
            (
                (i16::from(index) - i16::from(lane.lane_idx)).unsigned_abs(),
                index.unsigned_abs(),
                candidate,
            )
        })
}
