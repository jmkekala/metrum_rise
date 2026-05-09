//! Compiled roadbed top-surface and lane-marking rendering.

use crate::config::ROAD_TOP_SURFACE_RENDER_Z_BIAS_M;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::surface::{
    RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind, RoadSurfaceSection, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon,
};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::HashSet;

use super::{
    MARKING_RENDER_Z_BIAS_M, MARKING_WIDTH, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData,
    concrete_color, curb_color, earthwork_color, road_color, sidewalk_color,
};

const BAND_EPSILON_M: f32 = 0.001;
const BRIDGE_CONCRETE_THICKNESS_M: f32 = 0.35;
const EARTHWORK_RENDER_Z_BIAS_M: f32 = ROAD_TOP_SURFACE_RENDER_Z_BIAS_M;
const SIDEWALK_RENDER_Z_BIAS_M: f32 = ROAD_TOP_SURFACE_RENDER_Z_BIAS_M;
const ROAD_RENDER_SURFACE_Z_BIAS_M: f32 = ROAD_TOP_SURFACE_RENDER_Z_BIAS_M;
const MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;
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
    for (&edge_idx, _) in road_surface.compiled_visual_span_pieces() {
        if edge_idx < graph.edge_count() && edge_uses_compiled_surface(graph.edge(edge_idx)) {
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
    emit_compiled_earthwork_mesh(mesh, graph, road_surface, terrain, coverage);

    let mut edge_indices: Vec<usize> = coverage.edge_indices.iter().copied().collect();
    edge_indices.sort_unstable();
    for edge_idx in edge_indices {
        let edge = graph.edge(edge_idx);
        let Some(piece) = road_surface.compiled_visual_span_pieces().get(&edge_idx) else {
            continue;
        };
        for polygon in &piece.curb_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Curb, polygon, curb_color());
        }
        for polygon in &piece.curb_vertical_face_polygons {
            emit_vertical_surface_polygon(mesh, polygon, curb_color());
        }
        for polygon in &piece.sidewalk_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Sidewalk, polygon, sidewalk_color());
        }
        for polygon in &piece.road_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Road, polygon, road_color());
        }
        if edge.class == EdgeClass::Bridge {
            let Some(sections) = road_surface.compiled_sections().get(&edge_idx) else {
                continue;
            };
            for (start_index, end_index) in
                road_surface.visible_section_ranges_for_edge(graph, terrain, edge_idx, sections)
            {
                if end_index <= start_index {
                    continue;
                }
                emit_compiled_bridge_concrete(mesh, &sections[start_index..=end_index]);
            }
        }
    }

    let mut node_ids: Vec<u32> = coverage.node_ids.iter().copied().collect();
    node_ids.sort_unstable();
    for node_id in node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        for polygon in &piece.curb_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Curb, polygon, curb_color());
        }
        for polygon in &piece.curb_vertical_face_polygons {
            emit_vertical_surface_polygon(mesh, polygon, curb_color());
        }
        for polygon in &piece.sidewalk_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Sidewalk, polygon, sidewalk_color());
        }
        for polygon in &piece.road_surface_polygons {
            emit_surface_polygon(mesh, MeshLayer::Road, polygon, road_color());
        }
    }
}

fn emit_compiled_earthwork_mesh(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let mut edge_indices: Vec<usize> = coverage.edge_indices.iter().copied().collect();
    edge_indices.sort_unstable();
    for edge_idx in edge_indices {
        let Some(piece) = road_surface.compiled_visual_span_pieces().get(&edge_idx) else {
            continue;
        };
        if road_surface.span_piece_uses_visible_earthwork(piece) {
            emit_structural_earthwork_faces(mesh, &piece.render_earthwork_faces);
        }
    }

    let mut node_ids: Vec<u32> = coverage.node_ids.iter().copied().collect();
    node_ids.sort_unstable();
    for node_id in node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        if road_surface.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            emit_structural_earthwork_faces(mesh, &piece.render_earthwork_faces);
        }
    }
}

fn emit_structural_earthwork_faces(
    mesh: &mut NetworkMeshData,
    faces: &[crate::simulation::network::surface::RoadSurfaceEarthworkRenderFace],
) {
    for face in faces {
        match face.kind {
            RoadSurfaceEarthworkFaceKind::Slope => {
                emit_surface_polygon(mesh, MeshLayer::Earthwork, &face.polygon, earthwork_color());
            }
            RoadSurfaceEarthworkFaceKind::RetainingWall => {
                emit_surface_polygon(mesh, MeshLayer::Concrete, &face.polygon, concrete_color());
            }
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
    let center_start = start + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0);
    let center_end = end + Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0);
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

fn emit_surface_polygon(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
) {
    if polygon.triangles_world.is_empty() {
        return;
    }

    let render_z_bias_m = render_z_bias_for_layer(layer);
    for triangle in &polygon.triangles_world {
        let biased = [
            apply_render_z_bias(triangle[0], render_z_bias_m),
            apply_render_z_bias(triangle[1], render_z_bias_m),
            apply_render_z_bias(triangle[2], render_z_bias_m),
        ];
        if triangle_is_too_small(biased[0], biased[1], biased[2]) {
            continue;
        }
        super::push_triangle(
            mesh,
            layer,
            biased,
            [
                Vector2::ZERO,
                Vector2::new(1.0, 0.0),
                Vector2::new(1.0, 1.0),
            ],
            color,
        );
    }
}

fn emit_vertical_surface_polygon(
    mesh: &mut NetworkMeshData,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
) {
    let [upper_start, lower_start, lower_end, upper_end] = polygon.points_world.as_slice() else {
        return;
    };

    let lower_render_z_bias_m = render_z_bias_for_layer(MeshLayer::Road);
    let upper_render_z_bias_m = render_z_bias_for_layer(MeshLayer::Curb);
    let vertices = [
        apply_render_z_bias(*upper_start, upper_render_z_bias_m),
        apply_render_z_bias(*lower_start, lower_render_z_bias_m),
        apply_render_z_bias(*lower_end, lower_render_z_bias_m),
        apply_render_z_bias(*upper_end, upper_render_z_bias_m),
    ];
    let uvs = [
        Vector2::ZERO,
        Vector2::new(1.0, 0.0),
        Vector2::new(1.0, 1.0),
        Vector2::new(0.0, 1.0),
    ];

    for (triangle, triangle_uvs) in [
        (
            [vertices[0], vertices[1], vertices[2]],
            [uvs[0], uvs[1], uvs[2]],
        ),
        (
            [vertices[0], vertices[2], vertices[3]],
            [uvs[0], uvs[2], uvs[3]],
        ),
    ] {
        if triangle_is_too_small(triangle[0], triangle[1], triangle[2]) {
            continue;
        }
        super::push_triangle_preserving_winding(
            mesh,
            MeshLayer::CurbVertical,
            triangle,
            triangle_uvs,
            color,
        );
    }
}

fn apply_render_z_bias(point: Vector3, render_z_bias_m: f32) -> Vector3 {
    Vector3::new(point.x, point.y + render_z_bias_m, point.z)
}

fn render_z_bias_for_layer(layer: MeshLayer) -> f32 {
    match layer {
        MeshLayer::Earthwork => EARTHWORK_RENDER_Z_BIAS_M,
        MeshLayer::Curb => SIDEWALK_RENDER_Z_BIAS_M,
        MeshLayer::CurbVertical => 0.0,
        MeshLayer::Sidewalk => SIDEWALK_RENDER_Z_BIAS_M,
        MeshLayer::Road => ROAD_RENDER_SURFACE_Z_BIAS_M,
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
    let double_area_squared = (b - a).cross(c - a).length_squared();
    double_area_squared <= MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2 * MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2
}

#[cfg(test)]
mod tests {
    use super::triangle_is_too_small;
    use godot::prelude::Vector3;

    #[test]
    fn renderer_keeps_valid_skinny_surface_triangles() {
        let a = Vector3::new(0.0, 0.0, 0.0);
        let b = Vector3::new(2.0, 0.0, 0.0);
        let c = Vector3::new(2.0, 0.0, 0.0005);

        assert!(
            !triangle_is_too_small(a, b, c),
            "compiled road surfaces must not drop valid millimetre-scale closure triangles"
        );
    }

    #[test]
    fn renderer_drops_degenerate_surface_triangles() {
        let a = Vector3::new(0.0, 0.0, 0.0);
        let b = Vector3::new(1.0, 0.0, 0.0);
        let c = Vector3::new(2.0, 0.0, 0.0);

        assert!(triangle_is_too_small(a, b, c));
    }

    #[test]
    fn renderer_keeps_valid_vertical_earthwork_triangles() {
        let a = Vector3::new(0.0, 0.12, 0.0);
        let b = Vector3::new(1.0, 0.12, 0.0);
        let c = Vector3::new(1.0, 0.0, 0.0);

        assert!(
            !triangle_is_too_small(a, b, c),
            "retaining and wall faces are vertical and must survive render culling"
        );
    }
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
