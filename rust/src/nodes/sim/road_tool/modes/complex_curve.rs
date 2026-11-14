// SPDX-License-Identifier: GPL-2.0-only

//! Complex curve: the biarc S-curve (`GuidePath::set_biarc`, equal tangent lengths). Clicks: the
//! start `A`, a point giving the start tangent (`A -> TA`), a point giving the end tangent, then
//! the end `B`, which the road reaches along `TB -> B`. Before the last click there is no road
//! to draw yet (round N): the aim legs are gizmos only, never previewed as road.

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment};

pub(super) fn shape(placed: &[DVec2], live: DVec2, path: &mut GuidePath) -> Result<(), GuideError> {
    match *placed {
        [start, start_aim, end_aim, ..] => {
            path.set_biarc(start, start_aim - start, live, live - end_aim)
        }
        [.., last] => super::ghost(last, live, path),
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
