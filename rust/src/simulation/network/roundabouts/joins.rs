// SPDX-License-Identifier: GPL-2.0-only

//! A stroke joining a ring near a bare ring node (`ROAD-62`, D9). A pass-through node holds a join
//! as an arm would: the ring between them must take the join's two ring mouths, so a player who
//! places a bare ring and draws a crossroads onto it would be refused on the arms that land near
//! one. A bare node has no road of its own to keep it where it is, so the join takes its place: the
//! node slides along the circle to the join and its two ring edges are re-sampled from the circle
//! with their leads. The bare nodes left in the arcs either side of the join spread evenly along
//! them, and a bare node goes in mid-arc wherever an edge would turn a half turn or more (D8).
//! Only arm against arm crowds a ring, and that stays the junction layout's refusal. This all
//! happens on the stroke's excerpt, so a preview and a commit share one plan and one undo.

use super::super::graph::{RegionGraph, RoundaboutRing};
use super::super::topology::{SplitHalves, split_edge_graph};
use super::super::types::NodeType;
use super::{Vector2, Vector3, price_edge};
use crate::config;
use std::f32::consts::{PI, TAU};

// A ring edge's largest turn, short of a half turn by a rounding margin: arms drawn exactly
// opposite each other leave a half turn to a few ulps either way.
const HALF_TURN_RAD: f32 = PI - 1e-3;
// What the join's two corners take of the ring beyond the stroke's own width, either side.
const JOIN_CORNERS_M: f32 = 7.0;

/// The ring's edges through `edge`, in the sense of travel from it, each with the node it leaves.
/// `None` when the walk does not close, as on a part of a ring. `O(ring edges x degree)`.
pub(crate) fn ring_walk(graph: &RegionGraph, edge: usize) -> Option<Vec<(u32, usize)>> {
    let ring = graph.edge(edge).roundabout?;
    let mut walk = Vec::new();
    let mut at = edge;
    loop {
        let record = graph.edge(at);
        walk.push((graph.get_valid_node(record.start_node), at));
        let end = graph.get_valid_node(record.end_node);
        at = *graph.node_adjacency(end).iter().find(|&&next| {
            let next = graph.edge(next);
            !next.deleted
                && next.roundabout == Some(ring)
                && graph.get_valid_node(next.start_node) == end
        })?;
        if at == edge {
            return Some(walk);
        }
        if walk.len() >= graph.edge_count() {
            return None;
        }
    }
}

/// The ring edge a stroke end at `point` joins: the circle within the intersection tolerance, of
/// the lowest ring edge id near it. `O(log E + edges near)`.
pub(crate) fn ring_edge_at(graph: &RegionGraph, point: Vector3) -> Option<usize> {
    let mut near = graph.get_edges_near_point(point, config::INTERSECTION_TOLERANCE);
    near.sort_unstable();
    near.into_iter().find(|&id| {
        let edge = graph.edge(id);
        edge.roundabout.filter(|_| !edge.deleted).is_some_and(|ring| {
            let from_centre = Vector2::new(point.x, point.z).distance_to(ring.centre);
            (from_centre - ring.radius_m).abs() < config::INTERSECTION_TOLERANCE
        })
    })
}

// A ring node of the walk: its id, the edge leaving it, its angle and whether it has no arm.
#[derive(Clone, Copy)]
struct RingStop {
    node: u32,
    leaving: usize,
    angle: f32,
    bare: bool,
    moved: bool,
}

/// What a join near a bare node did to the excerpt.
pub(crate) struct RingJoin {
    /// The ring node at the join, where the stroke ends.
    pub(crate) node: u32,
    /// The ring edges changed, the halves of the splits among them.
    pub(crate) edges: Vec<usize>,
    /// Each bare node inserted, as the split of the edge it stands on.
    pub(crate) splits: Vec<(usize, SplitHalves)>,
}

/// The join of a stroke end at `point`, of a road `half_width_m` wide to its roadbed's edge, in the
/// excerpt `graph`: a bare ring node within the reach of the stroke's mouth takes its place, so
/// nothing but another arm can crowd it. `None` when no bare node is in reach: the join splits the
/// ring edge as any stroke splits a road. `taken` are nodes other ends of the stroke already took.
/// `O(ring edges x (degree + points))`.
pub(crate) fn join_near_bare_node(
    graph: &mut RegionGraph,
    point: Vector3,
    half_width_m: f32,
    taken: Option<u32>,
) -> Option<RingJoin> {
    let edge = ring_edge_at(graph, point)?;
    let ring = graph.edge(edge).roundabout?;
    let mut stops: Vec<RingStop> = ring_walk(graph, edge)?
        .into_iter()
        .map(|(node, leaving)| RingStop {
            node,
            leaving,
            angle: ring.angle_of(graph.node(node).pos),
            bare: Some(node) != taken && graph.live_node_connection_count(node) == 2,
            moved: false,
        })
        .collect();
    let angle = ring.angle_of(point);
    let reach_rad = (2.0 * half_width_m + JOIN_CORNERS_M) / ring.radius_m;
    let count = stops.len();
    let on = (0..count).find(|&i| {
        sweep(stops[i].angle, angle) < sweep(stops[i].angle, stops[(i + 1) % count].angle)
    })?;
    let next = (on + 1) % count;
    let (behind, ahead) = (
        sweep(stops[on].angle, angle),
        sweep(angle, stops[next].angle),
    );
    let near = |stop: RingStop, arc: f32| stop.bare && arc < reach_rad;
    let slid = if near(stops[on], behind) && (behind <= ahead || !near(stops[next], ahead)) {
        on
    } else if near(stops[next], ahead) {
        next
    } else {
        return None;
    };
    let mut join = RingJoin {
        node: stops[slid].node,
        edges: Vec::new(),
        splits: Vec::new(),
    };
    stops[slid].angle = angle;
    stops[slid].bare = false;
    spread_bare_nodes(&mut stops, slid);
    stops[slid].moved = true;
    for stop in stops.iter().filter(|stop| stop.moved) {
        move_node(graph, ring, *stop, &mut join);
    }
    close_half_turns(graph, ring, edge, &mut join)?;
    join.edges.sort_unstable();
    join.edges.dedup();
    Some(join)
}

// Spreads the bare nodes of the two arcs at the join `stops[join]`, between it and the next arm
// nodes, evenly along their arcs: they give the new arm's mouth what room the ring has.
fn spread_bare_nodes(stops: &mut [RingStop], join: usize) {
    let count = stops.len();
    let ahead = (1..count)
        .take_while(|step| stops[(join + step) % count].bare)
        .count();
    if ahead == count - 1 {
        spread_along(stops, join, ahead, TAU);
        return;
    }
    let behind = (1..count)
        .take_while(|step| stops[(join + count - step) % count].bare)
        .count();
    let arm_ahead = stops[(join + ahead + 1) % count].angle;
    spread_along(stops, join, ahead, sweep(stops[join].angle, arm_ahead));
    let arm_behind = (join + count - behind - 1) % count;
    let arc = sweep(stops[arm_behind].angle, stops[join].angle);
    spread_along(stops, arm_behind, behind, arc);
}

// Places the `bare` stops after `stops[from]` evenly along `arc` in the sense of travel.
fn spread_along(stops: &mut [RingStop], from: usize, bare: usize, arc: f32) {
    let start = stops[from].angle;
    let count = stops.len();
    for step in 1..=bare {
        let stop = &mut stops[(from + step) % count];
        let share = step as f32 / (bare + 1) as f32;
        stop.angle = start + super::travel_sense() * arc * share;
        stop.moved = true;
    }
}

// The turn from `from` to `to` in the sense of travel, in `[0, TAU)`.
fn sweep(from: f32, to: f32) -> f32 {
    (super::travel_sense() * (to - from)).rem_euclid(TAU)
}

// Moves `stop`'s node on `ring` to its angle and re-samples its two ring edges.
fn move_node(graph: &mut RegionGraph, ring: RoundaboutRing, stop: RingStop, join: &mut RingJoin) {
    let old = graph.node(stop.node).pos;
    let new = ring.at(stop.angle, ring.radius_m, old.y);
    graph.remove_node_from_spatial_index(stop.node, old);
    graph.nodes[stop.node as usize].pos = new;
    graph.add_node_to_spatial_index(stop.node);
    let arriving = graph.node_adjacency(stop.node).iter().copied().find(|&id| {
        let edge = graph.edge(id);
        !edge.deleted && edge.roundabout == Some(ring) && id != stop.leaving
    });
    for id in arriving.into_iter().chain([stop.leaving]) {
        resample(graph, ring, id);
        join.edges.push(id);
    }
}

// Rebuilds ring edge `id` between its nodes, its clips left to the solve.
fn resample(graph: &mut RegionGraph, ring: RoundaboutRing, id: usize) {
    let edge = graph.edge(id);
    let (from, to) = (edge.start_node, edge.end_node);
    let points = ring.arc(graph.node(from).pos, graph.node(to).pos);
    graph.remove_from_spatial_index(id);
    let edge = graph.edge_mut(id);
    edge.geometry = points.clone();
    edge.physical_geometry = points;
    price_edge(edge);
    graph.add_to_spatial_index(id);
}

// Splits every ring edge of a half turn or more at its middle with a new bare node (D8).
fn close_half_turns(
    graph: &mut RegionGraph,
    ring: RoundaboutRing,
    edge: usize,
    join: &mut RingJoin,
) -> Option<()> {
    for (node, id) in ring_walk(graph, edge)? {
        let far = graph.edge(id).end_node;
        let (from, to) = (
            ring.angle_of(graph.node(node).pos),
            ring.angle_of(graph.node(far).pos),
        );
        let turn = sweep(from, to);
        if turn < HALF_TURN_RAD {
            continue;
        }
        let y = graph.node(node).pos.y;
        let middle = ring.at(from + super::travel_sense() * 0.5 * turn, ring.radius_m, y);
        let segment = nearest_segment(&graph.edge(id).geometry, middle);
        let bare = graph.add_node(middle, NodeType::Junction);
        let halves = split_edge_graph(graph, id, segment, bare, None)?;
        join.edges.extend([id, halves.new_edge_id]);
        join.splits.push((id, halves));
    }
    Some(())
}

// The segment of `points` nearest `target`. `O(points)`.
fn nearest_segment(points: &[Vector3], target: Vector3) -> usize {
    let mut nearest = (f32::INFINITY, 0);
    for (segment, pair) in points.windows(2).enumerate() {
        let distance_m = pair[0].lerp(pair[1], 0.5).distance_to(target);
        if distance_m < nearest.0 {
            nearest = (distance_m, segment);
        }
    }
    nearest.1
}
