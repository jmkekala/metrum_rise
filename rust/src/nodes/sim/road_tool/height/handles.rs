// SPDX-License-Identifier: GPL-2.0-only

//! A draft's end height handles (`ROAD-112` round I, plan section 4): each end of a draft is
//! `Free` (an offset above its support, the ground or the road surface the pointer met) or
//! `Locked` to the graph height of the road it joins, which cannot move. A free end's handle
//! is a grip standing [`HANDLE_STEM_M`] above the end's height; dragging it projects the pointer
//! ray onto the grip's vertical line (the closest point of the two lines) and rounds the end's
//! offset to [`HEIGHT_STEP_M`] (Shift: free). PageUp/PageDown move the active handle by one
//! step, so keys and drags make the same offsets bit for bit. All `O(1)`, no allocation.

use super::super::session::Pointer;
use godot::prelude::Vector3;

/// The height step of a handle drag and of one PageUp/PageDown press, metres.
pub const HEIGHT_STEP_M: f32 = 0.5;
/// How far a free end's grip stands above the end's height, metres: so the grip never covers
/// the end itself (a press on the end keeps its meaning: a freeform node's grab, a placing
/// click).
pub const HANDLE_STEM_M: f32 = 4.0;
/// How close the pointer must pass a grip to take it, metres (the ray's distance to the grip).
pub const HANDLE_REACH_M: f32 = 2.0;
// Below this squared horizontal ray length the ray runs along the vertical line: no projection.
const PARALLEL_EPS: f32 = 1e-8;

/// An end of a draft.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DraftEnd {
    /// The draft's first point.
    Start,
    /// The draft's last point: the live end while placing.
    #[default]
    End,
}

/// One end's height handle, as the frame reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeightHandle {
    /// Which end.
    pub end: DraftEnd,
    /// The end on its support: the ground or road surface the pointer met, or the joined road's
    /// graph point for a locked end.
    pub support: Vector3,
    /// The end's world height (the grade line's height there).
    pub height_m: f32,
    /// The free end's offset above its support (0 for a locked end).
    pub offset_m: f32,
    /// The grip: [`HANDLE_STEM_M`] above the end's height.
    pub grip: Vector3,
    /// Locked to the graph height of the road the end joins: it cannot move.
    pub locked: bool,
    /// PageUp/PageDown move this handle.
    pub active: bool,
}

impl HeightHandle {
    /// The handle of an end standing on `support`, `offset_m` above it unless `locked`.
    pub fn new(end: DraftEnd, support: Vector3, offset_m: f32, locked: bool) -> Self {
        let offset_m = if locked { 0.0 } else { offset_m };
        let height_m = support.y + offset_m;
        Self {
            end,
            support,
            height_m,
            offset_m,
            grip: Vector3::new(support.x, height_m + HANDLE_STEM_M, support.z),
            locked,
            active: false,
        }
    }

    /// The end at its height.
    pub fn point(&self) -> Vector3 {
        Vector3::new(self.support.x, self.height_m, self.support.z)
    }
}

/// The session's end heights (round I): the offsets, straight grade, the start's lock and the
/// drag. The end's lock is the pose's (the live end's snap), so it is not kept.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EndHeights {
    // The start's offset above its support.
    pub(crate) start_offset_m: f32,
    // The end's offset: the live end's (the cursor's before a stroke starts, which a new start
    // takes), a freeform draft's last node's. Kept across strokes, as a tool setting.
    pub(crate) end_offset_m: f32,
    // Straight grade (U3): the grade line between the ends even with both offsets 0.
    pub(crate) straight_grade: bool,
    // The start joins a road (a network snap or the chain): locked to its graph height.
    pub(crate) start_locked: bool,
    // The grip being dragged.
    pub(crate) grip: Option<DraftEnd>,
    // The handle PageUp/PageDown move in an edit phase: the hovered or last dragged one.
    pub(crate) active: DraftEnd,
}

impl EndHeights {
    pub(crate) fn offset(&self, end: DraftEnd) -> f32 {
        match end {
            DraftEnd::Start => self.start_offset_m,
            DraftEnd::End => self.end_offset_m,
        }
    }

    pub(crate) fn set_offset(&mut self, end: DraftEnd, offset_m: f32) {
        match end {
            DraftEnd::Start => self.start_offset_m = offset_m,
            DraftEnd::End => self.end_offset_m = offset_m,
        }
    }

    // The handles of a draft from `start` to `end` (the ends on their supports); `end_locked`
    // when the end joins a road.
    pub(crate) fn handles(
        &self,
        start: Vector3,
        end: Vector3,
        end_locked: bool,
    ) -> [HeightHandle; 2] {
        [
            HeightHandle::new(
                DraftEnd::Start,
                start,
                self.start_offset_m,
                self.start_locked,
            ),
            HeightHandle::new(DraftEnd::End, end, self.end_offset_m, end_locked),
        ]
    }

    // Whether a draft with these handles takes a grade line: a free end off its support, or
    // straight grade on.
    pub(crate) fn graded(&self, handles: &[HeightHandle; 2]) -> bool {
        self.straight_grade || handles.iter().any(|handle| handle.offset_m != 0.0)
    }
}

/// The grip `pointer` takes among `handles` (start, end; `None` for none drawn): a free one
/// whose `grabbable` flag is set, within [`HANDLE_REACH_M`] of the pointer and nearer to it than
/// its own end (a press on the end keeps its meaning); the nearest when two qualify.
pub(crate) fn grip_at(
    handles: &[Option<HeightHandle>; 2],
    grabbable: [bool; 2],
    pointer: Pointer,
) -> Option<DraftEnd> {
    let distance = |point: Vector3| match pointer {
        Pointer::Ray { origin, dir } => ray_distance(origin, dir, point),
        Pointer::World(at) | Pointer::Scripted(at) => at.distance_to(point),
    };
    handles
        .iter()
        .zip(grabbable)
        .filter_map(|(handle, grabbable)| handle.filter(|handle| grabbable && !handle.locked))
        .map(|handle| (handle.end, distance(handle.grip), distance(handle.point())))
        .filter(|&(_, grip_m, end_m)| grip_m <= HANDLE_REACH_M && grip_m < end_m)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(end, _, _)| end)
}

/// The offset a drag of `handle`'s grip to `pointer` gives: a camera ray's closest point to the
/// grip's vertical line, a world point's own height, less the stem and the support, rounded to
/// the step unless `free`. `None` for a ray along the line.
pub(crate) fn drag_offset(handle: &HeightHandle, pointer: Pointer, free: bool) -> Option<f32> {
    let grip_m = match pointer {
        Pointer::Ray { origin, dir } => {
            ray_height_on_vertical(origin, dir, handle.support.x, handle.support.z)?
        }
        Pointer::World(at) | Pointer::Scripted(at) => at.y,
    };
    let raw_m = grip_m - HANDLE_STEM_M - handle.support.y;
    raw_m.is_finite().then(|| step_offset(raw_m, free))
}

/// `raw_m` rounded to the height step, unless `free`.
pub fn step_offset(raw_m: f32, free: bool) -> f32 {
    if free {
        raw_m
    } else {
        (raw_m / HEIGHT_STEP_M).round() * HEIGHT_STEP_M
    }
}

/// `offset_m` moved by `steps` height steps, from its nearest step (a free drag's offset first
/// rounds to the grid), so `n` presses from 0 equal a drag to `n` steps bit for bit.
pub fn stepped_by(offset_m: f32, steps: i32) -> f32 {
    ((offset_m / HEIGHT_STEP_M).round() + steps as f32) * HEIGHT_STEP_M
}

/// The height on the vertical line through `(x, z)` closest to the ray from `origin` along
/// `dir` (closest points of two lines); `None` when the ray runs along the line.
pub fn ray_height_on_vertical(origin: Vector3, dir: Vector3, x: f32, z: f32) -> Option<f32> {
    // Lines P(t) = origin + t dir and Q(s) = (x, s, z): with w = origin - (x, 0, z),
    // s = (a e - b d) / (a - b^2) for a = dir.dir, b = dir.y, d = dir.w, e = w.y.
    let w = Vector3::new(origin.x - x, origin.y, origin.z - z);
    let a = dir.dot(dir);
    let b = dir.y;
    let denom = a - b * b;
    if denom <= PARALLEL_EPS * a.max(f32::MIN_POSITIVE) {
        return None;
    }
    Some((a * w.y - b * dir.dot(w)) / denom)
}

/// The distance from the ray from `origin` along `dir` (the forward half-line) to `point`.
pub fn ray_distance(origin: Vector3, dir: Vector3, point: Vector3) -> f32 {
    let a = dir.dot(dir);
    if a <= 0.0 {
        return origin.distance_to(point);
    }
    let t = ((point - origin).dot(dir) / a).max(0.0);
    (origin + dir * t).distance_to(point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_finds_the_height_where_the_ray_passes_the_line() {
        // A camera 100 m south of and 100 m above (0, 0, 0), looking at (0, 10, 0).
        let origin = Vector3::new(0.0, 100.0, 100.0);
        let dir = Vector3::new(0.0, 10.0, 0.0) - origin;
        let height = ray_height_on_vertical(origin, dir, 0.0, 0.0).unwrap();
        assert!((height - 10.0).abs() < 1e-4, "{height}");
        // Off the line: the closest point of the two lines, not the ray's ground hit.
        let skew = Vector3::new(3.0, -1.0, -1.0);
        let height = ray_height_on_vertical(Vector3::new(-3.0, 7.0, 0.0), skew, 0.0, 0.0).unwrap();
        assert!((height - 6.1).abs() < 1e-5, "{height}");
        // Straight down the line: no answer.
        let down = Vector3::new(0.0, -1.0, 0.0);
        assert_eq!(
            ray_height_on_vertical(Vector3::new(0.0, 50.0, 0.0), down, 0.0, 0.0),
            None
        );
    }

    #[test]
    fn offsets_round_to_the_step_and_keys_equal_drags() {
        assert_eq!(step_offset(1.26, false), 1.5);
        assert_eq!(step_offset(1.24, false), 1.0);
        assert_eq!(step_offset(-2.74, false), -2.5);
        assert_eq!(step_offset(1.26, true), 1.26);
        let mut keyed = 0.0;
        for steps in 1..=9 {
            keyed = stepped_by(keyed, 1);
            assert_eq!(
                keyed.to_bits(),
                step_offset(steps as f32 * 0.5 + 0.1, false).to_bits()
            );
        }
        // A free drag's offset steps from its nearest step.
        assert_eq!(stepped_by(1.3, -1), 1.0);
    }

    #[test]
    fn a_locked_handle_has_no_offset() {
        let support = Vector3::new(4.0, 2.0, 5.0);
        let free = HeightHandle::new(DraftEnd::Start, support, 1.5, false);
        assert_eq!(free.height_m, 3.5);
        assert_eq!(free.grip, Vector3::new(4.0, 3.5 + HANDLE_STEM_M, 5.0));
        let locked = HeightHandle::new(DraftEnd::Start, support, 1.5, true);
        assert_eq!((locked.height_m, locked.offset_m), (2.0, 0.0));
    }
}
