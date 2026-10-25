// SPDX-License-Identifier: GPL-2.0-only

//! The road grade rule (`ROAD-68` open question 37, decided 2026-10-25): the one grade of a
//! profile, the 1:3 maximum placement refuses beyond, the 16% warning, and the construction of a
//! profile within a maximum grade that the stroke solve and the junction profile solve share.

use super::validate::visit_grades;
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
