//! Node polygon fills and junction topology rendering.

use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{TransitFlags, TransitType};
use godot::prelude::*;
use std::collections::HashMap;
use std::f32::consts::TAU;

use super::*;

pub(super) fn edge_endpoint_trim_distance(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    edge: &Edge,
    at_start: bool,
    half_width: f32,
) -> f32 {
    let node_id = graph.get_valid_node(if at_start { edge.start_node } else { edge.end_node });
    let state = node_states.get(&node_id).copied().unwrap_or_default();
    if state.kind == NodeRenderKind::PassThrough && is_sidewalk_pass_through_node(graph, node_incidents, node_id) {
        if edge.primary_type == TransitType::Foot { return state.outer_radius.max(half_width); }
        return 0.0;
    }
    match state.kind {
        NodeRenderKind::Junction if node_uses_polygon_fill(graph, node_states, node_incidents, node_id) => {
            let clip = if at_start { edge.start_clip } else { edge.end_clip };
            clip.max(half_width)
        }
        _ => 0.0,
    }
}

pub(super) fn node_uses_polygon_fill(
    graph: &RegionGraph,
    node_states: &HashMap<u32, NodeRenderState>,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    node_id: u32,
) -> bool {
    let node_id = graph.get_valid_node(node_id);
    if node_states.get(&node_id).copied().unwrap_or_default().kind != NodeRenderKind::Junction { return false; }
    let Some(incidents) = node_incidents.get(&node_id) else { return false; };
    if incidents.len() >= 3 { return true; }
    if incidents.len() != 2 { return false; }
    let (mut min_w, mut max_w) = (f32::INFINITY, 0.0_f32);
    for incident in incidents {
        let width = graph.edges[incident.edge_idx].width;
        min_w = min_w.min(width);
        max_w = max_w.max(width);
    }
    (max_w - min_w).abs() > 0.1
}

pub(super) fn is_sidewalk_pass_through_node(
    graph: &RegionGraph,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    node_id: u32,
) -> bool {
    let node_id = graph.get_valid_node(node_id);
    let Some(incidents) = node_incidents.get(&node_id) else { return false; };
    if incidents.len() < 3 { return false; }
    let (mut road_incs, mut has_foot) = (Vec::new(), false);
    for incident in incidents {
        let edge = &graph.edges[incident.edge_idx];
        if road_supports_sidewalk(edge) { road_incs.push(*incident); }
        else if edge.primary_type == TransitType::Foot && (edge.allowed_types & TransitFlags::FOOT != 0) { has_foot = true; }
        else { return false; }
    }
    if !has_foot || road_incs.len() != 2 { return false; }
    let first_edge = &graph.edges[road_incs[0].edge_idx];
    let second_edge = &graph.edges[road_incs[1].edge_idx];
    if (road_half_width(first_edge) - road_half_width(second_edge)).abs() > 0.1 { return false; }
    let Some(d1) = direction_at_endpoint(edge_points(first_edge), road_incs[0].at_start) else { return false; };
    let Some(d2) = direction_at_endpoint(edge_points(second_edge), road_incs[1].at_start) else { return false; };
    d1.dot(d2) <= PASS_THROUGH_DOT
}

pub(super) fn emit_node_fill_polygon(
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
    if boundary.len() < 3 { return; }
    let center = graph.nodes[node_id as usize].pos;
    let center = Vector3::new(center.x, center.y + y_offset, center.z);
    let center_uv = if color.a > 0.9 { Vector2::new(0.0, 1.0) } else { Vector2::ZERO };
    let rim_uv = if color.a > 0.9 { Vector2::new(1.0, 1.0) } else { Vector2::ZERO };
    for idx in 0..boundary.len() {
        let current = Vector3::new(boundary[idx].point.x, boundary[idx].point.y + y_offset, boundary[idx].point.z);
        let next = Vector3::new(boundary[(idx + 1) % boundary.len()].point.x, boundary[(idx + 1) % boundary.len()].point.y + y_offset, boundary[(idx + 1) % boundary.len()].point.z);
        push_triangle(mesh, layer, [center, current, next], [center_uv, rim_uv, rim_uv], color);
    }
}

pub(super) fn emit_polygon_fill(mesh: &mut NetworkMeshData, layer: MeshLayer, points: &[Vector3], y_offset: f32, color: Color) {
    let mut boundary = points.to_vec();
    boundary.dedup_by(|a, b| (*a - *b).length_squared() < 0.0001);
    if boundary.len() < 3 { return; }
    let mut center = Vector3::ZERO;
    for point in &boundary { center += *point; }
    center /= boundary.len() as f32;
    boundary.sort_by(|a, b| {
        let angle_a = (a.z - center.z).atan2(a.x - center.x);
        let angle_b = (b.z - center.z).atan2(b.x - center.x);
        angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
    });
    boundary.dedup_by(|a, b| (*a - *b).length_squared() < 0.0001);
    if boundary.len() < 3 { return; }
    if polygon_signed_area_points_xz(&boundary) < 0.0 { boundary.reverse(); }
    let center = Vector3::new(center.x, center.y + y_offset, center.z);
    let center_uv = if color.a > 0.9 { Vector2::new(0.0, 1.0) } else { Vector2::ZERO };
    let rim_uv = if color.a > 0.9 { Vector2::new(1.0, 1.0) } else { Vector2::ZERO };
    for idx in 0..boundary.len() {
        let current = Vector3::new(boundary[idx].x, boundary[idx].y + y_offset, boundary[idx].z);
        let next = Vector3::new(boundary[(idx + 1) % boundary.len()].x, boundary[(idx + 1) % boundary.len()].y + y_offset, boundary[(idx + 1) % boundary.len()].z);
        push_triangle(mesh, layer, [center, current, next], [center_uv, rim_uv, rim_uv], color);
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
    let Some(incidents) = node_incidents.get(&node_id) else { return Vec::new(); };
    let mut boundary = Vec::with_capacity(incidents.len() * 2);
    let center = graph.nodes[node_id as usize].pos;
    for incident in incidents {
        let edge = &graph.edges[incident.edge_idx];
        let hw = if outer { sidewalk_surface_half_width(edge) } else { road_half_width(edge) };
        let trim = edge_endpoint_trim_distance(graph, node_states, node_incidents, edge, incident.at_start, hw);
        let Some((section_center, direction)) = endpoint_section(edge_points(edge), incident.at_start, trim) else { continue; };
        let side = Vector2::new(-direction.y, direction.x) * hw;
        let left = lifted_offset(section_center, side, 0.0);
        let right = lifted_offset(section_center, -side, 0.0);
        boundary.push(BoundaryPoint { angle: (left.z - center.z).atan2(left.x - center.x), point: left });
        boundary.push(BoundaryPoint { angle: (right.z - center.z).atan2(right.x - center.x), point: right });
    }
    boundary.sort_by(|a, b| a.angle.partial_cmp(&b.angle).unwrap_or(std::cmp::Ordering::Equal));
    collapse_boundary_rays(&mut boundary, center);
    boundary.dedup_by(|a, b| (a.point - b.point).length_squared() < 0.0001);
    if boundary.len() >= 2 && (boundary[0].point - boundary[boundary.len() - 1].point).length_squared() < 0.0001 { boundary.pop(); }
    if polygon_signed_area_xz(&boundary) < 0.0 { boundary.reverse(); }
    boundary
}

pub(super) fn collapse_boundary_rays(boundary: &mut Vec<BoundaryPoint>, center: Vector3) {
    const ANGLE_EPSILON: f32 = 0.001;
    if boundary.len() < 2 { return; }
    let mut collapsed: Vec<BoundaryPoint> = Vec::with_capacity(boundary.len());
    for point in boundary.iter().copied() {
        if let Some(last) = collapsed.last_mut() {
            if (point.angle - last.angle).abs() <= ANGLE_EPSILON {
                if boundary_distance_sq(point, center) > boundary_distance_sq(*last, center) { *last = point; }
                continue;
            }
        }
        collapsed.push(point);
    }
    if collapsed.len() >= 2 {
        let wrap_delta = (collapsed[0].angle + TAU - collapsed[collapsed.len() - 1].angle).min(collapsed[collapsed.len() - 1].angle + TAU - collapsed[0].angle);
        if wrap_delta <= ANGLE_EPSILON {
            let last = collapsed.pop().unwrap();
            if boundary_distance_sq(last, center) > boundary_distance_sq(collapsed[0], center) { collapsed[0] = last; }
        }
    }
    *boundary = collapsed;
}

fn boundary_distance_sq(point: BoundaryPoint, center: Vector3) -> f32 { (point.point - center).length_squared() }

fn polygon_signed_area_xz(points: &[BoundaryPoint]) -> f32 {
    if points.len() < 3 { return 0.0; }
    let mut area = 0.0_f32;
    for idx in 0..points.len() {
        let current = points[idx].point;
        let next = points[(idx + 1) % points.len()].point;
        area += current.x * next.z - current.z * next.x;
    }
    area * 0.5
}

fn polygon_signed_area_points_xz(points: &[Vector3]) -> f32 {
    if points.len() < 3 { return 0.0; }
    let mut area = 0.0_f32;
    for idx in 0..points.len() {
        let c = points[idx];
        let n = points[(idx + 1) % points.len()];
        area += c.x * n.z - c.z * n.x;
    }
    area * 0.5
}

fn endpoint_section(points: &[Vector3], at_start: bool, trim: f32) -> Option<(Vector3, Vector2)> {
    let trimmed = if at_start { road_strip::trimmed_polyline(points, trim, 0.0) } else { road_strip::trimmed_polyline(points, 0.0, trim) };
    if trimmed.len() < 2 { return None; }
    if at_start {
        let d = direction_xz(trimmed[1] - trimmed[0])?;
        Some((trimmed[0], d))
    } else {
        let li = trimmed.len() - 1;
        let d = direction_xz(trimmed[li - 1] - trimmed[li])?;
        Some((trimmed[li], d))
    }
}

fn direction_xz(delta: Vector3) -> Option<Vector2> {
    let flat = Vector2::new(delta.x, delta.z);
    let len = flat.length();
    if len < MIN_SEGMENT_LEN { None } else { Some(flat / len) }
}

pub(super) fn emit_sidewalk_pass_through_aprons(mesh: &mut NetworkMeshData, graph: &RegionGraph, node_states: &HashMap<u32, NodeRenderState>, node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>) {
    for (&node_id, state) in node_states {
        if state.kind != NodeRenderKind::PassThrough { continue; }
        let Some(incidents) = node_incidents.get(&node_id) else { continue; };
        let Some((road_incs, foot_incs)) = sidewalk_pass_through_components(graph, incidents) else { continue; };
        for foot_inc in foot_incs {
            if let Some(poly) = sidewalk_apron_polygon(graph, node_id, &road_incs, foot_inc, *state) {
                emit_polygon_fill(mesh, MeshLayer::Sidewalk, &poly, SIDEWALK_LAYER_Y, sidewalk_color());
            }
        }
    }
}

pub(super) fn sidewalk_pass_through_components(graph: &RegionGraph, incidents: &[IncidentEdgeEndpoint]) -> Option<([IncidentEdgeEndpoint; 2], Vec<IncidentEdgeEndpoint>)> {
    let (mut road_incs, mut foot_incs) = (Vec::new(), Vec::new());
    for incident in incidents {
        let edge = &graph.edges[incident.edge_idx];
        if road_supports_sidewalk(edge) { road_incs.push(*incident); }
        else if edge.primary_type == TransitType::Foot && (edge.allowed_types & TransitFlags::FOOT != 0) { foot_incs.push(*incident); }
        else { return None; }
    }
    if road_incs.len() != 2 || foot_incs.is_empty() { return None; }
    Some(([road_incs[0], road_incs[1]], foot_incs))
}

pub(super) fn sidewalk_apron_polygon(graph: &RegionGraph, node_id: u32, road_incidents: &[IncidentEdgeEndpoint; 2], foot_incident: IncidentEdgeEndpoint, node_state: NodeRenderState) -> Option<[Vector3; 4]> {
    let node_id = graph.get_valid_node(node_id);
    let center = graph.nodes[node_id as usize].pos;
    let first_road = &graph.edges[road_incidents[0].edge_idx];
    let road_h = road_half_width(first_road);
    let apron_l = node_state.outer_radius.max(road_h);
    if apron_l <= 0.0 { return None; }
    let road_ax = direction_at_endpoint(edge_points(first_road), road_incidents[0].at_start)?.normalized();
    let road_norm = Vector2::new(-road_ax.y, road_ax.x);
    let foot_edge = &graph.edges[foot_incident.edge_idx];
    let foot_h = sidewalk_surface_half_width(foot_edge);
    let trim = node_state.outer_radius.max(foot_h);
    let (foot_c, foot_d) = endpoint_section(edge_points(foot_edge), foot_incident.at_start, trim)?;
    let ss = if foot_d.dot(road_norm) >= 0.0 { 1.0 } else { -1.0 };
    let sel_norm = road_norm * ss;
    let foot_side = Vector2::new(-foot_d.y, foot_d.x) * foot_h;
    let mut foot_pts = [lifted_offset(foot_c, foot_side, 0.0), lifted_offset(foot_c, -foot_side, 0.0)];
    sort_pair_by_axis(&mut foot_pts, center, road_ax);
    let mut curb_pts = [Vector3::ZERO; 2];
    for (idx, incident) in road_incidents.iter().enumerate() {
        let d = direction_at_endpoint(edge_points(&graph.edges[incident.edge_idx]), incident.at_start)?;
        let anchor_c = center + Vector3::new(d.x * apron_l, 0.0, d.y * apron_l);
        curb_pts[idx] = lifted_offset(anchor_c, sel_norm * road_h, 0.0);
    }
    sort_pair_by_axis(&mut curb_pts, center, road_ax);
    Some([curb_pts[0], curb_pts[1], foot_pts[1], foot_pts[0]])
}

fn sort_pair_by_axis(points: &mut [Vector3; 2], origin: Vector3, axis: Vector2) {
    if projected_dist_xz(points[0], origin, axis) > projected_dist_xz(points[1], origin, axis) { points.swap(0, 1); }
}

fn projected_dist_xz(point: Vector3, origin: Vector3, axis: Vector2) -> f32 {
    Vector2::new(point.x - origin.x, point.z - origin.z).dot(axis)
}

pub(super) fn pass_through_endpoint_side_override(graph: &RegionGraph, node_states: &HashMap<u32, NodeRenderState>, node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>, edge: &Edge, at_start: bool) -> Option<Vector2> {
    if edge.primary_type != TransitType::Road { return None; }
    let node_id = graph.get_valid_node(if at_start { edge.start_node } else { edge.end_node });
    if node_states.get(&node_id).copied().unwrap_or_default().kind != NodeRenderKind::PassThrough { return None; }
    let incidents = node_incidents.get(&node_id)?;
    let self_idx = incidents.iter().find_map(|inc| {
        if inc.at_start == at_start {
            let candidate = &graph.edges[inc.edge_idx];
            if std::ptr::eq(candidate, edge) { return Some(inc.edge_idx); }
        }
        None
    })?;
    let other_inc = incidents.iter().copied().find(|inc| inc.edge_idx != self_idx && graph.edges[inc.edge_idx].primary_type == TransitType::Road)?;
    let pts = edge_points(edge);
    let self_side = road_strip::polyline_side_at(pts, if at_start { 0 } else { pts.len() - 1 });
    let self_dir = direction_at_endpoint(pts, at_start)?;
    let other_dir = direction_at_endpoint(edge_points(&graph.edges[other_inc.edge_idx]), other_inc.at_start)?;
    let (prev_d, next_d) = if at_start { (-other_dir, self_dir) } else { (-self_dir, other_dir) };
    let mut shared_side = road_strip::miter_side_for_dirs(prev_d, next_d);
    if shared_side.dot(self_side) < 0.0 { shared_side = -shared_side; }
    Some(shared_side)
}

pub(super) fn emit_width_transition_taper(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    graph: &RegionGraph,
    node_id: u32,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
    outer: bool,
    y_offset: f32,
    color: Color,
) {
    let node_id = graph.get_valid_node(node_id);
    let Some(incidents) = node_incidents.get(&node_id) else { return; };
    if incidents.len() != 2 { return; }
    let hwf = |e: &Edge| { if outer { sidewalk_surface_half_width(e) } else { road_half_width(e) } };
    let inc0 = incidents[0];
    let inc1 = incidents[1];
    let hw0 = hwf(&graph.edges[inc0.edge_idx]);
    let hw1 = hwf(&graph.edges[inc1.edge_idx]);
    let (n_inc, n_hw, w_hw) = if hw0 <= hw1 { (inc0, hw0, hw1) } else { (inc1, hw1, hw0) };
    let taper_d = (w_hw - n_hw) * 2.0;
    if taper_d < 0.01 { return; }
    let pts = edge_points(&graph.edges[n_inc.edge_idx]);
    let Some(dir) = direction_at_endpoint(pts, n_inc.at_start) else { return; };
    let Some((t_end, _)) = endpoint_section(pts, n_inc.at_start, taper_d) else { return; };
    let perp = Vector2::new(dir.y, -dir.x);
    let center = graph.nodes[node_id as usize].pos;
    let uv = Vector2::new(1.0, 1.0);
    push_quad(mesh, layer, [lifted_offset(center, perp * w_hw, y_offset), lifted_offset(t_end, perp * n_hw, y_offset), lifted_offset(t_end, perp * -n_hw, y_offset), lifted_offset(center, perp * -w_hw, y_offset)], [uv, uv, uv, uv], color);
}
