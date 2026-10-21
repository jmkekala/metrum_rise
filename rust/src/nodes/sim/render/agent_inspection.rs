// SPDX-License-Identifier: GPL-2.0-only

//! Agent inspector (`DEBUG-05`): picking an agent, its details and its route as lines.
//!
//! Called by the inspector when the player clicks and while one agent is inspected; nothing
//! here runs on the simulation tick.

use crate::nodes::sim::core::SimCore;
use crate::nodes::sim::render::lane_pose::sample_lane_pose;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::tick::{PlannedRoute, RouteEnd, planned_route};
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, ACTIVITY_WORK, MODE_BIKE, MODE_BUS_PASSENGER, MODE_CAR,
    MODE_TRAIN_PASSENGER, MODE_WALK, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS,
    TRANSIT_IMMIGRATING, TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{Lane, LaneSystem};
use crate::simulation::zoning::ZoneType;
use godot::prelude::*;

/// How far a lane can lie from its road's centre line, which the edge index stores.
const LANE_REACH_M: f32 = 20.0;
/// Lines float this far above the lane or ground so the road does not hide them.
const LINE_LIFT_M: f32 = 0.6;
const MARKER_ARM_M: f32 = 3.0;
const MARKER_POST_M: f32 = 8.0;

/// Current lane.
pub(crate) const CURRENT_LANE_COLOR: Color = Color::from_rgb(0.1, 0.45, 1.0);
/// Junction connectors.
pub(crate) const CONNECTOR_COLOR: Color = Color::from_rgb(1.0, 0.75, 0.1);
/// The first road lane after the current one.
pub(crate) const NEXT_LANE_COLOR: Color = Color::from_rgb(0.15, 1.0, 0.35);
/// Road lanes after the next one.
pub(crate) const LATER_LANE_COLOR: Color = Color::from_rgb(0.2, 0.8, 1.0);
/// Driveways between a door and the road.
pub(crate) const DRIVEWAY_COLOR: Color = Color::from_rgb(1.0, 0.95, 0.4);
/// The destination door.
pub(crate) const DESTINATION_COLOR: Color = Color::from_rgb(1.0, 0.25, 1.0);
/// Where the route stops without reaching a destination.
pub(crate) const DEAD_END_COLOR: Color = Color::from_rgb(1.0, 0.15, 0.1);

/// The agent on a road or junction lane nearest `pos` (XZ) within `radius`, ties to the lower
/// render ID. Visits only the lanes of roads and junctions near `pos` and the agents on them,
/// from the lane occupancy snapshot; an entry made stale by a later removal or move fails the
/// lane check. Agents on a driveway are not on a lane and cannot be picked.
pub(crate) fn pick_agent(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    agents: &AgentSystem,
    pos: Vector3,
    radius: f32,
) -> Option<usize> {
    let reach = Vector3::new(radius + LANE_REACH_M, 0.0, radius + LANE_REACH_M);
    let mut best: Option<(f32, u64, usize)> = None;
    let mut consider = |lane_id: usize| {
        let Some(lane) = lanes.lanes.get(lane_id) else {
            return;
        };
        for &(_, idx) in agents.lane_buckets.get(lane_id).into_iter().flatten() {
            if idx >= agents.len()
                || agents.current_lane_id[idx] != lane_id
                || !matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
            {
                continue;
            }
            let Some((at, _)) = sample_lane_pose(lane, agents.lane_distance[idx]) else {
                continue;
            };
            let distance_sq = (at.x - pos.x).powi(2) + (at.z - pos.z).powi(2);
            let candidate = (distance_sq, agents.render_id[idx], idx);
            if distance_sq <= radius * radius
                && best.is_none_or(|(d, id, _)| (distance_sq, candidate.1) < (d, id))
            {
                best = Some(candidate);
            }
        }
    };
    graph.visit_edges_near_aabb(pos - reach, pos + reach, |edge_id| {
        for &lane_id in lanes.edge_lanes.get(&edge_id).into_iter().flatten() {
            consider(lane_id);
        }
    });
    graph.visit_nodes_near_aabb(pos - reach, pos + reach, |node_id| {
        for &lane_id in lanes
            .node_lanes
            .get(&(node_id as usize))
            .into_iter()
            .flatten()
        {
            consider(lane_id);
        }
    });
    best.map(|(_, _, idx)| idx)
}

/// The inspector panel's fields for agent `idx`. O(1) apart from the building names.
pub(crate) fn inspection_details(core: &SimCore, idx: usize) -> VarDictionary {
    let agents = &core.agents;
    let lane_id = agents.current_lane_id[idx];
    let lane = core.transit_network.lane_system.lanes.get(lane_id);
    vdict! {
        "active": true,
        "render_id": agents.render_id[idx] as i64,
        "row": idx as i64,
        "mode": mode_label(agents.transit_mode[idx]),
        "state": transit_label(agents.transit[idx]),
        "speed_mps": agents.speed[idx],
        "activity": activity_label(agents.activity[idx]),
        "freight": agents.freight_shipment_id[idx] != u64::MAX,
        "home": building_details(core, agents.home_building[idx]),
        "work": building_details(core, agents.work_building[idx]),
        "destination": building_details(core, agents.target_building[idx]),
        "inside": building_details(core, agents.current_building[idx]),
        "lane": lane.map_or(-1, |_| lane_id as i64),
        "lane_edge": lane.map_or(-1, |lane| edge_label(lane.edge_id)),
        "lane_distance_m": agents.lane_distance[idx],
        "lane_length_m": lane.map_or(0.0, |lane| lane.length),
        "path_index": agents.current_path_index[idx] as i64,
        "path_length": agents.current_path[idx].len() as i64,
        "position": agent_position(core, idx),
    }
}

/// Where agent `idx` is: on its lane, at the door of the building it is in, or at its
/// off-road position on a driveway.
pub(crate) fn agent_position(core: &SimCore, idx: usize) -> Vector3 {
    let agents = &core.agents;
    if matches!(agents.transit[idx], TRANSIT_NETWORK | TRANSIT_INTERSECTION)
        && let Some(lane) = core
            .transit_network
            .lane_system
            .lanes
            .get(agents.current_lane_id[idx])
        && let Some((at, _)) = sample_lane_pose(lane, agents.lane_distance[idx])
    {
        return at;
    }
    let ground = if agents.transit[idx] == TRANSIT_IN_BUILDING
        && let Some(entrance) = core.allocator.entrances.get(agents.current_building[idx])
    {
        entrance.door_pos
    } else {
        Vector2::new(agents.pos_x[idx], agents.pos_y[idx])
    };
    on_ground(core, ground, 0.0)
}

/// The inspected agent's route as a line list, with its length and how it ends.
pub(crate) struct RouteLines {
    pub(crate) points: Vec<Vector3>,
    pub(crate) colors: Vec<Color>,
    /// Lanes and connectors on the route.
    pub(crate) lanes: usize,
    /// Metres left: lanes, connectors and driveways.
    pub(crate) remaining_m: f32,
    pub(crate) end: String,
}

/// Agent `idx`'s route from where it is to its destination door. O(route lanes and their
/// geometry points).
pub(crate) fn route_lines(core: &SimCore, idx: usize) -> RouteLines {
    let route = planned_route(
        &core.agents,
        idx,
        &core.allocator,
        &core.transit_network,
        &core.region_graph,
    );
    let mut out = RouteLines {
        points: Vec::new(),
        colors: Vec::new(),
        lanes: route.spans.len(),
        remaining_m: 0.0,
        end: end_label(core, idx, &route),
    };
    let mut last = route
        .egress
        .last()
        .map(|&point| on_ground(core, point, LINE_LIFT_M))
        .unwrap_or_else(|| agent_position(core, idx) + Vector3::new(0.0, LINE_LIFT_M, 0.0));
    append_driveway(core, &route.egress, &mut out);

    // On a road the first span is the agent's own lane or connector; leaving a building, the
    // first road lane is the next one.
    let on_road = route.egress.is_empty();
    let lanes = &core.transit_network.lane_system.lanes;
    let mut road_lanes_ahead = 0;
    for (k, span) in route.spans.iter().enumerate() {
        let Some(lane) = lanes.get(span.lane_id) else {
            continue;
        };
        let color = if k == 0 && on_road {
            CURRENT_LANE_COLOR
        } else if lane.edge_id == usize::MAX {
            CONNECTOR_COLOR
        } else {
            road_lanes_ahead += 1;
            if road_lanes_ahead == 1 {
                NEXT_LANE_COLOR
            } else {
                LATER_LANE_COLOR
            }
        };
        if let Some(end) = append_lane_span(lane, span.from_d, span.to_d, color, &mut out) {
            last = end;
        }
        out.remaining_m += span.to_d - span.from_d;
    }

    match route.end {
        RouteEnd::Destination => {
            let mut driveway = Vec::with_capacity(route.ingress.len() + 1);
            if !route.spans.is_empty() {
                driveway.push(Vector2::new(last.x, last.z));
            }
            driveway.extend_from_slice(&route.ingress);
            append_driveway(core, &driveway, &mut out);
            if let Some(&door) = route.ingress.last() {
                append_marker(on_ground(core, door, 0.0), DESTINATION_COLOR, &mut out);
            }
        }
        RouteEnd::PathEnd { .. } | RouteEnd::NoConnection { .. } => {
            append_marker(last, DEAD_END_COLOR, &mut out);
        }
        RouteEnd::None => {}
    }
    out
}

fn end_label(core: &SimCore, idx: usize, route: &PlannedRoute) -> String {
    match route.end {
        RouteEnd::Destination => match core.agents.target_building[idx] {
            usize::MAX => "destination door".to_string(),
            building => format!("destination building {building}"),
        },
        RouteEnd::PathEnd { node } => format!("path ends at node {node}"),
        RouteEnd::NoConnection { node } => {
            format!("no lane leads on to node {node}; the agent replans there")
        }
        RouteEnd::None => "no route on the roads".to_string(),
    }
}

// Appends the part of `lane` between `from_d` and `to_d`; returns its end point.
fn append_lane_span(
    lane: &Lane,
    from_d: f32,
    to_d: f32,
    color: Color,
    out: &mut RouteLines,
) -> Option<Vector3> {
    let lift = Vector3::new(0.0, LINE_LIFT_M, 0.0);
    let mut previous = sample_lane_pose(lane, from_d)?.0 + lift;
    let inner = lane
        .cum_dist
        .iter()
        .zip(&lane.geometry)
        .filter(|&(&d, _)| d > from_d && d < to_d)
        .map(|(_, &point)| point + lift);
    let end = sample_lane_pose(lane, to_d).map(|(point, _)| point + lift);
    for point in inner.chain(end) {
        out.points.extend([previous, point]);
        out.colors.extend([color, color]);
        previous = point;
    }
    Some(previous)
}

fn append_driveway(core: &SimCore, points: &[Vector2], out: &mut RouteLines) {
    for pair in points.windows(2) {
        out.points.extend([
            on_ground(core, pair[0], LINE_LIFT_M),
            on_ground(core, pair[1], LINE_LIFT_M),
        ]);
        out.colors.extend([DRIVEWAY_COLOR, DRIVEWAY_COLOR]);
        out.remaining_m += pair[0].distance_to(pair[1]);
    }
}

fn append_marker(at: Vector3, color: Color, out: &mut RouteLines) {
    let arms = [
        Vector3::new(MARKER_ARM_M, 0.0, 0.0),
        Vector3::new(0.0, 0.0, MARKER_ARM_M),
    ];
    for arm in arms {
        out.points.extend([at - arm, at + arm]);
        out.colors.extend([color, color]);
    }
    out.points
        .extend([at, at + Vector3::new(0.0, MARKER_POST_M, 0.0)]);
    out.colors.extend([color, color]);
}

fn on_ground(core: &SimCore, point: Vector2, lift: f32) -> Vector3 {
    Vector3::new(
        point.x,
        core.get_world_surface_height_internal(point) + lift,
        point.y,
    )
}

fn building_details(core: &SimCore, building_id: usize) -> VarDictionary {
    let Some(building) = core.allocator.buildings.get(building_id) else {
        return VarDictionary::new();
    };
    let kind = match building.zone_type {
        ZoneType::None => "utility",
        ZoneType::Residential => "residential",
        ZoneType::Commercial => "commercial",
        ZoneType::Industrial => "industrial",
        ZoneType::Office => "office",
        ZoneType::Mixed => "mixed",
    };
    let profile = core
        .demand
        .runtime_catalog()
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .map(|profile| profile.id.as_str())
        .unwrap_or_default();
    vdict! {
        "id": building_id as i64,
        "kind": kind,
        "profile": profile,
        "position": on_ground(core, Vector2::new(building.center_x, building.center_y), 0.0),
    }
}

fn edge_label(edge_id: usize) -> i64 {
    if edge_id == usize::MAX {
        -1
    } else {
        edge_id as i64
    }
}

fn mode_label(mode: u8) -> &'static str {
    match mode {
        MODE_WALK => "walking",
        MODE_CAR => "car",
        MODE_BIKE => "bike",
        MODE_BUS_PASSENGER => "bus passenger",
        MODE_TRAIN_PASSENGER => "train passenger",
        _ => "unknown",
    }
}

fn transit_label(transit: u8) -> &'static str {
    match transit {
        TRANSIT_IN_BUILDING => "in a building",
        TRANSIT_ACCESS_EGRESS => "leaving a building",
        TRANSIT_NETWORK => "on a road",
        TRANSIT_ACCESS_INGRESS => "arriving at a building",
        TRANSIT_IMMIGRATING => "moving into the city",
        TRANSIT_INTERSECTION => "in a junction",
        _ => "unknown",
    }
}

fn activity_label(activity: u8) -> &'static str {
    match activity {
        ACTIVITY_HOME => "home",
        ACTIVITY_WORK => "work",
        ACTIVITY_SHOPPING => "shopping or visit",
        _ => "unknown",
    }
}

/// A line list as a ribbon for the `route_ribbon` shader, which widens it to a constant
/// screen width: two triangles per segment. Each corner carries its segment's other end in
/// `uv` (x, y) and `uv2.x`, and in `uv2.y` the side (+1 or -1) it moves to on screen.
#[derive(Debug, Default)]
pub(crate) struct Ribbon {
    pub(crate) vertices: Vec<Vector3>,
    pub(crate) uv: Vec<Vector2>,
    pub(crate) uv2: Vec<Vector2>,
    pub(crate) colors: Vec<Color>,
}

/// Builds the ribbon for a line list (`points` in pairs, one color per point). O(points).
pub(crate) fn ribbon(points: &[Vector3], colors: &[Color]) -> Ribbon {
    let segments = points.len() / 2;
    let mut out = Ribbon {
        vertices: Vec::with_capacity(segments * 6),
        uv: Vec::with_capacity(segments * 6),
        uv2: Vec::with_capacity(segments * 6),
        colors: Vec::with_capacity(segments * 6),
    };
    for (pair, color) in points
        .as_chunks::<2>()
        .0
        .iter()
        .zip(colors.as_chunks::<2>().0)
    {
        let (a, b) = (pair[0], pair[1]);
        // Seen from `b` the segment points the other way, so the same on-screen side is the
        // opposite sign there.
        let corner = |at: Vector3, other: Vector3, side: f32| {
            (
                at,
                Vector2::new(other.x, other.y),
                Vector2::new(other.z, side),
            )
        };
        let a_left = corner(a, b, 1.0);
        let a_right = corner(a, b, -1.0);
        let b_left = corner(b, a, -1.0);
        let b_right = corner(b, a, 1.0);
        for (at, uv, uv2) in [a_left, a_right, b_left, a_right, b_right, b_left] {
            out.vertices.push(at);
            out.uv.push(uv);
            out.uv2.push(uv2);
        }
        out.colors
            .extend([color[0], color[0], color[1], color[0], color[1], color[1]]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::lanes::LaneType;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };

    fn add_road(graph: &mut RegionGraph, start: u32, end: u32) -> usize {
        let p0 = graph.node(start).pos;
        let p1 = graph.node(end).pos;
        graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 14.0,
            base_cost: 1.0,
            physical_length: p0.distance_to(p1),
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p0, p1],
            physical_geometry: vec![p0, p1],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        })
    }

    // An L of two roads meeting at (0, 0): west arm from (-200, 0), north arm to (0, -200).
    fn network() -> (RegionGraph, LaneSystem, u32) {
        let mut graph = RegionGraph::new();
        let west = graph.add_node(Vector3::new(-200.0, 0.0, 0.0), NodeType::Junction);
        let corner = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let north = graph.add_node(Vector3::new(0.0, 0.0, -200.0), NodeType::Junction);
        add_road(&mut graph, west, corner);
        add_road(&mut graph, corner, north);
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes, corner)
    }

    fn vehicle_lane(lanes: &LaneSystem, edge_id: usize, is_fwd: bool) -> usize {
        lanes.edge_lanes[&edge_id]
            .iter()
            .copied()
            .find(|&id| {
                let lane = &lanes.lanes[id];
                lane.is_fwd == is_fwd && lane.lane_type == LaneType::Vehicle
            })
            .unwrap()
    }

    // Puts a car on `lane_id` at `lane_d` and in that lane's occupancy bucket.
    fn car(agents: &mut AgentSystem, lanes: &LaneSystem, lane_id: usize, lane_d: f32) -> usize {
        let idx = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let transit = if lanes.lanes[lane_id].edge_id == usize::MAX {
            TRANSIT_INTERSECTION
        } else {
            TRANSIT_NETWORK
        };
        agents.transit[idx] = transit;
        agents.transit_mode[idx] = MODE_CAR;
        agents.current_lane_id[idx] = lane_id;
        agents.lane_distance[idx] = lane_d;
        agents.lane_buckets.resize(lanes.lanes.len(), Vec::new());
        agents.lane_buckets[lane_id].push((lane_d, idx));
        idx
    }

    fn at(lanes: &LaneSystem, lane_id: usize, lane_d: f32) -> Vector3 {
        sample_lane_pose(&lanes.lanes[lane_id], lane_d).unwrap().0
    }

    #[test]
    fn pick_takes_the_nearest_car_on_a_nearby_lane() {
        let (graph, lanes, _) = network();
        let east_bound = vehicle_lane(&lanes, 0, true);
        let west_bound = vehicle_lane(&lanes, 0, false);
        let mut agents = AgentSystem::new();
        let near = car(&mut agents, &lanes, east_bound, 100.0);
        let other = car(&mut agents, &lanes, west_bound, 90.0);
        let click = at(&lanes, east_bound, 101.0);
        assert_eq!(pick_agent(&graph, &lanes, &agents, click, 4.0), Some(near));
        let click = at(&lanes, west_bound, 91.0);
        assert_eq!(pick_agent(&graph, &lanes, &agents, click, 4.0), Some(other));
        // Nothing within the radius.
        let click = at(&lanes, east_bound, 150.0);
        assert_eq!(pick_agent(&graph, &lanes, &agents, click, 4.0), None);
    }

    #[test]
    fn pick_skips_stale_bucket_entries_and_finds_cars_in_junctions() {
        let (graph, lanes, corner) = network();
        let lane_id = vehicle_lane(&lanes, 0, true);
        let mut agents = AgentSystem::new();
        let moved = car(&mut agents, &lanes, lane_id, 100.0);
        // The car left the lane after the occupancy snapshot was taken.
        agents.current_lane_id[moved] = usize::MAX;
        let click = at(&lanes, lane_id, 100.0);
        assert_eq!(pick_agent(&graph, &lanes, &agents, click, 4.0), None);

        let connector = lanes.node_lanes[&(corner as usize)]
            .iter()
            .copied()
            .find(|&id| lanes.lanes[id].lane_type == LaneType::Vehicle)
            .unwrap();
        let turning = car(&mut agents, &lanes, connector, 2.0);
        let click = at(&lanes, connector, 2.0);
        assert_eq!(
            pick_agent(&graph, &lanes, &agents, click, 4.0),
            Some(turning)
        );
    }

    #[test]
    fn lane_span_lines_run_between_the_requested_distances() {
        let (_, lanes, _) = network();
        let lane = &lanes.lanes[vehicle_lane(&lanes, 0, true)];
        let mut out = RouteLines {
            points: Vec::new(),
            colors: Vec::new(),
            lanes: 0,
            remaining_m: 0.0,
            end: String::new(),
        };
        let end = append_lane_span(lane, 30.0, 70.0, NEXT_LANE_COLOR, &mut out).unwrap();
        let lift = Vector3::new(0.0, LINE_LIFT_M, 0.0);
        assert_eq!(
            out.points.first(),
            Some(&(sample_lane_pose(lane, 30.0).unwrap().0 + lift))
        );
        assert_eq!(out.points.last(), Some(&end));
        assert_eq!(end, sample_lane_pose(lane, 70.0).unwrap().0 + lift);
        assert!(out.points.len().is_multiple_of(2) && out.points.len() == out.colors.len());
        let drawn: f32 = out
            .points
            .chunks(2)
            .map(|pair| pair[0].distance_to(pair[1]))
            .sum();
        assert!((drawn - 40.0).abs() < 0.01, "drew {drawn} m");
    }

    #[test]
    fn ribbon_corners_carry_the_other_end_and_matching_sides() {
        let a = Vector3::new(1.0, 2.0, 3.0);
        let b = Vector3::new(4.0, 5.0, 6.0);
        let ribbon = ribbon(
            &[a, b, b, a],
            &[
                NEXT_LANE_COLOR,
                NEXT_LANE_COLOR,
                CONNECTOR_COLOR,
                CONNECTOR_COLOR,
            ],
        );
        assert_eq!(ribbon.vertices.len(), 12);
        assert!(ribbon.uv.len() == 12 && ribbon.uv2.len() == 12 && ribbon.colors.len() == 12);
        // Every corner names the segment's other end.
        for k in 0..6 {
            let other = Vector3::new(ribbon.uv[k].x, ribbon.uv[k].y, ribbon.uv2[k].x);
            assert_eq!(other, if ribbon.vertices[k] == a { b } else { a });
        }
        // One side of the segment: +1 seen from `a` is -1 seen from `b`.
        let side = |at: Vector3, sign: f32| (at, sign);
        let corners: Vec<_> = (0..6)
            .map(|k| side(ribbon.vertices[k], ribbon.uv2[k].y))
            .collect();
        assert_eq!(corners[0], (a, 1.0));
        assert_eq!(corners[2], (b, -1.0));
        assert_eq!(corners[1], (a, -1.0));
        assert_eq!(corners[4], (b, 1.0));
        assert_eq!(ribbon.colors[6], CONNECTOR_COLOR);
    }
}
