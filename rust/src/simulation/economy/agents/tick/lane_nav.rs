//! Lane graph navigation helpers used by agent movement and planning.

use crate::simulation::economy::agents::ACCESS_PLAN_VALID;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

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

fn connector_targets_edge(
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

fn connector_targets_lane(
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

/// Returns the connector lane that should be used after the current road lane.
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
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX {
        return None;
    }
    let terminal_node = lane_terminal_node(lane_id, transit_network, graph)?;

    if (access_flags & ACCESS_PLAN_VALID) != 0
        && terminal_node == planned_detach_node
        && planned_detach_lane_id != usize::MAX
    {
        if lane_origin_node(planned_detach_lane_id, transit_network, graph)
            == Some(planned_detach_node)
        {
            return connection_lane_to_lane_unclaimed(
                lane_id,
                planned_detach_lane_id,
                transit_network,
            );
        }
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
    connection_lane_to_edge(lane_id, target_edge, transit_network)
}
