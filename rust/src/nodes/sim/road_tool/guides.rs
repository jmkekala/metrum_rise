// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's guide gizmos of round V (`ROAD-112`): the zoning-depth guide and the
//! alignment guides as the player sees them.
//!
//! Zoning depth: a road that zones offers [`CELL_DEPTH`] rows of cells on each side, starting at
//! its kerb line, `width / 2 + SIDEWALK_WIDTH` from the centreline (`frontage_line.rs`
//! `kerb_offset_m`), so the cells' outer edge lies [`zoning_depth_m`] from it. The guide is a
//! circle of that radius around the stroke's start and end and the stroke offset by it on both
//! sides (the guide kernel's exact offset; a side is left out where it is refused: a curve
//! tighter than the depth, a freeform corner). A road drawn along the guide leaves blocks
//! exactly six cells deep between the two roads. Item 9 splits it by each point's distance to
//! the stroke (its flattened polyline, within the flattening rule's 5 cm sag): the outside,
//! where that distance is the depth (the envelope: side offsets and the circles' outer caps),
//! and the inside, nearer than the depth (the circles' parts over the stroke, and any offset a
//! winding stroke brings back over itself). Near an end the distance grows only quadratically
//! towards the cap's edge, so a circle's cap is also bounded exactly by the line through its
//! centre across the end tangent (the half facing away from the stroke). A run ends where the
//! class changes, at the crossing found by bisection, so the two sets meet without a gap.
//!
//! Alignment: a guide line that holds the cursor is drawn from where it comes from (the dead
//! end, node or anchor it is anchored on) through the snapped point and [`GUIDE_PAST_M`] past it.
//!
//! Grid curbs (round V item 8): the zoning-grid snap (`RoadGridSnap`) puts the road's curbs,
//! sidewalks included, on cell lines: [`grid_curb_offset_m`] from the centreline, of the same
//! width the snap is given, so the snapped centreline moves with the road's width. While the
//! snap holds the cursor the two curb lines are drawn along the stroke and on past its end, with
//! a tick across each where it meets the grid line the end snapped to (the snap ends a road half
//! a road past its last full cell line).

use super::frame::{GizmoLineClass, Gizmos};
use super::measure::RoadTypeSpec;
use super::pick::lane_width_m;
use super::snap::GuideLine;
use crate::config::SIDEWALK_WIDTH;
use crate::simulation::network::graph::Edge;
use crate::simulation::network::guide::{DVec2, GuidePath};
use crate::simulation::zoning::cells::CELL_DEPTH;
use godot::prelude::Vector3;

/// Metres an alignment guide runs on past the snapped point.
pub const GUIDE_PAST_M: f64 = 12.0;
// Chords of a zoning-depth circle.
const CIRCLE_CHORDS: usize = 64;
// Half the length of a grid curb's tick, in metres.
const TICK_HALF_M: f64 = 2.0;
// A point nearer to the flattened stroke than the depth by more than this is inside: the
// flattening rule's 5 cm sag and a centimetre, so an exact offset of a curve reads as the
// outside.
const INSIDE_TOLERANCE_M: f64 = 0.06;
// Segments of the stroke per bounding box of the distance query.
const CHUNK_SEGMENTS: usize = 16;
// Bisection steps locating a class change between two guide points (2^-20 of a chord).
const CROSSING_STEPS: usize = 20;

/// Reused buffers of the zoning-depth guide: the offsets, the stroke in XZ and its chunk boxes.
#[derive(Debug, Default)]
pub struct ZoningScratch {
    offset: Vec<DVec2>,
    stroke: Vec<DVec2>,
    boxes: Vec<(DVec2, DVec2)>,
}

/// The distance from the centreline to each curb the zoning-grid snap aligns for a road of
/// `spec`, in metres: half the width the snap is given (`lane_width_m`, lanes times the lane
/// width, at least 2 m) plus the sidewalk, as `RoadGridSnap::snap_road_cursor` reads it. `O(1)`.
pub fn grid_curb_offset_m(spec: &RoadTypeSpec) -> f64 {
    let width = lane_width_m(i32::from(spec.fwd_lanes), i32::from(spec.bkw_lanes));
    f64::from(width) * 0.5 + f64::from(SIDEWALK_WIDTH)
}

/// Pushes the grid curbs of a straight leg from `start` to `end` at `offset` metres either side:
/// each curb from the start to [`GUIDE_PAST_M`] past the end, and a tick across it (along the
/// grid line) `offset` before the end, where the snap put the last cell line. `O(1)`.
pub fn push_grid_curbs(gizmos: &mut Gizmos, start: Vector3, end: Vector3, offset: f64) {
    let (a, b) = (xz(start), xz(end));
    let Some(dir) = (b - a).try_normalize() else {
        return;
    };
    let normal = dir.perp();
    let line = b - dir * offset;
    for side in [offset, -offset] {
        let shift = normal * side;
        gizmos.push_line(
            GizmoLineClass::GridCurb,
            [
                at(a + shift, start.y),
                at(b + dir * GUIDE_PAST_M + shift, end.y),
            ],
        );
        let meet = line + shift;
        gizmos.push_line(
            GizmoLineClass::GridCurb,
            [
                at(meet - normal * TICK_HALF_M, end.y),
                at(meet + normal * TICK_HALF_M, end.y),
            ],
        );
    }
}

/// How far the zoning a road of `spec` generates reaches from its centreline, in metres, on
/// cells of `cell_m`: the kerb offset plus [`CELL_DEPTH`] rows. `None` for a type that does not
/// zone (zoning off, a walkway, no sidewalk on either side). `O(1)`.
pub fn zoning_depth_m(spec: &RoadTypeSpec, cell_m: f64) -> Option<f64> {
    let car_road = spec.fwd_lanes + spec.bkw_lanes > 0;
    if !(spec.attributes.zoning && car_road && spec.attributes.sidewalks.any()) {
        return None;
    }
    let width =
        Edge::cross_section_width_m(spec.fwd_lanes, spec.bkw_lanes, spec.median, spec.parking);
    // Bit for bit `kerb_offset_m` of the edge the stroke builds (no pocket on a new edge).
    let kerb = f64::from(width) * 0.5 + f64::from(SIDEWALK_WIDTH);
    Some(kerb + CELL_DEPTH as f64 * cell_m)
}

/// Pushes the zoning-depth guide of a draft along `path`, whose stroke request is `stroke`
/// (world points, for the heights and the distance), at `depth` metres: the end circles and the
/// side offsets, each split into outside and inside runs (module docs). `O(P * (C + 16 K))` for
/// `P` guide points, `C` chunks of 16 stroke segments and `K` chunks within reach of a point, no
/// allocation beyond growing `scratch` and the gizmo buffers.
pub fn push_zoning_guide(
    gizmos: &mut Gizmos,
    path: &GuidePath,
    stroke: &[Vector3],
    depth: f64,
    scratch: &mut ZoningScratch,
) {
    let (Some(&start), Some(&end)) = (stroke.first(), stroke.last()) else {
        return;
    };
    let ZoningScratch {
        offset,
        stroke: line,
        boxes,
    } = scratch;
    line.clear();
    line.extend(stroke.iter().map(|point| xz(*point)));
    boxes.clear();
    for first in (0..line.len().saturating_sub(1)).step_by(CHUNK_SEGMENTS) {
        let last = (first + CHUNK_SEGMENTS).min(line.len() - 1);
        let (mut min, mut max) = (line[first], line[first]);
        for point in &line[first..=last] {
            min = min.min(*point);
            max = max.max(*point);
        }
        boxes.push((min, max));
    }
    let (line, boxes) = (&*line, &*boxes);
    let inside = |p: DVec2| distance_below(line, boxes, p, depth - INSIDE_TOLERANCE_M);
    let ends = [
        (
            xz(start),
            start.y,
            -path.start_tangent().unwrap_or(DVec2::NEG_X),
        ),
        (xz(end), end.y, path.end_tangent().unwrap_or(DVec2::X)),
    ];
    for (centre, y, outward) in ends {
        let point = |index: usize| {
            let angle = std::f64::consts::TAU * index as f64 / CIRCLE_CHORDS as f64;
            let (sin, cos) = angle.sin_cos();
            centre + DVec2::new(cos, sin) * depth
        };
        let over = |p: DVec2| (p - centre).dot(outward) < 0.0 || inside(p);
        push_split(gizmos, CIRCLE_CHORDS + 1, point, |_| y, &over);
    }
    for side in [depth, -depth] {
        if path.flatten_offset_into(side, offset).is_err() || offset.len() < 2 {
            continue;
        }
        let last = (offset.len() - 1) as f32;
        let height = |index: usize| start.y + (end.y - start.y) * index as f32 / last;
        let offset = &*offset;
        push_split(gizmos, offset.len(), |index| offset[index], height, &inside);
    }
}

// Pushes the guide polyline `point(0..count)` as runs of outside and inside, splitting a chord
// whose ends differ at the crossing (bisection on the chord). `O(count)` distance queries.
fn push_split(
    gizmos: &mut Gizmos,
    count: usize,
    point: impl Fn(usize) -> DVec2,
    height: impl Fn(usize) -> f32,
    inside: &impl Fn(DVec2) -> bool,
) {
    let class = |inner: bool| {
        if inner {
            GizmoLineClass::ZoningInside
        } else {
            GizmoLineClass::ZoningOutside
        }
    };
    let mut previous = point(0);
    let mut previous_inside = inside(previous);
    gizmos.begin_line(class(previous_inside));
    gizmos.push_point(at(previous, height(0)));
    for index in 1..count {
        let current = point(index);
        let current_inside = inside(current);
        if current_inside != previous_inside {
            // The crossing lies between the last point of the old class and the first of the new.
            let (mut lo, mut hi) = (0.0, 1.0);
            for _ in 0..CROSSING_STEPS {
                let mid = (lo + hi) * 0.5;
                if inside(previous.lerp(current, mid)) == previous_inside {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let crossing = previous.lerp(current, (lo + hi) * 0.5);
            let y = (height(index - 1) + height(index)) * 0.5;
            gizmos.push_point(at(crossing, y));
            gizmos.begin_line(class(current_inside));
            gizmos.push_point(at(crossing, y));
            previous_inside = current_inside;
        }
        gizmos.push_point(at(current, height(index)));
        previous = current;
    }
}

// Whether `p` lies nearer than `limit` to the polyline `line`, whose chunks of
// `CHUNK_SEGMENTS` segments are bounded by `boxes`: a chunk whose box lies `limit` or more away
// is skipped. `O(chunks + segments of the chunks in reach)`.
fn distance_below(line: &[DVec2], boxes: &[(DVec2, DVec2)], p: DVec2, limit: f64) -> bool {
    if line.len() < 2 {
        return line.first().is_some_and(|q| q.distance(p) < limit);
    }
    let limit_sq = limit * limit;
    boxes.iter().enumerate().any(|(chunk, (min, max))| {
        let gap = (*min - p).max(p - *max).max(DVec2::ZERO);
        if gap.length_squared() >= limit_sq {
            return false;
        }
        let first = chunk * CHUNK_SEGMENTS;
        let last = (first + CHUNK_SEGMENTS).min(line.len() - 1);
        line[first..=last].windows(2).any(|pair| {
            let (a, b) = (pair[0], pair[1]);
            let ab = b - a;
            let length_sq = ab.length_squared();
            let t = if length_sq > 0.0 {
                ((p - a).dot(ab) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            (a + ab * t).distance_squared(p) < limit_sq
        })
    })
}

/// Pushes the alignment guide `guide` that holds the cursor at `snapped` (height `y`): from its
/// origin (the road end, node or anchor it comes from) through `snapped` and [`GUIDE_PAST_M`]
/// on. `O(1)`.
pub fn push_alignment_guide(gizmos: &mut Gizmos, guide: GuideLine, snapped: DVec2, y: f32) {
    let along = (snapped - guide.origin).dot(guide.dir);
    // The guide runs both ways from its origin: draw the half the cursor is on.
    let dir = if along < 0.0 { -guide.dir } else { guide.dir };
    let past = guide.origin + dir * (along.abs() + GUIDE_PAST_M);
    gizmos.push_line(GizmoLineClass::Guide, [at(guide.origin, y), at(past, y)]);
}

fn xz(p: Vector3) -> DVec2 {
    DVec2::new(f64::from(p.x), f64::from(p.z))
}

fn at(p: DVec2, y: f32) -> Vector3 {
    Vector3::new(p.x as f32, y, p.y as f32)
}
