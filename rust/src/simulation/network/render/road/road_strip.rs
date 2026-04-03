//! Polyline ribbon generation for asphalt and sidewalks.

use crate::config::LANE_WIDTH;
use crate::simulation::network::graph::{Edge, RegionGraph};
use godot::prelude::*;
use std::collections::HashMap;

use super::*;

pub(super) fn edge_render_geometry(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    edge: &Edge,
    half_width: f32,
    _outer: bool,
) -> (Vec<Vector3>, f32, f32, Option<Vector2>, Option<Vector2>) {
    let start_trim = junction_fill::edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, true, half_width);
    let end_trim = junction_fill::edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, false, half_width);
    let points = trimmed_polyline(edge_points(edge), start_trim, end_trim);
    let start_half_width = if start_trim > 0.0 { half_width } else { half_width };
    let end_half_width = if end_trim > 0.0 { half_width } else { half_width };
    let start_side_override = if start_trim > 0.0 { None } else { junction_fill::pass_through_endpoint_side_override(graph, node_states, node_incidents, edge, true) };
    let end_side_override = if end_trim > 0.0 { None } else { junction_fill::pass_through_endpoint_side_override(graph, node_states, node_incidents, edge, false) };
    (points, start_half_width, end_half_width, start_side_override, end_side_override)
}

pub(super) fn trimmed_polyline(points: &[Vector3], start_trim: f32, end_trim: f32) -> Vec<Vector3> {
    if points.len() < 2 { return points.to_vec(); }
    let total_length = polyline_length(points);
    let clip_start = start_trim.clamp(0.0, total_length);
    let clip_end = (total_length - end_trim).clamp(0.0, total_length);
    if clip_end - clip_start < MIN_SEGMENT_LEN { return Vec::new(); }
    clip_polyline_range(points, clip_start, clip_end)
}

pub(super) fn clip_polyline_range(points: &[Vector3], start_distance: f32, end_distance: f32) -> Vec<Vector3> {
    let mut result = Vec::new();
    let mut travelled = 0.0_f32;
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let delta = Vector2::new(end.x - start.x, end.z - start.z);
        let segment_length = delta.length();
        if segment_length < MIN_SEGMENT_LEN { continue; }
        let seg_start = travelled;
        let seg_end = travelled + segment_length;
        let clip_start = start_distance.max(seg_start);
        let clip_end = end_distance.min(seg_end);
        if clip_end <= clip_start { travelled = seg_end; continue; }
        let local_a = (clip_start - seg_start) / segment_length;
        let local_b = (clip_end - seg_start) / segment_length;
        push_unique_point(&mut result, start.lerp(end, local_a));
        push_unique_point(&mut result, start.lerp(end, local_b));
        travelled = seg_end;
    }
    result
}

fn push_unique_point(points: &mut Vec<Vector3>, point: Vector3) {
    if points.last().map(|last| (*last - point).length_squared() < 0.0001).unwrap_or(false) { return; }
    points.push(point);
}

pub(super) fn emit_polyline_fill(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    points: &[Vector3],
    half_width: f32,
    start_half_width: f32,
    end_half_width: f32,
    start_side_override: Option<Vector2>,
    end_side_override: Option<Vector2>,
    y_offset: f32,
    color: Color,
) {
    if points.len() < 2 || half_width <= 0.0 { return; }
    let sections = build_polyline_sections(points, half_width, start_half_width, end_half_width, start_side_override, end_side_override, y_offset);
    if sections.len() < 2 { return; }
    let mut travelled = 0.0_f32;
    for idx in 0..sections.len() - 1 {
        let delta = Vector2::new(points[idx + 1].x - points[idx].x, points[idx + 1].z - points[idx].z);
        let length = delta.length();
        if length < MIN_SEGMENT_LEN { continue; }
        let uvs = if color.a > 0.9 {
            [Vector2::new(travelled, 1.0), Vector2::new(travelled, 1.0), Vector2::new(travelled + length, 1.0), Vector2::new(travelled + length, 1.0)]
        } else {
            [Vector2::new(travelled, 0.0), Vector2::new(travelled, 1.0), Vector2::new(travelled + length, 1.0), Vector2::new(travelled + length, 0.0)]
        };
        push_quad(mesh, layer, [sections[idx].0, sections[idx].1, sections[idx + 1].1, sections[idx + 1].0], uvs, color);
        travelled += length;
    }
}

fn build_polyline_sections(
    points: &[Vector3],
    half_width: f32,
    start_half_width: f32,
    end_half_width: f32,
    start_side_override: Option<Vector2>,
    end_side_override: Option<Vector2>,
    y_offset: f32,
) -> Vec<(Vector3, Vector3)> {
    let mut sections = Vec::with_capacity(points.len());
    for idx in 0..points.len() {
        let point_half_width = if idx == 0 { start_half_width } else if idx == points.len() - 1 { end_half_width } else { half_width };
        let side = if idx == 0 {
            start_side_override.unwrap_or_else(|| polyline_side_at(points, idx))
        } else if idx == points.len() - 1 {
            end_side_override.unwrap_or_else(|| polyline_side_at(points, idx))
        } else {
            polyline_side_at(points, idx)
        } * point_half_width;
        sections.push((lifted_offset(points[idx], side, y_offset), lifted_offset(points[idx], -side, y_offset)));
    }
    sections
}

pub(super) fn polyline_side_at(points: &[Vector3], idx: usize) -> Vector2 {
    let prev_dir = if idx > 0 { direction_xz(points[idx] - points[idx - 1]) } else { None };
    let next_dir = if idx + 1 < points.len() { direction_xz(points[idx + 1] - points[idx]) } else { None };
    match (prev_dir, next_dir) {
        (Some(prev), Some(next)) => miter_side_for_dirs(prev, next),
        (Some(prev), None) => Vector2::new(-prev.y, prev.x),
        (None, Some(next)) => Vector2::new(-next.y, next.x),
        (None, None) => Vector2::ZERO,
    }
}

pub(super) fn miter_side_for_dirs(prev: Vector2, next: Vector2) -> Vector2 {
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

fn direction_xz(delta: Vector3) -> Option<Vector2> {
    let flat = Vector2::new(delta.x, delta.z);
    let length = flat.length();
    if length < MIN_SEGMENT_LEN { None } else { Some(flat / length) }
}

pub(super) fn emit_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    edge: &Edge,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
) {
    let points = edge_points(edge);
    if points.len() < 2 { return; }
    let total_lanes = edge.fwd_lanes as usize + edge.bkw_lanes as usize;
    if total_lanes <= 1 { return; }
    let start_trim = junction_fill::edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, true, road_half_width(edge));
    let end_trim = junction_fill::edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, false, road_half_width(edge));
    let total_length = polyline_length(points);
    if total_length <= start_trim + end_trim + 0.5 { return; }
    let mut marking_specs = Vec::new();
    for divider in 1..total_lanes {
        let offset = -road_half_width(edge) + divider as f32 * LANE_WIDTH;
        let is_center = edge.fwd_lanes > 0 && edge.bkw_lanes > 0 && divider == edge.bkw_lanes as usize;
        marking_specs.push((offset, is_center));
    }
    for (offset, is_center) in marking_specs {
        emit_marking_polyline(mesh, points, start_trim, total_length - end_trim, offset, MARKING_WIDTH * 0.5, if is_center { marking_center_color() } else { marking_dash_color() });
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
    if end_distance <= start_distance || points.len() < 2 { return; }
    let mut travelled = 0.0;
    for segment in points.windows(2) {
        let start = segment[0];
        let end = segment[1];
        let delta = Vector2::new(end.x - start.x, end.z - start.z);
        let segment_length = delta.length();
        if segment_length < MIN_SEGMENT_LEN { continue; }
        let seg_start = travelled;
        let seg_end = travelled + segment_length;
        let clip_start = start_distance.max(seg_start);
        let clip_end = end_distance.min(seg_end);
        if clip_end <= clip_start { travelled = seg_end; continue; }
        let local_a = (clip_start - seg_start) / segment_length;
        let local_b = (clip_end - seg_start) / segment_length;
        emit_marking_segment(mesh, start.lerp(end, local_a), start.lerp(end, local_b), lateral_offset, half_width, clip_start, clip_end, color);
        travelled = seg_end;
    }
}

fn emit_marking_segment(mesh: &mut NetworkMeshData, start: Vector3, end: Vector3, lateral_offset: f32, half_width: f32, uv_start: f32, uv_end: f32, color: Color) {
    let delta = Vector2::new(end.x - start.x, end.z - start.z);
    let length = delta.length();
    if length < MIN_SEGMENT_LEN { return; }
    let tangent = delta / length;
    let side = Vector2::new(-tangent.y, tangent.x);
    let center_start = lifted_offset(start, side * lateral_offset, MARKING_LAYER_Y);
    let center_end = lifted_offset(end, side * lateral_offset, MARKING_LAYER_Y);
    let eo = side * half_width;
    let a_l = Vector3::new(center_start.x + eo.x, center_start.y, center_start.z + eo.y);
    let a_r = Vector3::new(center_start.x - eo.x, center_start.y, center_start.z - eo.y);
    let b_l = Vector3::new(center_end.x + eo.x, center_end.y, center_end.z + eo.y);
    let b_r = Vector3::new(center_end.x - eo.x, center_end.y, center_end.z - eo.y);
    push_quad(mesh, MeshLayer::Marking, [a_l, a_r, b_r, b_l], [Vector2::new(uv_start, 1.0), Vector2::new(uv_start, 1.0), Vector2::new(uv_end, 1.0), Vector2::new(uv_end, 1.0)], color);
}
