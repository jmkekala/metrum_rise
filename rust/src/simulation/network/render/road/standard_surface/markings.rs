//! Lane-divider marking placement and crosswalk-mouth clearance.

use super::super::crosswalks::CROSSWALK_STRIPE_LEN;
use super::super::{
    MARKING_RENDER_Z_BIAS_M, MARKING_WIDTH, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData,
    marking_center_color, marking_dash_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{emit_surface_quad, section_world_point_at_lateral_offset};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::lanes::{Lane, LaneSystem, LaneType};
use crate::simulation::network::surface::{
    RoadSurfaceBandKind, RoadSurfaceSection, RoadSurfaceSystem,
};
use crate::simulation::network::types::TransitType;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Color, Vector2, Vector3};
use std::collections::HashMap;

const LANE_MARKING_CROSSWALK_CLEARANCE_M: f32 = 0.25;
const CROSSWALK_MOUTH_CENTER_MATCH_TOLERANCE_M: f32 = 0.25;

pub(in crate::simulation::network::render::road) fn emit_compiled_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    lane_system: &LaneSystem,
    road_surface: &RoadSurfaceSystem,
    terrain: &TerrainSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    let crosswalk_endpoint_flags =
        lane_marking_crosswalk_endpoint_flags_by_edge(graph, lane_system);
    for &edge_idx in &coverage.edge_indices {
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
                marking_center_color()
            } else {
                marking_dash_color()
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
    let crosswalk_gap_m =
        config::CROSSWALK_INSET + CROSSWALK_STRIPE_LEN * 0.5 + LANE_MARKING_CROSSWALK_CLEARANCE_M;

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

fn crosswalk_lane_center_xz(lane: &Lane) -> Vector2 {
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
    let last_band = carriageway.next_back().unwrap_or(first_band);
    Some((first_band.lateral_start_m, last_band.lateral_end_m))
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
