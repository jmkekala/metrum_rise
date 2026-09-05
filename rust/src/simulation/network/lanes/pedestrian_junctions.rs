// SPDX-License-Identifier: GPL-2.0-only

//! Authoritative pedestrian junction routes and rendered-crossing geometry.

use super::super::graph::RegionGraph;
use super::super::surface::{RoadVec2, rounded_sidewalk_corner_path_xz};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::{build_cum_dist, road_half_width};
use super::{CrosswalkMarking, Lane, LaneType};
use crate::config;
use godot::prelude::*;
use std::collections::{BTreeMap, HashMap};
use std::f32::consts::{PI, TAU};

const CONNECTION_POINT_EPSILON_M: f32 = 0.001;
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
    /// Used to classify the sidewalk side of the road arm.
    pub dir: Vector3,
    /// Planar sidewalk tangent pointing from the junction into the physical lane body.
    pub sidewalk_dir: Vector3,
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
    /// Asphalt half-width used to keep defensive corner fallbacks outside the junction core.
    pub road_half_width_m: f32,
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
    crosswalk_edge_id: Option<usize>,
    crosswalk_marking: Option<CrosswalkMarking>,
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

fn sidewalk_corner_route(
    start: &SidewalkMouth,
    end: &SidewalkMouth,
    node_pos: Vector3,
) -> Vec<Vector3> {
    // Physical sidewalk lanes end at the zebra, CROSSWALK_INSET metres beyond the
    // compiled node mouth. Restore those node-mouth tangent points before asking the
    // road-surface side-join backend for its canonical rounded corner.
    let start_corner_mouth = start.mouth_world_pos - start.sidewalk_dir * config::CROSSWALK_INSET;
    let end_corner_mouth = end.mouth_world_pos - end.sidewalk_dir * config::CROSSWALK_INSET;
    let start_xz = RoadVec2::new(
        f64::from(start_corner_mouth.x),
        f64::from(start_corner_mouth.z),
    );
    let end_xz = RoadVec2::new(f64::from(end_corner_mouth.x), f64::from(end_corner_mouth.z));
    let rounded_xz = rounded_sidewalk_corner_path_xz(
        start_xz,
        RoadVec2::new(
            f64::from(start.sidewalk_dir.x),
            f64::from(start.sidewalk_dir.z),
        ),
        end_xz,
        RoadVec2::new(f64::from(end.sidewalk_dir.x), f64::from(end.sidewalk_dir.z)),
    )
    .unwrap_or_else(|| vec![start_xz]);

    let total_length_m = rounded_xz
        .windows(2)
        .map(|segment| segment[0].distance(segment[1]))
        .sum::<f64>();
    let mut cumulative_length_m = 0.0;
    let mut route = Vec::with_capacity(rounded_xz.len() + 2);
    push_distinct_point(&mut route, start.mouth_world_pos);
    for (index, point_xz) in rounded_xz.iter().copied().enumerate() {
        if index > 0 {
            cumulative_length_m += rounded_xz[index - 1].distance(point_xz);
        }
        let t = if total_length_m > f64::EPSILON {
            (cumulative_length_m / total_length_m) as f32
        } else {
            0.0
        };
        push_distinct_point(
            &mut route,
            Vector3::new(
                point_xz.x as f32,
                start_corner_mouth.y.lerp(end_corner_mouth.y, t),
                point_xz.y as f32,
            ),
        );
    }
    push_distinct_point(&mut route, end.mouth_world_pos);
    let minimum_radius =
        start.road_half_width_m.max(end.road_half_width_m) + config::SIDEWALK_WIDTH * 0.5;
    if route.windows(2).any(|segment| {
        distance_to_segment_xz(node_pos, segment[0], segment[1]) + CONNECTION_POINT_EPSILON_M
            < minimum_radius
    }) {
        safe_node_centered_corner_route(start, end, node_pos)
    } else {
        route
    }
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

fn append_pedestrian_step(
    steps: &mut Vec<PedestrianStep>,
    outgoing_steps: &mut [Vec<usize>],
    start_mouth: usize,
    end_mouth: usize,
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

    let step_id = steps.len();
    steps.push(PedestrianStep {
        start_mouth,
        end_mouth,
        geometry,
        length,
        crosswalk_edge_id,
        crosswalk_marking,
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
        cum_dist: vec![0.0, 0.0],
        lane_type: LaneType::Foot,
        crosswalk_edge_id: None,
        crosswalk_marking: None,
        next_lanes: vec![mouth.out_id],
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
        // Lane offsets are authored relative to the edge's start-to-end direction. At an
        // end-node junction, `dir` points the opposite way (from the node back into the arm),
        // so using its normal would place the mouth on the opposite sidewalk.
        let edge_dir = if is_start { dir } else { dir * -1.0 };
        let edge_side_vec = Vector3::new(-edge_dir.z, 0.0, edge_dir.x);

        let side_multiplier = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
        let asphalt_half_width = road_half_width(edge);

        for &l_idx in &[-100_i8, 100_i8] {
            let side = (l_idx as f32) / 100.0;
            let offset =
                -(asphalt_half_width + config::SIDEWALK_WIDTH * 0.5) * side * side_multiplier;

            let arm_angle = dir.z.atan2(dir.x);

            let road_edge_offset = -asphalt_half_width * side * side_multiplier;

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
                let sidewalk_dir = lanes[out_id]
                    .geometry
                    .windows(2)
                    .find_map(|segment| {
                        let delta = segment[1] - segment[0];
                        let planar_length = Vector2::new(delta.x, delta.z).length();
                        (planar_length > 1.0e-4).then_some(Vector3::new(
                            delta.x / planar_length,
                            0.0,
                            delta.z / planar_length,
                        ))
                    })
                    .unwrap_or(dir);
                debug_assert!(
                    lanes[out_id]
                        .geometry
                        .first()
                        .is_some_and(|outbound_start| outbound_start
                            .distance_squared_to(mouth_world_pos)
                            <= CONNECTION_POINT_EPSILON_M.powi(2)),
                    "opposing sidewalk lanes must share one junction mouth"
                );
                // Move laterally from the authoritative sidewalk endpoint to the asphalt edge.
                // Reconstructing both points independently from centerline arc length diverges
                // on curved offset lanes and can introduce a longitudinal backtrack.
                let road_edge_pos = mouth_world_pos + edge_side_vec * (road_edge_offset - offset);
                mouths.push(SidewalkMouth {
                    edge_idx: e_idx,
                    lane_idx: l_idx,
                    angle: arm_angle,
                    dir,
                    sidewalk_dir,
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
    let mut steps = Vec::with_capacity(road_degree * 4);
    let mut outgoing_steps = (0..mouths.len())
        .map(|_| Vec::with_capacity(2))
        .collect::<Vec<_>>();
    let mut crosswalks_added = 0;
    for arm in &arms {
        let edge_idx = arm.edge_idx;
        let clockwise = &mouths[arm.clockwise_mouth];
        let counter_clockwise = &mouths[arm.counter_clockwise_mouth];
        let (start_index, end_index) = if clockwise.lane_idx <= counter_clockwise.lane_idx {
            (arm.clockwise_mouth, arm.counter_clockwise_mouth)
        } else {
            (arm.counter_clockwise_mouth, arm.clockwise_mouth)
        };
        let start = &mouths[start_index];
        let end = &mouths[end_index];
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
        append_pedestrian_step(
            &mut steps,
            &mut outgoing_steps,
            start_index,
            end_index,
            route.clone(),
            Some(edge_idx),
            Some(marking),
        );
        append_pedestrian_step(
            &mut steps,
            &mut outgoing_steps,
            end_index,
            start_index,
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
        append_pedestrian_step(
            &mut steps,
            &mut outgoing_steps,
            start_index,
            end_index,
            route.clone(),
            None,
            None,
        );
        append_pedestrian_step(
            &mut steps,
            &mut outgoing_steps,
            end_index,
            start_index,
            route.into_iter().rev().collect(),
            None,
            None,
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
            let (crosswalk_edge_id, crosswalk_marking) = if route.len() == 1 {
                let step = &steps[route[0]];
                (step.crosswalk_edge_id, step.crosswalk_marking)
            } else {
                (None, None)
            };
            append_pedestrian_connection(
                lanes,
                graph,
                node_lanes,
                node_id,
                &mouths[source_mouth],
                &mouths[target_mouth],
                pedestrian_route_geometry(&route, &steps),
                crosswalk_edge_id,
                crosswalk_marking,
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
