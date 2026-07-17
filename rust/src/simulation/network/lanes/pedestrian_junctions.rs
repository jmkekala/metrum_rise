//! Authoritative pedestrian junction routes and rendered-crossing geometry.

use super::super::graph::RegionGraph;
use super::super::types::{TransitFlags, TransitType};
use super::geometry::{build_cum_dist, road_half_width};
use super::{CrosswalkMarking, Lane, LaneType};
use crate::config;
use godot::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::f32::consts::{PI, TAU};

const CONNECTION_POINT_EPSILON_M: f32 = 0.001;
const CORNER_MITER_LIMIT_MULTIPLIER: f32 = 4.0;
const CORNER_ARC_MAX_STEP_RAD: f32 = PI / 12.0;

/// A classification of a sidewalk-end at a junction, used for sorting.
pub struct SidewalkMouth {
    /// Index of the road edge this mouth belongs to.
    pub edge_idx: usize,
    /// Lane index of the sidewalk (100 or -100).
    pub lane_idx: i8,
    /// Road-arm angle in radians relative to the junction center.
    pub angle: f32,
    /// Planar unit direction pointing from the junction node into the road body.
    /// Used to intersect adjacent sidewalk centerlines.
    pub dir: Vector3,
    /// ID of the inbound lane relative to the junction.
    pub in_id: usize,
    /// ID of the outbound lane relative to the junction.
    pub out_id: usize,
    /// 3-D world position at the road mouth offset to the sidewalk centreline.
    /// Used for agent routing (pedestrians walk along sidewalks).
    pub mouth_world_pos: Vector3,
    /// 3-D world position at the road mouth offset to the asphalt edge (no sidewalk).
    /// Used for crosswalk geometry so stripes stay on the car lanes only.
    pub road_edge_pos: Vector3,
    /// Asphalt half-width used to reject corner miters that enter the junction core.
    pub road_half_width_m: f32,
}

struct RoadArm {
    edge_idx: usize,
    angle: f32,
    clockwise_mouth: usize,
    counter_clockwise_mouth: usize,
}

/// Returns the 3-D point `dist` metres along `geom` measured from the start (`from_start=true`)
/// or from the end (`from_start=false`). Clamps to the endpoint if `dist` exceeds the length.
fn walk_geom_from_end(geom: &[Vector3], dist: f32, from_start: bool) -> Vector3 {
    if geom.is_empty() {
        return Vector3::ZERO;
    }
    if dist <= 0.0 {
        return if from_start {
            geom[0]
        } else {
            *geom.last().unwrap()
        };
    }
    let mut remaining = dist;
    if from_start {
        for i in 0..geom.len().saturating_sub(1) {
            let seg = geom[i].distance_to(geom[i + 1]);
            if remaining <= seg || i == geom.len() - 2 {
                let t = (remaining / seg).min(1.0);
                return geom[i].lerp(geom[i + 1], t);
            }
            remaining -= seg;
        }
        geom[0]
    } else {
        let n = geom.len();
        for i in (1..n).rev() {
            let seg = geom[i - 1].distance_to(geom[i]);
            if remaining <= seg || i == 1 {
                let t = (remaining / seg).min(1.0);
                return geom[i].lerp(geom[i - 1], t);
            }
            remaining -= seg;
        }
        *geom.last().unwrap()
    }
}

fn push_distinct_point(points: &mut Vec<Vector3>, point: Vector3) {
    if points
        .last()
        .is_none_or(|last| last.distance_squared_to(point) > CONNECTION_POINT_EPSILON_M.powi(2))
    {
        points.push(point);
    }
}

fn polyline_length(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .map(|pair| pair[0].distance_to(pair[1]))
        .sum()
}

fn crosswalk_route(start: &SidewalkMouth, end: &SidewalkMouth) -> Vec<Vector3> {
    let mut route = Vec::with_capacity(4);
    for point in [
        start.mouth_world_pos,
        start.road_edge_pos,
        end.road_edge_pos,
        end.mouth_world_pos,
    ] {
        push_distinct_point(&mut route, point);
    }
    route
}

fn xz_cross(a: Vector3, b: Vector3) -> f32 {
    a.x * b.z - a.z * b.x
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

fn sidewalk_centerline_intersection(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    node_pos: Vector3,
) -> Option<Vector3> {
    let denominator = xz_cross(start.dir, end.dir);
    if denominator.abs() <= 1.0e-4 {
        return None;
    }

    let delta = end.mouth_world_pos - start.mouth_world_pos;
    let start_t = xz_cross(delta, end.dir) / denominator;
    let mut intersection = start.mouth_world_pos + start.dir * start_t;
    intersection.y = (start.mouth_world_pos.y + end.mouth_world_pos.y) * 0.5;

    let start_leg = start.mouth_world_pos.distance_to(intersection);
    let end_leg = end.mouth_world_pos.distance_to(intersection);
    let local_scale = start
        .mouth_world_pos
        .distance_to(node_pos)
        .max(end.mouth_world_pos.distance_to(node_pos))
        .max(config::SIDEWALK_WIDTH);
    let minimum_radius =
        start.road_half_width_m.max(end.road_half_width_m) + config::SIDEWALK_WIDTH * 0.5;
    if !intersection.is_finite()
        || start_leg > local_scale * CORNER_MITER_LIMIT_MULTIPLIER
        || end_leg > local_scale * CORNER_MITER_LIMIT_MULTIPLIER
        || distance_to_segment_xz(node_pos, start.mouth_world_pos, intersection)
            + CONNECTION_POINT_EPSILON_M
            < minimum_radius
        || distance_to_segment_xz(node_pos, intersection, end.mouth_world_pos)
            + CONNECTION_POINT_EPSILON_M
            < minimum_radius
    {
        return None;
    }
    Some(intersection)
}

fn shortest_signed_angle_delta(start: f32, end: f32) -> f32 {
    (end - start + PI).rem_euclid(TAU) - PI
}

fn sidewalk_corner_arc_route(
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
        let y = start.mouth_world_pos.y.lerp(end.mouth_world_pos.y, t);
        push_distinct_point(
            &mut route,
            Vector3::new(
                node_pos.x + angle.cos() * arc_radius,
                y,
                node_pos.z + angle.sin() * arc_radius,
            ),
        );
    }
    push_distinct_point(&mut route, end.mouth_world_pos);
    route
}

fn sidewalk_corner_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    node_pos: Vector3,
) -> Vec<Vector3> {
    let mut route = Vec::with_capacity(3);
    push_distinct_point(&mut route, start.mouth_world_pos);
    if let Some(corner) = sidewalk_centerline_intersection(start, end, node_pos) {
        push_distinct_point(&mut route, corner);
    } else {
        return sidewalk_corner_arc_route(start, end, node_pos);
    }
    push_distinct_point(&mut route, end.mouth_world_pos);
    route
}

fn append_pedestrian_connection(
    lanes: &mut Vec<Lane>,
    graph: &mut RegionGraph,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
    node_id: usize,
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    geometry: Vec<Vector3>,
    crosswalk_edge_id: Option<usize>,
    crosswalk_marking: Option<CrosswalkMarking>,
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
        lane_type: LaneType::Foot,
        crosswalk_edge_id,
        crosswalk_marking,
        next_lanes: vec![end.out_id],
        node_id,
    });
    node_lanes.entry(node_id).or_default().push(connection_id);
    lanes[start.in_id].next_lanes.push(connection_id);
    graph.nodes[node_id]
        .lane_connections
        .entry((start.edge_idx, start.lane_idx))
        .or_default()
        .push((end.edge_idx, end.lane_idx));
}

/// Builds pedestrian sidewalk connection lanes (crosswalks) at a single node.
pub fn build_pedestrian_connections_at_node(
    lanes: &mut Vec<Lane>,
    lane_map: &HashMap<(usize, bool, i8), usize>,
    graph: &mut RegionGraph,
    node_id: usize,
    node_lanes: &mut HashMap<usize, Vec<usize>>,
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
        let side_vec = Vector3::new(-dir.z, 0.0, dir.x);

        // Position at the junction mouth — clip distance from the node along the road geometry.
        // This is where the road mesh visually starts/ends, and where crosswalks must sit.
        let clip = (if is_start {
            edge.start_clip
        } else {
            edge.end_clip
        }) + config::CROSSWALK_INSET;
        let mouth_center = walk_geom_from_end(geometry, clip, is_start);
        let side_multiplier = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        let asphalt_half_width = road_half_width(edge);

        for &l_idx in &[-100_i8, 100_i8] {
            let side = (l_idx as f32) / 100.0;
            let offset =
                -(asphalt_half_width + config::SIDEWALK_WIDTH * 0.5) * side * side_multiplier;

            let arm_angle = dir.z.atan2(dir.x);

            // Sidewalk-centreline position: used for agent routing geometry.
            let mouth_world_pos = Vector3::new(
                mouth_center.x + side_vec.x * offset,
                mouth_center.y,
                mouth_center.z + side_vec.z * offset,
            );
            // Asphalt-edge position: used for crosswalk stripe geometry so stripes
            // stay on the car lanes and do not extend onto the sidewalk.
            let road_edge_offset = -asphalt_half_width * side * side_multiplier;
            let road_edge_pos = Vector3::new(
                mouth_center.x + side_vec.x * road_edge_offset,
                mouth_center.y,
                mouth_center.z + side_vec.z * road_edge_offset,
            );

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
                mouths.push(SidewalkMouth {
                    edge_idx: e_idx,
                    lane_idx: l_idx,
                    angle: arm_angle,
                    dir,
                    in_id,
                    out_id,
                    mouth_world_pos,
                    road_edge_pos,
                    road_half_width_m: asphalt_half_width,
                });
            }
        }
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
    if num_mouths < 2 {
        return;
    }

    // Crosswalk ownership is exact: the two sidewalks must belong to the same road arm.
    // Direction similarity is insufficient because distinct parallel arms can share a node.
    let mut mouths_by_edge: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for (mouth_index, mouth) in mouths.iter().enumerate() {
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
    let mut crosswalks_added = 0;
    for arm in &arms {
        let edge_idx = arm.edge_idx;
        let clockwise = &mouths[arm.clockwise_mouth];
        let counter_clockwise = &mouths[arm.counter_clockwise_mouth];
        let (start, end) = if clockwise.lane_idx <= counter_clockwise.lane_idx {
            (clockwise, counter_clockwise)
        } else {
            (counter_clockwise, clockwise)
        };
        let crosswalk_override = graph.nodes[node_id]
            .crosswalk_overrides
            .get(&edge_idx)
            .copied();
        if crosswalk_override == Some(false)
            || (crosswalk_override.is_none() && road_degree == 2 && crosswalks_added >= 1)
        {
            continue;
        }

        let marking = CrosswalkMarking {
            edge_id: edge_idx,
            start: start.road_edge_pos,
            end: end.road_edge_pos,
        };
        let route = crosswalk_route(start, end);
        append_pedestrian_connection(
            lanes,
            graph,
            node_lanes,
            node_id,
            start,
            end,
            route.clone(),
            Some(edge_idx),
            Some(marking),
        );
        append_pedestrian_connection(
            lanes,
            graph,
            node_lanes,
            node_id,
            end,
            start,
            route.into_iter().rev().collect(),
            Some(edge_idx),
            None,
        );
        crosswalks_added += 1;
    }

    // Adjacent arms connect around their shared sidewalk corner. The old direct
    // mouth-to-mouth chord could cross the asphalt junction interior.
    for i in 0..arms.len() {
        let j = (i + 1) % arms.len();
        if arms[i].edge_idx == arms[j].edge_idx {
            continue;
        }
        let start_index = arms[i].counter_clockwise_mouth;
        let end_index = arms[j].clockwise_mouth;
        let start = &mouths[start_index];
        let end = &mouths[end_index];

        let route = sidewalk_corner_route(start, end, node_pos);
        append_pedestrian_connection(
            lanes,
            graph,
            node_lanes,
            node_id,
            start,
            end,
            route.clone(),
            None,
            None,
        );
        append_pedestrian_connection(
            lanes,
            graph,
            node_lanes,
            node_id,
            end,
            start,
            route.into_iter().rev().collect(),
            None,
            None,
        );
    }
}
