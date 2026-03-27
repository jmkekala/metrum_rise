//! Graph-dilation road renderer.
//!
//! This replaces the old junction contour solver with the same core idea used in the
//! `graph-road-renderer` proof of concept:
//! - draw a wider sidewalk base from the road graph skeleton
//! - draw the asphalt surface on top from the same skeleton
//! - add circular node fills only where the node is a true junction or terminal
//! - keep lane markings as a separate overlay mesh trimmed out of true junctions

use super::{NetworkMeshData, TransitRenderer};
use crate::config::{LANE_WIDTH, ROAD_H_OFFSET, SIDEWALK_WIDTH, Z_FIGHT_BIAS};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::*;
use std::collections::HashMap;
use std::f32::consts::TAU;

const PASS_THROUGH_DOT: f32 = -0.985;
const MIN_SEGMENT_LEN: f32 = 0.01;
const SIDEWALK_LAYER_Y: f32 = ROAD_H_OFFSET;
const ROAD_LAYER_Y: f32 = ROAD_H_OFFSET + Z_FIGHT_BIAS;
const MARKING_LAYER_Y: f32 = ROAD_H_OFFSET + Z_FIGHT_BIAS * 2.0;
const BRIDGE_CONCRETE_Y: f32 = ROAD_H_OFFSET - 0.05;
const MARKING_WIDTH: f32 = 0.16;

#[derive(Clone, Copy, PartialEq, Eq)]
enum NodeRenderKind {
    None,
    Terminal,
    PassThrough,
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
enum MeshLayer {
    Main,
    Marking,
    Concrete,
}

/// Top-surface road renderer built from strips, disks, and overlays.
pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(&self, graph: &RegionGraph, _terrain: &TerrainSystem) -> NetworkMeshData {
        let node_states = build_node_render_states(graph);
        let mut mesh = NetworkMeshData::new();

        for edge in &graph.edges {
            if edge.deleted {
                continue;
            }

            match edge.primary_type {
                TransitType::Road => {
                    if has_sidewalk(edge) {
                        emit_polyline_fill(
                            &mut mesh,
                            MeshLayer::Main,
                            edge_points(edge),
                            outer_half_width(edge),
                            SIDEWALK_LAYER_Y,
                            sidewalk_color(),
                        );
                    }
                    if edge.class == EdgeClass::Bridge {
                        emit_polyline_fill(
                            &mut mesh,
                            MeshLayer::Concrete,
                            edge_points(edge),
                            outer_half_width(edge) + 0.25,
                            BRIDGE_CONCRETE_Y,
                            concrete_color(),
                        );
                    }
                }
                TransitType::Foot => {
                    emit_polyline_fill(
                        &mut mesh,
                        MeshLayer::Main,
                        edge_points(edge),
                        (edge.width.max(2.0) * 0.5) + 0.4,
                        SIDEWALK_LAYER_Y,
                        sidewalk_color(),
                    );
                }
                _ => {}
            }
        }

        for (node_id, state) in &node_states {
            if state.kind == NodeRenderKind::None || state.kind == NodeRenderKind::PassThrough {
                continue;
            }
            if state.outer_radius > 0.0 {
                emit_disk(
                    &mut mesh,
                    MeshLayer::Main,
                    graph.nodes[*node_id as usize].pos,
                    state.outer_radius,
                    SIDEWALK_LAYER_Y,
                    sidewalk_color(),
                );
            }
        }

        for edge in &graph.edges {
            if edge.deleted {
                continue;
            }

            match edge.primary_type {
                TransitType::Road => emit_polyline_fill(
                    &mut mesh,
                    MeshLayer::Main,
                    edge_points(edge),
                    road_half_width(edge),
                    ROAD_LAYER_Y,
                    road_color(),
                ),
                TransitType::Foot => {}
                _ => {}
            }
        }

        for (node_id, state) in &node_states {
            if state.kind == NodeRenderKind::None || state.kind == NodeRenderKind::PassThrough {
                continue;
            }
            if state.road_radius > 0.0 {
                emit_disk(
                    &mut mesh,
                    MeshLayer::Main,
                    graph.nodes[*node_id as usize].pos,
                    state.road_radius,
                    ROAD_LAYER_Y,
                    road_color(),
                );
            }
        }

        for edge in &graph.edges {
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.class == EdgeClass::Tunnel
            {
                continue;
            }
            emit_lane_markings(&mut mesh, graph, edge, &node_states);
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
                let same_width = (info.road_radius - info.road_radius_min).abs() <= 0.1;
                if same_width && info.directions[0].dot(info.directions[1]) <= PASS_THROUGH_DOT {
                    NodeRenderKind::PassThrough
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

fn emit_polyline_fill(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    points: &[Vector3],
    half_width: f32,
    y_offset: f32,
    color: Color,
) {
    if points.len() < 2 || half_width <= 0.0 {
        return;
    }

    for segment in points.windows(2) {
        emit_segment_quad(
            mesh, layer, segment[0], segment[1], half_width, y_offset, color,
        );
    }

    for point in points.iter().skip(1).take(points.len().saturating_sub(2)) {
        emit_disk(mesh, layer, *point, half_width, y_offset, color);
    }
}

fn emit_segment_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    start: Vector3,
    end: Vector3,
    half_width: f32,
    y_offset: f32,
    color: Color,
) {
    let delta = Vector2::new(end.x - start.x, end.z - start.z);
    let length = delta.length();
    if length < MIN_SEGMENT_LEN {
        return;
    }

    let tangent = delta / length;
    let side = Vector2::new(-tangent.y, tangent.x) * half_width;

    let a_left = lifted_offset(start, side, y_offset);
    let a_right = lifted_offset(start, -side, y_offset);
    let b_left = lifted_offset(end, side, y_offset);
    let b_right = lifted_offset(end, -side, y_offset);
    let uvs = if color.a > 0.9 {
        [
            Vector2::new(0.0, 1.0),
            Vector2::new(0.0, 1.0),
            Vector2::new(length, 1.0),
            Vector2::new(length, 1.0),
        ]
    } else {
        [
            Vector2::new(0.0, 0.0),
            Vector2::new(0.0, 1.0),
            Vector2::new(length, 1.0),
            Vector2::new(length, 0.0),
        ]
    };

    push_quad(mesh, layer, [a_left, a_right, b_right, b_left], uvs, color);
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
) {
    let points = edge_points(edge);
    if points.len() < 2 {
        return;
    }

    let total_lanes = edge.fwd_lanes as usize + edge.bkw_lanes as usize;
    if total_lanes <= 1 {
        return;
    }

    let start_trim = junction_trim(graph, node_states, edge.start_node);
    let end_trim = junction_trim(graph, node_states, edge.end_node);
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

fn junction_trim(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_id: u32,
) -> f32 {
    let node_id = graph.get_valid_node(node_id);
    match node_states.get(&node_id).copied().unwrap_or_default().kind {
        NodeRenderKind::Junction => node_states
            .get(&node_id)
            .map(|state| state.road_radius)
            .unwrap_or(0.0),
        _ => 0.0,
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
        MeshLayer::Main => (
            &mut mesh.vertices,
            &mut mesh.normals,
            &mut mesh.uvs,
            &mut mesh.colors,
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
