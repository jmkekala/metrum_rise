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
//! does every lane a junction's turn whitelist lists. A ring entry's lanes end in the give-way
//! triangle row (`ROAD-62`), just outside the zebra, where its cars stand.

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
use crate::simulation::network::roundabouts::enters_ring;
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
// One lane each way at residential speed stays unmarked, as Finnish residential streets are: a
// road type at 40 km/h or less. The built-in 2-Lane Road (50 km/h) has its dashed centre line
// (the user, 2026-11-05; road types carry their speed since `ROAD-101`).
const UNMARKED_STREET_MAX_KMH: f32 = 40.0;
// The give-way line (Traficom, Liite 15, L2): a row of triangles 0.5 m at the base and 0.6 m long,
// 0.3-0.5 m apart, each pointing at the driver who gives way. Four to a 3.5 m lane at 0.4 m, drawn
// per lane so a lane line runs between two of them. The row's bases lie this far back from the
// mouth: the lanes end there, so a car stands at the row, and the zebra's bars start 0.25 m past
// it.
const GIVE_WAY_TRIANGLE_BASE_M: f32 = 0.5;
const GIVE_WAY_TRIANGLE_LENGTH_M: f32 = 0.6;
const GIVE_WAY_TRIANGLE_GAP_M: f32 = 0.4;
const GIVE_WAY_ROW_BACK_M: f32 = 0.25;

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
        let lines_s_range = emit_give_way_rows(mesh, graph, edge_idx, sections, marking_s_range);
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
        let marking_s_range = lines_s_range;
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
            // kerb being the line before it. Beside a kerb pocket the transition's piece narrows
            // the median (round R5b1a), so the divider starts at the span's mouth.
            let narrowing = PocketNarrowing::on_span(edge).map(|narrowing| match narrowing.end {
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
        // is the centre line: a no-crossing line (K3, solid over the whole range) where several
        // lanes run one way on the road's own cross-section (its pocket lanes not counted) or
        // beside a left turn pocket (`ROAD-95`, the inner pocket lane next to the oncoming lanes),
        // else dashed over the whole range (the user, 2026-11-05).
        let inner_pocket = edge.pocket.is_some_and(|pocket| pocket.inner_outer().0);
        let several_one_way = edge.fwd_lanes - edge.pocket_lanes(true) > 1
            || edge.bkw_lanes - edge.pocket_lanes(false) > 1;
        let centre = if inner_pocket || several_one_way {
            (marking_s_range.0, marking_s_range.0)
        } else {
            marking_s_range
        };
        let (left_lanes, left, right) = if config::DRIVE_ON_LEFT {
            (edge.fwd_lanes, forward, backward)
        } else {
            (edge.bkw_lanes, backward, forward)
        };
        for divider in 1..total_lanes {
            let lane_fraction = divider as f32 / total_lanes as f32;
            let dashed = match divider.cmp(&usize::from(left_lanes)) {
                std::cmp::Ordering::Less => left,
                std::cmp::Ordering::Equal => centre,
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
// sections vertex by vertex. A turn pocket edge's lanes towards its junction carry a second glyph
// where they begin (`pocket_arrow_station`). Left and right are the driver's, so the glyphs need no mirroring
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
    let pocket_node = edge.pocket.map(|pocket| match pocket.end {
        EdgeEnd::Start => edge.start_node,
        EdgeEnd::End => edge.end_node,
    });
    let pocket_back = pocket_arrow_station(range_end - range_start);
    for node in [edge.end_node, edge.start_node] {
        // Travel runs with the stations for forward lanes, against them for backward lanes.
        let (direction, range_end) = if node == edge.end_node {
            (1.0, range_end)
        } else {
            (-1.0, range_start)
        };
        let second = pocket_back.filter(|_| pocket_node == Some(node));
        approach_arrow_masks(graph, edge_idx, node, |lane_idx, mask| {
            let lateral = vehicle_lane_offset(edge, lane_idx);
            for back in std::iter::once(back).chain(second) {
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
            }
        });
    }
}

// The give-way triangle row (`ROAD-62`, D19) across `edge_idx`'s lanes into a ring at either end,
// at the marking range's end on that side, which is the mouth: a ring node is a junction, so its
// arms' ranges end there. The entry's zebra lies past it, on the node's piece, and the lanes stop
// short of the zebra, so the paint is where cars give way. A range too short for the row has none.
// Returns the range the road's lines take, which end at the row's tips as they would at a stop
// line: the range itself on every road without a row. O(degree + lanes x triangles x (log sections
// + bands)) per edge, allocation-free; a road that enters no ring pays the two `enters_ring` tests.
fn emit_give_way_rows(
    mesh: &mut NetworkMeshData,
    graph: &RegionGraph,
    edge_idx: usize,
    sections: &[RoadSection],
    (range_start, range_end): (f32, f32),
) -> (f32, f32) {
    let edge = graph.edge(edge_idx);
    let row_m = GIVE_WAY_ROW_BACK_M + GIVE_WAY_TRIANGLE_LENGTH_M;
    let mut lines = (range_start, range_end);
    if range_end - range_start < 2.0 * row_m {
        return lines;
    }
    for (node, direction, range_end, lanes) in [
        (edge.end_node, 1.0, range_end, edge.fwd_lanes),
        (edge.start_node, -1.0, range_start, edge.bkw_lanes),
    ] {
        if !enters_ring(graph, edge_idx, node) {
            continue;
        }
        let base = range_end - direction * GIVE_WAY_ROW_BACK_M;
        if direction > 0.0 {
            lines.1 = range_end - row_m;
        } else {
            lines.0 = range_end + row_m;
        }
        for rank in 0..lanes as i8 {
            let lane_idx = if direction > 0.0 { rank } else { -rank - 1 };
            let lateral = vehicle_lane_offset(edge, lane_idx);
            // Row coordinates as the arrows': `x` metres to the driver's right, `y` ahead.
            let point = |(x, y): (f32, f32)| {
                point_at(sections, base + direction * y, lateral + direction * x)
            };
            give_way_row(config::LANE_WIDTH, |triangle| {
                if let [Some(a), Some(b), Some(c)] = triangle.map(point) {
                    emit_marking_triangle(mesh, [a, b, c]);
                }
            });
        }
    }
    lines
}

// The triangles of one lane `width` wide, in row coordinates: as many as fit, centred on the lane,
// bases on `y = 0` and tips towards the driver.
fn give_way_row(width: f32, mut fill: impl FnMut([(f32, f32); 3])) {
    let pitch = GIVE_WAY_TRIANGLE_BASE_M + GIVE_WAY_TRIANGLE_GAP_M;
    let count = ((width + GIVE_WAY_TRIANGLE_GAP_M) / pitch).floor();
    let half = GIVE_WAY_TRIANGLE_BASE_M * 0.5;
    for index in 0..count as usize {
        let x = (index as f32 - (count - 1.0) * 0.5) * pitch;
        fill([
            (x - half, 0.0),
            (x, -GIVE_WAY_TRIANGLE_LENGTH_M),
            (x + half, 0.0),
        ]);
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

// The station of a turn pocket edge's second arrow set (`ROAD-95`, the user, 2026-11-05: where a
// driver picks the lane), metres back from the end of a marking range `length` long: 8 m in from
// the range's other end, just past the transition's piece, on a range of 28 m or more, so the two
// glyphs (5 m long) stand at least 3 m apart. A 40 m pocket's range (about 30 m) has it, a 30 m
// pocket's (about 20 m) not.
fn pocket_arrow_station(length: f32) -> Option<f32> {
    (length >= 28.0).then_some(length - 8.0)
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
    use crate::simulation::network::render::road::RoadRenderer;
    use crate::simulation::network::roadbed::fixtures::{self, converted_ring};
    use crate::simulation::terrain::TerrainSystem;

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

    // `ROAD-62` D19: a lane's row is four triangles, inside the lane, each pointing at the driver.
    #[test]
    fn a_give_way_row_fills_a_lane_with_triangles_pointing_back() {
        let mut triangles = Vec::new();
        give_way_row(config::LANE_WIDTH, |triangle| triangles.push(triangle));
        assert_eq!(triangles.len(), 4);
        for [a, tip, c] in triangles.iter().copied() {
            assert!(a.0.abs() < 1.75 && c.0.abs() < 1.75, "inside the lane");
            assert_eq!((a.1, c.1), (0.0, 0.0), "bases on the row's line");
            assert_eq!(tip.1, -GIVE_WAY_TRIANGLE_LENGTH_M);
            assert_eq!(tip.0, (a.0 + c.0) * 0.5);
        }
        let (first, last) = (triangles[0][0].0, triangles[3][2].0);
        assert!((first + last).abs() < 1.0e-5, "centred on the lane");
    }

    // The give-way triangles of a mesh of `graph`: the marking layer's triangles with the row's
    // side lengths, which no arrow or line has.
    fn give_way_triangles(graph: &RegionGraph, roadbed: &RoadbedSystem) -> Vec<[Vector3; 3]> {
        let terrain = TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
        let mesh = RoadRenderer.generate_mesh_data_with_surface(graph, &terrain, roadbed);
        let leg = GIVE_WAY_TRIANGLE_BASE_M.hypot(2.0 * GIVE_WAY_TRIANGLE_LENGTH_M) * 0.5;
        let near = |a: Vector3, b: Vector3, length: f32| (a.distance_to(b) - length).abs() < 1.0e-3;
        // Each as `[base, tip, base]`, however the mesh wound it.
        let (triangles, _) = mesh.marking_vertices.as_chunks::<3>();
        triangles
            .iter()
            .flat_map(|&[a, b, c]| [[a, b, c], [b, c, a], [c, a, b]])
            .filter(|&[a, tip, c]| {
                near(a, c, GIVE_WAY_TRIANGLE_BASE_M) && near(a, tip, leg) && near(c, tip, leg)
            })
            .collect()
    }

    fn compiled(graph: &RegionGraph) -> RoadbedSystem {
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(graph);
        roadbed
    }

    // `ROAD-62` D19: each lane into a ring ends in its four triangles, on the entering half of the
    // arm, tips towards the driver, 0.25 m before the mouth and clear of the zebra's bars.
    #[test]
    fn every_lane_into_a_ring_ends_in_a_triangle_row() {
        for (diameter_m, arms, (out, into)) in [
            (36.0, 3, (1, 1)),
            (36.0, 4, (1, 1)),
            (50.0, 4, (2, 2)),
            (60.0, 5, (1, 2)),
        ] {
            let angles: Vec<_> = (0..arms).map(|i| i as f32 * 360.0 / arms as f32).collect();
            let (graph, applied) = converted_ring(diameter_m, &angles, (out, into), false).unwrap();
            let roadbed = compiled(&graph);
            let found = give_way_triangles(&graph, &roadbed);
            let name = format!("D{diameter_m}, {arms} arms of {out} + {into}");
            assert_eq!(found.len(), 4 * arms * usize::from(into), "{name}");
            for &arm in &applied.arms {
                let edge = graph.edge(arm);
                let ring_at_start = enters_ring(&graph, arm, edge.start_node);
                let sections = roadbed.sections(arm).unwrap();
                let mouth = &sections[if ring_at_start { 0 } else { sections.len() - 1 }];
                let centre = Vector3::new(mouth.center_xz.x as f32, 0.0, mouth.center_xz.y as f32);
                let outward = centre.normalized();
                let right = Vector3::new(outward.z, 0.0, -outward.x);
                let row: Vec<_> = found
                    .iter()
                    .filter(|t| {
                        let from_mouth = t[1] - centre;
                        from_mouth.dot(outward).abs() < 2.0 && from_mouth.dot(right).abs() < 16.0
                    })
                    .collect();
                assert_eq!(row.len(), 4 * usize::from(into), "{name}: arm {arm}");
                for &[a, tip, c] in row {
                    let back = |p: Vector3| (p - centre).dot(outward);
                    assert!((back(a) - GIVE_WAY_ROW_BACK_M).abs() < 1.0e-3, "{name}");
                    assert!((back(c) - GIVE_WAY_ROW_BACK_M).abs() < 1.0e-3, "{name}");
                    let tip_back = GIVE_WAY_ROW_BACK_M + GIVE_WAY_TRIANGLE_LENGTH_M;
                    assert!((back(tip) - tip_back).abs() < 1.0e-3, "{name}: tip at the driver");
                    let across = (tip - centre).dot(right);
                    // The right kerb, then the lanes in; an uneven road's centre is not their edge.
                    let kerb = f32::from(out + into) * config::LANE_WIDTH * 0.5;
                    let lanes_in = kerb - f32::from(into) * config::LANE_WIDTH..kerb;
                    assert!(lanes_in.contains(&across), "{name}: on the lanes in, {across}");
                }
            }
        }
    }

    // An arm's lines end at the row's tips: nothing but the row is painted between them and the
    // mouth, across the whole arm.
    #[test]
    fn an_arms_lines_stop_at_the_triangle_row() {
        let angles = [0.0, 90.0, 180.0, 270.0];
        let (graph, applied) = converted_ring(50.0, &angles, (2, 2), false).unwrap();
        let roadbed = compiled(&graph);
        let terrain = TerrainSystem::with_chunking(256, 256, 1.0, 8, 0.0);
        let mesh = RoadRenderer.generate_mesh_data_with_surface(&graph, &terrain, &roadbed);
        for &arm in &applied.arms {
            let edge = graph.edge(arm);
            let ring_at_start = enters_ring(&graph, arm, edge.start_node);
            let sections = roadbed.sections(arm).unwrap();
            let mouth = &sections[if ring_at_start { 0 } else { sections.len() - 1 }];
            let centre = Vector3::new(mouth.center_xz.x as f32, 0.0, mouth.center_xz.y as f32);
            let outward = centre.normalized();
            let right = Vector3::new(outward.z, 0.0, -outward.x);
            let zone = GIVE_WAY_ROW_BACK_M + GIVE_WAY_TRIANGLE_LENGTH_M - 0.01;
            let in_zone = mesh.marking_vertices.iter().filter(|&&p| {
                let from_mouth = p - centre;
                (-0.5..zone).contains(&from_mouth.dot(outward)) && from_mouth.dot(right).abs() < 8.0
            });
            // Two base corners of each of the two lanes' four triangles.
            assert_eq!(in_zone.count(), 2 * 2 * 4, "arm {arm}");
        }
    }

    // A plain junction and a bare ring have no row, nor has an arm that only leaves a ring.
    #[test]
    fn only_lanes_into_a_ring_have_a_triangle_row() {
        let plain = fixtures::t_junction(false);
        assert!(give_way_triangles(&plain, &compiled(&plain)).is_empty());
        let (bare, _) = fixtures::bare_ring(36.0, 1, false);
        assert!(give_way_triangles(&bare, &compiled(&bare)).is_empty());
        let (mut graph, applied) = fixtures::bare_ring(50.0, 1, false);
        let angle = |graph: &RegionGraph, index: usize| {
            let node = graph.node(applied.nodes[index]).pos;
            node.z.atan2(node.x).to_degrees()
        };
        let (out, into) = (angle(&graph, 0), angle(&graph, 1));
        fixtures::join_ring_arm(&mut graph, out, (1, 0), false);
        let (node, _) = fixtures::join_ring_arm(&mut graph, into, (0, 1), false);
        fixtures::finalize(&mut graph);
        let at = graph.node(node).pos;
        let roadbed = compiled(&graph);
        assert_eq!(roadbed.failed_nodes().len(), 0, "both arms lay out");
        let found = give_way_triangles(&graph, &roadbed);
        assert_eq!(found.len(), 4, "the arm in, not the arm out");
        assert!(found.iter().all(|t| t[1].dot(at.normalized()) > at.length()));
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
