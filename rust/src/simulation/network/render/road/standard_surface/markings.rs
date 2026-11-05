// SPDX-License-Identifier: GPL-2.0-only

//! Lane-divider and turn-arrow marking placement along span sections.
//!
//! Zebra crossings sit on node pieces, inside the mouths (`ROAD-68-C`), so span markings run the
//! whole span, except past a pass-through node's crossing strip, which lies on the span (D8).
//! A painted median (`ROAD-80`, R3-c) is paint alone: edge lines and 45 degree hatching across
//! the gap between the lane groups, nosing out in a pointed chevron from the raised medians'
//! nose tips. Before a junction whose approach lanes serve different movements (`ROAD-94`), the
//! dividers between those lanes are solid over the last `APPROACH_SOLID_LENGTH_M`, where no car
//! changes lanes, and each of those lanes carries turn arrows for its movements (`ROAD-75`), as
//! does every lane a junction's turn whitelist lists.

use super::super::{
    MARKING_RENDER_Z_BIAS_M, MIN_SEGMENT_LEN, MeshLayer, NetworkMeshData, NetworkMeshOwner,
    marking_dash_color,
};
use super::coverage::CompiledSurfaceCoverage;
use super::geometry::{
    emit_surface_quad, emit_surface_triangle, section_world_point_at_lateral_offset,
};
use crate::config;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::graph::MedianStyle;
use crate::simulation::network::lanes::rebuild::vehicle_lane_offset;
use crate::simulation::network::lanes::{
    APPROACH_SOLID_LENGTH_M, approach_arrow_masks, approach_needs_lane_choice,
};
use crate::simulation::network::roadbed::{
    BandKind, EdgeEnd, LEFT_POCKET_TAPER_M, NOSE_CLEARANCE_M, PocketNarrowing, RoadSection,
    RoadbedSystem, painted_median_continues,
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
        // Arrows first: a whitelisted single lane carries them too (`ROAD-75`).
        emit_approach_arrows(mesh, graph, edge_idx, sections, marking_s_range);
        if total_lanes <= 1 {
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
        // Each lane group's dashed stations: solid over the last `APPROACH_SOLID_LENGTH_M` before
        // the node it approaches when its lanes must pick their movement there. Forward lanes
        // approach the end node, backward lanes the start node. O(degree) per edge end.
        let forward = (
            marking_s_range.0,
            if approach_needs_lane_choice(graph, edge_idx, edge.end_node) {
                marking_s_range.1 - APPROACH_SOLID_LENGTH_M
            } else {
                marking_s_range.1
            },
        );
        let backward = (
            if approach_needs_lane_choice(graph, edge_idx, edge.start_node) {
                marking_s_range.0 + APPROACH_SOLID_LENGTH_M
            } else {
                marking_s_range.0
            },
            marking_s_range.1,
        );
        if edge.median.is_some() {
            // A divided road's lane groups lie apart (`ROAD-80`): dividers within each group, at
            // its lanes' edges, and no centre line between them.
            let side = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
            let (centre, half) = (edge.lane_group_offset(), edge.median_half_width());
            // A left turn pocket's lane forks off beside the median's narrowing (`ROAD-95`, D3):
            // its divider from the through lane starts where the narrowing ends, the median
            // kerb being the line before it.
            let narrowing = PocketNarrowing::of(edge).map(|narrowing| match narrowing.end {
                EdgeEnd::Start => (true, (sections[0].s_m + LEFT_POCKET_TAPER_M, f32::MAX)),
                EdgeEnd::End => (
                    false,
                    (
                        f32::MIN,
                        sections[sections.len() - 1].s_m - LEFT_POCKET_TAPER_M,
                    ),
                ),
            });
            for (lanes, direction, dashed, fwd) in [
                (edge.fwd_lanes, 1.0, forward, true),
                (edge.bkw_lanes, -1.0, backward, false),
            ] {
                for divider in 1..lanes {
                    let lateral =
                        centre + (half + f32::from(divider) * config::LANE_WIDTH) * side * direction;
                    let (range, dashed) = match narrowing {
                        Some((pocket_fwd, (from, to))) if pocket_fwd == fwd && divider == 1 => (
                            (marking_s_range.0.max(from), marking_s_range.1.min(to)),
                            (dashed.0.max(from), dashed.1.min(to)),
                        ),
                        _ => (marking_s_range, dashed),
                    };
                    emit_lane_marking_sections(mesh, sections, range, dashed, line, |_| {
                        Some(lateral)
                    });
                }
            }
            continue;
        }
        // Undivided, the carriageway holds the left group's lanes, then the right group's (the
        // backward lanes on the left under right-hand traffic); the divider between the groups
        // is the centre line, dashed over the whole range.
        let (left_lanes, left, right) = if config::DRIVE_ON_LEFT {
            (edge.fwd_lanes, forward, backward)
        } else {
            (edge.bkw_lanes, backward, forward)
        };
        for divider in 1..total_lanes {
            let lane_fraction = divider as f32 / total_lanes as f32;
            let dashed = match divider.cmp(&usize::from(left_lanes)) {
                std::cmp::Ordering::Less => left,
                std::cmp::Ordering::Equal => marking_s_range,
                std::cmp::Ordering::Greater => right,
            };
            emit_lane_marking_sections(mesh, sections, marking_s_range, dashed, line, |section| {
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

// The turn arrows (`ROAD-75`) of `edge_idx`'s approach lanes, from `lanes::approach_arrow_masks`
// (graph-only): forward lanes before the end node, backward lanes before the start node. Each
// lane carries one glyph, centred on the lane at `approach_arrow_station` back from the marking
// range's end on its side, which already stops at a mouth or a span-owned crossing strip, and drapes on the
// sections vertex by vertex. Left and right are the driver's, so the glyphs need no mirroring
// under `DRIVE_ON_LEFT`: the lane offsets and the lane ranks behind the masks are mirrored.
// O(degree + lanes x 29 triangles x (log sections + bands)) per edge, allocation-free.
fn emit_approach_arrows(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    edge_idx: usize,
    sections: &[RoadSection],
    (range_start, range_end): (f32, f32),
) {
    let edge = graph.edge(edge_idx);
    let Some(back) = approach_arrow_station(range_end - range_start) else {
        return;
    };
    for node in [edge.end_node, edge.start_node] {
        // Travel runs with the stations for forward lanes, against them for backward lanes.
        let (direction, range_end) = if node == edge.end_node {
            (1.0, range_end)
        } else {
            (-1.0, range_start)
        };
        approach_arrow_masks(graph, edge_idx, node, |lane_idx, mask| {
            let lateral = vehicle_lane_offset(edge, lane_idx);
            let centre = range_end - direction * back;
            // Glyph coordinates: `x` metres to the driver's right, `y` ahead.
            let point = |(x, y): (f32, f32)| {
                point_at(sections, centre + direction * y, lateral + direction * x)
            };
            arrow_glyph(&ARROW_STYLE, mask, |triangle| {
                if let [Some(a), Some(b), Some(c)] = triangle.map(point) {
                    emit_marking_triangle(mesh, [a, b, c]);
                }
            });
        });
    }
}

// The station of an approach's one arrow set (user decision 2026-11-04: one per lane, near the
// junction), metres back from the end of a marking range `length` long: 12 m, inside the 20 m
// solid zone, on a range of 20 m or more; a shorter range holds its arrow halfway, at least 4 m
// back and only where the glyph (5 m long) keeps 1.5 m clear of both range ends, so a range under
// 8 m has none.
fn approach_arrow_station(length: f32) -> Option<f32> {
    if length >= 20.0 {
        return Some(12.0);
    }
    let back = (length * 0.5).max(4.0);
    (back <= length - 4.0).then_some(back)
}

// The turn-arrow glyph's outline table (`ROAD-75`), in glyph metres: `x` to the driver's right,
// `y` ahead. One table is one style; the emitter below draws every movement mask from it.
struct ArrowStyle {
    tail_y: f32,
    tip_y: f32,
    shaft_half: f32,
    head_half: f32,
    head_length: f32,
    // A branch's centreline radius, the station where it leaves the shaft beside a straight
    // arrow, where a turn-only arrow's shaft bends, and the quarter circle's segments.
    branch_radius: f32,
    branch_y: f32,
    turn_y: f32,
    branch_segments: usize,
    // The widest a glyph may reach either side of its lane's centre (lanes are 3.5 m wide).
    max_half_width: f32,
}

// Style A, the European road-marking arrow (user decision 2026-11-04): a filled shaft a third as
// wide as its heads, heads as long as they are wide, and each turn a branch as thick as the shaft
// leaving it a little below the middle in a smooth quarter circle to its own head pointing to the
// side; 5 m from the tail to the straight tip.
const ARROW_STYLE: ArrowStyle = ArrowStyle {
    tail_y: -2.5,
    tip_y: 2.5,
    shaft_half: 0.2,
    head_half: 0.6,
    head_length: 1.2,
    branch_radius: 0.5,
    branch_y: -0.6,
    turn_y: 0.8,
    branch_segments: 6,
    max_half_width: 1.25,
};

// Triangles of the largest glyph (left, straight and right): shaft 2, straight head 1, and per
// branch its arc's quads and its head.
#[cfg(test)]
const ARROW_MAX_TRIANGLES: usize = 3 + 2 * (2 * ARROW_STYLE.branch_segments + 1);

// Emits the glyph of movement bits `mask` (1 left, 2 straight, 4 right) in `style` as filled
// triangles in glyph coordinates, through `fill`: the shaft (up to the straight head, or to the
// bend of a turn-only arrow), the straight head, and a branch per turn. Shifted sideways so its
// extent is centred on the lane, and narrowed only when both turns would reach past
// `max_half_width`. S: 3 triangles; L+S, S+R: 16; L, R: 15; L+R: 28; L+S+R: 29
// (`ARROW_MAX_TRIANGLES`).
fn arrow_glyph(style: &ArrowStyle, mask: u8, mut fill: impl FnMut([(f32, f32); 3])) {
    let straight = mask & 2 != 0;
    let (w, r) = (style.shaft_half, style.branch_radius);
    let branch_tip = r + style.head_length;
    let side_extent = |bit: u8| {
        if mask & bit != 0 {
            branch_tip
        } else if straight {
            style.head_half
        } else {
            w
        }
    };
    let (left, right) = (side_extent(1), side_extent(4));
    let shift = (left - right) * 0.5;
    let scale = (style.max_half_width / ((left + right) * 0.5)).min(1.0);
    let mut emit =
        |triangle: [(f32, f32); 3]| fill(triangle.map(|(x, y)| ((x + shift) * scale, y)));
    let fork_y = if straight {
        style.branch_y
    } else {
        style.turn_y
    };
    let shaft_top = if straight {
        style.tip_y - style.head_length
    } else {
        fork_y
    };
    fill_quad(
        &mut emit,
        [
            (-w, style.tail_y),
            (w, style.tail_y),
            (w, shaft_top),
            (-w, shaft_top),
        ],
    );
    if straight {
        let h = style.head_half;
        emit([(-h, shaft_top), (h, shaft_top), (0.0, style.tip_y)]);
    }
    for (bit, side) in [(1, -1.0), (4, 1.0)] {
        if mask & bit == 0 {
            continue;
        }
        // A quarter circle about (side * r, fork_y) from the shaft (heading ahead) to heading
        // `side`, as a band the shaft's width; its first row spans the shaft exactly.
        let ring = |step: usize, radius: f32| {
            let angle = std::f32::consts::FRAC_PI_2 * step as f32 / style.branch_segments as f32;
            (
                side * (r - radius * angle.cos()),
                fork_y + radius * angle.sin(),
            )
        };
        for step in 0..style.branch_segments {
            fill_quad(
                &mut emit,
                [
                    ring(step, r - w),
                    ring(step, r + w),
                    ring(step + 1, r + w),
                    ring(step + 1, r - w),
                ],
            );
        }
        let (x, y) = (side * r, fork_y + r);
        emit([
            (x, y - style.head_half),
            (x, y + style.head_half),
            (side * branch_tip, y),
        ]);
    }
}

// A quad `[a, b, c, d]` (in order round its edge) as two triangles.
fn fill_quad(emit: &mut impl FnMut([(f32, f32); 3]), [a, b, c, d]: [(f32, f32); 4]) {
    emit([a, b, c]);
    emit([a, c, d]);
}

// One filled marking triangle (solid pattern) lifted by the marking bias, as
// `emit_marking_segment` lifts its quads.
fn emit_marking_triangle(mesh: &mut NetworkMeshData, vertices: [Vector3; 3]) {
    let lift = Vector3::new(0.0, MARKING_RENDER_Z_BIAS_M, 0.0);
    emit_surface_triangle(
        mesh,
        MeshLayer::Marking,
        vertices.map(|vertex| vertex + lift),
        [Vector2::new(0.0, MARKING_PATTERN_SOLID); 3],
        marking_dash_color(),
    );
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
// section: in `line`'s pattern between the `dashed` stations and solid outside them (the solid
// approach zone). A section pair is cut at those stations; a pair inside them is emitted as one
// segment, exactly as without a zone.
fn emit_lane_marking_sections(
    mesh: &mut NetworkMeshData,
    sections: &[RoadSection],
    marking_s_range: (f32, f32),
    dashed: (f32, f32),
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
        let point = |s: f32| {
            start.lerp(
                end,
                ((s - segment_start_s) / (segment_end_s - segment_start_s)).clamp(0.0, 1.0),
            )
        };
        // Solid before `dashed.0`, the line's pattern up to `dashed.1`, solid after. Stations in
        // the wrong order (a range shorter than both zones) leave the whole line solid.
        let mut cuts = [
            clipped_start_s,
            dashed.0.clamp(clipped_start_s, clipped_end_s),
            dashed.1.clamp(clipped_start_s, clipped_end_s),
            clipped_end_s,
        ];
        cuts.sort_by(f32::total_cmp);
        for piece in cuts.windows(2) {
            let (a, b) = (piece[0], piece[1]);
            if b <= a + MIN_SEGMENT_LEN {
                continue;
            }
            let middle = (a + b) * 0.5;
            let pattern = if middle < dashed.0 || middle > dashed.1 {
                MARKING_PATTERN_SOLID
            } else {
                line.pattern
            };
            emit_marking_segment(
                mesh,
                point(a),
                point(b),
                a,
                b,
                line.width_m * 0.5,
                pattern,
                marking_dash_color(),
            );
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    // `ROAD-75` style A: every movement mask's glyph has its fixed triangle count, no degenerate
    // triangle, stays within its 5 m and `max_half_width` of the lane's centre, and is shifted
    // away from a lone branch's side (its tail sits right of centre for a left branch).
    #[test]
    fn arrow_glyphs_are_filled_bounded_and_fixed_size() {
        let style = &ARROW_STYLE;
        for (mask, triangles, tail_side) in [
            (2, 3, 0.0),
            (1 | 2, 16, 1.0),
            (2 | 4, 16, -1.0),
            (1, 15, 1.0),
            (4, 15, -1.0),
            (1 | 4, 28, 0.0),
            (1 | 2 | 4, ARROW_MAX_TRIANGLES, 0.0),
        ] {
            let mut count = 0;
            let mut tail_x = Vec::new();
            arrow_glyph(style, mask, |triangle| {
                count += 1;
                let [a, b, c] = triangle;
                let area = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
                assert!(area.abs() > 1.0e-4, "mask {mask}: a filled triangle");
                for (x, y) in triangle {
                    assert!(
                        x.abs() <= style.max_half_width + 1.0e-4,
                        "mask {mask}: x {x}"
                    );
                    assert!(
                        (style.tail_y..=style.tip_y).contains(&y),
                        "mask {mask}: y {y}"
                    );
                    if y == style.tail_y {
                        tail_x.push(x);
                    }
                }
            });
            assert_eq!(count, triangles, "mask {mask}");
            // The tail's middle, between its two corners.
            let (low, high) = tail_x
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), &x| (lo.min(x), hi.max(x)));
            let tail = (low + high) * 0.5;
            assert!(
                (tail.signum() * f32::from(tail.abs() > 1.0e-4) - tail_side).abs() < 1.0e-6,
                "mask {mask}: tail at {tail}"
            );
        }
    }

    // `ROAD-75`: the one arrow glyph (2.5 m either side of its station) stays 1.5 m clear of both
    // ends of its marking range; 12 m back on a range of 20 m or more, halfway on a shorter one
    // down to 8 m, none below.
    #[test]
    fn arrow_stations_stay_clear_of_the_range_ends() {
        for step in 0..=800 {
            let length = step as f32 * 0.25;
            let station = approach_arrow_station(length);
            if let Some(back) = station {
                assert!(
                    back - 2.5 >= 1.5 && back + 2.5 <= length - 1.5,
                    "range {length}: arrow at {back}"
                );
            }
            let expected = match length {
                l if l >= 20.0 => Some(12.0),
                l if l >= 8.0 => Some((l * 0.5).max(4.0)),
                _ => None,
            };
            assert_eq!(station, expected, "range {length}");
        }
    }
}
