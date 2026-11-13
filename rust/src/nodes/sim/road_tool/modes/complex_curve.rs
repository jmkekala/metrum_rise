// SPDX-License-Identifier: GPL-2.0-only

//! Complex curve: the biarc S-curve (`GuidePath::set_biarc`, equal tangent lengths). Clicks: the
//! start `A`, a point giving the start tangent (`A -> TA`), a point giving the end tangent, then
//! the end `B`, which the road reaches along `TB -> B`. Before the last click the draft is the
//! ghost line from the last placed point.

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath};

pub(super) fn shape(placed: &[DVec2], live: DVec2, path: &mut GuidePath) -> Result<(), GuideError> {
    match *placed {
        [start, start_aim, end_aim, ..] => {
            path.set_biarc(start, start_aim - start, live, live - end_aim)
        }
        [.., last] => super::ghost(last, live, path),
        [] => unreachable!("the session shapes only a started stroke"),
    }
}
