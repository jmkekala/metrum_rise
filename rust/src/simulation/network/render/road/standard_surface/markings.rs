// SPDX-License-Identifier: GPL-2.0-only

//! Lane-divider marking placement along span sections.
//!
//! Zebra crossings sit on node pieces, inside the mouths (`ROAD-68-C`), so span markings run the
//! whole span.

use super::super::{
    MARKING_RENDER_Z_BIAS_M, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData, NetworkMeshOwner,
    marking_dash_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{emit_surface_quad, section_world_point_at_lateral_offset};
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{BandKind, RoadSection, RoadbedSystem};
use crate::simulation::network::types::TransitType;
use godot::prelude::{Color, Vector2, Vector3};
// Traficom, Liite 14 (2023), K0 table: centre line K1 and lane line K2_1 are 1 m dashes with
// 3 m gaps at 50 km/h or less, and 3 m dashes with 9 m gaps above; 0.10 m wide, or 0.15 m
// (K1_3, K2_1_3) above 50 km/h. Every longitudinal line is white since the 2020 Road Traffic Act.
const FINNISH_LINE_SPEED_THRESHOLD_KMH: f32 = 50.0;
const FINNISH_LINE_WIDTH_M: f32 = 0.10;
const FINNISH_FAST_LINE_WIDTH_M: f32 = 0.15;
// Marking UV.y carries the dash pattern for road_marking.gdshader: 2 solid, then 1 m / 3 m,
// then 3 m / 9 m. Zebra paint is now part of the asphalt surface.
const MARKING_PATTERN_SHORT_DASH: f32 = 3.0;
const MARKING_PATTERN_LONG_DASH: f32 = 4.0;
// One lane each way at urban speed stays unmarked, as Finnish residential streets are. The road
// tool has no street hierarchy, so a 1+1 road at 50 km/h (its default) counts as residential.
const UNMARKED_STREET_MAX_KMH: f32 = 50.0;

#[derive(Clone, Copy)]
struct FinnishLaneLine {
    width_m: f32,
    pattern: f32,
}

impl FinnishLaneLine {
    fn for_edge(edge: &Edge) -> Option<Self> {
        let speed_kmh = (edge.speed_limit * 3.6).round();
        if edge.fwd_lanes == 1 && edge.bkw_lanes == 1 && speed_kmh <= UNMARKED_STREET_MAX_KMH {
            return None;
        }
        Some(if speed_kmh <= FINNISH_LINE_SPEED_THRESHOLD_KMH {
            Self {
                width_m: FINNISH_LINE_WIDTH_M,
                pattern: MARKING_PATTERN_SHORT_DASH,
            }
        } else {
            Self {
                width_m: FINNISH_FAST_LINE_WIDTH_M,
                pattern: MARKING_PATTERN_LONG_DASH,
            }
        })
    }
}

pub(in crate::simulation::network::render::road) fn emit_compiled_lane_markings(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
    coverage: &CompiledSurfaceCoverage,
) {
    for &edge_idx in &coverage.edge_indices {
        mesh.set_owner(NetworkMeshOwner::Edge(edge_idx));
        let edge = graph.edge(edge_idx);
        if edge.deleted || edge.primary_type != TransitType::Road {
            continue;
        }
        let total_lanes = edge.fwd_lanes as usize + edge.bkw_lanes as usize;
        if total_lanes <= 1 {
            continue;
        }

        let Some(sections) = roadbed.sections(edge_idx).filter(|s| s.len() >= 2) else {
            continue;
        };
        let marking_s_range = (
            sections[0].s_m,
            sections
                .last()
                .map_or(sections[0].s_m, |section| section.s_m),
        );
        if marking_s_range.1 <= marking_s_range.0 + MIN_SEGMENT_LEN {
            continue;
        }

        let Some(line) = FinnishLaneLine::for_edge(edge) else {
            continue;
        };
        for divider in 1..total_lanes {
            emit_lane_marking_sections(mesh, sections, divider, total_lanes, marking_s_range, line);
        }
    }
}

fn emit_lane_marking_sections(
    mesh: &mut NetworkMeshData,
    sections: &[RoadSection],
    divider: usize,
    total_lanes: usize,
    marking_s_range: (f32, f32),
    line: FinnishLaneLine,
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
            line.width_m * 0.5,
            line.pattern,
            marking_dash_color(),
        );
    }
}

fn carriageway_bounds(section: &RoadSection) -> Option<(f32, f32)> {
    let mut carriageway = section
        .bands
        .iter()
        .filter(|band| band.kind == BandKind::Carriageway);
    let first_band = carriageway.next()?;
    let last_band = carriageway.next_back().unwrap_or(first_band);
    Some((first_band.lateral_start_m, last_band.lateral_end_m))
}

#[allow(clippy::too_many_arguments)]
fn emit_marking_segment(
    mesh: &mut NetworkMeshData,
    start: Vector3,
    end: Vector3,
    uv_start: f32,
    uv_end: f32,
    half_width: f32,
    pattern: f32,
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
            Vector2::new(uv_start, pattern),
            Vector2::new(uv_start, pattern),
            Vector2::new(uv_end, pattern),
            Vector2::new(uv_end, pattern),
        ],
        color,
    );
}
