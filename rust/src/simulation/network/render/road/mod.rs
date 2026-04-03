//! Graph-dilation road renderer coordinator.

use crate::config::{ROAD_H_OFFSET, SIDEWALK_WIDTH};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
use godot::prelude::*;
use std::collections::HashMap;

use super::{NetworkMeshData, TransitRenderer};

/// Polyline ribbon generation for asphalt and sidewalks.
pub mod road_strip;
/// Node polygon fills and junction topology.
pub mod junction_fill;
/// Intersection crosswalk markings.
pub mod crosswalks;
/// Terminal cap geometry and circular fills.
pub mod caps;
/// Unit tests for the road renderer.
#[cfg(test)]
pub mod tests;

pub(super) const PASS_THROUGH_DOT: f32 = -0.985;
pub(super) const MIN_SEGMENT_LEN: f32 = 0.01;
pub(super) const SIDEWALK_LAYER_Y: f32 = ROAD_H_OFFSET;
pub(super) const ROAD_LAYER_Y: f32 = ROAD_H_OFFSET + 0.02;
pub(super) const MARKING_LAYER_Y: f32 = ROAD_H_OFFSET + 0.04;
pub(super) const BRIDGE_CONCRETE_Y: f32 = ROAD_H_OFFSET - 0.05;
pub(super) const MARKING_WIDTH: f32 = 0.16;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum NodeRenderKind {
    None,
    Terminal,
    PassThrough,
    WidthTransition,
    Junction,
}

#[derive(Clone, Copy)]
pub(super) struct NodeRenderState {
    pub kind: NodeRenderKind,
    pub road_radius: f32,
    pub outer_radius: f32,
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
pub(super) struct NodeAccum {
    pub degree: usize,
    pub road_radius: f32,
    pub road_radius_min: f32,
    pub outer_radius: f32,
    pub directions: Vec<Vector2>,
}

#[derive(Clone, Copy)]
pub(super) struct IncidentEdgeEndpoint {
    pub edge_idx: usize,
    pub at_start: bool,
}

#[derive(Clone, Copy)]
pub(super) struct BoundaryPoint {
    pub angle: f32,
    pub point: Vector3,
}

#[derive(Clone, Copy)]
pub(super) enum MeshLayer {
    Sidewalk,
    Road,
    Marking,
    Concrete,
}

/// Top-surface road renderer built from strips, disks, and overlays.
pub struct RoadRenderer;

impl TransitRenderer for RoadRenderer {
    fn generate_mesh_data(
        &self,
        graph: &RegionGraph,
        lane_system: &crate::simulation::network::lanes::LaneSystem,
        _terrain: &crate::simulation::terrain::TerrainSystem,
    ) -> NetworkMeshData {
        let road_node_states = build_node_render_states(graph);
        let road_node_incidents = build_surface_node_incidents(graph);
        let sidewalk_node_incidents = build_sidewalk_node_incidents(graph);
        let sidewalk_node_states =
            build_sidewalk_node_render_states(graph, &sidewalk_node_incidents);
        let mut mesh = NetworkMeshData::new();

        // 0. CROSSWALKS
        crosswalks::emit_crosswalk_markings(&mut mesh, lane_system);

        // Pass 1: draw binary base (sidewalk)
        for edge in &graph.edges {
            if edge.deleted { continue; }

            match edge.primary_type {
                TransitType::Road => {
                    if road_supports_sidewalk(edge) {
                        let half_width = outer_half_width(edge);
                        let (points, shw, ehw, sso, eso) = road_strip::edge_render_geometry(
                            graph, &sidewalk_node_states, &sidewalk_node_incidents, edge, half_width, true,
                        );
                        road_strip::emit_polyline_fill(
                            &mut mesh, MeshLayer::Sidewalk, &points, half_width, shw, ehw, sso, eso,
                            SIDEWALK_LAYER_Y, sidewalk_color(),
                        );
                    }
                    if edge.class == EdgeClass::Bridge {
                        let half_width = outer_half_width(edge) + 0.25;
                        let points = road_strip::trimmed_polyline(
                            edge_points(edge),
                            junction_fill::edge_endpoint_trim_distance(graph, &road_node_states, &road_node_incidents, edge, true, half_width),
                            junction_fill::edge_endpoint_trim_distance(graph, &road_node_states, &road_node_incidents, edge, false, half_width),
                        );
                        road_strip::emit_polyline_fill(
                            &mut mesh, MeshLayer::Concrete, &points, half_width, half_width, half_width, None, None,
                            BRIDGE_CONCRETE_Y, concrete_color(),
                        );
                    }
                }
                TransitType::Foot => {
                    let half_width = sidewalk_surface_half_width(edge);
                    let (points, shw, ehw, sso, eso) = road_strip::edge_render_geometry(
                        graph, &sidewalk_node_states, &sidewalk_node_incidents, edge, half_width, true,
                    );
                    road_strip::emit_polyline_fill(
                        &mut mesh, MeshLayer::Sidewalk, &points, half_width, shw, ehw, sso, eso,
                        SIDEWALK_LAYER_Y, sidewalk_color(),
                    );
                }
                _ => {}
            }
        }

        // Pass 1b: caps and junction fills (sidewalk)
        for (node_id, state) in &sidewalk_node_states {
            match state.kind {
                NodeRenderKind::Terminal if state.outer_radius > 0.0 => {
                    caps::emit_disk(&mut mesh, MeshLayer::Sidewalk, graph.nodes[*node_id as usize].pos, state.outer_radius, SIDEWALK_LAYER_Y, sidewalk_color());
                }
                NodeRenderKind::Junction => {
                    if junction_fill::node_uses_polygon_fill(graph, &sidewalk_node_states, &sidewalk_node_incidents, *node_id) {
                        junction_fill::emit_node_fill_polygon(&mut mesh, MeshLayer::Sidewalk, graph, *node_id, &sidewalk_node_states, &sidewalk_node_incidents, true, SIDEWALK_LAYER_Y, sidewalk_color());
                    } else if state.outer_radius > 0.0 {
                        caps::emit_disk(&mut mesh, MeshLayer::Sidewalk, graph.nodes[*node_id as usize].pos, state.outer_radius, SIDEWALK_LAYER_Y, sidewalk_color());
                    }
                }
                NodeRenderKind::WidthTransition => {
                    junction_fill::emit_width_transition_taper(&mut mesh, MeshLayer::Sidewalk, graph, *node_id, &sidewalk_node_incidents, true, SIDEWALK_LAYER_Y, sidewalk_color());
                }
                _ => {}
            }
        }

        // Pass 1c: sidewalk pass-through aprons
        junction_fill::emit_sidewalk_pass_through_aprons(&mut mesh, graph, &sidewalk_node_states, &sidewalk_node_incidents);

        // Pass 2: asphalt overdraw
        for edge in &graph.edges {
            if edge.deleted { continue; }
            if edge.primary_type == TransitType::Road {
                let half_width = road_half_width(edge);
                let (points, shw, ehw, sso, eso) = road_strip::edge_render_geometry(
                    graph, &road_node_states, &road_node_incidents, edge, half_width, false,
                );
                road_strip::emit_polyline_fill(
                    &mut mesh, MeshLayer::Road, &points, half_width, shw, ehw, sso, eso,
                    ROAD_LAYER_Y, road_color(),
                )
            }
        }

        // Pass 2b: road caps and junction fills
        for (node_id, state) in &road_node_states {
            match state.kind {
                NodeRenderKind::Terminal if state.road_radius > 0.0 => {
                    caps::emit_disk(&mut mesh, MeshLayer::Road, graph.nodes[*node_id as usize].pos, state.road_radius, ROAD_LAYER_Y, road_color());
                }
                NodeRenderKind::Junction => {
                    if junction_fill::node_uses_polygon_fill(graph, &road_node_states, &road_node_incidents, *node_id) {
                        junction_fill::emit_node_fill_polygon(&mut mesh, MeshLayer::Road, graph, *node_id, &road_node_states, &road_node_incidents, false, ROAD_LAYER_Y, road_color());
                    } else if state.road_radius > 0.0 {
                        caps::emit_disk(&mut mesh, MeshLayer::Road, graph.nodes[*node_id as usize].pos, state.road_radius, ROAD_LAYER_Y, road_color());
                    }
                }
                NodeRenderKind::WidthTransition => {
                    junction_fill::emit_width_transition_taper(&mut mesh, MeshLayer::Road, graph, *node_id, &road_node_incidents, false, ROAD_LAYER_Y, road_color());
                }
                _ => {}
            }
        }

        // Pass 3: markings
        for edge in &graph.edges {
            if edge.deleted || edge.primary_type != TransitType::Road || edge.class == EdgeClass::Tunnel { continue; }
            road_strip::emit_lane_markings(&mut mesh, graph, edge, &road_node_states, &road_node_incidents);
        }

        mesh
    }
}

pub(super) fn build_node_render_states(graph: &RegionGraph) -> HashMap<u32, NodeRenderState> {
    build_node_render_states_for_edges(graph, visible_surface_road, road_half_width, outer_half_width)
}

pub(super) fn build_sidewalk_node_render_states(
    graph: &RegionGraph,
    node_incidents: &HashMap<u32, Vec<IncidentEdgeEndpoint>>,
) -> HashMap<u32, NodeRenderState> {
    let mut states = build_node_render_states_for_edges(
        graph, visible_sidewalk_surface, sidewalk_surface_half_width, sidewalk_surface_half_width,
    );
    for (&node_id, state) in states.iter_mut() {
        if junction_fill::is_sidewalk_pass_through_node(graph, node_incidents, node_id) {
            state.kind = NodeRenderKind::PassThrough;
        }
    }
    states
}

fn build_node_render_states_for_edges(
    graph: &RegionGraph,
    include: fn(&Edge) -> bool,
    road_radius_for_edge: fn(&Edge) -> f32,
    outer_radius_for_edge: fn(&Edge) -> f32,
) -> HashMap<u32, NodeRenderState> {
    let mut accum: HashMap<u32, NodeAccum> = HashMap::new();

    for edge in &graph.edges {
        if edge.deleted || !include(edge) { continue; }

        let road_radius = road_radius_for_edge(edge);
        let outer_radius = outer_radius_for_edge(edge);

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
                if info.directions[0].dot(info.directions[1]) <= PASS_THROUGH_DOT {
                    if same_width { NodeRenderKind::PassThrough } else { NodeRenderKind::WidthTransition }
                } else {
                    NodeRenderKind::Junction
                }
            }
            _ => NodeRenderKind::Junction,
        };
        result.insert(node_id, NodeRenderState { kind, road_radius: info.road_radius, outer_radius: info.outer_radius });
    }
    result
}

pub(super) fn build_surface_node_incidents(graph: &RegionGraph) -> HashMap<u32, Vec<IncidentEdgeEndpoint>> {
    build_node_incidents_for_edges(graph, visible_surface_road)
}

pub(super) fn build_sidewalk_node_incidents(graph: &RegionGraph) -> HashMap<u32, Vec<IncidentEdgeEndpoint>> {
    build_node_incidents_for_edges(graph, visible_sidewalk_surface)
}

fn build_node_incidents_for_edges(
    graph: &RegionGraph,
    include: fn(&Edge) -> bool,
) -> HashMap<u32, Vec<IncidentEdgeEndpoint>> {
    let mut incidents = HashMap::new();
    for (edge_idx, edge) in graph.edges.iter().enumerate() {
        if edge.deleted || !include(edge) { continue; }
        let start_node = graph.get_valid_node(edge.start_node);
        incidents.entry(start_node).or_insert_with(Vec::new).push(IncidentEdgeEndpoint { edge_idx, at_start: true });
        let end_node = graph.get_valid_node(edge.end_node);
        if end_node != start_node {
            incidents.entry(end_node).or_insert_with(Vec::new).push(IncidentEdgeEndpoint { edge_idx, at_start: false });
        }
    }
    incidents
}

pub(super) fn edge_points(edge: &Edge) -> &[Vector3] {
    if edge.geometry.len() >= 2 { &edge.geometry } else { &edge.physical_geometry }
}

fn visible_surface_road(edge: &Edge) -> bool {
    edge.primary_type == TransitType::Road && matches!(edge.class, EdgeClass::Standard | EdgeClass::Bridge)
}

fn visible_sidewalk_surface(edge: &Edge) -> bool {
    road_supports_sidewalk(edge) || (edge.primary_type == TransitType::Foot && (edge.allowed_types & TransitFlags::FOOT != 0))
}

pub(super) fn road_supports_sidewalk(edge: &Edge) -> bool {
    visible_surface_road(edge) && (edge.allowed_types & TransitFlags::FOOT != 0)
}

pub(super) fn road_half_width(edge: &Edge) -> f32 {
    edge.width.max(2.0) * 0.5
}

pub(super) fn outer_half_width(edge: &Edge) -> f32 {
    road_half_width(edge) + SIDEWALK_WIDTH
}

pub(super) fn sidewalk_surface_half_width(edge: &Edge) -> f32 {
    if edge.primary_type == TransitType::Foot {
        (edge.width.max(2.0) * 0.5) + 0.4
    } else {
        outer_half_width(edge)
    }
}

pub(super) fn direction_at_endpoint(points: &[Vector3], at_start: bool) -> Option<Vector2> {
    if points.len() < 2 { return None; }
    if at_start {
        let origin = points[0];
        for point in &points[1..] {
            let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN { return Some(delta.normalized()); }
        }
    } else {
        let origin = *points.last().unwrap();
        for point in points[..points.len() - 1].iter().rev() {
            let delta = Vector2::new(point.x - origin.x, point.z - origin.z);
            if delta.length_squared() > MIN_SEGMENT_LEN * MIN_SEGMENT_LEN { return Some(delta.normalized()); }
        }
    }
    None
}

pub(super) fn lifted_offset(point: Vector3, offset_xz: Vector2, y_offset: f32) -> Vector3 {
    Vector3::new(point.x + offset_xz.x, point.y + y_offset, point.z + offset_xz.y)
}

pub(super) fn polyline_length(points: &[Vector3]) -> f32 {
    points.windows(2).map(|segment| {
        let delta = Vector2::new(segment[1].x - segment[0].x, segment[1].z - segment[0].z);
        delta.length()
    }).sum()
}

pub(super) fn push_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    push_triangle(mesh, layer, [vertices[0], vertices[1], vertices[2]], [uvs[0], uvs[1], uvs[2]], color);
    push_triangle(mesh, layer, [vertices[0], vertices[2], vertices[3]], [uvs[0], uvs[2], uvs[3]], color);
}

pub(super) fn push_triangle(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 3],
    uvs: [Vector2; 3],
    color: Color,
) {
    let target = match layer {
        MeshLayer::Sidewalk => (&mut mesh.sidewalk_vertices, &mut mesh.sidewalk_normals, &mut mesh.sidewalk_uvs, &mut mesh.sidewalk_colors),
        MeshLayer::Road => (&mut mesh.road_vertices, &mut mesh.road_normals, &mut mesh.road_uvs, &mut mesh.road_colors),
        MeshLayer::Marking => (&mut mesh.marking_vertices, &mut mesh.marking_normals, &mut mesh.marking_uvs, &mut mesh.marking_colors),
        MeshLayer::Concrete => (&mut mesh.concrete_vertices, &mut mesh.concrete_normals, &mut mesh.concrete_uvs, &mut mesh.concrete_colors),
    };
    for index in 0..3 {
        target.0.push(vertices[index]);
        target.1.push(Vector3::UP);
        target.2.push(uvs[index]);
        target.3.push(color);
    }
}

pub(super) fn road_color() -> Color { Color::from_rgba(0.0, 0.0, 0.0, 0.0) }
pub(super) fn sidewalk_color() -> Color { Color::from_rgba(0.0, 0.0, 0.0, 1.0) }
pub(super) fn concrete_color() -> Color { Color::from_rgba(0.75, 0.75, 0.75, 1.0) }
pub(super) fn marking_center_color() -> Color { Color::from_rgba(0.0, 1.0, 1.0, 0.0) }
pub(super) fn marking_dash_color() -> Color { Color::from_rgba(0.0, 1.0, 0.0, 0.0) }
