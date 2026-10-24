// SPDX-License-Identifier: GPL-2.0-only

//! Authoritative pedestrian junction routes and rendered-crossing geometry.
//!
//! At a node with a junction layout, crossings and sidewalk corner routes come from the layout
//! (`roadbed::NodeWalkways`): a crossing on each crossable arm, square to the road on its crossing line,
//! and a corner route along the sidewalk around every sector between two arms with sidewalks. A
//! footpath ends at its layout mouth and joins the walk of the sector it lies in where its
//! centreline meets it (D5); footpaths meeting only footpaths, or roads without sidewalks, meet
//! at the node.
//! A pass-through node's crossing (one by default, D8) lies on its arm's span and comes from the
//! span's section there, through the same walkways. At nodes without a layout (pass-through
//! pairs, footpath nodes, structure ends and failed layouts) neighbouring sidewalks join by a
//! straight walk, or by an arc about the node where that walk would cut across an arm's asphalt;
//! only pass-through nodes have crossings there.

use super::super::graph::RegionGraph;
use super::super::roadbed::{ArmKey, EdgeEnd, FootpathJoin, NodeWalkways, RoadVec3};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::{build_cum_dist, road_half_width};
use super::{CrosswalkMarking, Lane, LaneCrosswalk, LaneSuccessors, LaneType};
use crate::config;
use godot::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::f32::consts::{PI, TAU};

const CONNECTION_POINT_EPSILON_M: f32 = 0.001;
const CORNER_ARC_MAX_STEP_RAD: f32 = PI / 12.0;
// Footpaths that meet only other footpaths stop this far short of the node, so the walk
// between them turns through a short connector rather than a zero-length one.
const FOOTPATH_NODE_INSET_M: f32 = 1.0;
// sin(10 degrees): a footpath closer than this to a road arm's direction runs alongside the road,
// and its exit from the paved area is capped as if it left at that angle.
const FOOTPATH_MIN_EXIT_SIN: f32 = 0.17;
// Length a footpath keeps between its two trimmed ends.
const FOOTPATH_MIN_KEPT_M: f32 = 0.5;

/// A classification of a sidewalk-end at a junction, used for sorting.
pub struct SidewalkMouth {
    /// Index of the road edge this mouth belongs to.
    pub edge_idx: usize,
    /// Lane index of the sidewalk (100 or -100).
    pub lane_idx: i8,
    /// Road-arm angle in radians relative to the junction center.
    pub angle: f32,
    /// Planar unit direction pointing from the junction node into the road body.
    /// Used to classify the sidewalk side of the road arm.
    pub dir: Vector3,
    /// The arm this mouth belongs to: its edge and the edge end at this node.
    pub arm: ArmKey,
    /// Whether the sidewalk lies on the positive lateral side of its edge's forward frame.
    pub positive_lateral: bool,
    /// ID of the inbound lane relative to the junction.
    pub in_id: usize,
    /// ID of the outbound lane relative to the junction.
    pub out_id: usize,
    /// 3-D world position where the sidewalk lanes end at this node, on the sidewalk
    /// centreline; every pedestrian connector of the mouth starts or ends exactly here.
    pub mouth_world_pos: Vector3,
    /// Asphalt half-width; a corner walk at a node without a layout must stay outside it.
    pub road_half_width_m: f32,
    /// The arm's centreline from the node out to just past this mouth: a corner route that
    /// comes within `road_half_width_m` of it is on the arm's asphalt. Empty for footpaths.
    pub arm_centreline: Vec<Vector3>,
}

struct RoadArm {
    edge_idx: usize,
    angle: f32,
    clockwise_mouth: usize,
    counter_clockwise_mouth: usize,
}

struct PedestrianStep {
    start_mouth: usize,
    end_mouth: usize,
    geometry: Vec<Vector3>,
    length: f32,
    crosswalk: Option<LaneCrosswalk>,
}

fn push_distinct_point(points: &mut Vec<Vector3>, point: Vector3) {
    if points
        .last()
        .is_none_or(|last| last.distance_squared_to(point) > CONNECTION_POINT_EPSILON_M.powi(2))
    {
        points.push(point);
    }
}

// Ends a route exactly on `point`, a lane's end: it replaces a last point within the connection
// epsilon instead of being dropped as a repeat of it.
fn push_end_point(points: &mut Vec<Vector3>, point: Vector3) {
    if points
        .last()
        .is_some_and(|last| last.distance_squared_to(point) <= CONNECTION_POINT_EPSILON_M.powi(2))
        && points.len() > 1
    {
        points.pop();
    }
    points.push(point);
}

fn planar_distance(a: Vector3, b: Vector3) -> f32 {
    Vector2::new(b.x - a.x, b.z - a.z).length()
}

fn polyline_length(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum()
}

fn crosswalk_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    marking: CrosswalkMarking,
) -> Vec<Vector3> {
    let mut route = Vec::with_capacity(4);
    for point in [
        start.mouth_world_pos,
        marking.start,
        marking.end,
        end.mouth_world_pos,
    ] {
        push_distinct_point(&mut route, point);
    }
    route
}

fn distance_to_segment_xz(point: Vector3, start: Vector3, end: Vector3) -> f32 {
    let point = Vector2::new(point.x, point.z);
    let start = Vector2::new(start.x, start.z);
    let end = Vector2::new(end.x, end.z);
    let segment = end - start;
    let length_squared = segment.length_squared();
    if length_squared <= f32::EPSILON {
        return point.distance_to(start);
    }
    let t = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    point.distance_to(start + segment * t)
}

fn shortest_signed_angle_delta(start: f32, end: f32) -> f32 {
    (end - start + PI).rem_euclid(TAU) - PI
}

fn safe_node_centered_corner_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    node_pos: Vector3,
) -> Vec<Vector3> {
    let start_offset = start.mouth_world_pos - node_pos;
    let end_offset = end.mouth_world_pos - node_pos;
    let start_radius = Vector2::new(start_offset.x, start_offset.z).length();
    let end_radius = Vector2::new(end_offset.x, end_offset.z).length();
    if start_radius <= CONNECTION_POINT_EPSILON_M || end_radius <= CONNECTION_POINT_EPSILON_M {
        return Vec::new();
    }

    let start_angle = start_offset.z.atan2(start_offset.x);
    let end_angle = end_offset.z.atan2(end_offset.x);
    let angle_delta = shortest_signed_angle_delta(start_angle, end_angle);
    let segment_count = (angle_delta.abs() / CORNER_ARC_MAX_STEP_RAD)
        .ceil()
        .max(1.0) as usize;
    let segment_angle = angle_delta.abs() / segment_count as f32;
    let minimum_radius = start_radius
        .max(end_radius)
        .max(start.road_half_width_m.max(end.road_half_width_m) + config::SIDEWALK_WIDTH * 0.5);
    let arc_radius =
        (minimum_radius + CONNECTION_POINT_EPSILON_M) / (segment_angle * 0.5).cos().max(0.25);

    let mut route = Vec::with_capacity(segment_count + 3);
    push_distinct_point(&mut route, start.mouth_world_pos);
    for index in 0..=segment_count {
        let t = index as f32 / segment_count as f32;
        let angle = start_angle + angle_delta * t;
        push_distinct_point(
            &mut route,
            Vector3::new(
                node_pos.x + angle.cos() * arc_radius,
                start.mouth_world_pos.y.lerp(end.mouth_world_pos.y, t),
                node_pos.z + angle.sin() * arc_radius,
            ),
        );
    }
    push_distinct_point(&mut route, end.mouth_world_pos);
    route
}

// The walk between two neighbouring sidewalk mouths at a node without a junction layout:
// straight from one sidewalk end to the other, or round the node where that line would run onto
// either arm's asphalt (`TRAFFIC-04`). `ROAD-68-F` gives footpath nodes layouts of their own.
fn node_corner_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    node_pos: Vector3,
) -> Vec<Vector3> {
    let mut route = Vec::with_capacity(2);
    push_distinct_point(&mut route, start.mouth_world_pos);
    push_distinct_point(&mut route, end.mouth_world_pos);
    if [start, end]
        .iter()
        .any(|arm| route_enters_arm_asphalt(&route, arm))
    {
        safe_node_centered_corner_route(start, end, node_pos)
    } else {
        route
    }
}

// A layout corner route between the sidewalk ends of `start` and `end`: the lanes end exactly
// on the route's end points in plan, so the connector starts and ends on the lanes' own points.
// Their heights may already be lifted onto the roadbed, which the layout's are not.
fn layout_corner_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    points: &[RoadVec3],
) -> Vec<Vector3> {
    let point = |p: RoadVec3| Vector3::new(p.x as f32, p.y as f32, p.z as f32);
    let same_xz = |p: RoadVec3, q: Vector3| p.x as f32 == q.x && p.z as f32 == q.z;
    debug_assert!(
        points
            .first()
            .is_some_and(|&p| same_xz(p, start.mouth_world_pos))
            && points
                .last()
                .is_some_and(|&p| same_xz(p, end.mouth_world_pos)),
        "sidewalk lanes must end on their node's layout corner routes"
    );
    let mut route = Vec::with_capacity(points.len());
    push_distinct_point(&mut route, start.mouth_world_pos);
    for &p in points.iter().skip(1).take(points.len().saturating_sub(2)) {
        push_distinct_point(&mut route, point(p));
    }
    push_end_point(&mut route, end.mouth_world_pos);
    route
}

// O(route segments × centreline segments): both are short near a node.
fn route_enters_arm_asphalt(route: &[Vector3], arm: &SidewalkMouth) -> bool {
    route.windows(2).any(|segment| {
        arm.arm_centreline.windows(2).any(|centreline| {
            segment_distance_xz(segment[0], segment[1], centreline[0], centreline[1])
                + CONNECTION_POINT_EPSILON_M
                < arm.road_half_width_m
        })
    })
}

fn segment_distance_xz(a0: Vector3, a1: Vector3, b0: Vector3, b1: Vector3) -> f32 {
    let side = |p: Vector3, q0: Vector3, q1: Vector3| {
        (q1.x - q0.x) * (p.z - q0.z) - (q1.z - q0.z) * (p.x - q0.x)
    };
    let crosses =
        side(a0, b0, b1) * side(a1, b0, b1) < 0.0 && side(b0, a0, a1) * side(b1, a0, a1) < 0.0;
    if crosses {
        return 0.0;
    }
    distance_to_segment_xz(a0, b0, b1)
        .min(distance_to_segment_xz(a1, b0, b1))
        .min(distance_to_segment_xz(b0, a0, a1))
        .min(distance_to_segment_xz(b1, a0, a1))
}

// Unit planar direction from an edge's end at the node into the edge.
fn arm_direction_xz(points: &[Vector3], at_start: bool) -> Option<Vector2> {
    let n = points.len();
    let at = |i: usize| points[if at_start { i } else { n - 1 - i }];
    let origin = if n > 0 { at(0) } else { return None };
    (1..n).find_map(|i| {
        let point = at(i);
        let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
        (delta.length() > CONNECTION_POINT_EPSILON_M).then(|| delta.normalized())
    })
}

/// Metres to trim from a footpath's start and end so its lanes stop where the path leaves the
/// paved area of the roads at that node: the outer edge of their sidewalks.
///
/// A ray from the node at angle `d` to a road arm leaves that arm's paved strip of half-width
/// `w` after `w / |sin d|` when it runs along the arm, and after `w` when it points away. A
/// footpath meeting only footpaths stops [`FOOTPATH_NODE_INSET_M`] short. O(node degree).
pub(super) fn footpath_end_insets(graph: &RegionGraph, edge_idx: usize) -> (f32, f32) {
    let edge = graph.edge(edge_idx);
    let inset_at = |node: u32, at_start: bool| -> f32 {
        let Some(path_dir) = arm_direction_xz(&edge.physical_geometry, at_start) else {
            return 0.0;
        };
        let mut paths = 0;
        let mut roads = 0;
        let mut inset = 0.0_f32;
        for &other_idx in graph.node_adjacency(node) {
            let other = graph.edge(other_idx);
            if other_idx == edge_idx || other.deleted || other.physical_geometry.len() < 2 {
                continue;
            }
            match other.primary_type {
                TransitType::Foot => paths += 1,
                TransitType::Road => {
                    let other_at_start = graph.get_valid_node(other.start_node) == node;
                    let Some(road_dir) = arm_direction_xz(&other.physical_geometry, other_at_start)
                    else {
                        continue;
                    };
                    roads += 1;
                    let sidewalk = if other.allowed_types & TransitFlags::FOOT != 0 {
                        config::SIDEWALK_WIDTH
                    } else {
                        0.0
                    };
                    let half_width = road_half_width(other) + sidewalk;
                    let sin = (path_dir.x * road_dir.y - path_dir.y * road_dir.x).abs();
                    inset = inset.max(if path_dir.dot(road_dir) > 0.0 {
                        half_width / sin.max(FOOTPATH_MIN_EXIT_SIN)
                    } else {
                        half_width
                    });
                }
                _ => {}
            }
        }
        if roads == 0 && paths > 0 {
            FOOTPATH_NODE_INSET_M
        } else {
            inset
        }
    };
    let start = inset_at(graph.get_valid_node(edge.start_node), true);
    let end = inset_at(graph.get_valid_node(edge.end_node), false);
    // Keep some of a short path rather than trimming it away.
    let available = (edge.physical_length - FOOTPATH_MIN_KEPT_M).max(0.0);
    if start + end > available && start + end > 0.0 {
        let scale = available / (start + end);
        (start * scale, end * scale)
    } else {
        (start, end)
    }
}

// Steps between footpath mouth `path` and the walkways it meets: the sidewalk corner on each side
// of it between two road arms, the two sidewalks of a single road arm, or, where only footpaths
// meet, every later footpath through the node. At a layout node the corner is the layout's corner
// route of the footpath's sector, which at a road end wraps the cap to both sidewalks.
#[allow(clippy::too_many_arguments)]
fn append_footpath_steps(
    steps: &mut Vec<PedestrianStep>,
    outgoing_steps: &mut [Vec<usize>],
    mouths: &[SidewalkMouth],
    arms: &[RoadArm],
    path: usize,
    road_mouth_count: usize,
    node_pos: Vector3,
    walkways: Option<&NodeWalkways>,
) {
    let mut both_ways = |steps: &mut Vec<PedestrianStep>, target: usize, route: Vec<Vector3>| {
        let reversed = route.iter().rev().copied().collect();
        append_pedestrian_step(steps, outgoing_steps, path, target, route, None);
        append_pedestrian_step(steps, outgoing_steps, target, path, reversed, None);
    };
    let from = mouths[path].mouth_world_pos;
    let layout = walkways.filter(|walkways| walkways.has_layout());
    if let Some(join) = layout.and_then(|walkways| walkways.footpath_join(mouths[path].arm)) {
        let mouth_at = |arm: ArmKey, left: bool| {
            mouths[..road_mouth_count].iter().position(|mouth| {
                mouth.arm == arm && (mouth.positive_lateral == (arm.end == EdgeEnd::Start)) == left
            })
        };
        let corner = &join.corner;
        if let (Some(start), Some(end)) = (mouth_at(corner.from, true), mouth_at(corner.to, false))
        {
            let [to_start, to_end] = layout_join_routes(mouths, from, (start, end), &join);
            both_ways(steps, start, to_start);
            both_ways(steps, end, to_end);
        }
        return;
    }
    match arms {
        _ if layout.is_some() && !arms.is_empty() => {}
        [] => {
            for (other, mouth) in mouths.iter().enumerate().skip(path + 1) {
                debug_assert!(other >= road_mouth_count);
                let to = mouth.mouth_world_pos;
                let through = Vector3::new(node_pos.x, 0.5 * (from.y + to.y), node_pos.z);
                let mut route = Vec::with_capacity(3);
                for point in [from, through, to] {
                    push_distinct_point(&mut route, point);
                }
                both_ways(steps, other, route);
            }
        }
        [arm] => {
            for target in [arm.clockwise_mouth, arm.counter_clockwise_mouth] {
                both_ways(steps, target, vec![from, mouths[target].mouth_world_pos]);
            }
        }
        _ => {
            // The corner from arm i's counter-clockwise sidewalk to arm i+1's clockwise one spans
            // the angles between the two arms, as in the corner loop above.
            let angle = mouths[path].angle;
            let sector = (0..arms.len()).find(|&i| {
                let next = &arms[(i + 1) % arms.len()];
                let span = (next.angle - arms[i].angle).rem_euclid(TAU);
                (angle - arms[i].angle).rem_euclid(TAU) < span.max(f32::EPSILON)
            });
            let Some(i) =
                sector.filter(|&i| arms[i].edge_idx != arms[(i + 1) % arms.len()].edge_idx)
            else {
                return;
            };
            let (start, end) = (
                arms[i].counter_clockwise_mouth,
                arms[(i + 1) % arms.len()].clockwise_mouth,
            );
            let corner = node_corner_route(&mouths[start], &mouths[end], node_pos);
            join_corner(&mut both_ways, steps, from, (start, end), &corner);
        }
    }
}

// A footpath's two routes from its end `from` in to `joint`, on segment `segment` of a walk
// (`point(i)` for `i < len`), then along the walk to walk points `ends` (the two sidewalk mouths),
// which the lanes end on exactly: their own positions `targets`.
fn join_routes(
    from: Vector3,
    (segment, joint): (usize, Vector3),
    point: impl Fn(usize) -> Vector3,
    ends: [usize; 2],
    targets: [Vector3; 2],
) -> [Vec<Vector3>; 2] {
    [0, 1].map(|side| {
        let (end, target) = (ends[side], targets[side]);
        let mut route = Vec::with_capacity(segment.abs_diff(end) + 3);
        push_distinct_point(&mut route, from);
        push_distinct_point(&mut route, joint);
        // Walk points from the joint's segment to `end`, which the lane's own point replaces.
        let mut walk = |index: usize| push_distinct_point(&mut route, point(index));
        if end <= segment {
            (end + 1..=segment).rev().for_each(&mut walk);
        } else {
            (segment + 1..end).for_each(&mut walk);
        }
        push_end_point(&mut route, target);
        route
    })
}

// An attached footpath's two routes from its end `from` (D5): in along its centreline to where it
// meets its sector's walk (`join`), then along the walk to each of its two sidewalk mouths.
fn layout_join_routes(
    mouths: &[SidewalkMouth],
    from: Vector3,
    (start, end): (usize, usize),
    join: &FootpathJoin<'_>,
) -> [Vec<Vector3>; 2] {
    let point = |p: RoadVec3| Vector3::new(p.x as f32, p.y as f32, p.z as f32);
    join_routes(
        from,
        (join.segment, point(join.point)),
        |index| point(join.walk[index]),
        [join.route.start, join.route.end - 1],
        [start, end].map(|mouth| mouths[mouth].mouth_world_pos),
    )
}

// Joins a footpath end `from` to a corner route at its nearest point, and follows the route both
// ways to its two sidewalk mouths.
fn join_corner(
    both_ways: &mut impl FnMut(&mut Vec<PedestrianStep>, usize, Vec<Vector3>),
    steps: &mut Vec<PedestrianStep>,
    from: Vector3,
    (start, end): (usize, usize),
    corner: &[Vector3],
) {
    let Some(joint) = nearest_point_on_polyline_xz(corner, from) else {
        return;
    };
    let last = corner.len() - 1;
    let [to_start, to_end] = join_routes(
        from,
        joint,
        |index| corner[index],
        [0, last],
        [corner[0], corner[last]],
    );
    both_ways(steps, start, to_start);
    both_ways(steps, end, to_end);
}

// The segment index and planar-nearest point of `polyline` to `point`, with the height
// interpolated along that segment.
fn nearest_point_on_polyline_xz(polyline: &[Vector3], point: Vector3) -> Option<(usize, Vector3)> {
    let target = Vector2::new(point.x, point.z);
    polyline
        .windows(2)
        .enumerate()
        .map(|(index, segment)| {
            let start = Vector2::new(segment[0].x, segment[0].z);
            let delta = Vector2::new(segment[1].x, segment[1].z) - start;
            let t = if delta.length_squared() > f32::EPSILON {
                ((target - start).dot(delta) / delta.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let nearest = segment[0].lerp(segment[1], t);
            (
                index,
                nearest,
                Vector2::new(nearest.x, nearest.z).distance_to(target),
            )
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
        .map(|(index, nearest, _)| (index, nearest))
}

#[allow(clippy::too_many_arguments)]
fn append_pedestrian_connection(
    lanes: &mut Vec<Lane>,
    graph: &mut RegionGraph,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
    crosswalks: &mut HashMap<usize, LaneCrosswalk>,
    node_id: usize,
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    geometry: Vec<Vector3>,
    crosswalk: Option<LaneCrosswalk>,
) {
    if geometry.len() < 2 {
        return;
    }
    let length = polyline_length(&geometry);
    if length <= CONNECTION_POINT_EPSILON_M {
        return;
    }

    let connection_id = lanes.len();
    lanes.push(Lane {
        edge_id: usize::MAX,
        is_fwd: true,
        lane_idx: 0,
        cum_dist: build_cum_dist(&geometry),
        geometry,
        length,
        frontage_delay_penalty_s: 0.0,
        live_frontage_delay_s: 0.0,
        lane_type: LaneType::Foot,
        next_lanes: LaneSuccessors::one(end.out_id),
        node_id,
    });
    if let Some(crosswalk) = crosswalk {
        crosswalks.insert(connection_id, crosswalk);
    }
    node_lanes.entry(node_id).or_default().push(connection_id);
    lanes[start.in_id].next_lanes.push(connection_id);
    graph.nodes[node_id]
        .lane_connections
        .entry((start.edge_idx, start.lane_idx))
        .or_default()
        .push((end.edge_idx, end.lane_idx));
}

fn append_pedestrian_step(
    steps: &mut Vec<PedestrianStep>,
    outgoing_steps: &mut [Vec<usize>],
    start_mouth: usize,
    end_mouth: usize,
    geometry: Vec<Vector3>,
    crosswalk: Option<LaneCrosswalk>,
) {
    if geometry.len() < 2 {
        return;
    }
    let length = polyline_length(&geometry);
    if length <= CONNECTION_POINT_EPSILON_M {
        return;
    }

    let step_id = steps.len();
    steps.push(PedestrianStep {
        start_mouth,
        end_mouth,
        geometry,
        length,
        crosswalk,
    });
    outgoing_steps[start_mouth].push(step_id);
}

fn pedestrian_route_tree(
    source_mouth: usize,
    steps: &[PedestrianStep],
    outgoing_steps: &[Vec<usize>],
) -> (Vec<f32>, Vec<usize>) {
    let mouth_count = outgoing_steps.len();
    let mut distance = vec![f32::INFINITY; mouth_count];
    let mut previous_step = vec![usize::MAX; mouth_count];
    let mut visited = vec![false; mouth_count];
    distance[source_mouth] = 0.0;

    for _ in 0..mouth_count {
        let Some(current) = (0..mouth_count)
            .filter(|&mouth| !visited[mouth] && distance[mouth].is_finite())
            .min_by(|&a, &b| distance[a].total_cmp(&distance[b]).then_with(|| a.cmp(&b)))
        else {
            break;
        };
        visited[current] = true;

        for &step_id in &outgoing_steps[current] {
            let step = &steps[step_id];
            let candidate = distance[current] + step.length;
            let old = distance[step.end_mouth];
            if candidate + CONNECTION_POINT_EPSILON_M < old
                || ((candidate - old).abs() <= CONNECTION_POINT_EPSILON_M
                    && step_id < previous_step[step.end_mouth])
            {
                distance[step.end_mouth] = candidate;
                previous_step[step.end_mouth] = step_id;
            }
        }
    }

    (distance, previous_step)
}

fn pedestrian_route_to_mouth(
    source_mouth: usize,
    target_mouth: usize,
    steps: &[PedestrianStep],
    distance: &[f32],
    previous_step: &[usize],
) -> Option<Vec<usize>> {
    if target_mouth == source_mouth || !distance[target_mouth].is_finite() {
        return None;
    }

    let mut route = Vec::new();
    let mut current = target_mouth;
    while current != source_mouth {
        let step_id = previous_step[current];
        if step_id == usize::MAX {
            return None;
        }
        route.push(step_id);
        current = steps[step_id].start_mouth;
    }
    route.reverse();
    Some(route)
}

fn pedestrian_route_geometry(route: &[usize], steps: &[PedestrianStep]) -> Vec<Vector3> {
    let point_capacity = route
        .iter()
        .map(|&step_id| steps[step_id].geometry.len())
        .sum();
    let mut geometry = Vec::with_capacity(point_capacity);
    for &step_id in route {
        for &point in &steps[step_id].geometry {
            push_distinct_point(&mut geometry, point);
        }
    }
    geometry
}

fn append_stationary_pedestrian_connection(
    lanes: &mut Vec<Lane>,
    graph: &mut RegionGraph,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
    node_id: usize,
    mouth: &SidewalkMouth,
) {
    let point = mouth.mouth_world_pos;
    let connection_id = lanes.len();
    lanes.push(Lane {
        edge_id: usize::MAX,
        is_fwd: true,
        lane_idx: 0,
        geometry: vec![point, point],
        length: 0.0,
        frontage_delay_penalty_s: 0.0,
        live_frontage_delay_s: 0.0,
        cum_dist: vec![0.0, 0.0],
        lane_type: LaneType::Foot,
        next_lanes: LaneSuccessors::one(mouth.out_id),
        node_id,
    });
    node_lanes.entry(node_id).or_default().push(connection_id);
    lanes[mouth.in_id].next_lanes.push(connection_id);
    graph.nodes[node_id]
        .lane_connections
        .entry((mouth.edge_idx, mouth.lane_idx))
        .or_default()
        .push((mouth.edge_idx, mouth.lane_idx));
}

// Crossings and corner routes of a node's walkways, both directions of each: a layout's, or a
// pass-through node's span-owned crossings, which come without corner routes. A mouth is found
// by its arm and layout side; a side without sidewalk lanes has neither. O(degree * mouths +
// route points).
fn append_layout_steps(
    steps: &mut Vec<PedestrianStep>,
    outgoing_steps: &mut [Vec<usize>],
    mouths: &[SidewalkMouth],
    walkways: &NodeWalkways,
) {
    // The layout's left is the positive lateral side at an edge's start, the negative at its end.
    let mouth_at = |arm: ArmKey, left: bool| {
        mouths.iter().position(|mouth| {
            mouth.arm == arm && (mouth.positive_lateral == (arm.end == EdgeEnd::Start)) == left
        })
    };
    let mut both_ways = |steps: &mut Vec<PedestrianStep>,
                         (start, end): (usize, usize),
                         route: Vec<Vector3>,
                         crosswalk: Option<(usize, CrosswalkMarking)>| {
        let reversed = route.iter().rev().copied().collect();
        let crossing = |marking: bool| {
            crosswalk.map(|(edge_id, owned)| LaneCrosswalk {
                edge_id,
                marking: marking.then_some(owned),
            })
        };
        append_pedestrian_step(steps, outgoing_steps, start, end, route, crossing(true));
        append_pedestrian_step(steps, outgoing_steps, end, start, reversed, crossing(false));
    };
    for (arm, [right_kerb, left_kerb]) in walkways.crossings() {
        let (Some(right), Some(left)) = (mouth_at(arm, false), mouth_at(arm, true)) else {
            continue;
        };
        // The direction from the lower lane index owns the marking, as it always has.
        let (start, end, start_kerb, end_kerb) = if mouths[right].lane_idx <= mouths[left].lane_idx
        {
            (right, left, right_kerb, left_kerb)
        } else {
            (left, right, left_kerb, right_kerb)
        };
        let marking = CrosswalkMarking {
            edge_id: arm.edge,
            start: start_kerb,
            end: end_kerb,
        };
        let route = crosswalk_route(&mouths[start], &mouths[end], marking);
        both_ways(steps, (start, end), route, Some((arm.edge, marking)));
    }
    for corner in walkways.corners() {
        let (Some(start), Some(end)) = (mouth_at(corner.from, true), mouth_at(corner.to, false))
        else {
            continue;
        };
        let route = layout_corner_route(&mouths[start], &mouths[end], corner.points);
        both_ways(steps, (start, end), route, None);
    }
}

// Corner walks between neighbouring road arms at a node without a layout; no crossings.
fn append_node_corner_steps(
    steps: &mut Vec<PedestrianStep>,
    outgoing_steps: &mut [Vec<usize>],
    mouths: &[SidewalkMouth],
    arms: &[RoadArm],
    node_pos: Vector3,
) {
    for i in 0..arms.len() {
        let j = (i + 1) % arms.len();
        if arms[i].edge_idx == arms[j].edge_idx {
            continue;
        }
        let start_index = arms[i].counter_clockwise_mouth;
        let end_index = arms[j].clockwise_mouth;
        let route = node_corner_route(&mouths[start_index], &mouths[end_index], node_pos);
        let reversed = route.iter().rev().copied().collect();
        append_pedestrian_step(steps, outgoing_steps, start_index, end_index, route, None);
        append_pedestrian_step(
            steps,
            outgoing_steps,
            end_index,
            start_index,
            reversed,
            None,
        );
    }
}

/// Builds pedestrian sidewalk connection lanes (crosswalks) at a single node.
///
/// `walkways` is the node's walkways (its layout's, or a pass-through node's crossings), `None`
/// at another node without a layout; the node's sidewalk lanes must have been built against the
/// same walkways.
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_pedestrian_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &mut RegionGraph,
    node_id: usize,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
    crosswalks: &mut HashMap<usize, LaneCrosswalk>,
    walkways: Option<&NodeWalkways>,
) {
    let node_pos = graph.node(node_id as u32).pos;
    let adj: Vec<usize> = graph.node_adjacency(node_id as u32).to_vec();
    let mut mouths: Vec<SidewalkMouth> = Vec::new();

    for &e_idx in &adj {
        let edge = graph.edge(e_idx);
        if edge.deleted
            || (edge.allowed_types & TransitFlags::FOOT) == 0
            || edge.primary_type != TransitType::Road
            || edge.physical_geometry.len() < 2
        {
            continue;
        }

        let is_start = edge.start_node as usize == node_id;
        let geometry = &edge.physical_geometry;

        // Direction from node into the road body (used for angular sorting and offset).
        let other_p = if is_start {
            geometry[1]
        } else {
            geometry[geometry.len() - 2]
        };
        let diff = other_p - node_pos;
        let planar_length = Vector2::new(diff.x, diff.z).length();
        let dir = if planar_length > 1e-4 {
            Vector3::new(diff.x / planar_length, 0.0, diff.z / planar_length)
        } else {
            Vector3::ZERO
        };
        let asphalt_half_width = road_half_width(edge);
        // The centreline walked out from the node.
        let arm_points: Vec<Vector3> = if is_start {
            geometry.clone()
        } else {
            geometry.iter().rev().copied().collect()
        };
        let arm = ArmKey {
            edge: e_idx,
            end: if is_start {
                EdgeEnd::Start
            } else {
                EdgeEnd::End
            },
        };

        for &l_idx in &[-100_i8, 100_i8] {
            let arm_angle = dir.z.atan2(dir.x);
            let (inbound, outbound) = if is_start {
                (
                    lane_map.get(&(e_idx, false, l_idx)).copied(),
                    lane_map.get(&(e_idx, true, l_idx)).copied(),
                )
            } else {
                (
                    lane_map.get(&(e_idx, true, l_idx)).copied(),
                    lane_map.get(&(e_idx, false, l_idx)).copied(),
                )
            };
            if let (Some(in_id), Some(out_id)) = (inbound, outbound) {
                let Some(&mouth_world_pos) = lanes[in_id].geometry.last() else {
                    continue;
                };
                debug_assert!(
                    lanes[out_id]
                        .geometry
                        .first()
                        .is_some_and(|outbound_start| outbound_start
                            .distance_squared_to(mouth_world_pos)
                            <= CONNECTION_POINT_EPSILON_M.powi(2)),
                    "opposing sidewalk lanes must share one junction mouth"
                );
                let mouth_reach = planar_distance(node_pos, mouth_world_pos) + asphalt_half_width;
                let mut arm_centreline = Vec::new();
                for &point in &arm_points {
                    arm_centreline.push(point);
                    if planar_distance(node_pos, point) > mouth_reach {
                        break;
                    }
                }
                mouths.push(SidewalkMouth {
                    edge_idx: e_idx,
                    lane_idx: l_idx,
                    angle: arm_angle,
                    dir,
                    arm,
                    positive_lateral: super::rebuild::sidewalk_offset(edge, l_idx) > 0.0,
                    in_id,
                    out_id,
                    mouth_world_pos,
                    road_half_width_m: asphalt_half_width,
                    arm_centreline,
                });
            }
        }
    }

    // Footpaths join as one-mouth arms after the road mouths, so the road arms and their
    // crosswalks below stay exactly as they were.
    let road_mouth_count = mouths.len();
    for &e_idx in &adj {
        let edge = graph.edge(e_idx);
        if edge.deleted
            || (edge.allowed_types & TransitFlags::FOOT) == 0
            || edge.primary_type != TransitType::Foot
        {
            continue;
        }
        let is_start = edge.start_node as usize == node_id;
        let inbound = lane_map.get(&(e_idx, !is_start, 0)).copied();
        let outbound = lane_map.get(&(e_idx, is_start, 0)).copied();
        let (Some(in_id), Some(out_id)) = (inbound, outbound) else {
            continue;
        };
        let (Some(&mouth_world_pos), Some(dir)) = (
            lanes[in_id].geometry.last(),
            arm_direction_xz(&lanes[out_id].geometry, true),
        ) else {
            continue;
        };
        let dir = Vector3::new(dir.x, 0.0, dir.y);
        mouths.push(SidewalkMouth {
            edge_idx: e_idx,
            lane_idx: 0,
            angle: dir.z.atan2(dir.x),
            dir,
            arm: ArmKey {
                edge: e_idx,
                end: if is_start {
                    EdgeEnd::Start
                } else {
                    EdgeEnd::End
                },
            },
            positive_lateral: false,
            in_id,
            out_id,
            mouth_world_pos,
            road_half_width_m: 0.0,
            arm_centreline: Vec::new(),
        });
    }

    let num_mouths = mouths.len();
    let to_remove: Vec<_> = graph.nodes[node_id]
        .lane_connections
        .keys()
        .filter(|&&(edge_idx, lane_idx)| {
            let edge = graph.edge(edge_idx);
            lane_idx == 100
                || lane_idx == -100
                || edge.primary_type == crate::simulation::network::types::TransitType::Foot
        })
        .copied()
        .collect();

    for key in to_remove {
        graph.nodes[node_id].lane_connections.remove(&key);
    }
    // A footpath's dead end keeps its walkers' turn back: the stationary connector below.
    if num_mouths == 0 {
        return;
    }

    // Crosswalk ownership is exact: the two sidewalks must belong to the same road arm.
    // Direction similarity is insufficient because distinct parallel arms can share a node.
    let mut mouths_by_edge: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (mouth_index, mouth) in mouths.iter().take(road_mouth_count).enumerate() {
        mouths_by_edge
            .entry(mouth.edge_idx)
            .or_default()
            .push(mouth_index);
    }
    let mut arms = Vec::with_capacity(mouths_by_edge.len());
    for (&edge_idx, mouth_indices) in &mouths_by_edge {
        if mouth_indices.len() != 2 {
            continue;
        }
        let first = mouth_indices[0];
        let second = mouth_indices[1];
        let side_vec = Vector3::new(-mouths[first].dir.z, 0.0, mouths[first].dir.x);
        let first_lateral = (mouths[first].mouth_world_pos - node_pos).dot(side_vec);
        let second_lateral = (mouths[second].mouth_world_pos - node_pos).dot(side_vec);
        let (clockwise_mouth, counter_clockwise_mouth) = if first_lateral <= second_lateral {
            (first, second)
        } else {
            (second, first)
        };
        arms.push(RoadArm {
            edge_idx,
            angle: mouths[first].angle,
            clockwise_mouth,
            counter_clockwise_mouth,
        });
    }
    arms.sort_by(|a, b| {
        a.angle
            .total_cmp(&b.angle)
            .then_with(|| a.edge_idx.cmp(&b.edge_idx))
    });

    let road_degree = arms.len();
    let mut steps = Vec::with_capacity(road_degree * 4);
    let mut outgoing_steps = (0..mouths.len())
        .map(|_| Vec::with_capacity(2))
        .collect::<Vec<_>>();
    if let Some(walkways) = walkways {
        append_layout_steps(
            &mut steps,
            &mut outgoing_steps,
            &mouths[..road_mouth_count],
            walkways,
        );
    }
    // A pass-through node's walkways hold only its span-owned crossings.
    if walkways.is_none_or(|walkways| !walkways.has_layout()) {
        append_node_corner_steps(&mut steps, &mut outgoing_steps, &mouths, &arms, node_pos);
    }

    for path_mouth in road_mouth_count..mouths.len() {
        append_footpath_steps(
            &mut steps,
            &mut outgoing_steps,
            &mouths,
            &arms,
            path_mouth,
            road_mouth_count,
            node_pos,
            walkways,
        );
    }

    // Agent movement consumes one connector per road-graph junction. Materialize one
    // shortest legal route from every incoming sidewalk mouth to every reachable
    // outbound mouth, composing only the authoritative crosswalk and perimeter steps
    // above. Exact building access can require either sidewalk of a destination arm.
    for source_mouth in 0..mouths.len() {
        // Rebuild-only bounded Dijkstra: O(D²) per mouth and O(D³) for a node
        // with road degree D. Live movement only scans the O(D) materialized routes.
        let (distance, previous_step) =
            pedestrian_route_tree(source_mouth, &steps, &outgoing_steps);
        for target_mouth in 0..mouths.len() {
            let Some(route) = pedestrian_route_to_mouth(
                source_mouth,
                target_mouth,
                &steps,
                &distance,
                &previous_step,
            ) else {
                continue;
            };
            let Some(&last_step_id) = route.last() else {
                continue;
            };
            debug_assert_eq!(steps[last_step_id].end_mouth, target_mouth);
            let crosswalk = match route.as_slice() {
                &[step_id] => steps[step_id].crosswalk,
                _ => None,
            };
            append_pedestrian_connection(
                lanes,
                graph,
                node_lanes,
                crosswalks,
                node_id,
                &mouths[source_mouth],
                &mouths[target_mouth],
                pedestrian_route_geometry(&route, &steps),
                crosswalk,
            );
        }

        // Reversing along the same sidewalk requires no street crossing. Keep it as
        // an explicit zero-length connector so exact frontage plans do not clear the
        // lane and reattach farther along the opposite-direction sidewalk.
        append_stationary_pedestrian_connection(
            lanes,
            graph,
            node_lanes,
            node_id,
            &mouths[source_mouth],
        );
    }
}
