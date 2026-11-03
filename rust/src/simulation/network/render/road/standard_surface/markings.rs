// SPDX-License-Identifier: GPL-2.0-only

//! Lane-divider marking placement along span sections.
//!
//! Zebra crossings sit on node pieces, inside the mouths (`ROAD-68-C`), so span markings run the
//! whole span, except past a pass-through node's crossing strip, which lies on the span (D8).
//! A painted median (`ROAD-80`, R3-c) is paint alone: edge lines and 45 degree hatching across
//! the gap between the lane groups, nosing out in a pointed chevron from the raised medians'
//! nose tips.

use super::super::{
    MARKING_RENDER_Z_BIAS_M, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData, NetworkMeshOwner,
    marking_dash_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{emit_surface_quad, section_world_point_at_lateral_offset};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::graph::MedianStyle;
use crate::simulation::network::roadbed::{
    BandKind, EdgeEnd, NOSE_CLEARANCE_M, RoadSection, RoadbedSystem,
    painted_median_continues,
};
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
// A painted median's hatching (`ROAD-80`): solid lines (pattern 2) of the urban lane-line width,
// diagonals at 45 degrees every `HATCH_SPACING_M` along the road between its two edge lines.
const MARKING_PATTERN_SOLID: f32 = 2.0;
const HATCH_SPACING_M: f32 = 3.0;
// A painted median's chevron: its edge lines open from the tip to the full width over this
// length. A raised median's nose is a semicircle instead (`roadbed::span`), as a painted nose
// stays pointed in road-marking practice.
const PAINTED_NOSE_LENGTH_M: f32 = 4.0;
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
        let span = roadbed.span_piece(edge_idx);
        let strip = |end: EdgeEnd| span.and_then(|span| span.crossing_strip_m(end));
        let marking_s_range = (
            strip(EdgeEnd::Start).map_or(sections[0].s_m, |(_, high)| high),
            strip(EdgeEnd::End).map_or(
                sections
                    .last()
                    .map_or(sections[0].s_m, |section| section.s_m),
                |(low, _)| low,
            ),
        );
        if marking_s_range.1 <= marking_s_range.0 + MIN_SEGMENT_LEN {
            continue;
        }
        if edge.median.is_some_and(|median| median.style == MedianStyle::Painted) {
            let through = [edge.start_node, edge.end_node]
                .map(|node| painted_median_continues(graph, graph.get_valid_node(node)));
            emit_painted_median(mesh, edge, sections, marking_s_range, through);
        }

        let Some(line) = FinnishLaneLine::for_edge(edge) else {
            continue;
        };
        if edge.median.is_some() {
            // A divided road's lane groups lie apart (`ROAD-80`): dividers within each group, at
            // its lanes' edges, and no centre line between them.
            let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
            let (centre, half) = (edge.lane_group_offset(), edge.median_half_width());
            for (lanes, direction) in [(edge.fwd_lanes, 1.0), (edge.bkw_lanes, -1.0)] {
                for divider in 1..lanes {
                    let lateral =
                        centre + (half + f32::from(divider) * config::LANE_WIDTH) * side * direction;
                    emit_lane_marking_sections(mesh, sections, marking_s_range, line, |_| {
                        Some(lateral)
                    });
                }
            }
            continue;
        }
        for divider in 1..total_lanes {
            let lane_fraction = divider as f32 / total_lanes as f32;
            emit_lane_marking_sections(mesh, sections, marking_s_range, line, |section| {
                let (left, right) = carriageway_bounds(section)?;
                Some(left + (right - left) * lane_fraction)
            });
        }
    }
}

// A painted median's paint between `marking_s_range` (`ROAD-80`): two edge lines at
// `lane_group_offset` +- half the median's width and 45 degree diagonals between them. An end
// where the median runs on through its node (`through`) has no nose; at every other end the
// lines meet on the centre line `NOSE_CLEARANCE_M` inside the range, a raised median's nose tip
// (the range already starts past a crossing strip the span holds), and open in a straight
// chevron to the full width `PAINTED_NOSE_LENGTH_M` further on. Chevrons that would overlap
// leave the span unpainted. O(sections + length / HATCH_SPACING_M), allocation-free.
fn emit_painted_median(
    mesh: &mut NetworkMeshData,
    edge: &Edge,
    sections: &[RoadSection],
    (range_start, range_end): (f32, f32),
    through: [bool; 2],
) {
    let (centre, half) = (edge.lane_group_offset(), edge.median_half_width());
    let tips = [
        (!through[0]).then_some(range_start + NOSE_CLEARANCE_M),
        (!through[1]).then_some(range_end - NOSE_CLEARANCE_M),
    ];
    let (low, high) = (tips[0].unwrap_or(range_start), tips[1].unwrap_or(range_end));
    if let [Some(start), Some(end)] = tips
        && end - start < 2.0 * PAINTED_NOSE_LENGTH_M
    {
        return;
    }
    let (full_low, full_high) = (
        tips[0].map_or(low, |tip| tip + PAINTED_NOSE_LENGTH_M),
        tips[1].map_or(high, |tip| tip - PAINTED_NOSE_LENGTH_M),
    );
    // The half width the paint spans at station `s`: zero at a tip, the full half width beyond
    // the nose.
    let half_at = |s: f32| {
        let rise = |tip: Option<f32>, sign: f32| {
            tip.map_or(1.0, |tip| {
                ((s - tip) * sign / PAINTED_NOSE_LENGTH_M).clamp(0.0, 1.0)
            })
        };
        half * rise(tips[0], 1.0).min(rise(tips[1], -1.0))
    };
    let width = FINNISH_LINE_WIDTH_M * 0.5;
    for side in [-1.0, 1.0] {
        let lateral_at = |s: f32| centre + side * half_at(s);
        // Each section pair, cut at the nose stations so the taper's kinks are exact.
        for pair in sections.windows(2) {
            let (s0, s1) = (pair[0].s_m.max(low), pair[1].s_m.min(high));
            if s1 <= s0 + MIN_SEGMENT_LEN {
                continue;
            }
            let mut cuts = [s0, full_low, full_high, s1];
            cuts[1..3].iter_mut().for_each(|cut| *cut = cut.clamp(s0, s1));
            for piece in cuts.windows(2) {
                let (a, b) = (piece[0], piece[1]);
                if b <= a + MIN_SEGMENT_LEN {
                    continue;
                }
                let (Some(start), Some(end)) = (
                    point_between(pair, a, lateral_at(a)),
                    point_between(pair, b, lateral_at(b)),
                ) else {
                    continue;
                };
                emit_marking_segment(
                    mesh,
                    start,
                    end,
                    a,
                    b,
                    width,
                    MARKING_PATTERN_SOLID,
                    marking_dash_color(),
                );
            }
        }
    }
    // Diagonals over the full width, each rising 2 * half over 2 * half along the road.
    let rise_m = 2.0 * half;
    let mut s = full_low + HATCH_SPACING_M * 0.5;
    while s + rise_m <= full_high {
        if let (Some(start), Some(end)) = (
            point_at(sections, s, centre - half),
            point_at(sections, s + rise_m, centre + half),
        ) {
            emit_marking_segment(
                mesh,
                start,
                end,
                s,
                s + rise_m,
                width,
                MARKING_PATTERN_SOLID,
                marking_dash_color(),
            );
        }
        s += HATCH_SPACING_M;
    }
}

// The point at station `s_m` and `lateral` between the two sections of `pair`, as the surface
// between their rows lies. O(bands).
fn point_between(pair: &[RoadSection], s_m: f32, lateral: f32) -> Option<Vector3> {
    let (a, b) = (&pair[0], &pair[1]);
    let t = ((s_m - a.s_m) / (b.s_m - a.s_m)).clamp(0.0, 1.0);
    let start = section_world_point_at_lateral_offset(a, lateral)?;
    let end = section_world_point_at_lateral_offset(b, lateral)?;
    Some(start.lerp(end, t))
}

// The point at station `s_m` and `lateral` on the span's sections. O(log sections + bands).
fn point_at(sections: &[RoadSection], s_m: f32, lateral: f32) -> Option<Vector3> {
    let next = sections
        .partition_point(|section| section.s_m < s_m)
        .clamp(1, sections.len() - 1);
    point_between(&sections[next - 1..=next], s_m, lateral)
}

// One longitudinal line over the span's sections, at the lateral offset `lateral_at` gives each
// section.
fn emit_lane_marking_sections(
    mesh: &mut NetworkMeshData,
    sections: &[RoadSection],
    marking_s_range: (f32, f32),
    line: FinnishLaneLine,
    lateral_at: impl Fn(&RoadSection) -> Option<f32>,
) {
    if sections.len() < 2 {
        return;
    }

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

        let Some(lateral_a) = lateral_at(&pair[0]) else {
            continue;
        };
        let Some(lateral_b) = lateral_at(&pair[1]) else {
            continue;
        };
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
