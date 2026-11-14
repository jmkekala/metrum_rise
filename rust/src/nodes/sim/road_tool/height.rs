// SPDX-License-Identifier: GPL-2.0-only

//! A draft's heights and its automatic bridge/tunnel segmentation (`ROAD-112` round H, plan
//! section 4).
//!
//! A draft either follows the ground (both ends on it and straight grade off: one stroke of its
//! own points with [`RoadProfile::FollowGround`], today's solve bit for bit; the session sends
//! it as it sends any stroke) or takes the grade line between its two end heights over arc
//! length (a handle moved off the ground or straight grade on, round I). The grade line is cut into strokes by its clearance `c(s)` = grade line minus the ground under
//! it ([`DraftGround`]: a grounded carriageway, else the terrain), read on the draft's 4 m tunnel
//! lattice with crossings interpolated linearly between knots:
//!
//! | Run | Stroke ([`RoadProfile`]) |
//! | --- | --- |
//! | `c` within +-1.0 m | `HeldGround`, holding the grade line |
//! | `c` above +1.0 m somewhere: the whole run where `c > 0`, landing to landing | `Bridge` |
//! | `c` below -1.0 m somewhere: the whole run where `c < 0`, reaching bore cover | `Tunnel` |
//! | the same run reaching no bore cover, no deeper than 5 m | `HeldGround`, in a cut |
//! | the same run deeper than 5 m | refused, [`CUT_TOO_DEEP_REASON`] |
//!
//! Consecutive held ground runs make one stroke of the draft's own points. A held ground run
//! shorter than the shortest stroke the checks accept (2 m) joins a neighbouring structure
//! (round I's short-run rule, [`segment_grade_line`]). The strokes are consecutive, each starting at
//! the point where the last ends, bit for bit, and commit as one batch (round F,
//! [`draft_batch`]): one undo step. The existing preparation, validation and commit judge each
//! stroke as they judge a stroke of its profile drawn alone.
//!
//! Cost: `O(draft length / 4 m)` ground queries (each `O(pieces in the point's query cell)`), one
//! cover test per below-ground run (`O(its knots x section lattice x deck points)`), and the
//! returned strokes; it runs per draft change, never per tick.

use crate::nodes::sim::core::{RoadBatchRequest, RoadCommitRequest};
use crate::simulation::network::grade::ROAD_SOURCE_DELTA_MAX_M;
use crate::simulation::network::graph::{RegionGraph, RoadMedian, RoadParking};
use crate::simulation::network::road_edit::{
    DraftGround, PREVIEW_CLEARANCE_M, PREVIEW_MIN_ENDPOINT_SEGMENT_M, RoadProfile,
    tunnel_run_reaches_cover,
};
use crate::simulation::network::road_types::RoadAttributes;
use crate::simulation::network::roadbed::RoadbedSystem;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

pub mod ground_marks;
pub mod handles;

/// The refusal of a draft whose grade line runs more than 5 m below the ground (the source
/// band, `grade::ROAD_SOURCE_DELTA_MAX_M`) where no knot reaches bore cover: neither a cut nor
/// a tunnel can hold it.
pub(crate) const CUT_TOO_DEEP_REASON: &str = "cut_too_deep";

// The grade line's lattice: the tunnel lattice's step (`road_edit::tunnel::TUNNEL_LATTICE_M`).
const GRADE_LATTICE_M: f32 = 4.0;
// Runs closer than this (metres along the draft) are one point: no stroke between them.
const RUN_EPSILON_M: f32 = 1e-3;

/// One stroke of a segmented draft, in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DraftStroke {
    /// The stroke's points: the draft's own between its run's ends, at the grade line's heights.
    pub(crate) points: Vec<Vector3>,
    /// How the stroke's heights are made.
    pub(crate) profile: RoadProfile,
}

/// The cross-section and options every stroke of a draft shares: what the commit needs beside
/// the points and the profile, and what sizes a tunnel run's section for its cover test.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DraftSection {
    /// Forward lanes.
    pub(crate) fwd_lanes: i32,
    /// Backward lanes.
    pub(crate) bkw_lanes: i32,
    /// Whether the strokes' ends snap to existing nodes (the joints between them included).
    pub(crate) snap_to_existing_roads: bool,
    /// The road tool's median (`ROAD-80`).
    pub(crate) median: Option<RoadMedian>,
    /// The road tool's kerbside parking (`PARK-05`).
    pub(crate) parking: RoadParking,
    /// The road type's attributes (`ROAD-101`).
    pub(crate) attributes: RoadAttributes,
}

impl DraftSection {
    fn lanes(self) -> (u8, u8) {
        let lanes = |count: i32| count.clamp(0, i32::from(u8::MAX)) as u8;
        (lanes(self.fwd_lanes), lanes(self.bkw_lanes))
    }
}

/// The strokes of a draft along `path` (the flattened guide; its `y` is not read) whose grade
/// line runs from `heights[0]` at its first point to `heights[1]` at its last (world heights),
/// on one terrain, graph and roadbed generation (the road tool's view): the grade line's runs
/// ([`segment_grade_line`]) over [`DraftGround`], the tunnel runs' cover tested for the draft's
/// `section`. `Err` names the refusal.
pub(crate) fn segment_draft(
    path: &[Vector3],
    heights: [f32; 2],
    section: DraftSection,
    terrain: &TerrainSystem,
    graph: &RegionGraph,
    roadbed: &RoadbedSystem,
) -> Result<Vec<DraftStroke>, &'static str> {
    let ground = DraftGround::new(terrain, roadbed);
    let mut strokes = Vec::new();
    segment_grade_line(
        path,
        heights,
        |x, z| ground.height_at(x, z),
        |run| {
            tunnel_run_reaches_cover(
                run,
                section.lanes(),
                (section.median, section.parking, section.attributes),
                terrain,
                graph,
            )
        },
        &mut strokes,
    )?;
    Ok(strokes)
}

/// A draft's strokes as one batch request (round F's `AddRoadBatch`): drawing order, one undo
/// step, all or nothing.
pub(crate) fn draft_batch(strokes: Vec<DraftStroke>, section: DraftSection) -> RoadBatchRequest {
    RoadBatchRequest {
        strokes: strokes
            .into_iter()
            .map(|stroke| RoadCommitRequest {
                points: stroke.points,
                fwd_lanes: section.fwd_lanes,
                bkw_lanes: section.bkw_lanes,
                snap_to_existing_roads: section.snap_to_existing_roads,
                profile: stroke.profile,
                median: section.median,
                parking: section.parking,
                attributes: section.attributes,
                preview: None,
            })
            .collect(),
        preview: None,
    }
}

/// Sets `points`' heights to the grade line from `heights[0]` at the first point to
/// `heights[1]` at the last over horizontal arc length, exactly as [`segment_grade_line`] reads
/// it (the same sums in the same order), so a graded stroke request's points stand on the line
/// its segmentation cuts. `O(points)`, no allocation.
pub(crate) fn apply_grade_line(points: &mut [Vector3], heights: [f32; 2]) {
    let horizontal = |a: Vector3, b: Vector3| (b.x - a.x).hypot(b.z - a.z);
    let length_m: f32 = (1..points.len())
        .map(|index| horizontal(points[index - 1], points[index]))
        .fold(0.0, |length, run| length + run);
    let mut station = 0.0;
    for index in 0..points.len() {
        if index > 0 {
            station += horizontal(points[index - 1], points[index]);
        }
        points[index].y = grade_height(heights, station, length_m);
    }
}

// The grade line's height at arc length `s` of `length_m`: exactly the end heights at the ends.
fn grade_height(heights: [f32; 2], s: f32, length_m: f32) -> f32 {
    if s >= length_m {
        return heights[1];
    }
    heights[0] + (heights[1] - heights[0]) * (s / length_m)
}

// What a run of the grade line is before the cover test.
#[derive(Clone, Copy, Debug, PartialEq)]
enum RunKind {
    Above,
    Below,
}

// One knot of the grade line's lattice: arc length and clearance.
#[derive(Clone, Copy, Debug)]
struct Knot {
    s: f32,
    c: f32,
}

/// Cuts the grade line from `heights[0]` at `path`'s first point to `heights[1]` at its last into
/// strokes (the module's table) into `out` (cleared first): `ground` gives the ground height
/// under an XZ point and `reaches_cover` whether a below-ground run's points reach bore cover.
/// The strokes' points are `path`'s own between the run ends, the first and last bit for bit,
/// at the grade line's heights (the ends exactly `heights`). A path shorter than one point or of
/// zero length is one held ground stroke of itself.
pub(crate) fn segment_grade_line(
    path: &[Vector3],
    heights: [f32; 2],
    mut ground: impl FnMut(f32, f32) -> f32,
    mut reaches_cover: impl FnMut(&[Vector3]) -> bool,
    out: &mut Vec<DraftStroke>,
) -> Result<(), &'static str> {
    out.clear();
    let line = GradeLine::new(path, heights);
    if line.length_m <= RUN_EPSILON_M {
        out.push(DraftStroke {
            points: line.points(0.0, line.length_m),
            profile: RoadProfile::HeldGround,
        });
        return Ok(());
    }
    let knots = line.knots(&mut ground);
    // The runs as arc-length ranges, held ground ones joined, then their points.
    let mut runs: Vec<(f32, f32, RoadProfile)> = Vec::new();
    let mut push = |from_m: f32, to_m: f32, profile: RoadProfile| match runs.last_mut() {
        Some(last) if profile == RoadProfile::HeldGround && last.2 == RoadProfile::HeldGround => {
            last.1 = to_m;
        }
        _ => runs.push((from_m, to_m, profile)),
    };
    let mut cursor_m = 0.0;
    for (start_m, end_m, kind, least_m) in structure_runs(&knots, line.length_m) {
        if start_m - cursor_m > RUN_EPSILON_M {
            push(cursor_m, start_m, RoadProfile::HeldGround);
        }
        let profile = match kind {
            RunKind::Above => RoadProfile::Bridge,
            RunKind::Below if reaches_cover(&line.points(start_m, end_m)) => RoadProfile::Tunnel,
            RunKind::Below if least_m >= -ROAD_SOURCE_DELTA_MAX_M => RoadProfile::HeldGround,
            RunKind::Below => return Err(CUT_TOO_DEEP_REASON),
        };
        push(start_m, end_m, profile);
        cursor_m = end_m;
    }
    if line.length_m - cursor_m > RUN_EPSILON_M {
        push(cursor_m, line.length_m, RoadProfile::HeldGround);
    }
    absorb_short_held_runs(&mut runs, |s| line.height_at(s) - ground_at(&line, &mut ground, s));
    out.extend(runs.into_iter().map(|(from_m, to_m, profile)| DraftStroke {
        points: line.points(from_m, to_m),
        profile,
    }));
    Ok(())
}

// The short-run rule (round I): a held ground run shorter than the shortest stroke the checks
// accept (`PREVIEW_MIN_ENDPOINT_SEGMENT_M`, 2 m, `too_short`) cannot be a stroke of its own, so it
// joins a neighbouring structure run, which holds the authored heights exactly (a bridge's deck,
// a tunnel stroke's approach) and so keeps the grade line through it. Of its neighbours it joins
// the one on its side of the ground: a bridge when the grade line at the run's middle stands on or
// above the ground (`clearance(s) >= 0`), a tunnel (as more of its open approach) when below;
// when neither neighbour is of that class, the one before it, else the one after. A run with no
// structure beside it (a draft of one held run) stays. `O(runs)` plus one ground query per short
// run.
fn absorb_short_held_runs(
    runs: &mut Vec<(f32, f32, RoadProfile)>,
    mut clearance: impl FnMut(f32) -> f32,
) {
    let mut index = 0;
    while index < runs.len() {
        let (from_m, to_m, profile) = runs[index];
        if profile != RoadProfile::HeldGround || to_m - from_m >= PREVIEW_MIN_ENDPOINT_SEGMENT_M {
            index += 1;
            continue;
        }
        let before = index.checked_sub(1);
        let after = (index + 1 < runs.len()).then_some(index + 1);
        let wanted = if clearance((from_m + to_m) * 0.5) >= 0.0 {
            RoadProfile::Bridge
        } else {
            RoadProfile::Tunnel
        };
        let matches = |neighbour: Option<usize>| neighbour.filter(|&n| runs[n].2 == wanted);
        let Some(target) = matches(before).or(matches(after)).or(before).or(after) else {
            index += 1;
            continue;
        };
        // Held runs are joined when pushed, so both neighbours are structures.
        if target < index {
            runs[target].1 = to_m;
        } else {
            runs[target].0 = from_m;
        }
        runs.remove(index);
    }
}

// The ground under the grade line at arc length `s`.
fn ground_at(line: &GradeLine, ground: &mut impl FnMut(f32, f32) -> f32, s: f32) -> f32 {
    let point = line.point_at(s);
    ground(point.x, point.z)
}

// The structure runs of the knots in order: `(start_m, end_m, kind, least clearance)` for each
// maximal run of knots with `c > 0` whose clearance exceeds +1.0 m somewhere and each with
// `c < 0` below -1.0 m somewhere, from the crossing before its first knot (the draft's start
// for the first knot) to the crossing after its last (its end). Crossings interpolate `c`
// linearly between the two knots around them.
fn structure_runs(knots: &[Knot], length_m: f32) -> Vec<(f32, f32, RunKind, f32)> {
    let mut runs = Vec::new();
    let mut first = 0;
    while first < knots.len() {
        let sign = knots[first].c.partial_cmp(&0.0);
        let mut last = first;
        while last + 1 < knots.len() && knots[last + 1].c.partial_cmp(&0.0) == sign {
            last += 1;
        }
        let run = &knots[first..=last];
        let kind = match sign {
            Some(std::cmp::Ordering::Greater)
                if run.iter().any(|knot| knot.c > PREVIEW_CLEARANCE_M) =>
            {
                Some(RunKind::Above)
            }
            Some(std::cmp::Ordering::Less)
                if run.iter().any(|knot| knot.c < -PREVIEW_CLEARANCE_M) =>
            {
                Some(RunKind::Below)
            }
            _ => None,
        };
        if let Some(kind) = kind {
            let start_m = if first == 0 {
                0.0
            } else {
                crossing_m(knots[first - 1], knots[first])
            };
            let end_m = if last + 1 == knots.len() {
                length_m
            } else {
                crossing_m(knots[last], knots[last + 1])
            };
            let least_m = run.iter().map(|knot| knot.c).fold(f32::INFINITY, f32::min);
            runs.push((start_m, end_m, kind, least_m));
        }
        first = last + 1;
    }
    runs
}

// Where `c` crosses zero between two neighbouring knots of different sign (one may be zero).
fn crossing_m(a: Knot, b: Knot) -> f32 {
    a.s + (b.s - a.s) * (a.c / (a.c - b.c))
}

// The grade line along a path: its points' arc lengths and the heights at its ends.
struct GradeLine<'a> {
    path: &'a [Vector3],
    // Arc length at each path point.
    stations: Vec<f32>,
    length_m: f32,
    heights: [f32; 2],
}

impl<'a> GradeLine<'a> {
    fn new(path: &'a [Vector3], heights: [f32; 2]) -> Self {
        let mut stations = Vec::with_capacity(path.len());
        let mut length_m = 0.0;
        for (index, point) in path.iter().enumerate() {
            if index > 0 {
                let before = path[index - 1];
                length_m += (point.x - before.x).hypot(point.z - before.z);
            }
            stations.push(length_m);
        }
        Self {
            path,
            stations,
            length_m,
            heights,
        }
    }

    // The grade line's height at arc length `s`: exactly the end heights at the ends.
    fn height_at(&self, s: f32) -> f32 {
        grade_height(self.heights, s, self.length_m)
    }

    // The lattice: each segment of the path in equal steps of at most 4 m, its end points
    // included once, with the clearance of the grade line over `ground`.
    fn knots(&self, ground: &mut impl FnMut(f32, f32) -> f32) -> Vec<Knot> {
        let mut knots = Vec::new();
        let mut knot = |s: f32, x: f32, z: f32| Knot {
            s,
            c: self.height_at(s) - ground(x, z),
        };
        if let Some(first) = self.path.first() {
            knots.push(knot(0.0, first.x, first.z));
        }
        for index in 1..self.path.len() {
            let (a, b) = (self.path[index - 1], self.path[index]);
            let (s0, s1) = (self.stations[index - 1], self.stations[index]);
            if s1 - s0 <= 0.0 {
                continue;
            }
            let steps = ((s1 - s0) / GRADE_LATTICE_M).ceil().max(1.0) as usize;
            for step in 1..steps {
                let t = step as f32 / steps as f32;
                knots.push(knot(
                    s0 + (s1 - s0) * t,
                    a.x + (b.x - a.x) * t,
                    a.z + (b.z - a.z) * t,
                ));
            }
            knots.push(knot(s1, b.x, b.z));
        }
        knots
    }

    // The path between arc lengths `from_m` and `to_m`: the point at each end and the path's
    // own points strictly between, at the grade line's heights. An end at the path's first or
    // last point is that point's XZ bit for bit.
    fn points(&self, from_m: f32, to_m: f32) -> Vec<Vector3> {
        let mut points = vec![self.point_at(from_m)];
        points.extend(
            self.path
                .iter()
                .zip(&self.stations)
                .filter(|&(_, &s)| s > from_m && s < to_m)
                .map(|(point, &s)| Vector3::new(point.x, self.height_at(s), point.z)),
        );
        if to_m > from_m || self.path.len() > 1 {
            points.push(self.point_at(to_m));
        }
        points
    }

    // The point at arc length `s` on the path, at the grade line's height.
    fn point_at(&self, s: f32) -> Vector3 {
        let last = self.path.len() - 1;
        if s <= 0.0 {
            let first = self.path[0];
            return Vector3::new(first.x, self.heights[0], first.z);
        }
        if s >= self.length_m {
            let end = self.path[last];
            return Vector3::new(end.x, self.heights[1], end.z);
        }
        // The segment holding `s`: the first point at or beyond it ends it.
        let index = self
            .stations
            .partition_point(|&station| station < s)
            .clamp(1, last);
        let (a, b) = (self.path[index - 1], self.path[index]);
        let (s0, s1) = (self.stations[index - 1], self.stations[index]);
        let t = if s1 > s0 { (s - s0) / (s1 - s0) } else { 0.0 };
        Vector3::new(
            a.x + (b.x - a.x) * t,
            self.height_at(s),
            a.z + (b.z - a.z) * t,
        )
    }
}

#[cfg(test)]
mod tests;
