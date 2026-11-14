// SPDX-License-Identifier: GPL-2.0-only

//! Simple curve (U1): start, a free control point, end. The draft is the fillet arc tangent to
//! both legs with tangent length `min(|AC|, |CB|)` plus a straight lead on the longer leg
//! (`GuidePath::set_fillet`); collinear legs give the line. Started with a road tangent, the
//! control point is held on the tangent's forward ray, so the curve leaves tangent to the road;
//! the placed control is projected onto the ray again for the shape (its stored `f32` point is
//! off the ray by rounding), so it leaves along the tangent exactly (round CT).

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath};

pub(super) fn shape(
    placed: &[DVec2],
    live: DVec2,
    tangent: Option<DVec2>,
    path: &mut GuidePath,
) -> Result<(), GuideError> {
    match *placed {
        [start] => super::ghost(start, held_control(start, live, tangent), path),
        [start, control, ..] => path.set_fillet(start, held_control(start, control, tangent), live),
        [] => unreachable!("the session shapes only a started stroke"),
    }
}

/// The control point a click at `point` places: on the start tangent's forward ray when the
/// stroke has one (its projection, never behind the start), else the point itself.
pub(crate) fn held_control(start: DVec2, point: DVec2, tangent: Option<DVec2>) -> DVec2 {
    tangent.map_or(point, |tangent| {
        start + tangent * (point - start).dot(tangent).max(0.0)
    })
}
