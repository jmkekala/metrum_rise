//! Compiled roadbed top-surface and lane-marking rendering.

use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::{LaneSystem, LaneType};
use crate::simulation::network::surface::{
    RoadSurfaceBandKind, RoadSurfaceEarthworkFaceKind, RoadSurfaceSection, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, RoadVec3,
};
use crate::simulation::network::types::{EdgeClass, TransitType};
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::{HashMap, HashSet};

use super::{
    MARKING_RENDER_Z_BIAS_M, MARKING_WIDTH, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData,
    concrete_color, curb_color, earthwork_color, road_color, sidewalk_color,
};

const BAND_EPSILON_M: f32 = 0.001;
const BRIDGE_CONCRETE_THICKNESS_M: f32 = 0.35;
const BRIDGE_PIER_GROUND_EMBED_M: f32 = 0.08;
const BRIDGE_PIER_HALF_DEPTH_M: f32 = 0.55;
const BRIDGE_PIER_HALF_WIDTH_M: f32 = 0.55;
const BRIDGE_PIER_MIN_CLEARANCE_M: f32 = 1.0;
const BRIDGE_PIER_SPACING_M: f32 = 28.0;
const LANE_MARKING_CROSSWALK_CLEARANCE_M: f32 = 0.25;
const MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2: f32 = 1.0e-8;
const CROSSWALK_MOUTH_CENTER_MATCH_TOLERANCE_M: f32 = 0.25;
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
        for polygon in &piece.raised_step_face_polygons {
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
                emit_compiled_bridge_concrete(mesh, terrain, &sections[start_index..=end_index]);
            }
        }
    }

    let mut node_ids: Vec<u32> = coverage.node_ids.iter().copied().collect();
    node_ids.sort_unstable();
    for node_id in node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Curb,
            &piece.curb_surface_polygons,
            curb_color(),
        );
        for polygon in &piece.raised_step_face_polygons {
            emit_vertical_surface_polygon(mesh, polygon, curb_color());
        }
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Sidewalk,
            &piece.sidewalk_surface_polygons,
            sidewalk_color(),
        );
        emit_node_top_surface_polygons(
            mesh,
            MeshLayer::Road,
            &piece.road_surface_polygons,
            road_color(),
        );
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
        emit_structural_span_earthwork_faces(mesh, road_surface, &piece.render_earthwork_faces);
    }

    let mut node_ids: Vec<u32> = coverage.node_ids.iter().copied().collect();
    node_ids.sort_unstable();
    for node_id in node_ids {
        let Some(piece) = road_surface.compiled_visual_node_pieces().get(&node_id) else {
            continue;
        };
        if road_surface.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
            emit_structural_node_earthwork_faces(
                mesh,
                graph,
                road_surface,
                terrain,
                node_id,
                piece,
            );
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

fn emit_structural_span_earthwork_faces(
    mesh: &mut NetworkMeshData,
    road_surface: &RoadSurfaceSystem,
    faces: &[crate::simulation::network::surface::RoadSurfaceEarthworkRenderFace],
) {
    for face in faces {
        if !road_surface.span_earthwork_face_uses_visible_earthwork(face) {
            continue;
        }
        emit_structural_earthwork_faces(mesh, std::slice::from_ref(face));
    }
}

fn emit_structural_node_earthwork_faces(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    node_id: u32,
    piece: &crate::simulation::network::surface::RoadSurfaceVisualNodePiece,
) {
    for face in &piece.render_earthwork_faces {
        if !road_surface
            .node_earthwork_face_uses_visible_earthwork(graph, terrain, node_id, piece, face)
        {
            continue;
        }
        emit_structural_earthwork_faces(mesh, std::slice::from_ref(face));
    }
}

pub(super) fn emit_compiled_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    lane_system: &LaneSystem,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let mut edge_indices: Vec<usize> = coverage.edge_indices.iter().copied().collect();
    edge_indices.sort_unstable();
    let crosswalk_endpoint_flags =
        lane_marking_crosswalk_endpoint_flags_by_edge(graph, lane_system);
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
        let marking_s_range = edge_lane_marking_s_range(
            edge,
            sections,
            crosswalk_endpoint_flags
                .get(&edge_idx)
                .copied()
                .unwrap_or((false, false)),
        );
        if marking_s_range.1 <= marking_s_range.0 + MIN_SEGMENT_LEN {
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
                    marking_s_range,
                    color,
                );
            }
        }
    }
}

fn edge_uses_compiled_surface(edge: &Edge) -> bool {
    !edge.deleted && matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
}

fn emit_compiled_bridge_concrete(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    sections: &[RoadSurfaceSection],
) {
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

    emit_compiled_bridge_piers(mesh, terrain, sections);
}

fn emit_compiled_bridge_piers(
    mesh: &mut NetworkMeshData,
    terrain: &TerrainSystem,
    sections: &[RoadSurfaceSection],
) {
    let mut last_emitted_s_m = f32::NEG_INFINITY;
    for (section_index, section) in sections.iter().enumerate() {
        let is_endpoint = section_index == 0 || section_index + 1 == sections.len();
        if !is_endpoint && section.s_m - last_emitted_s_m < BRIDGE_PIER_SPACING_M {
            continue;
        }

        let top_y = section.center_height_m - BRIDGE_CONCRETE_THICKNESS_M;
        let base_y = terrain
            .sample_visual_height_world(section.center_xz.x as f32, section.center_xz.y as f32)
            * config::HEIGHT_SCALE
            - BRIDGE_PIER_GROUND_EMBED_M;
        if top_y - base_y < BRIDGE_PIER_MIN_CLEARANCE_M {
            continue;
        }

        emit_bridge_pier(mesh, section, base_y, top_y);
        last_emitted_s_m = section.s_m;
    }
}

fn emit_bridge_pier(
    mesh: &mut NetworkMeshData,
    section: &RoadSurfaceSection,
    base_y: f32,
    top_y: f32,
) {
    let tangent = Vector2::new(section.tangent_xz.x as f32, section.tangent_xz.y as f32);
    let lateral = Vector2::new(section.lateral_xz.x as f32, section.lateral_xz.y as f32);
    if tangent.length_squared() <= 1e-8 || lateral.length_squared() <= 1e-8 {
        return;
    }

    let tangent = tangent.normalized();
    let lateral = lateral.normalized();
    let center = Vector2::new(section.center_xz.x as f32, section.center_xz.y as f32);
    let tangent_offset = tangent * BRIDGE_PIER_HALF_DEPTH_M;
    let lateral_offset = lateral * BRIDGE_PIER_HALF_WIDTH_M;
    let footprint = [
        center - tangent_offset - lateral_offset,
        center + tangent_offset - lateral_offset,
        center + tangent_offset + lateral_offset,
        center - tangent_offset + lateral_offset,
    ];
    let bottom = footprint.map(|point| Vector3::new(point.x, base_y, point.y));
    let top = footprint.map(|point| Vector3::new(point.x, top_y, point.y));
    let tangent_normal = Vector3::new(tangent.x, 0.0, tangent.y);
    let lateral_normal = Vector3::new(lateral.x, 0.0, lateral.y);

    emit_bridge_pier_quad(
        mesh,
        [top[0], top[1], bottom[1], bottom[0]],
        -lateral_normal,
    );
    emit_bridge_pier_quad(mesh, [top[1], top[2], bottom[2], bottom[1]], tangent_normal);
    emit_bridge_pier_quad(mesh, [top[2], top[3], bottom[3], bottom[2]], lateral_normal);
    emit_bridge_pier_quad(
        mesh,
        [top[3], top[0], bottom[0], bottom[3]],
        -tangent_normal,
    );
    emit_bridge_pier_quad(mesh, [top[0], top[1], top[2], top[3]], Vector3::UP);
}

fn emit_bridge_pier_quad(mesh: &mut NetworkMeshData, vertices: [Vector3; 4], normal: Vector3) {
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
        super::push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::Concrete,
            triangle,
            triangle_uvs,
            concrete_color(),
            normal,
        );
    }
}

fn emit_lane_marking_sections(
    mesh: &mut NetworkMeshData,
    sections: &[RoadSurfaceSection],
    divider: usize,
    total_lanes: usize,
    marking_s_range: (f32, f32),
    color: Color,
) {
    if sections.len() < 2 {
        return;
    }

    let lane_fraction = divider as f32 / total_lanes as f32;
    for pair in sections.windows(2) {
        let segment_start_s = pair[0].s_m;
        let segment_end_s = pair[1].s_m;
        if segment_end_s <= segment_start_s + MIN_SEGMENT_LEN {
            continue;
        }
        let clipped_start_s = segment_start_s.max(marking_s_range.0);
        let clipped_end_s = segment_end_s.min(marking_s_range.1);
        if clipped_end_s <= clipped_start_s + MIN_SEGMENT_LEN {
            continue;
        }

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
        let t_start = ((clipped_start_s - segment_start_s) / (segment_end_s - segment_start_s))
            .clamp(0.0, 1.0);
        let t_end =
            ((clipped_end_s - segment_start_s) / (segment_end_s - segment_start_s)).clamp(0.0, 1.0);
        let clipped_start = start.lerp(end, t_start);
        let clipped_end = start.lerp(end, t_end);
        emit_marking_segment(
            mesh,
            clipped_start,
            clipped_end,
            clipped_start_s,
            clipped_end_s,
            MARKING_WIDTH * 0.5,
            color,
        );
    }
}

fn edge_lane_marking_s_range(
    edge: &Edge,
    sections: &[RoadSurfaceSection],
    crosswalk_endpoint_flags: (bool, bool),
) -> (f32, f32) {
    let total_s = sections.last().map_or(0.0, |section| section.s_m);
    let mut start_s: f32 = 0.0;
    let mut end_s = total_s;
    let (has_start_crosswalk, has_end_crosswalk) = crosswalk_endpoint_flags;
    let crosswalk_gap_m = config::CROSSWALK_INSET
        + super::crosswalks::CROSSWALK_STRIPE_LEN * 0.5
        + LANE_MARKING_CROSSWALK_CLEARANCE_M;

    if has_start_crosswalk {
        start_s = start_s.max(edge.start_clip + crosswalk_gap_m);
    }
    if has_end_crosswalk {
        end_s = end_s.min(total_s - edge.end_clip - crosswalk_gap_m);
    }

    (start_s.clamp(0.0, total_s), end_s.clamp(0.0, total_s))
}

fn lane_marking_crosswalk_endpoint_flags_by_edge(
    graph: &RegionGraph,
    lane_system: &LaneSystem,
) -> HashMap<usize, (bool, bool)> {
    let mut flags_by_edge = HashMap::new();
    for lane in &lane_system.lanes {
        if lane.edge_id != usize::MAX
            || lane.lane_type != LaneType::Foot
            || !lane.is_crosswalk
            || lane.geometry.len() < 2
        {
            continue;
        }

        let node_id = lane.node_id as u32;
        if node_id as usize >= graph.node_adjacency_count() {
            continue;
        }
        let center = crosswalk_lane_center_xz(lane);
        for &edge_idx in graph.node_adjacency(node_id) {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let start_node = graph.get_valid_node(edge.start_node);
            let end_node = graph.get_valid_node(edge.end_node);
            let entry = flags_by_edge.entry(edge_idx).or_insert((false, false));
            if node_id == start_node
                && crosswalk_mouth_center(edge, true).is_some_and(|mouth| {
                    mouth.distance_to(center) <= CROSSWALK_MOUTH_CENTER_MATCH_TOLERANCE_M
                })
            {
                entry.0 = true;
            }
            if node_id == end_node
                && crosswalk_mouth_center(edge, false).is_some_and(|mouth| {
                    mouth.distance_to(center) <= CROSSWALK_MOUTH_CENTER_MATCH_TOLERANCE_M
                })
            {
                entry.1 = true;
            }
        }
    }

    flags_by_edge
}

fn crosswalk_mouth_center(edge: &Edge, from_start: bool) -> Option<Vector2> {
    let distance_m = if from_start {
        edge.start_clip
    } else {
        edge.end_clip
    } + config::CROSSWALK_INSET;
    let point = walk_edge_geometry_from_end(&edge.geometry, distance_m, from_start)?;
    Some(Vector2::new(point.x, point.z))
}

fn crosswalk_lane_center_xz(lane: &crate::simulation::network::lanes::Lane) -> Vector2 {
    let first = lane.geometry.first().copied().unwrap_or(Vector3::ZERO);
    let last = lane.geometry.last().copied().unwrap_or(first);
    Vector2::new((first.x + last.x) * 0.5, (first.z + last.z) * 0.5)
}

fn walk_edge_geometry_from_end(
    points: &[Vector3],
    distance_m: f32,
    from_start: bool,
) -> Option<Vector3> {
    let first = points.first().copied()?;
    let last = points.last().copied()?;
    if distance_m <= 0.0 {
        return Some(if from_start { first } else { last });
    }

    let mut remaining = distance_m;
    if from_start {
        for pair in points.windows(2) {
            let segment_len = pair[0].distance_to(pair[1]);
            if remaining <= segment_len || pair[1] == last {
                let t = if segment_len <= f32::EPSILON {
                    0.0
                } else {
                    (remaining / segment_len).clamp(0.0, 1.0)
                };
                return Some(pair[0].lerp(pair[1], t));
            }
            remaining -= segment_len;
        }
        Some(last)
    } else {
        for index in (1..points.len()).rev() {
            let start = points[index];
            let end = points[index - 1];
            let segment_len = start.distance_to(end);
            if remaining <= segment_len || index == 1 {
                let t = if segment_len <= f32::EPSILON {
                    0.0
                } else {
                    (remaining / segment_len).clamp(0.0, 1.0)
                };
                return Some(start.lerp(end, t));
            }
            remaining -= segment_len;
        }
        Some(first)
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
    emit_surface_polygon_with_group_normal(mesh, layer, polygon, color, None);
}

fn emit_node_top_surface_polygons(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygons: &[RoadSurfaceVisualPolygon],
    color: Color,
) {
    let group_normal = stable_surface_group_normal(polygons);
    for polygon in polygons {
        emit_surface_polygon_with_group_normal(mesh, layer, polygon, color, group_normal);
    }
}

fn emit_surface_polygon_with_group_normal(
    mesh: &mut NetworkMeshData,
    layer: MeshLayer,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
    group_normal: Option<Vector3>,
) {
    if polygon.triangles_world.is_empty() {
        return;
    }

    for triangle in &polygon.triangles_world {
        if !RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(*triangle) {
            continue;
        }
        let triangle = road_triangle_to_render(*triangle);
        if let Some(normal) = group_normal {
            super::push_triangle_with_normal(
                mesh,
                layer,
                triangle,
                world_xz_uvs_for_triangle(triangle),
                color,
                normal,
            );
        } else {
            super::push_triangle(
                mesh,
                layer,
                triangle,
                world_xz_uvs_for_triangle(triangle),
                color,
            );
        }
    }
}

fn emit_vertical_surface_polygon(
    mesh: &mut NetworkMeshData,
    polygon: &RoadSurfaceVisualPolygon,
    color: Color,
) {
    if !polygon.triangles_world.is_empty() {
        for triangle in &polygon.triangles_world {
            let triangle = road_triangle_to_render(*triangle);
            if triangle_is_too_small(triangle[0], triangle[1], triangle[2]) {
                continue;
            }
            let normal = vertical_surface_visible_normal(triangle);
            super::push_triangle_preserving_winding_with_exact_normal(
                mesh,
                MeshLayer::RaisedStep,
                triangle,
                [
                    Vector2::ZERO,
                    Vector2::new(1.0, 0.0),
                    Vector2::new(1.0, 1.0),
                ],
                color,
                normal,
            );
        }
        return;
    }

    let [upper_start, lower_start, lower_end, upper_end] = polygon.points_world.as_slice() else {
        return;
    };

    let vertices = [
        road_vec3_to_render(*upper_start),
        road_vec3_to_render(*lower_start),
        road_vec3_to_render(*lower_end),
        road_vec3_to_render(*upper_end),
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
        let normal = vertical_surface_visible_normal(triangle);
        super::push_triangle_preserving_winding_with_exact_normal(
            mesh,
            MeshLayer::RaisedStep,
            triangle,
            triangle_uvs,
            color,
            normal,
        );
    }
}

fn vertical_surface_visible_normal(triangle: [Vector3; 3]) -> Vector3 {
    -((triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]))
}

fn section_boundary_world_point(
    section: &RoadSurfaceSection,
    lateral_offset_m: f32,
    height_m: f32,
) -> Vector3 {
    Vector3::new(
        (section.center_xz.x + section.lateral_xz.x * f64::from(lateral_offset_m)) as f32,
        height_m,
        (section.center_xz.y + section.lateral_xz.y * f64::from(lateral_offset_m)) as f32,
    )
}

fn road_vec3_to_render(point: RoadVec3) -> Vector3 {
    Vector3::new(point.x as f32, point.y as f32, point.z as f32)
}

fn road_triangle_to_render(triangle: [RoadVec3; 3]) -> [Vector3; 3] {
    [
        road_vec3_to_render(triangle[0]),
        road_vec3_to_render(triangle[1]),
        road_vec3_to_render(triangle[2]),
    ]
}

fn triangle_is_too_small(a: Vector3, b: Vector3, c: Vector3) -> bool {
    let double_area_squared = (b - a).cross(c - a).length_squared();
    double_area_squared <= MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2 * MIN_RENDER_TRIANGLE_DOUBLE_AREA_M2
}

fn stable_surface_group_normal(polygons: &[RoadSurfaceVisualPolygon]) -> Option<Vector3> {
    let mut normal = Vector3::ZERO;
    for polygon in polygons {
        for triangle in &polygon.triangles_world {
            if !RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(*triangle) {
                continue;
            }
            let triangle = road_triangle_to_render(*triangle);
            let mut triangle_normal = (triangle[1] - triangle[0]).cross(triangle[2] - triangle[0]);
            if triangle_normal.y < 0.0 {
                triangle_normal = -triangle_normal;
            }
            normal += triangle_normal;
        }
    }
    (normal.length_squared() > 1e-8).then(|| normal.normalized())
}

fn world_xz_uvs_for_triangle(triangle: [Vector3; 3]) -> [Vector2; 3] {
    [
        Vector2::new(triangle[0].x, triangle[0].z),
        Vector2::new(triangle[1].x, triangle[1].z),
        Vector2::new(triangle[2].x, triangle[2].z),
    ]
}

#[cfg(test)]
mod tests {
    use super::{stable_surface_group_normal, triangle_is_too_small, world_xz_uvs_for_triangle};
    use crate::simulation::network::surface::{
        RoadSurfaceSystem, RoadSurfaceVisualPolygon, RoadVec3,
    };
    use godot::prelude::Vector3;

    #[test]
    fn compiled_surface_renderer_has_no_artificial_top_surface_offset_path() {
        let source = include_str!("standard_surface.rs");
        let render_prefix = concat!("ren", "der_");
        let vertical_offset_token = concat!("b", "ias");
        let forbidden = [
            concat!("ROAD_TOP_SURFACE_RENDER_", "Z_", "BIAS_M").to_owned(),
            format!("{render_prefix}z_{vertical_offset_token}_for_layer"),
            format!("apply_{render_prefix}z_{vertical_offset_token}"),
        ];
        for forbidden in forbidden {
            assert!(
                !source.contains(forbidden.as_str()),
                "compiled road surfaces must render at solved physical coordinates, not through `{forbidden}`"
            );
        }
    }

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
    fn renderer_drops_top_surface_needle_triangles() {
        let triangle = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(3.687, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 0.000002826),
        ];

        assert!(!RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(
            triangle
        ));
    }

    #[test]
    fn renderer_keeps_stable_top_surface_triangles() {
        let triangle = [
            RoadVec3::new(0.0, 0.0, 0.0),
            RoadVec3::new(2.0, 0.0, 0.0),
            RoadVec3::new(0.0, 0.0, 2.0),
        ];

        assert!(RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(
            triangle
        ));
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

    #[test]
    fn renderer_uses_world_xz_uvs_for_compiled_top_surfaces() {
        let triangle = [
            Vector3::new(10.0, 1.0, -2.0),
            Vector3::new(12.5, 1.4, -2.0),
            Vector3::new(12.5, 1.8, 3.5),
        ];
        let uvs = world_xz_uvs_for_triangle(triangle);

        assert_eq!(uvs[0].x, 10.0);
        assert_eq!(uvs[0].y, -2.0);
        assert_eq!(uvs[1].x, 12.5);
        assert_eq!(uvs[1].y, -2.0);
        assert_eq!(uvs[2].x, 12.5);
        assert_eq!(uvs[2].y, 3.5);
    }

    #[test]
    fn renderer_uses_group_normal_for_node_top_surface_slivers() {
        let flat = RoadSurfaceVisualPolygon {
            points_world: Vec::new(),
            triangles_world: vec![
                [
                    RoadVec3::new(0.0, 0.0, 0.0),
                    RoadVec3::new(6.0, 0.0, 0.0),
                    RoadVec3::new(0.0, 0.0, 6.0),
                ],
                [
                    RoadVec3::new(6.0, 0.0, 0.0),
                    RoadVec3::new(6.0, 0.0, 6.0),
                    RoadVec3::new(0.0, 0.0, 6.0),
                ],
            ],
        };
        let skinny_mouth = RoadSurfaceVisualPolygon {
            points_world: Vec::new(),
            triangles_world: vec![[
                RoadVec3::new(0.0, 0.0, 0.0),
                RoadVec3::new(0.002, 2.0, 0.0),
                RoadVec3::new(0.0, 0.0, 0.002),
            ]],
        };

        let normal = stable_surface_group_normal(&[flat, skinny_mouth])
            .expect("dominant node top surface should provide a stable render normal");

        assert!(
            normal.y > 0.99,
            "stable group normal should be dominated by real top surface area, got {normal:?}"
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
