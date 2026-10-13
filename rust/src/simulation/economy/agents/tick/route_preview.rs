// SPDX-License-Identifier: GPL-2.0-only

//! Lane-by-lane route an agent will follow from where it is now, for the agent inspector.
//!
//! The walk uses the movement tick's own rules: it steps the path index as `handle_lane_end`
//! does, continues straight at pass-through nodes and enters the zero-hop connector to the
//! planned detach lane. Among equally short connectors it takes the stable pick, which is the
//! connector a car takes when the junction is free. A car waiting for a busy connector can
//! take another one, and overtaking or returning to the outer lane changes lanes; the
//! inspector redraws from the car's new lane. Reads only; nothing here runs on the tick.

use super::super::data::AgentVec;
use super::super::determinism::stable_index;
use super::super::{
    ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
    TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use super::access::{LocalAccessPath, local_access_path, local_access_target_segment};
use super::lane_nav::{
    collect_connector_lanes_to_edge, collect_connector_lanes_to_lane, connector_choice_seed,
    direct_vehicle_lane_to_edge, direct_vehicle_lane_to_lane, lane_origin_node, lane_terminal_node,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

/// One stretch of lane on the route, from `from_d` to `to_d` metres along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RouteSpan {
    pub(crate) lane_id: usize,
    pub(crate) from_d: f32,
    pub(crate) to_d: f32,
}

/// Where the route stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RouteEnd {
    /// The agent is not travelling, or its lane or plan is gone.
    #[default]
    None,
    /// Leaves the road into the destination's access path, then the door (`ingress`).
    Destination,
    /// The node path ends at `node` without a planned detach: a border, or a replan there.
    PathEnd { node: u32 },
    /// No lane leads on towards `node`; the movement tick drops the path and replans here.
    NoConnection { node: u32 },
}

/// The route from the agent's position: driveway out, lanes, driveway in.
#[derive(Debug, Default)]
pub(crate) struct PlannedRoute {
    /// Remaining driveway points to the road while the agent leaves a building.
    pub(crate) egress: Vec<Vector2>,
    /// Lanes and connectors in driving order; the first starts at the agent.
    pub(crate) spans: Vec<RouteSpan>,
    /// Driveway points from the road to the destination door, the last one the door.
    pub(crate) ingress: Vec<Vector2>,
    pub(crate) end: RouteEnd,
}

/// Plans agent `i`'s route. Cost is O(path nodes), plus the connectors at each junction.
pub(crate) fn planned_route(
    agents: &AgentVec,
    i: usize,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> PlannedRoute {
    let mut route = PlannedRoute::default();
    let mode = agents.transit_mode[i];
    let plan_valid = agents.access_flags[i] & ACCESS_PLAN_VALID != 0;
    let here = Vector2::new(agents.pos_x[i], agents.pos_y[i]);
    let (lane_id, lane_d) = match agents.transit[i] {
        TRANSIT_NETWORK | TRANSIT_INTERSECTION => {
            (agents.current_lane_id[i], agents.lane_distance[i])
        }
        TRANSIT_ACCESS_EGRESS => {
            let attach = lane_index(agents.planned_attach_lane_id[i]);
            let attach_d = agents.planned_attach_lane_d[i];
            if let Some(entrance) = allocator.entrances.get(agents.current_building[i])
                && plan_valid
                && let Some(path) =
                    local_access_path(mode, entrance, attach, attach_d, network, graph, false)
            {
                route.egress = remaining_access_points(here, &path);
            }
            if !plan_valid || attach == usize::MAX {
                return route;
            }
            (attach, attach_d)
        }
        TRANSIT_ACCESS_INGRESS => {
            route.ingress = ingress_points(agents, i, allocator, network, graph)
                .map(|path| remaining_access_points(here, &path))
                .unwrap_or_default();
            route.end = RouteEnd::Destination;
            return route;
        }
        _ => return route,
    };
    walk_lanes(agents, i, lane_id, lane_d, network, graph, &mut route);
    if route.end == RouteEnd::Destination
        && let Some(path) = ingress_points(agents, i, allocator, network, graph)
    {
        route.ingress = path.points[..path.count].to_vec();
    }
    route
}

// Follows lanes from `lane_id` at `lane_d` the way `handle_lane_end` would.
fn walk_lanes(
    agents: &AgentVec,
    i: usize,
    mut lane_id: usize,
    mut lane_d: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
    route: &mut PlannedRoute,
) {
    let mode = agents.transit_mode[i];
    let plan_valid = agents.access_flags[i] & ACCESS_PLAN_VALID != 0;
    let path = &agents.current_path[i];
    let mut path_idx = agents.current_path_index[i];
    let detach_lane = lane_index(agents.planned_detach_lane_id[i]);
    let detach_d = agents.planned_detach_lane_d[i];
    let detach_node = agents.planned_detach_node[i];
    let detaches = plan_valid && detach_lane != usize::MAX;
    let mut connectors = Vec::new();
    // Each path node adds at most one road lane and one connector; the detach adds two more.
    for _ in 0..2 * path.len() + 4 {
        let Some(lane) = network.lane_system.lanes.get(lane_id) else {
            return;
        };
        let mut span = |to_d: f32| {
            route.spans.push(RouteSpan {
                lane_id,
                from_d: lane_d.min(to_d),
                to_d,
            })
        };
        if lane.edge_id == usize::MAX {
            span(lane.length);
            match lane.next_lanes.first() {
                Some(&next) => (lane_id, lane_d) = (next, 0.0),
                None => {
                    route.end = RouteEnd::NoConnection {
                        node: lane.node_id as u32,
                    };
                    return;
                }
            }
            continue;
        }
        // The detach lane itself, or a sibling the car changes from before the detach point.
        if detaches
            && (lane_id == detach_lane || lane_d < detach_d)
            && network
                .lane_system
                .lanes
                .get(detach_lane)
                .is_some_and(|detach| {
                    detach.edge_id == lane.edge_id && detach.is_fwd == lane.is_fwd
                })
        {
            span(detach_d.min(lane.length));
            route.end = RouteEnd::Destination;
            return;
        }
        span(lane.length);

        let Some(node) = lane_terminal_node(lane_id, network, graph) else {
            return;
        };
        let before = path_idx.min(path.len());
        let hold_frontage = plan_valid && before == 1 && path.first() == Some(&node);
        let hold_tail = plan_valid && before >= path.len();
        path_idx = if hold_frontage || hold_tail {
            before
        } else {
            before + 1
        };

        if let Some(&next_node) = path.get(path_idx) {
            let Some(edge) = graph.get_edge_between_nodes(node, next_node) else {
                route.end = RouteEnd::NoConnection { node: next_node };
                return;
            };
            if let Some(direct) = direct_vehicle_lane_to_edge(lane_id, edge, network) {
                (lane_id, lane_d) = (direct, 0.0);
                continue;
            }
            if !collect_connector_lanes_to_edge(lane_id, edge, network, &mut connectors) {
                route.end = RouteEnd::NoConnection { node: next_node };
                return;
            }
            let seed = connector_choice_seed(i, lane_id, edge, node, path_idx);
            (lane_id, lane_d) = (connectors[stable_index(seed, connectors.len())], 0.0);
            continue;
        }

        // Zero hop: the path ends at the detach node, so cross it into the detach lane.
        if !(detaches
            && node == detach_node
            && lane_origin_node(detach_lane, network, graph) == Some(node))
        {
            route.end = RouteEnd::PathEnd { node };
            return;
        }
        if mode == MODE_CAR && direct_vehicle_lane_to_lane(lane_id, detach_lane, network) {
            (lane_id, lane_d) = (detach_lane, 0.0);
            continue;
        }
        if !collect_connector_lanes_to_lane(lane_id, detach_lane, network, &mut connectors) {
            route.end = RouteEnd::NoConnection { node };
            return;
        }
        let seed = connector_choice_seed(i, lane_id, detach_lane, node, path_idx);
        (lane_id, lane_d) = (connectors[stable_index(seed, connectors.len())], 0.0);
    }
}

fn ingress_points(
    agents: &AgentVec,
    i: usize,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<LocalAccessPath> {
    let entrance = allocator.entrances.get(agents.target_building[i])?;
    local_access_path(
        agents.transit_mode[i],
        entrance,
        lane_index(agents.planned_detach_lane_id[i]),
        agents.planned_detach_lane_d[i],
        network,
        graph,
        true,
    )
}

// The agent's position, then the access points still ahead of it.
fn remaining_access_points(here: Vector2, path: &LocalAccessPath) -> Vec<Vector2> {
    let next = local_access_target_segment(here, path).unwrap_or(path.count.saturating_sub(1));
    let mut points = Vec::with_capacity(path.count + 1);
    points.push(here);
    points.extend_from_slice(&path.points[next.min(path.count)..path.count]);
    points
}

fn lane_index(lane_id: u32) -> usize {
    if lane_id == u32::MAX {
        usize::MAX
    } else {
        lane_id as usize
    }
}
