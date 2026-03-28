//! Graph-dilation road renderer.
//!
//! This replaces the old junction contour solver with the same core idea used in the
//! `graph-road-renderer` proof of concept:
//! - draw a wider sidewalk base from the road graph skeleton
//! - draw the asphalt surface on top from the same skeleton
//! - add circular node fills only where the node is a true junction or terminal
//! - keep lane markings as a separate overlay mesh trimmed out of true junctions

use super::{NetworkMeshData, TransitRenderer};
use crate::config::{LANE_WIDTH, ROAD_H_OFFSET, SIDEWALK_WIDTH};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::*;
use std::collections::HashMap;
use std::f32::consts::TAU;

const PASS_THROUGH_DOT: f32 = -0.985;
const MIN_SEGMENT_LEN: f32 = 0.01;
const SIDEWALK_LAYER_Y: f32 = ROAD_H_OFFSET;
const ROAD_LAYER_Y: f32 = ROAD_H_OFFSET + 0.02;
const MARKING_LAYER_Y: f32 = ROAD_H_OFFSET + 0.04;
const BRIDGE_CONCRETE_Y: f32 = ROAD_H_OFFSET - 0.05;
const MARKING_WIDTH: f32 = 0.16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NodeRenderKind {
    None,
    Terminal,
    PassThrough,
    WidthTransition,
    Junction,
}

#[derive(Clone, Copy)]
struct NodeRenderState {
    kind: NodeRenderKind,
    road_radius: f32,
    outer_radius: f32,
}

impl Default for NodeRenderState {
    fn default() -> Self {
        Self {
            kind: NodeRenderKind::None,
            road_radius: 0.0,
            outer_radius: 0.0,
        }
    }
}

#[derive(Default)]
struct NodeAccum {
    degree: usize,
    road_radius: f32,
    road_radius_min: f32,
    outer_radius: f32,
    directions: Vec<Vector2>,
}

#[derive(Clone, Copy)]
struct IncidentEdgeEndpoint {
    edge_idx: usize,
    at_start: bool,
}

#[derive(Clone, Copy)]
struct BoundaryPoint {
    angle: f32,
    point: Vector3,
}

#[derive(Clone, Copy)]
enum MeshLayer {
    Sidewalk,
    Road,
    Marking,
    Concrete,
}

/// Top-surface road renderer built from strips, disks, and overlays.
pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, _terrain: &TerrainSystem) -> NetworkMeshData {
        let node_states = build_node_render_states(graph);
        let node_incidents = build_surface_node_incidents(graph);
        let mut mesh = NetworkMeshData::new();

        // Pass 1: draw the widened sidewalk-colored base for every visible surface corridor.
        for edge in &graph.edges {
            if edge.deleted {
                continue;
            }

            match edge.primary_type {
                TransitType::Road => {
                    if has_sidewalk(edge) {
                        let half_width = outer_half_width(edge);
                        let (points, start_half_width, end_half_width) = edge_render_geometry(
                            graph,
                            &node_states,
                            &node_incidents,
                            edge,
                            half_width,
                            true,
                        );
                        emit_polyline_fill(
                            &mut mesh,
                            MeshLayer::Sidewalk,
                            &points,
                            half_width,
                            start_half_width,
                            end_half_width,
                            SIDEWALK_LAYER_Y,
                            sidewalk_color(),
                        );
                    }
                    if edge.class == EdgeClass::Bridge {
                        let half_width = outer_half_width(edge) + 0.25;
                        let points = trimmed_polyline(
                            edge_points(edge),
                            edge_endpoint_trim_distance(
                                graph,
                                &node_states,
                                &node_incidents,
                                edge,
                                true,
                                half_width,
                            ),
                            edge_endpoint_trim_distance(
                                graph,
                                &node_states,
                                &node_incidents,
                                edge,
                                false,
                                half_width,
                            ),
                        );
                        emit_polyline_fill(
                            &mut mesh,
                            MeshLayer::Concrete,
                            &points,
                            half_width,
                            half_width,
                            half_width,
                            BRIDGE_CONCRETE_Y,
                            concrete_color(),
                        );
                    }
                }
                TransitType::Foot => {
                    let half_width = (edge.width.max(2.0) * 0.5) + 0.4;
                    emit_polyline_fill(
                        &mut mesh,
                        MeshLayer::Sidewalk,
                        edge_points(edge),
                        half_width,
                        half_width,
                        half_width,
                        SIDEWALK_LAYER_Y,
                        sidewalk_color(),
                    );
                }
                _ => {}
            }
        }

        // Pass 1b: render terminal caps and angle-aware junction fills for the sidewalk base.
        for (node_id, state) in &node_states {
            match state.kind {
                NodeRenderKind::Terminal if state.outer_radius > 0.0 => {
                    emit_disk(
                        &mut mesh,
                        MeshLayer::Sidewalk,
                        graph.nodes[*node_id as usize].pos,
                        state.outer_radius,
                        SIDEWALK_LAYER_Y,
                        sidewalk_color(),
                    );
                }
                NodeRenderKind::Junction => {
                    if node_uses_polygon_fill(graph, &node_states, &node_incidents, *node_id) {
                        emit_node_fill_polygon(
                            &mut mesh,
                            MeshLayer::Sidewalk,
                            graph,
                            *node_id,
                            &node_states,
                            &node_incidents,
                            true,
                            SIDEWALK_LAYER_Y,
                            sidewalk_color(),
                        );
                    } else if state.outer_radius > 0.0 {
                        emit_disk(
                            &mut mesh,
                            MeshLayer::Sidewalk,
                            graph.nodes[*node_id as usize].pos,
                            state.outer_radius,
                            SIDEWALK_LAYER_Y,
                            sidewalk_color(),
                        );
                    }
                }
                _ => {}
            }
        }

        // Pass 2: asphalt overdraw. This is the visible ownership rule for the top surface.
        for edge in &graph.edges {
            if edge.deleted {
                continue;
            }

            match edge.primary_type {
                TransitType::Road => {
                    let half_width = road_half_width(edge);
                    let (points, start_half_width, end_half_width) = edge_render_geometry(
                        graph,
                        &node_states,
                        &node_incidents,
                        edge,
                        half_width,
                        false,
                    );
                    emit_polyline_fill(
                        &mut mesh,
                        MeshLayer::Road,
                        &points,
                        half_width,
                        start_half_width,
                        end_half_width,
                        ROAD_LAYER_Y,
                        road_color(),
                    )
                }
                TransitType::Foot => {}
                _ => {}
            }
        }

        // Pass 2b: road caps and junction fills sit above the sidewalk base.
        for (node_id, state) in &node_states {
            match state.kind {
                NodeRenderKind::Terminal if state.road_radius > 0.0 => {
                    emit_disk(
                        &mut mesh,
                        MeshLayer::Road,
                        graph.nodes[*node_id as usize].pos,
                        state.road_radius,
                        ROAD_LAYER_Y,
                        road_color(),
                    );
                }
                NodeRenderKind::Junction => {
                    if node_uses_polygon_fill(graph, &node_states, &node_incidents, *node_id) {
                        emit_node_fill_polygon(
                            &mut mesh,
                            MeshLayer::Road,
                            graph,
                            *node_id,
                            &node_states,
                            &node_incidents,
                            false,
                            ROAD_LAYER_Y,
                            road_color(),
                        );
                    } else if state.road_radius > 0.0 {
                        emit_disk(
                            &mut mesh,
                            MeshLayer::Road,
                            graph.nodes[*node_id as usize].pos,
                            state.road_radius,
                            ROAD_LAYER_Y,
                            road_color(),
                        );
                    }
                }
                _ => {}
            }
        }

        // Pass 3: lane markings are a separate overlay mesh trimmed only by true junction disks.
        for edge in &graph.edges {
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.class == EdgeClass::Tunnel
            {
                continue;
            }
            emit_lane_markings(&mut mesh, graph, edge, &node_states, &node_incidents);
        }

        mesh
    }
}

fn build_node_render_states(graph: &RegionGraph) -> HashMap<u32, NodeRenderState> {
    let mut accum: HashMap<u32, NodeAccum> = HashMap::new();

    for edge in &graph.edges {
        if edge.deleted || edge.primary_type != TransitType::Road || !is_surface_road(edge) {
            continue;
        }

        let road_radius = road_half_width(edge);
        let outer_radius = outer_half_width(edge);

        let start_node = graph.get_valid_node(edge.start_node);
        let end_node = graph.get_valid_node(edge.end_node);

        let start_entry = accum.entry(start_node).or_insert_with(|| NodeAccum {
            road_radius_min: f32::INFINITY,
            ..NodeAccum::default()
        });
        start_entry.degree += 1;
        start_entry.road_radius = start_entry.road_radius.max(road_radius);
        start_entry.outer_radius = start_entry.outer_radius.max(outer_radius);
        start_entry.road_radius_min = start_entry.road_radius_min.min(road_radius);
        if let Some(direction) = direction_at_endpoint(edge_points(edge), true) {
            start_entry.directions.push(direction);
        }

        let end_entry = accum.entry(end_node).or_insert_with(|| NodeAccum {
            road_radius_min: f32::INFINITY,
            ..NodeAccum::default()
        });
        end_entry.degree += 1;
        end_entry.road_radius = end_entry.road_radius.max(road_radius);
        end_entry.outer_radius = end_entry.outer_radius.max(outer_radius);
        end_entry.road_radius_min = end_entry.road_radius_min.min(road_radius);
        if let Some(direction) = direction_at_endpoint(edge_points(edge), false) {
            end_entry.directions.push(direction);
        }
    }

    let mut result = HashMap::new();
    for (node_id, info) in accum {
        let kind = match info.degree {
            0 => NodeRenderKind::None,
            1 => NodeRenderKind::Terminal,
            2 if info.directions.len() >= 2 => {
                // Keep straight width-matched splits disk-free so long roads do not grow a bubble
                // at every topology split created by the editor.
                let same_width = (info.road_radius - info.road_radius_min).abs() <= 0.1;
                if info.directions[0].dot(info.directions[1]) <= PASS_THROUGH_DOT {
                    if same_width {
                        NodeRenderKind::PassThrough
                    } else {
                        NodeRenderKind::WidthTransition
                    }
                } else {
                    NodeRenderKind::Junction
                }
            }
            _ => NodeRenderKind::Junction,
        };

        result.insert(
            node_id,
            NodeRenderState {
                kind,
                road_radius: info.road_radius,
                outer_radius: info.outer_radius,
            },
        );
    }

    result
}

fn build_surface_node_incidents(graph: &RegionGraph) -> HashMap<u32, Vec<IncidentEdgeEndpoint>> {
    let mut incidents = HashMap::new();

    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        if edge.deleted || edge.primary_type != TransitType::Road || !is_surface_road(edge) {
            continue;
        }

        let start_node = graph.get_valid_node(edge.start_node);
        incidents
            .entry(start_node)
            .or_insert_with(Vec::new)
            .push(IncidentEdgeEndpoint {
                edge_idx,
                at_start: true,
            });

        let end_node = graph.get_valid_node(edge.end_node);
        if end_node != start_node {
            incidents
                .entry(end_node)
                .or_insert_with(Vec::new)
                .push(IncidentEdgeEndpoint {
                    edge_idx,
                    at_start: false,
                });
        }
    }

    incidents
}

fn edge_points(edge: &Edge) -> &[Vector3] {
    if edge.geometry.len() >= 2 {
        &edge.geometry
    } else {
        &edge.physical_geometry
    }
}

fn is_surface_road(edge: &Edge) -> bool {
    matches!(
        edge.class,
        EdgeClass::Standard | EdgeClass::Bridge | EdgeClass::Ramp
    )
}

fn has_sidewalk(edge: &Edge) -> bool {
    matches!(
        edge.class,
        EdgeClass::Standard | EdgeClass::Bridge | EdgeClass::Ramp
    )
}

fn road_half_width(edge: &Edge) -> f32 {
    edge.width.max(2.0) * 0.5
}

fn outer_half_width(edge: &Edge) -> f32 {
    road_half_width(edge) + SIDEWALK_WIDTH
}

fn direction_at_endpoint(points: &[Vector3], at_start: bool) -> Option<Vector2> {
    if points.len() < 2 {
        return None;
    }

    if at_start {
        let origin = points[0];
        for point in &points[1..] {
            let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN {
                return Some(delta.normalized());
            }
        }
    } else {
        let origin = *points.last().unwrap();
        for point in points[..points.len() - 1].iter().rev() {
            let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN {
                return Some(delta.normalized());
            }
        }
    }

    None
}

fn edge_render_geometry(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    edge: &Edge,
    half_width: f32,
    outer: bool,
) -> (Vec<Vector3>, f32, f32) {
    let start_trim =
        edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, true, half_width);
    let end_trim =
        edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, false, half_width);
    let points = trimmed_polyline(edge_points(edge), start_trim, end_trim);
    let start_half_width = if start_trim > 0.0 {
        half_width
    } else {
        node_endpoint_half_width(graph, node_states, edge.start_node, half_width, outer)
    };
    let end_half_width = if end_trim > 0.0 {
        half_width
    } else {
        node_endpoint_half_width(graph, node_states, edge.end_node, half_width, outer)
    };
    (points, start_half_width, end_half_width)
}

fn edge_endpoint_trim_distance(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    edge: &Edge,
    at_start: bool,
    half_width: f32,
) -> f32 {
    let node_id = graph.get_valid_node(if at_start { edge.start_node } else { edge.end_node });
    match node_states.get(&node_id).copied().unwrap_or_default().kind {
        NodeRenderKind::Junction if node_uses_polygon_fill(graph, node_states, node_incidents, node_id) => {
            let clip = if at_start {
                edge.start_clip
            } else {
                edge.end_clip
            };
            clip.max(half_width)
        }
        _ => 0.0,
    }
}

fn node_uses_polygon_fill(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    node_id: u32,
) -> bool {
    let node_id = graph.get_valid_node(node_id);
    if node_states
        .get(&node_id)
        .copied()
        .unwrap_or_default()
        .kind
        != NodeRenderKind::Junction
    {
        return false;
    }

    let Some(incidents) = node_incidents.get(&node_id) else {
        return false;
    };
    if incidents.len() >= 3 {
        return true;
    }
    if incidents.len() != 2 {
        return false;
    }

    let mut min_width = f32::INFINITY;
    let mut max_width = 0.0_f32;
    for incident in incidents {
        let width = graph.edges[incident.edge_idx].width;
        min_width = min_width.min(width);
        max_width = max_width.max(width);
    }

    (max_width - min_width).abs() > 0.1
}

fn trimmed_polyline(points: &[Vector3], start_trim: f32, end_trim: f32) -> Vec<Vector3> {
    if points.len() < 2 {
        return points.to_vec();
    }

    let total_length = polyline_length(points);
    let clip_start = start_trim.clamp(0.0, total_length);
    let clip_end = (total_length - end_trim).clamp(0.0, total_length);
    if clip_end - clip_start < MIN_SEGMENT_LEN {
        return Vec::new();
    }

    clip_polyline_range(points, clip_start, clip_end)
}

fn clip_polyline_range(points: &[Vector3], start_distance: f32, end_distance: f32) -> Vec<Vector3> {
    let mut result = Vec::new();
    let mut travelled = 0.0_f32;

    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let delta = Vector2::new(end.x - start.x, end.z - start.z);
        let segment_length = delta.length();
        if segment_length < MIN_SEGMENT_LEN {
            continue;
        }

        let seg_start = travelled;
        let seg_end = travelled + segment_length;
        let clip_start = start_distance.max(seg_start);
        let clip_end = end_distance.min(seg_end);
        if clip_end <= clip_start {
            travelled = seg_end;
            continue;
        }

        let local_a = (clip_start - seg_start) / segment_length;
        let local_b = (clip_end - seg_start) / segment_length;
        push_unique_point(&mut result, start.lerp(end, local_a));
        push_unique_point(&mut result, start.lerp(end, local_b));

        travelled = seg_end;
    }

    result
}

fn push_unique_point(points: &mut Vec<Vector3>, point: Vector3) {
    if points
        .last()
        .map(|last| (*last - point).length_squared() < 0.0001)
        .unwrap_or(false)
    {
        return;
    }
    points.push(point);
}

fn emit_polyline_fill(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    points: &[Vector3],
    half_width: f32,
    start_half_width: f32,
    end_half_width: f32,
    y_offset: f32,
    color: Color,
) {
    if points.len() < 2 || half_width <= 0.0 {
        return;
    }

    let sections = build_polyline_sections(points, half_width, start_half_width, end_half_width, y_offset);
    if sections.len() < 2 {
        return;
    }

    let mut travelled = 0.0_f32;
    for idx in 0..sections.len() - 1 {
        let delta = Vector2::new(
            points[idx + 1].x - points[idx].x,
            points[idx + 1].z - points[idx].z,
        );
        let length = delta.length();
        if length < MIN_SEGMENT_LEN {
            continue;
        }

        let uvs = if color.a > 0.9 {
            [
                Vector2::new(travelled, 1.0),
                Vector2::new(travelled, 1.0),
                Vector2::new(travelled + length, 1.0),
                Vector2::new(travelled + length, 1.0),
            ]
        } else {
            [
                Vector2::new(travelled, 0.0),
                Vector2::new(travelled, 1.0),
                Vector2::new(travelled + length, 1.0),
                Vector2::new(travelled + length, 0.0),
            ]
        };

        push_quad(
            mesh,
            layer,
            [
                sections[idx].0,
                sections[idx].1,
                sections[idx + 1].1,
                sections[idx + 1].0,
            ],
            uvs,
            color,
        );
        travelled += length;
    }
}

fn build_polyline_sections(
    points: &[Vector3],
    half_width: f32,
    start_half_width: f32,
    end_half_width: f32,
    y_offset: f32,
) -> Vec<(Vector3, Vector3)> {
    let mut sections = Vec::with_capacity(points.len());

    for idx in 0..points.len() {
        let point_half_width = if idx == 0 {
            start_half_width
        } else if idx == points.len() - 1 {
            end_half_width
        } else {
            half_width
        };
        let side = polyline_side_at(points, idx) * point_half_width;
        sections.push((
            lifted_offset(points[idx], side, y_offset),
            lifted_offset(points[idx], -side, y_offset),
        ));
    }

    sections
}

fn polyline_side_at(points: &[Vector3], idx: usize) -> Vector2 {
    let prev_dir = if idx > 0 {
        direction_xz(points[idx] - points[idx - 1])
    } else {
        None
    };
    let next_dir = if idx + 1 < points.len() {
        direction_xz(points[idx + 1] - points[idx])
    } else {
        None
    };

    match (prev_dir, next_dir) {
        (Some(prev), Some(next)) => {
            let prev_normal = Vector2::new(-prev.y, prev.x);
            let next_normal = Vector2::new(-next.y, next.x);
            let miter = prev_normal + next_normal;
            if miter.length_squared() < MIN_SEGMENT_LEN * MIN_SEGMENT_LEN {
                next_normal
            } else {
                let miter_dir = miter.normalized();
                let denom = miter_dir.dot(next_normal).abs().max(0.25);
                miter_dir * (1.0 / denom).min(2.0)
            }
        }
        (Some(prev), None) => Vector2::new(-prev.y, prev.x),
        (None, Some(next)) => Vector2::new(-next.y, next.x),
        (None, None) => Vector2::ZERO,
    }
}

fn direction_xz(delta: Vector3) -> Option<Vector2> {
    let flat = Vector2::new(delta.x, delta.z);
    let length = flat.length();
    if length < MIN_SEGMENT_LEN {
        None
    } else {
        Some(flat / length)
    }
}

fn node_endpoint_half_width(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_id: u32,
    fallback: f32,
    outer: bool,
) -> f32 {
    let node_id = graph.get_valid_node(node_id);
    match node_states.get(&node_id) {
        Some(state) if state.kind == NodeRenderKind::WidthTransition => {
            if outer {
                state.outer_radius
            } else {
                state.road_radius
            }
        }
        _ => fallback,
    }
}

fn emit_node_fill_polygon(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    graph: &RegionGraph,
    node_id: u32,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    outer: bool,
    y_offset: f32,
    color: Color,
) {
    let boundary = junction_boundary_points(graph, node_id, node_states, node_incidents, outer);
    if boundary.len() < 3 {
        return;
    }

    let center = graph.nodes[node_id as usize].pos;
    let center = Vector3::new(center.x, center.y + y_offset, center.z);
    let center_uv = if color.a > 0.9 {
        Vector2::new(0.0, 1.0)
    } else {
        Vector2::ZERO
    };
    let rim_uv = if color.a > 0.9 {
        Vector2::new(1.0, 1.0)
    } else {
        Vector2::ZERO
    };

    for idx in 0..boundary.len() {
        let current = boundary[idx].point;
        let next = boundary[(idx + 1) % boundary.len()].point;
        let current = Vector3::new(current.x, current.y + y_offset, current.z);
        let next = Vector3::new(next.x, next.y + y_offset, next.z);
        push_triangle(
            mesh,
            layer,
            [center, current, next],
            [center_uv, rim_uv, rim_uv],
            color,
        );
    }
}

fn junction_boundary_points(
    graph: &RegionGraph,
    node_id: u32,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    outer: bool,
) -> Vec<BoundaryPoint> {
    let node_id = graph.get_valid_node(node_id);
    let Some(incidents) = node_incidents.get(&node_id) else {
        return Vec::new();
    };

    let mut boundary = Vec::with_capacity(incidents.len() * 2);
    let center = graph.nodes[node_id as usize].pos;

    for incident in incidents {
        let edge = &graph.edges[incident.edge_idx];
        let half_width = if outer {
            outer_half_width(edge)
        } else {
            road_half_width(edge)
        };
        let trim = edge_endpoint_trim_distance(
            graph,
            node_states,
            node_incidents,
            edge,
            incident.at_start,
            half_width,
        );
        let Some((section_center, direction)) = endpoint_section(edge_points(edge), incident.at_start, trim) else {
            continue;
        };
        let side = Vector2::new(-direction.y, direction.x) * half_width;
        let left = lifted_offset(section_center, side, 0.0);
        let right = lifted_offset(section_center, -side, 0.0);
        boundary.push(BoundaryPoint {
            angle: (left.z - center.z).atan2(left.x - center.x),
            point: left,
        });
        boundary.push(BoundaryPoint {
            angle: (right.z - center.z).atan2(right.x - center.x),
            point: right,
        });
    }

    boundary.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap_or(std::cmp::Ordering::Equal));
    boundary.dedup_by(|a, b| (a.point - b.point).length_squared() < 0.0001);
    if boundary.len() >= 2
        && (boundary[0].point - boundary[boundary.len() - 1].point).length_squared() < 0.0001
    {
        boundary.pop();
    }

    if polygon_signed_area_xz(&boundary) < 0.0 {
        boundary.reverse();
    }

    boundary
}

fn polygon_signed_area_xz(points: &[BoundaryPoint]) -> f32 {
    if points.len() < 3 {
        return 0.0;
    }

    let mut area = 0.0_f32;
    for idx in 0..points.len() {
        let current = points[idx].point;
        let next = points[(idx + 1) % points.len()].point;
        area += current.x * next.z - current.z * next.x;
    }
    area * 0.5
}

fn endpoint_section(points: &[Vector3], at_start: bool, trim: f32) -> Option<(Vector3, Vector2)> {
    let trimmed = if at_start {
        trimmed_polyline(points, trim, 0.0)
    } else {
        trimmed_polyline(points, 0.0, trim)
    };
    if trimmed.len() < 2 {
        return None;
    }

    if at_start {
        let direction = direction_xz(trimmed[1] - trimmed[0])?;
        Some((trimmed[0], direction))
    } else {
        let last_idx = trimmed.len() - 1;
        let direction = direction_xz(trimmed[last_idx - 1] - trimmed[last_idx])?;
        Some((trimmed[last_idx], direction))
    }
}

fn emit_disk(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    center: Vector3,
    radius: f32,
    y_offset: f32,
    color: Color,
) {
    if radius <= 0.0 {
        return;
    }

    let center = Vector3::new(center.x, center.y + y_offset, center.z);
    let sectors = circle_segments(radius);
    let mut previous = circle_point(center, radius, 0.0);
    // Match the sidewalk-strip UV contract above so circular fills shade the same as widened
    // edge strips.
    let center_uv = if color.a > 0.9 {
        Vector2::new(0.0, 1.0)
    } else {
        Vector2::ZERO
    };
    let rim_uv = Vector2::new(1.0, 1.0);

    for step in 1..=sectors {
        let angle = step as f32 / sectors as f32 * TAU;
        let current = circle_point(center, radius, angle);
        push_triangle(
            mesh,
            layer,
            [center, previous, current],
            [center_uv, rim_uv, rim_uv],
            color,
        );
        previous = current;
    }
}

fn emit_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    edge: &Edge,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
) {
    let points = edge_points(edge);
    if points.len() < 2 {
        return;
    }

    let total_lanes = edge.fwd_lanes as usize + edge.bkw_lanes as usize;
    if total_lanes <= 1 {
        return;
    }

    let start_trim = edge_endpoint_trim_distance(
        graph,
        node_states,
        node_incidents,
        edge,
        true,
        road_half_width(edge),
    );
    let end_trim = edge_endpoint_trim_distance(
        graph,
        node_states,
        node_incidents,
        edge,
        false,
        road_half_width(edge),
    );
    let total_length = polyline_length(points);
    if total_length <= start_trim + end_trim + 0.5 {
        return;
    }

    let mut marking_specs = Vec::new();
    for divider in 1..total_lanes {
        let offset = -road_half_width(edge) + divider as f32 * LANE_WIDTH;
        let is_center =
            edge.fwd_lanes > 0 && edge.bkw_lanes > 0 && divider == edge.bkw_lanes as usize;
        marking_specs.push((offset, is_center));
    }

    for (offset, is_center) in marking_specs {
        emit_marking_polyline(
            mesh,
            points,
            start_trim,
            total_length - end_trim,
            offset,
            MARKING_WIDTH * 0.5,
            if is_center {
                marking_center_color()
            } else {
                marking_dash_color()
            },
        );
    }
}

fn emit_marking_polyline(
    mesh: &mut NetworkMeshData,
    points: &[Vector3],
    start_distance: f32,
    end_distance: f32,
    lateral_offset: f32,
    half_width: f32,
    color: Color,
) {
    if end_distance <= start_distance || points.len() < 2 {
        return;
    }

    let mut travelled = 0.0;
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let delta = Vector2::new(end.x - start.x, end.z - start.z);
        let segment_length = delta.length();
        if segment_length < MIN_SEGMENT_LEN {
            continue;
        }

        let seg_start = travelled;
        let seg_end = travelled + segment_length;
        let clip_start = start_distance.max(seg_start);
        let clip_end = end_distance.min(seg_end);
        if clip_end <= clip_start {
            travelled = seg_end;
            continue;
        }

        let local_a = (clip_start - seg_start) / segment_length;
        let local_b = (clip_end - seg_start) / segment_length;
        let point_a = start.lerp(end, local_a);
        let point_b = start.lerp(end, local_b);

        emit_marking_segment(
            mesh,
            point_a,
            point_b,
            lateral_offset,
            half_width,
            clip_start,
            clip_end,
            color,
        );

        travelled = seg_end;
    }
}

fn emit_marking_segment(
    mesh: &mut NetworkMeshData,
    start: Vector3,
    end: Vector3,
    lateral_offset: f32,
    half_width: f32,
    uv_start: f32,
    uv_end: f32,
    color: Color,
) {
    let delta = Vector2::new(end.x - start.x, end.z - start.z);
    let length = delta.length();
    if length < MIN_SEGMENT_LEN {
        return;
    }

    let tangent = delta / length;
    let side = Vector2::new(-tangent.y, tangent.x);
    let center_start = lifted_offset(start, side * lateral_offset, MARKING_LAYER_Y);
    let center_end = lifted_offset(end, side * lateral_offset, MARKING_LAYER_Y);
    let edge_offset = side * half_width;

    let a_left = Vector3::new(
        center_start.x + edge_offset.x,
        center_start.y,
        center_start.z + edge_offset.y,
    );
    let a_right = Vector3::new(
        center_start.x - edge_offset.x,
        center_start.y,
        center_start.z - edge_offset.y,
    );
    let b_left = Vector3::new(
        center_end.x + edge_offset.x,
        center_end.y,
        center_end.z + edge_offset.y,
    );
    let b_right = Vector3::new(
        center_end.x - edge_offset.x,
        center_end.y,
        center_end.z - edge_offset.y,
    );

    push_quad(
        mesh,
        MeshLayer::Marking,
        [a_left, a_right, b_right, b_left],
        [
            Vector2::new(uv_start, 1.0),
            Vector2::new(uv_start, 1.0),
            Vector2::new(uv_end, 1.0),
            Vector2::new(uv_end, 1.0),
        ],
        color,
    );
}

fn circle_segments(radius: f32) -> usize {
    ((radius * 2.0).ceil() as usize).clamp(12, 40)
}

fn circle_point(center: Vector3, radius: f32, angle: f32) -> Vector3 {
    Vector3::new(
        center.x + angle.cos() * radius,
        center.y,
        center.z + angle.sin() * radius,
    )
}

fn polyline_length(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .map(|segment| {
            let delta = Vector2::new(segment[1].x - segment[0].x, segment[1].z - segment[0].z);
            delta.length()
        })
        .sum()
}

fn lifted_offset(point: Vector3, offset_xz: Vector2, y_offset: f32) -> Vector3 {
    Vector3::new(
        point.x + offset_xz.x,
        point.y + y_offset,
        point.z + offset_xz.y,
    )
}

fn push_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[1], vertices[2]],
        [uvs[0], uvs[1], uvs[2]],
        color,
    );
    push_triangle(
        mesh,
        layer,
        [vertices[0], vertices[2], vertices[3]],
        [uvs[0], uvs[2], uvs[3]],
        color,
    );
}

fn push_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
) {
    let target = match layer {
        MeshLayer::Sidewalk => (
            &mut mesh.sidewalk_vertices,
            &mut mesh.sidewalk_normals,
            &mut mesh.sidewalk_uvs,
            &mut mesh.sidewalk_colors,
        ),
        MeshLayer::Road => (
            &mut mesh.road_vertices,
            &mut mesh.road_normals,
            &mut mesh.road_uvs,
            &mut mesh.road_colors,
        ),
        MeshLayer::Marking => (
            &mut mesh.marking_vertices,
            &mut mesh.marking_normals,
            &mut mesh.marking_uvs,
            &mut mesh.marking_colors,
        ),
        MeshLayer::Concrete => (
            &mut mesh.concrete_vertices,
            &mut mesh.concrete_normals,
            &mut mesh.concrete_uvs,
            &mut mesh.concrete_colors,
        ),
    };

    for index in 0..3 {
        target.0.push(vertices[index]);
        target.1.push(Vector3::UP);
        target.2.push(uvs[index]);
        target.3.push(color);
    }
}

fn road_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 0.0)
}

fn sidewalk_color() -> Color {
    Color::from_rgba(0.0, 0.0, 0.0, 1.0)
}

fn concrete_color() -> Color {
    Color::from_rgba(0.75, 0.75, 0.75, 1.0)
}

fn marking_center_color() -> Color {
    Color::from_rgba(0.0, 1.0, 1.0, 0.0)
}

fn marking_dash_color() -> Color {
    Color::from_rgba(0.0, 1.0, 0.0, 0.0)
}

#[cfg(test)]
mod tests {
    use super::{build_node_render_states, polyline_side_at, NodeRenderKind};
    use crate::simulation::network::graph::data::Edge;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
    use godot::prelude::Vector3;

    fn create_test_edge(n1: u32, n2: u32, p1: Vector3, p2: Vector3, width: f32) -> Edge {
        Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: 1 | 2,
            class: EdgeClass::Standard,
            width,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.0,
            base_cost: 0.0,
            physical_length: (p2 - p1).length(),
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![p1, p2],
            physical_geometry: vec![p1, p2],
            zoning_left: true,
            zoning_right: true,
            deleted: false,
        }
    }

    #[test]
    fn collinear_grade_change_keeps_constant_side_vector() {
        let points = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 5.0, 0.0),
            Vector3::new(20.0, 10.0, 0.0),
        ];
        let side = polyline_side_at(&points, 1);
        assert!((side.x - 0.0).abs() < 0.001);
        assert!((side.y - 1.0).abs() < 0.001);
    }

    #[test]
    fn horizontal_bend_expands_outer_join() {
        let points = [
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
            Vector3::new(10.0, 5.0, 10.0),
        ];
        let side = polyline_side_at(&points, 1);
        assert!(side.length() > 1.3);
    }

    #[test]
    fn straight_width_change_is_not_rendered_as_junction_disk() {
        let mut graph = RegionGraph::new();

        let n0 = graph.add_node(Vector3::new(-25.0, 0.0, 0.0), NodeType::Junction);
        let n1 = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(25.0, 0.0, 0.0), NodeType::Junction);

        graph.add_edge(create_test_edge(
            n0,
            n1,
            Vector3::new(-25.0, 0.0, 0.0),
            Vector3::ZERO,
            7.0,
        ));
        graph.add_edge(create_test_edge(
            n1,
            n2,
            Vector3::ZERO,
            Vector3::new(25.0, 0.0, 0.0),
            14.0,
        ));

        let node_states = build_node_render_states(&graph);
        assert_eq!(
            node_states.get(&n1).map(|state| state.kind),
            Some(NodeRenderKind::WidthTransition)
        );
    }
}
