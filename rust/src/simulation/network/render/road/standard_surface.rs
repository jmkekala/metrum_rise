//! Compiled roadbed top-surface and lane-marking rendering.

use crate::config::ROAD_H_OFFSET;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::surface::{
    RoadSurfaceBandKind, RoadSurfaceNodeBoundaryLoop, RoadSurfaceNodePatchClass,
    RoadSurfaceSection, RoadSurfaceSystem,
};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::HashSet;

use super::{
    MARKING_LAYER_Y, MARKING_WIDTH, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData, concrete_color,
    road_color, sidewalk_color,
};

const BAND_EPSILON_M: f32 = 0.001;
const BRIDGE_CONCRETE_THICKNESS_M: f32 = 0.35;
const SIDEWALK_SURFACE_LIFT_M: f32 = ROAD_H_OFFSET;
const ROAD_SURFACE_LIFT_M: f32 = ROAD_H_OFFSET + 0.02;
/// Surface classes replaced by the compiled roadbed renderer.
pub(super) struct CompiledSurfaceCoverage {
    pub edge_indices: HashSet<usize>,
    pub node_ids: HashSet<u32>,
}

pub(super) fn build_compiled_surface_coverage(
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
) -> CompiledSurfaceCoverage {
    let mut node_ids = HashSet::new();
    for node_id in 0..graph.node_count() as u32 {
        let valid = graph.get_valid_node(node_id);
        if road_surface.node_uses_visible_surface(graph, terrain, valid) {
            node_ids.insert(valid);
        }
    }

    let mut edge_indices = HashSet::new();
    for edge_idx in 0..graph.edge_count() {
        let edge = graph.edge(edge_idx);
        if !edge_uses_compiled_surface(edge) {
            continue;
        }
        let Some(sections) = road_surface.compiled_sections().get(&edge_idx) else {
            continue;
        };
        if !road_surface
            .visible_section_ranges_for_edge(graph, terrain, edge_idx, sections)
            .is_empty()
        {
            edge_indices.insert(edge_idx);
        }
    }

    node_ids.retain(|node_id| {
        *node_id as usize >= graph.node_adjacency_count()
            || graph
                .node_adjacency(*node_id)
                .iter()
                .any(|edge_idx| edge_indices.contains(edge_idx))
    });

    CompiledSurfaceCoverage {
        edge_indices,
        node_ids,
    }
}

pub(super) fn emit_compiled_surface_mesh(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let mut edge_indices: Vec<usize> = coverage.edge_indices.iter().copied().collect();
    edge_indices.sort_unstable();
    for edge_idx in edge_indices {
        let edge = graph.edge(edge_idx);
        let Some(sections) = road_surface.compiled_sections().get(&edge_idx) else {
            continue;
        };
        for (start_index, end_index) in
            road_surface.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections)
        {
            if end_index <= start_index {
                continue;
            }
            let visible_sections = &sections[start_index..=end_index];
            emit_compiled_edge_bands(mesh, visible_sections);
            if edge.class == EdgeClass::Bridge {
                emit_compiled_bridge_concrete(mesh, visible_sections);
            }
        }
    }

    let mut node_ids: Vec<u32> = coverage.node_ids.iter().copied().collect();
    node_ids.sort_unstable();
    for node_id in node_ids {
        let Some(patch) = road_surface.compiled_node_patches().get(&node_id) else {
            continue;
        };
        if patch.class == RoadSurfaceNodePatchClass::PassThrough {
            continue;
        }
        for loop_points in &patch.boundary_loops {
            emit_loop_fan(
                mesh,
                MeshLayer::Sidewalk,
                &loop_world_points(loop_points),
                sidewalk_color(),
            );
        }
        for loop_points in &patch.carriageway_boundary_loops {
            emit_loop_fan(
                mesh,
                MeshLayer::Road,
                &loop_world_points(loop_points),
                road_color(),
            );
        }
    }
}

pub(super) fn emit_compiled_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let mut edge_indices: Vec<usize> = coverage.edge_indices.iter().copied().collect();
    edge_indices.sort_unstable();
    for edge_idx in edge_indices {
        let edge = graph.edge(edge_idx);
        if edge.deleted || edge.primary_type != TransitType::Road {
            continue;
        }
        let total_lanes = edge.fwd_lanes as usize + edge.bkw_lanes as usize;
        if total_lanes <= 1 {
            continue;
        }

        let Some(sections) = road_surface.compiled_sections().get(&edge_idx) else {
            continue;
        };
        let ranges =
            road_surface.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections);
        if ranges.is_empty() {
            continue;
        }

        for divider in 1..total_lanes {
            let is_center =
                edge.fwd_lanes > 0 && edge.bkw_lanes > 0 && divider == edge.bkw_lanes as usize;
            let color = if is_center {
                super::marking_center_color()
            } else {
                super::marking_dash_color()
            };
            for (start_index, end_index) in &ranges {
                if *end_index <= *start_index {
                    continue;
                }
                emit_lane_marking_sections(
                    mesh,
                    &sections[*start_index..=*end_index],
                    divider,
                    total_lanes,
                    color,
                );
            }
        }
    }
}

fn edge_uses_compiled_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

fn emit_compiled_edge_bands(mesh: &mut NetworkMeshData, sections: &[RoadSurfaceSection]) {
    if sections.len() < 2 {
        return;
    }

    for pair in sections.windows(2) {
        if pair[0].bands.len() != pair[1].bands.len() {
            continue;
        }
        for (band_a, band_b) in pair[0].bands.iter().zip(&pair[1].bands) {
            let width_a = (band_a.lateral_end_m - band_a.lateral_start_m).abs();
            let width_b = (band_b.lateral_end_m - band_b.lateral_start_m).abs();
            if width_a <= BAND_EPSILON_M && width_b <= BAND_EPSILON_M {
                continue;
            }
            let layer = if band_a.kind == RoadSurfaceBandKind::Carriageway
                && band_b.kind == RoadSurfaceBandKind::Carriageway
            {
                MeshLayer::Road
            } else {
                MeshLayer::Sidewalk
            };
            let color = if layer == MeshLayer::Road {
                road_color()
            } else {
                sidewalk_color()
            };
            let surface_lift_m = surface_lift_for_layer(layer);

            let a0 = lift_surface_point(
                section_boundary_world_point(
                    &pair[0],
                    band_a.lateral_start_m,
                    band_a.height_start_m,
                ),
                surface_lift_m,
            );
            let a1 = lift_surface_point(
                section_boundary_world_point(&pair[0], band_a.lateral_end_m, band_a.height_end_m),
                surface_lift_m,
            );
            let b0 = lift_surface_point(
                section_boundary_world_point(
                    &pair[1],
                    band_b.lateral_start_m,
                    band_b.height_start_m,
                ),
                surface_lift_m,
            );
            let b1 = lift_surface_point(
                section_boundary_world_point(&pair[1], band_b.lateral_end_m, band_b.height_end_m),
                surface_lift_m,
            );
            if triangle_is_too_small(a0, b0, b1) && triangle_is_too_small(a0, b1, a1) {
                continue;
            }

            emit_surface_quad(
                mesh,
                layer,
                [a0, b0, b1, a1],
                [
                    Vector2::new(pair[0].s_m, 0.0),
                    Vector2::new(pair[1].s_m, 0.0),
                    Vector2::new(pair[1].s_m, 1.0),
                    Vector2::new(pair[0].s_m, 1.0),
                ],
                color,
            );
        }
    }
}

fn emit_compiled_bridge_concrete(mesh: &mut NetworkMeshData, sections: &[RoadSurfaceSection]) {
    if sections.len() < 2 {
        return;
    }

    for pair in sections.windows(2) {
        let Some((left_a, right_a)) = outer_surface_bounds(&pair[0]) else {
            continue;
        };
        let Some((left_b, right_b)) = outer_surface_bounds(&pair[1]) else {
            continue;
        };

        let a_left = section_boundary_world_point(
            &pair[0],
            left_a.0,
            left_a.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let a_right = section_boundary_world_point(
            &pair[0],
            right_a.0,
            right_a.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let b_left = section_boundary_world_point(
            &pair[1],
            left_b.0,
            left_b.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        let b_right = section_boundary_world_point(
            &pair[1],
            right_b.0,
            right_b.1 - BRIDGE_CONCRETE_THICKNESS_M,
        );
        if triangle_is_too_small(a_left, b_left, b_right)
            && triangle_is_too_small(a_left, b_right, a_right)
        {
            continue;
        }

        emit_surface_quad(
            mesh,
            MeshLayer::Concrete,
            [a_left, b_left, b_right, a_right],
            [
                Vector2::new(pair[0].s_m, 0.0),
                Vector2::new(pair[1].s_m, 0.0),
                Vector2::new(pair[1].s_m, 1.0),
                Vector2::new(pair[0].s_m, 1.0),
            ],
            concrete_color(),
        );
    }
}

fn emit_lane_marking_sections(
    mesh: &mut NetworkMeshData,
    sections: &[RoadSurfaceSection],
    divider: usize,
    total_lanes: usize,
    color: Color,
) {
    if sections.len() < 2 {
        return;
    }

    let lane_fraction = divider as f32 / total_lanes as f32;
    for pair in sections.windows(2) {
        let Some((left_a, right_a)) = carriageway_bounds(&pair[0]) else {
            continue;
        };
        let Some((left_b, right_b)) = carriageway_bounds(&pair[1]) else {
            continue;
        };
        let lateral_a = left_a + (right_a - left_a) * lane_fraction;
        let lateral_b = left_b + (right_b - left_b) * lane_fraction;
        let Some(start) = section_world_point_at_lateral_offset(&pair[0], lateral_a) else {
            continue;
        };
        let Some(end) = section_world_point_at_lateral_offset(&pair[1], lateral_b) else {
            continue;
        };
        emit_marking_segment(
            mesh,
            start,
            end,
            pair[0].s_m,
            pair[1].s_m,
            MARKING_WIDTH * 0.5,
            color,
        );
    }
}

fn carriageway_bounds(section: &RoadSurfaceSection) -> Option<(f32, f32)> {
    let mut carriageway = section
        .bands
        .iter()
        .filter(|band| band.kind == RoadSurfaceBandKind::Carriageway);
    let first_band = carriageway.next()?;
    let last_band = carriageway.last().unwrap_or(first_band);
    Some((first_band.lateral_start_m, last_band.lateral_end_m))
}

fn outer_surface_bounds(section: &RoadSurfaceSection) -> Option<((f32, f32), (f32, f32))> {
    let first_band = section.bands.first()?;
    let last_band = section.bands.last()?;
    Some((
        (first_band.lateral_start_m, first_band.height_start_m),
        (last_band.lateral_end_m, last_band.height_end_m),
    ))
}

fn section_world_point_at_lateral_offset(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
) -> Option<Vector3> {
    for band in &section.bands {
        let start = band.lateral_start_m.min(band.lateral_end_m);
        let end = band.lateral_start_m.max(band.lateral_end_m);
        if lateral_offset_m < start - BAND_EPSILON_M || lateral_offset_m > end + BAND_EPSILON_M {
            continue;
        }

        let span = (band.lateral_end_m - band.lateral_start_m).abs();
        let t = if span <= BAND_EPSILON_M {
            0.0
        } else {
            ((lateral_offset_m - band.lateral_start_m)
                / (band.lateral_end_m - band.lateral_start_m))
                .clamp(0.0, 1.0)
        };
        let height_m = band.height_start_m + (band.height_end_m - band.height_start_m) * t;
        return Some(section_boundary_world_point(
            section,
            lateral_offset_m,
            height_m,
        ));
    }

    None
}

fn emit_marking_segment(
    mesh: &mut NetworkMeshData,
    start: Vector3,
    end: Vector3,
    uv_start: f32,
    uv_end: f32,
    half_width: f32,
    color: Color,
) {
    let delta = Vector2::new(end.x - start.x, end.z - start.z);
    let length = delta.length();
    if length < MIN_SEGMENT_LEN {
        return;
    }

    let tangent = delta / length;
    let side = Vector2::new(-tangent.y, tangent.x);
    let center_start = start + Vector3::new(0.0, MARKING_LAYER_Y, 0.0);
    let center_end = end + Vector3::new(0.0, MARKING_LAYER_Y, 0.0);
    let eo = side * half_width;
    let a_l = Vector3::new(center_start.x + eo.x, center_start.y, center_start.z + eo.y);
    let a_r = Vector3::new(center_start.x - eo.x, center_start.y, center_start.z - eo.y);
    let b_l = Vector3::new(center_end.x + eo.x, center_end.y, center_end.z + eo.y);
    let b_r = Vector3::new(center_end.x - eo.x, center_end.y, center_end.z - eo.y);
    emit_surface_quad(
        mesh,
        MeshLayer::Marking,
        [a_l, a_r, b_r, b_l],
        [
            Vector2::new(uv_start, 1.0),
            Vector2::new(uv_start, 1.0),
            Vector2::new(uv_end, 1.0),
            Vector2::new(uv_end, 1.0),
        ],
        color,
    );
}

fn emit_loop_fan(mesh: &mut NetworkMeshData, layer: MeshLayer, points: &[Vector3], color: Color) {
    if points.len() < 3 {
        return;
    }

    let surface_lift_m = surface_lift_for_layer(layer);
    let mut center = Vector3::ZERO;
    for point in points {
        center += *point;
    }
    center /= points.len() as f32;
    center = lift_surface_point(center, surface_lift_m);

    for index in 0..points.len() {
        let current = lift_surface_point(points[index], surface_lift_m);
        let next = lift_surface_point(points[(index + 1) % points.len()], surface_lift_m);
        if triangle_is_too_small(center, current, next) {
            continue;
        }
        super::push_triangle(
            mesh,
            layer,
            [center, current, next],
            [
                Vector2::ZERO,
                Vector2::new(1.0, 0.0),
                Vector2::new(1.0, 1.0),
            ],
            color,
        );
    }
}

fn loop_world_points(boundary_loop: &RoadSurfaceNodeBoundaryLoop) -> Vec<Vector3> {
    boundary_loop
        .points
        .iter()
        .map(|point| point.point_world)
        .collect()
}

fn lift_surface_point(point: Vector3, lift_m: f32) -> Vector3 {
    Vector3::new(point.x, point.y + lift_m, point.z)
}

fn surface_lift_for_layer(layer: MeshLayer) -> f32 {
    match layer {
        MeshLayer::Sidewalk => SIDEWALK_SURFACE_LIFT_M,
        MeshLayer::Road => ROAD_SURFACE_LIFT_M,
        MeshLayer::Marking | MeshLayer::Concrete => 0.0,
    }
}

fn section_boundary_world_point(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
    height_m: f32,
) -> Vector3 {
    Vector3::new(
        section.center_xz.x + section.lateral_xz.x * lateral_offset_m,
        height_m,
        section.center_xz.y + section.lateral_xz.y * lateral_offset_m,
    )
}

fn triangle_is_too_small(a: Vector3, b: Vector3, c: Vector3) -> bool {
    let projected_cross = ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs();
    projected_cross <= 0.002
}

fn emit_surface_quad(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    vertices: [Vector3; 4],
    uvs: [Vector2; 4],
    color: Color,
) {
    if !triangle_is_too_small(vertices[0], vertices[1], vertices[2]) {
        super::push_triangle(
            mesh,
            layer,
            [vertices[0], vertices[1], vertices[2]],
            [uvs[0], uvs[1], uvs[2]],
            color,
        );
    }
    if !triangle_is_too_small(vertices[0], vertices[2], vertices[3]) {
        super::push_triangle(
            mesh,
            layer,
            [vertices[0], vertices[2], vertices[3]],
            [uvs[0], uvs[2], uvs[3]],
            color,
        );
    }
}
