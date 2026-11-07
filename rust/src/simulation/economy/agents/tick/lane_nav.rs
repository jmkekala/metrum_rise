// SPDX-License-Identifier: GPL-2.0-only

//! Lane graph navigation helpers used by agent movement and planning.

use crate::simulation::economy::agents::ACCESS_PLAN_VALID;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{
    APPROACH_SOLID_LENGTH_M, LaneType, approach_group, lane_rank, pocket_fork_sources,
};

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

/// The road lane `lane_id` is, or the one its junction connector leads onto. O(1).
pub(super) fn road_lane_ahead(lane_id: usize, transit_network: &TransitNetwork) -> Option<usize> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX {
        lane.next_lanes.first().copied()
    } else {
        Some(lane_id)
    }
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

/// The lanes of a road a car may enter for its route: ranks `first..first + len` (inner first)
/// among `count` lanes in its direction (`preferred_entry_lanes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::simulation::economy::agents) struct EntryLanes {
    first: usize,
    len: usize,
    count: usize,
}

impl EntryLanes {
    /// Whether the lane with index `lane_idx` is one of the preferred lanes.
    #[inline]
    pub(in crate::simulation::economy::agents) fn contains(&self, lane_idx: i8) -> bool {
        let rank = lane_rank(lane_idx, self.count);
        rank >= self.first && rank < self.first + self.len
    }
}

/// The entry preference for a zero-hop turn onto the planned detach road: the detach lane
/// itself. O(L).
pub(super) fn detach_lane_entry(
    detach_lane_id: usize,
    network: &TransitNetwork,
) -> Option<EntryLanes> {
    let detach = network.lane_system.lanes.get(detach_lane_id)?;
    if detach.edge_id == usize::MAX {
        return None;
    }
    let count = direction_lane_count(detach.edge_id, detach.is_fwd, network);
    Some(EntryLanes {
        first: lane_rank(detach.lane_idx, count),
        len: 1,
        count,
    })
}

/// Vehicle lanes of `edge` running in `is_fwd`'s direction. O(L), no allocation.
fn direction_lane_count(edge: usize, is_fwd: bool, network: &TransitNetwork) -> usize {
    let lanes = &network.lane_system;
    lanes.edge_lanes.get(&edge).map_or(0, |ids| {
        ids.iter()
            .filter(|&&id| {
                let lane = &lanes.lanes[id];
                lane.is_fwd == is_fwd && lane.lane_type == LaneType::Vehicle
            })
            .count()
    })
}

/// The lanes of road `edge`, entered at `node`, from which the car can follow its route without a
/// lane change until its next lane-selective movement; `path[next_idx]` is the route's node after
/// the road's far end. The walk follows the path and narrows the usable ranks at every node by the
/// default lane use's group for the movement made there (`approach_group`); through a group that
/// maps 1:1 onto the next road (straight on between equal counts, a pass-through, a one-lane turn
/// onto a one-lane road) it carries the ranks onto that road and walks on, and it stops at the
/// first group that does not (a turn onto a wider road, a fan), at the planned detach lane on the
/// road (frontage) or its zero-hop turn at the path's end, at a whitelisted node or a U-turn (no
/// rule to read), or where the next group shares no lane with the ranks kept so far (the car must
/// change lanes on a later road, so this road's entry keeps what holds up to there). `None` when
/// every lane is usable, including a road with one lane in that direction (a 1+1 city never pays
/// more than this edge read). Called at entry events only (a connector choice at a lane end, a
/// node entry) and before a discretionary change on a flagged lane: O(hops x degree) over the
/// remaining path, no allocation, no route query (`ROAD-94` D7, `TRAFFIC-23`).
#[allow(clippy::too_many_arguments)]
pub(in crate::simulation::economy::agents) fn preferred_entry_lanes(
    edge: usize,
    node: u32,
    path: &[u32],
    next_idx: usize,
    access_flags: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<EntryLanes> {
    let first_edge = graph.get_edge(edge)?;
    let is_fwd = first_edge.start_node == node;
    let count = usize::from(if is_fwd {
        first_edge.fwd_lanes
    } else {
        first_edge.bkw_lanes
    });
    if count <= 1 {
        return None;
    }
    let mut cur_edge = edge;
    let mut cur_node = if is_fwd {
        first_edge.end_node
    } else {
        first_edge.start_node
    };
    // `path[idx]` is the node after `cur_node`, the node `cur_edge` leads to. The usable lanes are
    // `cur_edge`'s ranks `lo..hi`; its rank `r` is the entered road's rank `offset + r`.
    let mut idx = next_idx;
    let (mut lo, mut hi, mut offset) = (0, count, 0);
    // The usable ranks as entered-road lanes, narrowed by a last group when it shares a lane.
    let usable = |lo: usize, hi: usize, offset: usize, last: Option<(usize, usize)>| {
        let (lo, hi) = last
            .map(|(first, len)| (lo.max(first), hi.min(first + len)))
            .filter(|&(a, b)| a < b)
            .unwrap_or((lo, hi));
        (hi - lo < count).then_some(EntryLanes {
            first: offset + lo,
            len: hi - lo,
            count,
        })
    };
    loop {
        let Some(&next) = path.get(idx) else {
            let detach = (access_flags & ACCESS_PLAN_VALID != 0)
                .then(|| network.lane_system.lanes.get(planned_detach_lane_id))
                .flatten();
            let last = detach.and_then(|detach| {
                if detach.edge_id == cur_edge {
                    // The car leaves the network from its frontage lane on this road.
                    let detach_count = direction_lane_count(cur_edge, detach.is_fwd, network);
                    Some((lane_rank(detach.lane_idx, detach_count), 1))
                } else if cur_node == planned_detach_node
                    && lane_origin_node(planned_detach_lane_id, network, graph) == Some(cur_node)
                {
                    approach_group(graph, cur_edge, cur_node, detach.edge_id)
                        .map(|(first, len, _)| (first, len))
                } else {
                    None
                }
            });
            return usable(lo, hi, offset, last);
        };
        let Some((out, (first, len, identity))) = graph
            .get_edge_between_nodes(cur_node, next)
            .and_then(|out| Some((out, approach_group(graph, cur_edge, cur_node, out)?)))
        else {
            return usable(lo, hi, offset, None);
        };
        let (group_lo, group_hi) = (lo.max(first), hi.min(first + len));
        if !identity
            && let Some(fork) = pocket_turn_sources(graph, cur_edge, cur_node, out, next, path, idx)
        {
            // A lane transition's fan into a pocket edge (`ROAD-95`): the pocket edge is too
            // short to change lanes on, so the lanes that fork into the turn made at its
            // junction are the ones to take.
            return usable(lo, hi, offset, Some(fork));
        }
        if !identity || group_lo >= group_hi {
            return usable(lo, hi, offset, Some((first, len)));
        }
        (lo, hi, offset) = (group_lo - first, group_hi - first, offset + first);
        cur_edge = out;
        cur_node = next;
        idx += 1;
    }
}

// Through lane transition `node` into pocket edge `pocket_edge`, whose far end is the junction
// `junction` (`path[idx]`): the ranks of `edge` whose straight connectors reach the pocket
// edge's lanes making the route's movement there, onto the road to `path[idx + 1]`. `None`
// anywhere else, or when the route ends on the pocket edge. O(degree + lanes).
fn pocket_turn_sources(
    graph: &RegionGraph,
    edge: usize,
    node: u32,
    pocket_edge: usize,
    junction: u32,
    path: &[u32],
    idx: usize,
) -> Option<(usize, usize)> {
    graph.edge(pocket_edge).pocket?;
    let after = graph.get_edge_between_nodes(junction, *path.get(idx + 1)?)?;
    let (first, len, _) = approach_group(graph, pocket_edge, junction, after)?;
    pocket_fork_sources(graph, edge, node, pocket_edge, (first, len))
}

/// The lanes of the road under road lane `lane_id` from which the car's route makes its next
/// lane-selective movement: `preferred_entry_lanes` for that road, the route read as
/// `planned_next_target` reads it. A discretionary overtake or return on a flagged lane must stay
/// inside them (`TRAFFIC-23`, S8). O(hops x degree), no allocation, no route query; called only
/// when such a change would otherwise start.
#[allow(clippy::too_many_arguments)]
pub(super) fn route_lane_group(
    lane_id: usize,
    path: &[u32],
    path_idx: usize,
    access_flags: u8,
    planned_detach_node: u32,
    planned_detach_lane_id: usize,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<EntryLanes> {
    let edge = network.lane_system.lanes.get(lane_id)?.edge_id;
    let origin = lane_origin_node(lane_id, network, graph)?;
    let terminal = lane_terminal_node(lane_id, network, graph)?;
    let next_idx = if path.get(path_idx) == Some(&terminal) {
        path_idx + 1
    } else {
        path_idx
    };
    preferred_entry_lanes(
        edge,
        origin,
        path,
        next_idx,
        access_flags,
        planned_detach_node,
        planned_detach_lane_id,
        network,
        graph,
    )
}

/// Whether a car entering road lane `lane_id` at its start can still reach any sibling lane
/// before the node: the turn-preparation budget (`turn_lane_change_length`) fits the farthest
/// step count on the lane's length at the slowest speed. A lane that cannot host it leaves a
/// car's lane fixed for its whole length, so a replan from its end must start on a road the
/// lane reaches (`plan_network_replan`'s `first_lane`). O(L).
pub(in crate::simulation::economy::agents) fn lane_hosts_turn_preparation(
    lane_id: usize,
    network: &TransitNetwork,
) -> bool {
    use super::traffic::{OVERTAKE_EDGE_BUFFER_M, lane_change_end, turn_lane_change_length};
    let Some(lane) = network.lane_system.lanes.get(lane_id) else {
        return false;
    };
    if lane.edge_id == usize::MAX {
        return false;
    }
    let count = direction_lane_count(lane.edge_id, lane.is_fwd, network) as u8;
    // One lane: nothing to prepare, every turn of the road is the lane's.
    if count <= 1 {
        return true;
    }
    let available =
        lane.length - (lane.length - lane_change_end(lane) - OVERTAKE_EDGE_BUFFER_M).max(0.0);
    turn_lane_change_length(0.0, available, u16::from(count.saturating_sub(1)))
        .is_some_and(|length| available > length + OVERTAKE_EDGE_BUFFER_M)
}

/// Fills `out` with connector lanes that lead from `from_lane_id` to `target_edge_id`: those onto
/// a lane in `prefer` when any connector reaches one (`TRAFFIC-23`), then the shortest.
pub(super) fn collect_connector_lanes_to_edge(
    from_lane_id: usize,
    target_edge_id: usize,
    transit_network: &TransitNetwork,
    out: &mut Vec<usize>,
    prefer: Option<EntryLanes>,
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
    if out.len() > 1
        && let Some(prefer) = prefer
    {
        let lanes = &transit_network.lane_system.lanes;
        let preferred = |conn: usize| {
            lanes[conn]
                .next_lanes
                .first()
                .is_some_and(|&target| prefer.contains(lanes[target].lane_idx))
        };
        if out.iter().any(|&conn| preferred(conn)) {
            out.retain(|&conn| preferred(conn));
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
/// (`LaneSystem::needs_turn_preparation`), at most `turn_preparation_window_m` of it remain and
/// the car has not reached the solid approach zone (`APPROACH_SOLID_LENGTH_M`), where no change
/// starts and the selector would be wasted work for the queue at the stop line. The flag is read
/// first, so a car on any other lane pays one read of its lane. O(1), no allocation.
#[inline]
pub(super) fn junction_approach_node(
    lane_id: usize,
    lane_d: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<u32> {
    let lane = network.lane_system.lanes.get(lane_id)?;
    if lane.solid_zone_m <= 0.0 || lane.edge_id == usize::MAX || lane.lane_type != LaneType::Vehicle
    {
        return None;
    }
    let edge = graph.get_edge(lane.edge_id)?;
    let count = if lane.is_fwd {
        edge.fwd_lanes
    } else {
        edge.bkw_lanes
    };
    let remaining = lane.length - lane_d;
    if count <= 1
        || remaining > turn_preparation_window_m(count)
        || remaining <= APPROACH_SOLID_LENGTH_M
    {
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
    // Before a lane transition every lane continues into the pocket edge, so the movement to
    // prepare is the one at the pocket's junction: the lanes that fork into it (`ROAD-95`).
    // Read only when the next road has a pocket: one edge read on every other approach.
    let fork = graph
        .get_edge(target_edge)
        .is_some_and(|target| target.pocket.is_some())
        .then(|| {
            let terminal = lane_terminal_node(lane_id, network, graph)?;
            let next_idx = path_idx + usize::from(path.get(path_idx) == Some(&terminal));
            let junction = *path.get(next_idx)?;
            let (first, len) = pocket_turn_sources(
                graph,
                lane.edge_id,
                terminal,
                target_edge,
                junction,
                path,
                next_idx,
            )?;
            let edge = graph.edge(lane.edge_id);
            let count = if lane.is_fwd {
                edge.fwd_lanes
            } else {
                edge.bkw_lanes
            };
            Some((first, len, usize::from(count)))
        })
        .flatten();
    lanes
        .edge_lanes
        .get(&lane.edge_id)?
        .iter()
        .copied()
        .filter(|&candidate| {
            let sibling = &lanes.lanes[candidate];
            sibling.is_fwd == lane.is_fwd
                && sibling.lane_type == LaneType::Vehicle
                && fork.map_or_else(
                    || vehicle_lane_reaches_edge(candidate, target_edge, network),
                    |(first, len, count)| {
                        (first..first + len).contains(&lane_rank(sibling.lane_idx, count))
                    },
                )
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
