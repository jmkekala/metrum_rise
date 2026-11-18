// SPDX-License-Identifier: GPL-2.0-only

//! Road ends make way along their own line (`ROAD-118` round JS5c, the user's decision D2 (a)).
//! Nothing the player drew moves and no existing point moves: a road end the edit meets grows
//! straight on along its own line, and the junction forms where the lines meet.
//!
//! - **The corner.** A stroke end whose cap touches a road end's cap, where the stroke's own line
//!   and the old road's line meet ahead of both ends: the corner node stands where the lines
//!   meet; the stroke runs straight on to it (a straight lead-in, [`super::lead_in`]) and the old
//!   road grows straight on to it, by at most the reach (the two roadbed half widths). The two
//!   roads meet in a bend whose arms stand at least the junction arm limit (20 degrees,
//!   `ROAD-54`) apart. Lines that do not meet within those bounds keep round JS5a's arc into the
//!   road end.
//! - **The pass.** A stroke whose side passes a road end's cap, where the old road's line meets
//!   the stroke's centreline within the reach at the arm limit or more: the old road grows straight
//!   on to that point and the stroke is split there, a T. A road end within the topology's 2 m
//!   end join of the stroke keeps that join, as before.
//!
//! The growth is a piece of road the plan lays from the road end with the live road's own
//! cross-section and attributes; the topology meets it like any road drawn from that end
//! (`process_intersections_splitting_at`), and round JS2's dissolve joins it to the old road,
//! which keeps its id and direction and runs on to the corner or the T, its points bit for bit
//! plus the growth. Guards, as round JS3's: a road end is only grown when it is a plain joint of
//! one ground car road (no authored crossing, no vehicle turn rule, no pocket, ring or structure)
//! that no occupied parcel fronts within the reach of its end, and when no other road lies within
//! the topology's 2 m end join of the growth or the lead-in. Otherwise the stroke end keeps round
//! JS5a's arc, and a passed road end stays as it is.
//!
//! Pure functions of the graph, the stroke and the land's frontages: preview and commit decide
//! alike. Per touched end `O(log E + candidates x points)` and one frontage probe; per pass
//! `O(road ends among the excerpt's roads x stroke points)`. No new spatial structure.

use super::contact::{MIN_CONTACT_ANGLE_DEG, meets_on_the_ground};
use super::lead_in::{LeadIn, end_frame, leaving};
use crate::config;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::guide::DVec2;
use crate::simulation::network::road_edit::RoadFrontages;
use crate::simulation::network::roadbed;
use crate::simulation::network::types::{NodeType, TransitType};
use godot::prelude::Vector3;

/// A road end that grows straight on along its own line (`ROAD-118` round JS5c, module docs):
/// the plan lays a piece with the road's cross-section from the end to `target` and dissolves the
/// end into it, so the road keeps its id and runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Growth {
    /// The road whose end grows, in the ids of the graph it was found in.
    pub(crate) road: usize,
    /// The road end the growth starts from, in the same ids.
    pub(crate) node: u32,
    /// Where the growth ends, on the road's line: the corner the stroke runs on to, or the point
    /// on the stroke's centreline the road meets.
    pub(crate) target: Vector3,
}

// A growth no longer than this is no growth: the stroke joins the road end itself (round JS5a's
// lead-in), as a lead-in this short replaces the drawn end.
const MIN_GROWTH_M: f64 = 0.05;

// The rounding of a stroke end standing on the old road's line: its corner may read a hair
// behind it.
const RUN_ROUNDING_M: f64 = 1.0e-3;

// The arm limit as the corner keeps it, as the lead-in's: a hundredth of a degree over, so the
// `f32` rounding of the built points never reads the arms under it.
const ARM_BOUND_DEG: f64 = MIN_CONTACT_ANGLE_DEG as f64 + 0.01;

/// The corner of stroke end `end` (0 the start, 1 the end) of `points`, whose cap touches road
/// end `node` of road `road` within `reach_m` (module docs): the straight lead-in from the drawn
/// end to the corner and the old road's growth to it. `None` where the lines do not meet ahead of
/// both ends within the bounds, the arms would stand under the arm limit, a guard keeps the road
/// end, or `far` (the live node the stroke's other end stands on) is the old road's other end.
/// `ground_y` gives the corner's height. `O(log E + candidates x points)` plus one frontage probe.
pub(super) fn corner(
    graph: &RegionGraph,
    frontages: &dyn RoadFrontages,
    (points, end, far): (&[Vector3], usize, Option<u32>),
    (node, road, reach_m): (u32, usize, f32),
    ground_y: impl Fn(f32, f32) -> f32,
) -> Option<(LeadIn, Growth)> {
    let u = grows(graph, frontages, node, road, reach_m)?;
    let record = graph.edge(road);
    let other = graph.get_valid_node(if graph.get_valid_node(record.end_node) == node {
        record.start_node
    } else {
        record.end_node
    });
    if far == Some(other) {
        return None;
    }
    let (tip, t) = end_frame(points, end)?;
    let n = flat(graph.node(node).pos);
    // The arms at the corner, back along the stroke and back along the road, stand as far apart
    // as the two directions.
    if t.dot(u).clamp(-1.0, 1.0).acos() < ARM_BOUND_DEG.to_radians() {
        return None;
    }
    let cross = t.perp_dot(u);
    if cross.abs() <= 1.0e-9 {
        return None;
    }
    // `tip + s t = n + g u`.
    let d = n - tip;
    let s = d.perp_dot(u) / cross;
    let g = d.perp_dot(t) / cross;
    let reach = f64::from(reach_m);
    let max_run = reach / f64::from(MIN_CONTACT_ANGLE_DEG).to_radians().sin();
    if !(-RUN_ROUNDING_M..=max_run).contains(&s) || g <= MIN_GROWTH_M || g > reach {
        return None;
    }
    let at = tip + t * s.max(0.0);
    let drawn = points[if end == 0 { 0 } else { points.len() - 1 }];
    let (x, z) = (at.x as f32, at.y as f32);
    let target = Vector3::new(x, ground_y(x, z), z);
    let node_pos = graph.node(node).pos;
    if !clear(graph, road, &[(node_pos, target), (drawn, target)]) {
        return None;
    }
    Some((
        LeadIn {
            target,
            trim_m: 0.0,
        },
        Growth { road, node, target },
    ))
}

/// The road ends among `roads` (ids of `graph`) whose caps the stroke `points` of roadbed half
/// width `half_width_m` passes, each grown straight on to the stroke's centreline (module docs),
/// appended to `out` sorted by node: a road end, of a road that neither the stroke reaches within
/// the topology's 2 m end join nor ends at a node in `ends` (the live nodes the stroke's ends
/// stand on and the road ends its corners grow), within the reach of the stroke, whose line meets
/// the stroke at the arm limit or more within the reach, at least the least end segment from
/// either end of the stroke, where nothing else lies in the way. `O(roads x road points x stroke
/// points)` plus one frontage probe per such end; no allocation unless an end is passed.
pub(super) fn passes(
    graph: &RegionGraph,
    frontages: &dyn RoadFrontages,
    (points, half_width_m, ends): (&[Vector3], f32, &[u32]),
    roads: impl Iterator<Item = usize>,
    out: &mut Vec<Growth>,
) {
    let first = out.len();
    for road in roads {
        let record = graph.edge(road);
        if record.deleted || !meets_on_the_ground(record) {
            continue;
        }
        let nodes = [record.start_node, record.end_node].map(|node| graph.get_valid_node(node));
        if nodes.iter().any(|node| ends.contains(node)) {
            continue;
        }
        for node in nodes {
            if out.iter().any(|growth| growth.node == node) {
                continue;
            }
            if let Some(growth) = pass(graph, frontages, points, half_width_m, (node, road)) {
                out.push(growth);
            }
        }
    }
    out[first..].sort_unstable_by_key(|growth| growth.node);
}

// Road end `node` of `road` as a pass of [`passes`], or `None`.
fn pass(
    graph: &RegionGraph,
    frontages: &dyn RoadFrontages,
    points: &[Vector3],
    half_width_m: f32,
    (node, road): (u32, usize),
) -> Option<Growth> {
    let live = graph
        .node_adjacency(node)
        .iter()
        .filter(|&&id| !graph.edge(id).deleted)
        .count();
    if live != 1 {
        return None;
    }
    let pos = graph.node(node).pos;
    let reach_m = super::contact::reach_m(half_width_m, graph.edge(road));
    let distance = nearest_distance(points, pos)?;
    if distance <= f64::from(config::INTERSECTION_TOLERANCE) || distance >= f64::from(reach_m) {
        return None;
    }
    // A road the stroke crosses or reaches within the end join meets it there, as before.
    let tolerance = f64::from(config::INTERSECTION_TOLERANCE);
    let road_points = &graph.edge(road).geometry;
    if road_points.windows(2).any(|road_pair| {
        points.windows(2).any(|pair| {
            segment_distance((pair[0], pair[1]), (road_pair[0], road_pair[1])) < tolerance
        })
    }) {
        return None;
    }
    let u = grows(graph, frontages, node, road, reach_m)?;
    let n = flat(pos);
    // The nearest crossing of the road's line with the stroke ahead of the end, within the reach:
    // the growth, the point, the sine of the angle there and the point's station on the stroke.
    let mut hit: Option<(f64, Vector3, f64, f64)> = None;
    let mut station = 0.0;
    for pair in points.windows(2) {
        let (a, b) = (flat(pair[0]), flat(pair[1]));
        let ab = b - a;
        let length = ab.length();
        let cross = u.perp_dot(ab);
        if length > 0.0 && cross.abs() > 1.0e-9 * length {
            let d = a - n;
            let g = d.perp_dot(ab) / cross;
            let along = d.perp_dot(u) / cross;
            if (0.0..=1.0).contains(&along)
                && g > MIN_GROWTH_M
                && g <= f64::from(reach_m)
                && hit.is_none_or(|(best, ..)| g < best)
            {
                let at = a + ab * along;
                let y = pair[0].y + (pair[1].y - pair[0].y) * along as f32;
                let sine = (cross / length).abs().min(1.0);
                let point = Vector3::new(at.x as f32, y, at.y as f32);
                hit = Some((g, point, sine, station + along * length));
            }
        }
        station += length;
    }
    let (_, target, sine, at) = hit?;
    let least = f64::from(super::PREVIEW_MIN_ENDPOINT_SEGMENT_M);
    if sine < ARM_BOUND_DEG.to_radians().sin() || at < least || station - at < least {
        return None;
    }
    clear(graph, road, &[(pos, target)]).then_some(Growth { road, node, target })
}

// The road's outward unit direction at road end `node` of `road` (horizontal) when the end may
// grow (module docs): a plain node, no authored crossing, no vehicle turn rule, one live road, a
// ground car road off any pocket, ring or structure, no occupied parcel fronting it within
// `reach_m` of the end. `None` otherwise. `O(degree + points)` plus one frontage probe.
fn grows(
    graph: &RegionGraph,
    frontages: &dyn RoadFrontages,
    node: u32,
    road: usize,
    reach_m: f32,
) -> Option<DVec2> {
    let record = graph.node(node);
    let edge = graph.edge(road);
    let live = graph
        .node_adjacency(node)
        .iter()
        .filter(|&&id| !graph.edge(id).deleted)
        .count();
    if record.node_type != NodeType::Junction
        || !record.crosswalk_overrides.is_empty()
        || roadbed::has_vehicle_turn_rules(graph, node)
        || live != 1
        || edge.deleted
        || !meets_on_the_ground(edge)
        || edge.primary_type != TransitType::Road
        || edge.pocket.is_some()
    {
        return None;
    }
    let at_end = graph.get_valid_node(edge.end_node) == node;
    if !at_end && graph.get_valid_node(edge.start_node) != node {
        return None;
    }
    let length = edge.physical_length;
    let stations = if at_end {
        [(length - reach_m).max(0.0), length]
    } else {
        [0.0, reach_m.min(length)]
    };
    if frontages.fronted(graph, road, stations) {
        return None;
    }
    // Into the road from its end, turned outward.
    let inward = if at_end {
        leaving(edge.geometry.iter().rev())
    } else {
        leaving(edge.geometry.iter())
    }?;
    Some(-inward)
}

// Whether no road other than `road` lies within the topology's end join (2 m) of any of the
// horizontal `segments`: the growth and the lead-in meet only the roads they are for.
// `O(log E + candidates x points)`.
fn clear(graph: &RegionGraph, road: usize, segments: &[(Vector3, Vector3)]) -> bool {
    let tolerance = config::INTERSECTION_TOLERANCE;
    let mut min = Vector3::new(f32::MAX, 0.0, f32::MAX);
    let mut max = Vector3::new(f32::MIN, 0.0, f32::MIN);
    for &(a, b) in segments {
        for p in [a, b] {
            min = Vector3::new(min.x.min(p.x), 0.0, min.z.min(p.z));
            max = Vector3::new(max.x.max(p.x), 0.0, max.z.max(p.z));
        }
    }
    let pad = Vector3::new(tolerance, 0.0, tolerance);
    let mut clear = true;
    graph.visit_edges_near_aabb(min - pad, max + pad, |id| {
        let edge = graph.edge(id);
        if !clear || id == road || edge.deleted {
            return;
        }
        clear = !edge.geometry.windows(2).any(|pair| {
            segments.iter().any(|&segment| {
                segment_distance(segment, (pair[0], pair[1])) < f64::from(tolerance)
            })
        });
    });
    clear
}

// The horizontal distance between two segments.
fn segment_distance((p, q): (Vector3, Vector3), (a, b): (Vector3, Vector3)) -> f64 {
    let (p, q, a, b) = (flat(p), flat(q), flat(a), flat(b));
    let side = |o: DVec2, s: DVec2, t: DVec2| (s - o).perp_dot(t - o);
    let (d1, d2, d3, d4) = (side(a, b, p), side(a, b, q), side(p, q, a), side(p, q, b));
    if d1 * d2 <= 0.0 && d3 * d4 <= 0.0 && !(d1 == 0.0 && d2 == 0.0) {
        return 0.0;
    }
    point_segment(p, (a, b))
        .min(point_segment(q, (a, b)))
        .min(point_segment(a, (p, q)))
        .min(point_segment(b, (p, q)))
}

fn point_segment(point: DVec2, (a, b): (DVec2, DVec2)) -> f64 {
    let ab = b - a;
    let length = ab.length_squared();
    let t = if length > 0.0 {
        ((point - a).dot(ab) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(a + ab * t)
}

// The horizontal distance from `point` to the polyline `points`. `O(points)`.
fn nearest_distance(points: &[Vector3], point: Vector3) -> Option<f64> {
    let point = flat(point);
    points
        .windows(2)
        .map(|pair| point_segment(point, (flat(pair[0]), flat(pair[1]))))
        .min_by(f64::total_cmp)
}

fn flat(point: Vector3) -> DVec2 {
    DVec2::new(f64::from(point.x), f64::from(point.z))
}
