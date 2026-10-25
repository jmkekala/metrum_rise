// SPDX-License-Identifier: GPL-2.0-only

//! The road grade rule (`ROAD-68` open question 37, decided 2026-10-25): the one grade of a
//! profile, the 1:3 maximum placement refuses beyond, the 16% warning, and the construction of a
//! profile within a maximum grade and the source-delta band (open question 39) that the stroke
//! solve and the junction profile solve share.

use super::validate::visit_grades;
use crate::config;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

/// The steep-street warning: the stroke solve's design grade, above which the road tool warns
/// without refusing.
pub(crate) const ROAD_GRADE_WARNING: f32 = 0.16;
/// The hard maximum grade, 1:3: just under the world's steepest streets (about 35%). Placement
/// refuses an edit whose profiles exceed it (`road_too_steep`).
pub(crate) const ROAD_GRADE_MAX: f32 = 1.0 / 3.0;
/// The grade profiles are built to where their constraints allow: 33%, a third of a percentage
/// point under the maximum, so the `f32` rounding of a profile built to it (world coordinates in
/// the thousands of metres, knots 5 cm apart at the closest) never reads above the maximum.
pub(crate) const ROAD_GRADE_BUILT: f32 = 0.33;
// The source-delta limit (`placements.json`'s `max_abs_source_delta_m`): a height the stroke
// solve or a junction blend constructs keeps within this of the source terrain under it, so the
// maximum grade is met by cut or fill only this deep, and an edit that cannot keep both is
// refused (`ROAD-68` open questions 37 and 39).
const ROAD_SOURCE_DELTA_MAX_M: f32 = 5.0;

/// The source terrain height under a world XZ point, which the source-delta limit measures from.
pub(crate) fn source_ground_m(terrain: &TerrainSystem, x: f32, z: f32) -> f32 {
    terrain.sample_height_world(x, z) * config::HEIGHT_SCALE
}

/// Whether `point` lies within the source-delta limit of the ground under it, as `f32` reads
/// it: then it lies inside every band [`source_bounds`] gives it.
pub(crate) fn within_source_band(terrain: &TerrainSystem, point: Vector3) -> bool {
    (point.y - source_ground_m(terrain, point.x, point.z)).abs() <= ROAD_SOURCE_DELTA_MAX_M
}

/// One profile point a construction bounds ([`source_bounds`]).
#[derive(Clone, Copy)]
pub(crate) struct SourcePoint {
    /// The source terrain height under the point ([`source_ground_m`]).
    pub(crate) ground_m: f32,
    /// The height of a point the construction keeps (a pin); `None` for one it may move.
    pub(crate) held_m: Option<f32>,
}

/// The steepest segment of a profile ([`steepest_grade`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SteepestGrade {
    /// Absolute rise over horizontal run, not a percentage.
    pub(crate) grade: f32,
    /// Index of the segment's first point.
    pub(crate) segment: usize,
    /// The segment's start station, in three-dimensional metres from the first point.
    pub(crate) station_m: f32,
    /// The segment's three-dimensional length.
    pub(crate) length_m: f32,
}

/// The grade of a road profile: its steepest segment's absolute rise over horizontal run, over
/// the segments [`visit_grades`] grades (at least 5 cm of run). The one definition the
/// validator's grade check, placement's maximum, the road tool's warning and the recorded
/// replays' quality check share. Ties keep the earliest segment; `None` when no segment has a
/// grade. O(points), no allocation.
pub(crate) fn steepest_grade(points: &[Vector3]) -> Option<SteepestGrade> {
    let mut steepest: Option<SteepestGrade> = None;
    let mut segment = 0;
    visit_grades(points, |station_m, length_m, grade, _| {
        if grade.abs() > steepest.map_or(-1.0, |s| s.grade) {
            steepest = Some(SteepestGrade {
                grade: grade.abs(),
                segment,
                station_m,
                length_m,
            });
        }
        segment += 1;
    });
    steepest
}

// The source-delta band at `ground` widened by `excess` below and above, as `f32` reads it:
// each bound steps one ulp inwards where the rounded sum's offset would read beyond it.
fn source_band(ground: f32, (below, above): (f32, f32)) -> (f32, f32) {
    let (reach_below, reach_above) = (
        ROAD_SOURCE_DELTA_MAX_M + below,
        ROAD_SOURCE_DELTA_MAX_M + above,
    );
    let mut low = ground - reach_below;
    if ground - low > reach_below {
        low = low.next_up();
    }
    let mut high = ground + reach_above;
    if high - ground > reach_above {
        high = high.next_down();
    }
    (low, high)
}

/// Every point's height bounds for a profile built within the maximum grade (`runs[i]` is the
/// horizontal run from point `i` to `i + 1`): a held point at its height, any other within the
/// source-delta limit of the ground. A held point further off the ground than the limit (a
/// joined road end on an embankment, a junction plane) widens its neighbours' band by its
/// excess, shrinking at the 16% design grade with the distance from it, so a profile may leave it
/// at that grade or steeper and only the ground between the held points can force a profile
/// past the maximum. A band shrinking at the grade the profile is built to would leave a profile
/// leaving such a point on flat ground exactly on its edge, where `f32` rounding decides
/// feasibility. Two passes carry the nearest held points' excess. O(points).
pub(crate) fn source_bounds(points: &[SourcePoint], runs: &[f32]) -> Vec<(f32, f32)> {
    let count = points.len();
    let mut excess = vec![(0.0_f32, 0.0_f32); count];
    for backward in [false, true] {
        // The excess below and above the band the nearest held point on this side leaves here.
        let mut carried = (0.0_f32, 0.0_f32);
        for step in 0..count {
            let index = if backward { count - 1 - step } else { step };
            if step > 0 {
                let run = runs[if backward { index } else { index - 1 }];
                carried = (
                    (carried.0 - ROAD_GRADE_WARNING * run).max(0.0),
                    (carried.1 - ROAD_GRADE_WARNING * run).max(0.0),
                );
            }
            let point = &points[index];
            if let Some(held_m) = point.held_m {
                carried = (
                    (point.ground_m - ROAD_SOURCE_DELTA_MAX_M - held_m).max(0.0),
                    (held_m - point.ground_m - ROAD_SOURCE_DELTA_MAX_M).max(0.0),
                );
            }
            let widest = &mut excess[index];
            *widest = (widest.0.max(carried.0), widest.1.max(carried.1));
        }
    }
    points
        .iter()
        .zip(excess)
        .map(|(point, excess)| match point.held_m {
            Some(held_m) => (held_m, held_m),
            None => source_band(point.ground_m, excess),
        })
        .collect()
}

/// The envelope ([`grade_envelope`]) of the profiles through `bounds` within 33%, or else within
/// the 1:3 maximum, with the grade it holds; `None` when no profile there keeps the maximum.
/// O(points).
pub(crate) fn maximum_envelope(
    bounds: &[(f32, f32)],
    runs: &[f32],
) -> Option<(f32, Vec<(f32, f32)>)> {
    [ROAD_GRADE_BUILT, ROAD_GRADE_MAX]
        .into_iter()
        .find_map(|grade| {
            let mut envelope = bounds.to_vec();
            grade_envelope(&mut envelope, runs, grade).then_some((grade, envelope))
        })
}

/// Builds `heights` inside `bounds` ([`source_bounds`]) within 33%, else within the 1:3 maximum,
/// to the heights nearest their own ([`construct_within_grade`]); where neither exists, at the
/// least grade the bounds allow, which is steeper than the maximum, so placement refuses the
/// edit that produces it (`road_too_steep`). Returns the grade built to. O(points); that least
/// grade O(points^2).
pub(crate) fn construct_within_bounds(
    heights: &mut [f32],
    bounds: &[(f32, f32)],
    runs: &[f32],
) -> f32 {
    let (max_grade, envelope) = maximum_envelope(bounds, runs).unwrap_or_else(|| {
        // From the pairwise bound up, an ulp at a time, to the first `f32` grade whose envelope
        // `f32` reads as feasible.
        let mut grade = least_grade(bounds, runs).max(ROAD_GRADE_MAX.next_up());
        loop {
            let mut envelope = bounds.to_vec();
            if grade_envelope(&mut envelope, runs, grade) {
                break (grade, envelope);
            }
            grade = grade.next_up();
        }
    });
    construct_within_grade(heights, &envelope, runs, max_grade);
    max_grade
}

/// The least maximum grade of any profile through height `bounds` (`runs` as for
/// [`grade_envelope`]): each pair of points needs at least the height their bounds keep apart
/// over the run between them, and on a chain the largest such pair decides it. O(points^2); only
/// a profile its constraints refuse takes it.
pub(crate) fn least_grade(bounds: &[(f32, f32)], runs: &[f32]) -> f32 {
    let mut least = 0.0_f32;
    for (first, &(a_low, a_high)) in bounds.iter().enumerate() {
        let mut run_m = 0.0;
        for (run, &(b_low, b_high)) in runs[first..].iter().zip(&bounds[first + 1..]) {
            run_m += run;
            least = least.max((b_low - a_high).max(a_low - b_high) / run_m);
        }
    }
    least
}

/// Narrows each point's height `bounds` to the heights some profile takes there with every
/// segment within `max_grade` (`runs[i]` is the horizontal run from point `i` to `i + 1`); false,
/// with `bounds` partly narrowed, when no such profile exists. The constraints form a chain, so a
/// forward pass decides feasibility and a backward pass leaves every interval exact: from any
/// height in one interval the next is reachable within `max_grade`. O(points).
pub(crate) fn grade_envelope(bounds: &mut [(f32, f32)], runs: &[f32], max_grade: f32) -> bool {
    for index in 1..bounds.len() {
        let reach = max_grade * runs[index - 1];
        let (low, high) = bounds[index - 1];
        let next = &mut bounds[index];
        *next = (next.0.max(low - reach), next.1.min(high + reach));
        if next.0 > next.1 {
            return false;
        }
    }
    for index in (0..bounds.len().saturating_sub(1)).rev() {
        let reach = max_grade * runs[index];
        let (low, high) = bounds[index + 1];
        let previous = &mut bounds[index];
        *previous = (previous.0.max(low - reach), previous.1.min(high + reach));
    }
    true
}

/// Builds `heights` within `max_grade` inside an exact `envelope` from [`grade_envelope`]: the
/// mean of two passes, first to last and last to first, that each move a point to the height
/// nearest its own inside its interval and within `max_grade` of the point before it in that
/// pass. Both passes are feasible and the constraints are convex, so their mean is too, and it
/// does not depend on the profile's direction. A profile already inside every bound is left as
/// it is, and a point whose interval is one height takes it exactly. O(points); allocates one
/// copy of the heights.
pub(crate) fn construct_within_grade(
    heights: &mut [f32],
    envelope: &[(f32, f32)],
    runs: &[f32],
    max_grade: f32,
) {
    let nearest = |height: f32, (low, high): (f32, f32), previous: Option<(f32, f32)>| {
        let (low, high) = previous.map_or((low, high), |(previous, run)| {
            let reach = max_grade * run;
            (low.max(previous - reach), high.min(previous + reach))
        });
        height.max(low).min(high)
    };
    let mut backward = heights.to_vec();
    for index in (0..backward.len()).rev() {
        let previous = (index + 1 < backward.len()).then(|| (backward[index + 1], runs[index]));
        backward[index] = nearest(backward[index], envelope[index], previous);
    }
    for index in 0..heights.len() {
        let previous = (index > 0).then(|| (heights[index - 1], runs[index - 1]));
        heights[index] = nearest(heights[index], envelope[index], previous);
    }
    for ((height, backward), &(low, high)) in heights.iter_mut().zip(backward).zip(envelope) {
        *height = if low == high {
            low
        } else {
            0.5 * (*height + backward)
        };
    }
}
