// SPDX-License-Identifier: GPL-3.0-or-later

//! Complex curve: the biarc S-curve (`GuidePath::set_biarc`, equal tangent lengths). Clicks: the
//! start `A`, a point giving the start tangent (`A -> TA`), a point giving the end tangent, then
//! the end `B`, which the road reaches along `TB -> B`. Until the end is live the draft is what
//! the other modes draw for the same points (round SC, the user's decision replacing round N's
//! gizmo-only aim legs): Straight's line from the start to the cursor, then the simple curve's
//! fillet of the start, the start aim and the cursor; the S-curve forms at the third click.
//! A continuous chain's S-curve (round CT) has its start direction carried in place of the start
//! aim click: before the end aim the arc leaving along it through the cursor (Straight's
//! continuous draft, the fillet whose start aim is where the start's and the cursor's tangents
//! meet), then the biarc from that direction exactly.

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment};

pub(super) fn shape(
    placed: &[DVec2],
    live: DVec2,
    carried: Option<DVec2>,
    path: &mut GuidePath,
) -> Result<(), GuideError> {
    if let Some(direction) = carried {
        return match *placed {
            [start, end_aim, ..] => path.set_biarc(start, direction, live, live - end_aim),
            [start] => super::straight::shape(start, live, Some(direction), path),
            [] => unreachable!("the session shapes only a started stroke"),
        };
    }
    match *placed {
        [start, start_aim, end_aim, ..] => {
            path.set_biarc(start, start_aim - start, live, live - end_aim)
        }
        [start, start_aim] => path.set_fillet(start, start_aim, live),
        [start] => super::ghost(start, live, path),
        [] => unreachable!("the session shapes only a started stroke"),
    }
}

/// The S-curve's control polygon's inner points: the first arc's control point, the joint and
/// the second arc's control point (each arc's control point is where its end tangents meet), or
/// `None` unless `path` is a biarc's two pieces. `O(1)`.
pub(crate) fn biarc_controls(path: &GuidePath) -> Option<[DVec2; 3]> {
    let [head, tail] = path.segments() else {
        return None;
    };
    Some([control_point(head)?, head.end(), control_point(tail)?])
}

/// Where a one-piece draft's end tangents meet (a continuous S-curve's implied start aim before
/// its end aim), or `None` unless `path` is one line or arc. `O(1)`.
pub(crate) fn single_control(path: &GuidePath) -> Option<DVec2> {
    let [segment] = path.segments() else {
        return None;
    };
    control_point(segment)
}

// Where a piece's end tangents meet: an arc's tangent length is `R tan(|sweep| / 2)`, which in
// the stored form is `chord (1 + b^2) / (2 (1 - b^2))` with the bulge `b = tan(sweep / 4)`; a
// line's is half its length (the equal-tangent rule puts the control point at its middle).
// `None` for a half turn or more, whose tangents meet behind it (a biarc's halves never turn
// that far).
fn control_point(segment: &GuideSegment) -> Option<DVec2> {
    let (start, end) = (segment.start(), segment.end());
    let bulge_sq = match segment {
        GuideSegment::Line(_) => 0.0,
        GuideSegment::Arc(arc) => arc.bulge() * arc.bulge(),
        GuideSegment::Bezier(_) => return None,
    };
    if bulge_sq >= 1.0 {
        return None;
    }
    let tangent_length = start.distance(end) * (1.0 + bulge_sq) / (2.0 * (1.0 - bulge_sq));
    Some(start + segment.start_tangent() * tangent_length)
}
