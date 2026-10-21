// SPDX-License-Identifier: GPL-2.0-only

//! Junction connector debug view: line geometry for the `junction_connectors` visual mode.
//!
//! Draws the connector lanes cars drive through junctions straight from `Lane::geometry`, so
//! the overlay shows the exact path the simulation uses. Junctions come from the 16 m node
//! lookup grid; a build costs O(grid cells in the area + nodes found + vertices drawn) and never
//! runs on the simulation tick.

use crate::config::{CAR_JUNCTION_MIN_SPEED_MS, CAR_JUNCTION_SPEED_MS};
use crate::nodes::sim::core::SimCore;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::agents::junction_stalls::waits_at_connector_end;
use crate::simulation::economy::agents::tick::connector_turn_speed;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::vehicle_junctions::node_whitelists_vehicle_turns;
use crate::simulation::network::lanes::{Lane, LaneSystem, LaneType};
use godot::prelude::{Color, Vector3};

// Lines float above the road surface so they do not z-fight with it.
const LIFT_M: f32 = 0.25;
const ARROW_LENGTH_M: f32 = 0.9;
const ARROW_HALF_WIDTH_M: f32 = 0.5;
const MARKER_RADIUS_M: f32 = 1.2;
// A car this slow at a connector end is held there.
const BLOCKED_SPEED_MS: f32 = 0.1;

const OPEN_COLOR: Color = Color::from_rgb(0.35, 0.75, 1.0);
const WHITELIST_COLOR: Color = Color::from_rgb(1.0, 0.6, 0.15);
const PASS_THROUGH_COLOR: Color = Color::from_rgb(0.85, 0.4, 1.0);
const ROAD_LANE_COLOR: Color = Color::from_rgba(0.9, 0.9, 0.9, 0.8);
const EMPTY_COLOR: Color = Color::from_rgba(0.55, 0.55, 0.55, 0.6);
const BLOCKED_COLOR: Color = Color::from_rgb(1.0, 0.1, 0.9);
// One, two and three or more cars.
const OCCUPANCY_COLORS: [Color; 3] = [
    Color::from_rgb(1.0, 0.95, 0.3),
    Color::from_rgb(1.0, 0.6, 0.1),
    Color::from_rgb(1.0, 0.2, 0.15),
];
const SLOW_COLOR: Color = Color::from_rgb(1.0, 0.15, 0.1);
const MID_COLOR: Color = Color::from_rgb(1.0, 0.9, 0.2);
const FAST_COLOR: Color = Color::from_rgb(0.2, 1.0, 0.3);

/// What the overlay colors connectors by; values match the visual mode's indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JunctionView {
    /// Whitelisted turns versus default-open turns.
    Arcs = 1,
    /// Curvature speed cap from red (minimum) to green (junction design speed).
    Speed = 2,
    /// Cars on each connector; a car held at the end is highlighted.
    Occupancy = 3,
    /// Arcs plus the road lane center lines of every incident edge.
    Lanes = 4,
}

impl JunctionView {
    /// The view for a `junction_connectors` index; `0` (off) and unknown indices are `None`.
    pub(crate) fn from_index(index: i64) -> Option<Self> {
        match index {
            1 => Some(Self::Arcs),
            2 => Some(Self::Speed),
            3 => Some(Self::Occupancy),
            4 => Some(Self::Lanes),
            _ => None,
        }
    }
}

/// Which junctions the overlay covers.
#[derive(Clone, Copy, Debug)]
pub(crate) enum JunctionArea {
    /// Every indexed node inside the inclusive XZ rectangle.
    Rect { min: Vector3, max: Vector3 },
    /// One focused node.
    Node(u32),
}

/// Line-list geometry: `points` holds segment endpoint pairs with one color per point.
#[derive(Debug, Default)]
pub(crate) struct JunctionLines {
    pub points: Vec<Vector3>,
    pub colors: Vec<Color>,
    /// Nodes drawn: those with a vehicle connector or a direct pass-through.
    pub junctions: usize,
    /// Vehicle connectors drawn.
    pub connectors: usize,
    /// Nodes marked as strict degree-two pass-throughs.
    pub pass_throughs: usize,
    /// Connectors with at least one car (occupancy view only).
    pub occupied: usize,
    /// Connectors holding a stopped car at their end (occupancy view only).
    pub blocked: usize,
}

/// Builds the overlay for `area`. Occupancy reads the retained lane buckets and shows every
/// connector empty while that snapshot is invalid.
pub(crate) fn build_junction_lines(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    agents: &AgentSystem,
    view: JunctionView,
    area: JunctionArea,
) -> JunctionLines {
    let nodes = match area {
        JunctionArea::Rect { min, max } => graph.get_nodes_near_aabb(min, max),
        JunctionArea::Node(node_id) => {
            let node_id = graph.get_valid_node(node_id);
            if (node_id as usize) < graph.node_count() {
                vec![node_id]
            } else {
                Vec::new()
            }
        }
    };
    let mut out = JunctionLines::default();
    let mut lane_edges = Vec::new();
    for node_id in nodes {
        let connectors = vehicle_connectors(lanes, node_id);
        let pass_through = connectors.is_empty() && is_direct_pass_through(graph, lanes, node_id);
        if connectors.is_empty() && !pass_through {
            continue;
        }
        out.junctions += 1;
        if pass_through {
            out.pass_throughs += 1;
            append_marker(&mut out, graph.node(node_id).pos, PASS_THROUGH_COLOR);
        }
        let whitelisted = node_whitelists_vehicle_turns(graph, node_id as usize);
        for &lane_id in &connectors {
            let lane = &lanes.lanes[lane_id];
            let color = connector_color(&mut out, agents, view, lane_id, lane, whitelisted);
            append_lane(&mut out, lane, color);
            out.connectors += 1;
        }
        if view == JunctionView::Lanes {
            lane_edges.extend_from_slice(graph.node_adjacency(node_id));
        }
    }
    lane_edges.sort_unstable();
    lane_edges.dedup();
    for edge_id in lane_edges {
        for &lane_id in lanes.edge_lanes.get(&edge_id).into_iter().flatten() {
            let lane = &lanes.lanes[lane_id];
            if lane.lane_type == LaneType::Vehicle {
                append_lane(&mut out, lane, ROAD_LANE_COLOR);
            }
        }
    }
    out
}

/// Nearest node within `radius` of `pos` (XZ) that the overlay would draw.
pub(crate) fn pick_junction(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    pos: Vector3,
    radius: f32,
) -> Option<u32> {
    let offset = Vector3::new(radius, 0.0, radius);
    let mut best: Option<(f32, u32)> = None;
    graph.visit_nodes_near_aabb(pos - offset, pos + offset, |node_id| {
        let node_pos = graph.node(node_id).pos;
        let distance_sq = (node_pos.x - pos.x).powi(2) + (node_pos.z - pos.z).powi(2);
        if distance_sq > radius * radius
            || best.is_some_and(|(best_sq, best_id)| (best_sq, best_id) <= (distance_sq, node_id))
        {
            return;
        }
        if !vehicle_connectors(lanes, node_id).is_empty()
            || is_direct_pass_through(graph, lanes, node_id)
        {
            best = Some((distance_sq, node_id));
        }
    });
    best.map(|(_, node_id)| node_id)
}

/// One line per vehicle connector at `node_id`, for the `events` log on focus.
pub(crate) fn describe_connectors(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    node_id: u32,
) -> Vec<String> {
    let whitelisted = node_whitelists_vehicle_turns(graph, node_id as usize);
    let rule = if whitelisted { "whitelisted" } else { "open" };
    let connectors = vehicle_connectors(lanes, node_id);
    let mut lines = Vec::with_capacity(connectors.len() + 1);
    lines.push(format!(
        "junction {node_id}: {} vehicle connectors, turns {rule}{}",
        connectors.len(),
        if connectors.is_empty() && is_direct_pass_through(graph, lanes, node_id) {
            ", direct pass-through"
        } else {
            ""
        }
    ));
    for lane_id in connectors {
        let lane = &lanes.lanes[lane_id];
        let from = inbound_lane(graph, lanes, node_id, lane_id).map(|from| &lanes.lanes[from]);
        let to = lane.next_lanes.first().map(|&to| &lanes.lanes[to]);
        lines.push(format!(
            "  connector {lane_id}: from {} to {}, {:.1} m, cap {:.1} m/s, {rule}",
            lane_label(from),
            lane_label(to),
            lane.length,
            connector_turn_speed(lane)
        ));
    }
    lines
}

fn lane_label(lane: Option<&Lane>) -> String {
    lane.map_or_else(
        || "?".to_owned(),
        |lane| format!("edge {} lane {}", lane.edge_id, lane.lane_idx),
    )
}

// The road lane whose successors include this connector; it arrives on an incident edge.
fn inbound_lane(
    graph: &RegionGraph,
    lanes: &LaneSystem,
    node_id: u32,
    connector: usize,
) -> Option<usize> {
    graph.node_adjacency(node_id).iter().find_map(|edge_id| {
        lanes
            .edge_lanes
            .get(edge_id)?
            .iter()
            .copied()
            .find(|&lane_id| lanes.lanes[lane_id].next_lanes.contains(&connector))
    })
}

fn vehicle_connectors(lanes: &LaneSystem, node_id: u32) -> Vec<usize> {
    lanes
        .node_lanes
        .get(&(node_id as usize))
        .into_iter()
        .flatten()
        .copied()
        .filter(|&lane_id| lanes.lanes[lane_id].lane_type == LaneType::Vehicle)
        .collect()
}

// A strict degree-two split links an arriving road lane straight to a departing one.
fn is_direct_pass_through(graph: &RegionGraph, lanes: &LaneSystem, node_id: u32) -> bool {
    if graph.node_adjacency_count_at(node_id) != 2 {
        return false;
    }
    graph.node_adjacency(node_id).iter().any(|&edge_id| {
        let edge = graph.edge(edge_id);
        lanes
            .edge_lanes
            .get(&edge_id)
            .into_iter()
            .flatten()
            .any(|&lane_id| {
                let lane = &lanes.lanes[lane_id];
                let arrives = if lane.is_fwd {
                    edge.end_node == node_id
                } else {
                    edge.start_node == node_id
                };
                lane.lane_type == LaneType::Vehicle
                    && arrives
                    && lane.next_lanes.iter().any(|&next| {
                        lanes.lanes[next].edge_id != usize::MAX
                            && lanes.lanes[next].lane_type == LaneType::Vehicle
                    })
            })
    })
}

fn connector_color(
    out: &mut JunctionLines,
    agents: &AgentSystem,
    view: JunctionView,
    lane_id: usize,
    lane: &Lane,
    whitelisted: bool,
) -> Color {
    match view {
        JunctionView::Arcs | JunctionView::Lanes if whitelisted => WHITELIST_COLOR,
        JunctionView::Arcs | JunctionView::Lanes => OPEN_COLOR,
        JunctionView::Speed => speed_color(connector_turn_speed(lane)),
        JunctionView::Occupancy => {
            let bucket = agents
                .lane_buckets
                .get(lane_id)
                .filter(|_| agents.lane_bucket_snapshot_valid)
                .map_or(&[][..], Vec::as_slice);
            if bucket.is_empty() {
                return EMPTY_COLOR;
            }
            out.occupied += 1;
            let held = bucket.iter().any(|&(lane_d, agent)| {
                waits_at_connector_end(lane_d, lane.length)
                    && agents
                        .speed
                        .get(agent)
                        .is_some_and(|&speed| speed < BLOCKED_SPEED_MS)
            });
            if held {
                out.blocked += 1;
                return BLOCKED_COLOR;
            }
            OCCUPANCY_COLORS[bucket.len().min(OCCUPANCY_COLORS.len()) - 1]
        }
    }
}

fn speed_color(cap: f32) -> Color {
    let t = ((cap - CAR_JUNCTION_MIN_SPEED_MS)
        / (CAR_JUNCTION_SPEED_MS - CAR_JUNCTION_MIN_SPEED_MS))
        .clamp(0.0, 1.0);
    // `Color::lerp` calls into the engine; mix by hand so tests run without Godot.
    let mix = |a: Color, b: Color, w: f32| {
        Color::from_rgb(
            a.r + (b.r - a.r) * w,
            a.g + (b.g - a.g) * w,
            a.b + (b.b - a.b) * w,
        )
    };
    if t < 0.5 {
        mix(SLOW_COLOR, MID_COLOR, t * 2.0)
    } else {
        mix(MID_COLOR, FAST_COLOR, t * 2.0 - 1.0)
    }
}

// The lane polyline plus a chevron three quarters along it, pointing the travel direction.
fn append_lane(out: &mut JunctionLines, lane: &Lane, color: Color) {
    let lift = Vector3::new(0.0, LIFT_M, 0.0);
    for segment in lane.geometry.windows(2) {
        push_segment(out, segment[0] + lift, segment[1] + lift, color);
    }
    let count = lane.geometry.len();
    if count < 2 {
        return;
    }
    let tip_index = (count * 3 / 4).clamp(1, count - 1);
    let tip = lane.geometry[tip_index] + lift;
    let delta = lane.geometry[tip_index] - lane.geometry[tip_index - 1];
    let flat = Vector3::new(delta.x, 0.0, delta.z);
    if flat.length_squared() <= 1.0e-8 {
        return;
    }
    let forward = flat.normalized();
    let side = Vector3::new(-forward.z, 0.0, forward.x) * ARROW_HALF_WIDTH_M;
    let back = tip - forward * ARROW_LENGTH_M;
    push_segment(out, tip, back + side, color);
    push_segment(out, tip, back - side, color);
}

// A diamond around the node.
fn append_marker(out: &mut JunctionLines, center: Vector3, color: Color) {
    let center = center + Vector3::new(0.0, LIFT_M, 0.0);
    let corners = [
        Vector3::new(MARKER_RADIUS_M, 0.0, 0.0),
        Vector3::new(0.0, 0.0, MARKER_RADIUS_M),
        Vector3::new(-MARKER_RADIUS_M, 0.0, 0.0),
        Vector3::new(0.0, 0.0, -MARKER_RADIUS_M),
    ];
    for index in 0..corners.len() {
        let next = corners[(index + 1) % corners.len()];
        push_segment(out, center + corners[index], center + next, color);
    }
}

fn push_segment(out: &mut JunctionLines, a: Vector3, b: Vector3, color: Color) {
    out.points.push(a);
    out.points.push(b);
    out.colors.push(color);
    out.colors.push(color);
}

impl SimCore {
    /// Junction overlay geometry for the live network; see [`build_junction_lines`].
    pub(crate) fn junction_debug_lines(
        &self,
        view: JunctionView,
        area: JunctionArea,
    ) -> JunctionLines {
        build_junction_lines(
            &self.region_graph,
            &self.transit_network.lane_system,
            &self.agents,
            view,
            area,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::{MODE_CAR, TRANSIT_INTERSECTION};
    use crate::simulation::network::graph::Edge;
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
            speed_limit: 50.0,
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

    // A T-junction at the origin (west, east and north arms) and, 300 m east, a straight
    // degree-two split. Returns the graph, lanes, T node, split node and the T arms.
    fn network(whitelist: bool) -> (RegionGraph, LaneSystem, u32, u32, [usize; 3]) {
        let mut graph = RegionGraph::new();
        let west = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let east = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let north = graph.add_node(Vector3::new(0.0, 0.0, -100.0), NodeType::Junction);
        let split = graph.add_node(Vector3::new(300.0, 0.0, 0.0), NodeType::Junction);
        let far = graph.add_node(Vector3::new(400.0, 0.0, 0.0), NodeType::Junction);
        let arms = [
            add_road(&mut graph, west, center),
            add_road(&mut graph, center, east),
            add_road(&mut graph, center, north),
        ];
        add_road(&mut graph, east, split);
        add_road(&mut graph, split, far);
        if whitelist {
            graph.add_lane_connection(center, arms[0], 0, arms[1], 0);
        }
        graph.rebuild_adjacency_list();
        graph.rebuild_intersection_clips();
        let mut lanes = LaneSystem::new();
        lanes.rebuild(&mut graph);
        (graph, lanes, center, split, arms)
    }

    fn around(center: Vector3, half: f32) -> JunctionArea {
        let half = Vector3::new(half, 0.0, half);
        JunctionArea::Rect {
            min: center - half,
            max: center + half,
        }
    }

    #[test]
    fn t_junction_draws_every_turn_and_marks_the_split() {
        let (graph, lanes, center, split, _) = network(false);
        let agents = AgentSystem::new();
        let at_t = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Arcs,
            JunctionArea::Node(center),
        );
        // Three arms with one lane each way: two non-U-turn exits per arriving lane.
        assert_eq!(
            (at_t.junctions, at_t.connectors, at_t.pass_throughs),
            (1, 6, 0)
        );
        assert_eq!(at_t.points.len(), at_t.colors.len());
        assert!(at_t.colors.iter().all(|&color| color == OPEN_COLOR));
        // Each connector's polyline plus its two-segment arrow.
        let expected: usize = vehicle_connectors(&lanes, center)
            .iter()
            .map(|&lane_id| (lanes.lanes[lane_id].geometry.len() - 1 + 2) * 2)
            .sum();
        assert_eq!(at_t.points.len(), expected);
        let first = &lanes.lanes[vehicle_connectors(&lanes, center)[0]];
        assert_eq!(
            at_t.points[0],
            first.geometry[0] + Vector3::new(0.0, LIFT_M, 0.0)
        );

        let at_split = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Arcs,
            JunctionArea::Node(split),
        );
        assert_eq!((at_split.junctions, at_split.connectors), (1, 0));
        assert_eq!(at_split.pass_throughs, 1);
        assert_eq!(at_split.points.len(), 8);
        assert!(
            at_split
                .colors
                .iter()
                .all(|&color| color == PASS_THROUGH_COLOR)
        );

        // The rectangle finds only the nodes inside it, through the node grid.
        let near_t = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Arcs,
            around(Vector3::ZERO, 50.0),
        );
        assert_eq!((near_t.junctions, near_t.connectors), (1, 6));
        // The east node and the split are both straight degree-two splits.
        let splits = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Arcs,
            around(Vector3::new(200.0, 0.0, 0.0), 120.0),
        );
        assert_eq!(
            (splits.junctions, splits.pass_throughs, splits.connectors),
            (2, 2, 0)
        );
        assert_eq!(
            pick_junction(&graph, &lanes, Vector3::new(3.0, 0.0, 2.0), 20.0),
            Some(center)
        );
        assert_eq!(
            pick_junction(&graph, &lanes, Vector3::new(50.0, 0.0, 0.0), 20.0),
            None
        );
    }

    #[test]
    fn whitelist_speed_and_lanes_views() {
        let (graph, lanes, center, _, arms) = network(true);
        let agents = AgentSystem::new();
        let arcs = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Arcs,
            JunctionArea::Node(center),
        );
        assert_eq!(arcs.connectors, 1);
        assert!(arcs.colors.iter().all(|&color| color == WHITELIST_COLOR));

        let lanes_view = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Lanes,
            JunctionArea::Node(center),
        );
        let road_lane_points: usize = arms
            .iter()
            .flat_map(|edge_id| &lanes.edge_lanes[edge_id])
            .filter(|&&lane_id| lanes.lanes[lane_id].lane_type == LaneType::Vehicle)
            .map(|&lane_id| (lanes.lanes[lane_id].geometry.len() - 1 + 2) * 2)
            .sum();
        assert_eq!(
            lanes_view.points.len(),
            arcs.points.len() + road_lane_points
        );

        // A straight connector runs at the design speed; open-node right turns are slower.
        let (graph, lanes, center, _, _) = network(false);
        let speed = build_junction_lines(
            &graph,
            &lanes,
            &agents,
            JunctionView::Speed,
            JunctionArea::Node(center),
        );
        let close =
            |a: Color, b: Color| (a.r - b.r).abs() + (a.g - b.g).abs() + (a.b - b.b).abs() < 1.0e-4;
        assert!(speed.colors.iter().any(|&color| close(color, FAST_COLOR)));
        assert!(speed.colors.iter().any(|&color| !close(color, FAST_COLOR)));
        assert!(close(speed_color(CAR_JUNCTION_MIN_SPEED_MS), SLOW_COLOR));
        assert!(close(speed_color(CAR_JUNCTION_SPEED_MS), FAST_COLOR));
        let lines = describe_connectors(&graph, &lanes, center);
        assert_eq!(lines.len(), 7);
        assert!(
            lines[0].contains("6 vehicle connectors, turns open"),
            "{}",
            lines[0]
        );
        assert!(
            lines[1..]
                .iter()
                .all(|line| line.contains("from edge ") && line.contains("cap "))
        );
    }

    #[test]
    fn occupancy_counts_cars_and_highlights_a_held_car() {
        let (graph, lanes, center, _, _) = network(false);
        let connectors = vehicle_connectors(&lanes, center);
        let mut agents = AgentSystem::new();
        let occupancy = |agents: &AgentSystem| {
            build_junction_lines(
                &graph,
                &lanes,
                agents,
                JunctionView::Occupancy,
                JunctionArea::Node(center),
            )
        };
        assert_eq!(occupancy(&agents).occupied, 0);

        agents.lane_buckets = vec![Vec::new(); lanes.lanes.len()];
        let moving = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let held = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        for agent in [moving, held] {
            agents.transit[agent] = TRANSIT_INTERSECTION;
            agents.transit_mode[agent] = MODE_CAR;
        }
        agents.speed[moving] = 4.0;
        agents.lane_buckets[connectors[0]].push((1.0, moving));
        agents.lane_buckets[connectors[1]].push((lanes.lanes[connectors[1]].length, held));
        // Buckets are ignored until the snapshot is valid.
        assert_eq!(occupancy(&agents).occupied, 0);
        agents.lane_bucket_snapshot_valid = true;
        let lines = occupancy(&agents);
        assert_eq!((lines.occupied, lines.blocked), (2, 1));
        assert!(lines.colors.contains(&BLOCKED_COLOR));
        assert!(lines.colors.contains(&OCCUPANCY_COLORS[0]));
        assert!(lines.colors.contains(&EMPTY_COLOR));
    }
}
