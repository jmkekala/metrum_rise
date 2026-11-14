// SPDX-License-Identifier: GPL-2.0-only

//! Straight mode: two clicks, one line. Its stroke request is the line's two ends, exactly the
//! two points today's tool sends (round A decision D2: the kernel's 4 m flattening is not sent).
//! In continuous mode at a chained start (the carried tangent) it is the line or arc leaving the start along the
//! tangent through the cursor (`GuideSegment::tangent_to`): a line only when the cursor lies
//! exactly ahead.

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment};

pub(super) fn shape(
    start: DVec2,
    live: DVec2,
    tangent: Option<DVec2>,
    path: &mut GuidePath,
) -> Result<(), GuideError> {
    let Some(tangent) = tangent else {
        return super::ghost(start, live, path);
    };
    path.clear();
    path.push(GuideSegment::tangent_to(start, tangent, live)?);
    Ok(())
}
