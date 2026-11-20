// SPDX-License-Identifier: GPL-3.0-or-later

//! Road-tool drawing modes (`ROAD-112` round B): one small state machine per mode, turning the
//! clicks placed so far and the live cursor into the draft's guide path.
//!
//! Every mode places points one click at a time: [`DrawMode::points`] is how many a stroke takes
//! (the start included); the last click confirms the stroke. Continuous mode adds the tangent
//! constraint to a stroke with a start tangent (a carried one, or a road it starts on): a
//! straight stroke becomes the line or arc leaving along it through the cursor (plan 2.2), a
//! simple curve's control point is held on its ray (U1). A continuous chain (round CT, the
//! user's decision replacing B8) also carries its direction into the S-curve, as its start aim,
//! and freeform, as its start's handle direction. Freeform (round N) keeps its nodes' handles
//! and its edit phase in `freeform::Freeform`, which the session drives beside its placed points.

mod complex_curve;
pub mod freeform;
mod simple_curve;
mod straight;

pub(crate) use complex_curve::{biarc_controls, single_control};
pub(crate) use simple_curve::held_control;

use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment, Line};

/// The drawing mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DrawMode {
    /// Two clicks: a straight line.
    #[default]
    Straight,
    /// Three clicks with a free control point (U1): the exact fillet arc plus a straight lead.
    SimpleCurve,
    /// Four clicks: start, a point giving the start tangent, a point giving the end tangent, end;
    /// the biarc S-curve.
    ComplexCurve,
    /// Freeform (round N): clicks place the nodes of cubic Bezier spans (up to
    /// [`freeform::MAX_NODES`]), whose handles the player edits before the road is built.
    Freeform,
}

impl DrawMode {
    /// Points a stroke takes, the start and the confirming end included.
    pub fn points(self) -> usize {
        match self {
            Self::Straight => 2,
            Self::SimpleCurve => 3,
            Self::ComplexCurve => 4,
            Self::Freeform => freeform::MAX_NODES,
        }
    }
}

/// Fills `path` with the draft of `mode` from the placed points (`placed[0]` the start) and the
/// live cursor `live`, which stands for the next point. `tangent` is the unit start tangent the
/// continuous constraint holds (`None` when continuous is off or the stroke has none; for the
/// S-curve the carried start aim, which then is not among `placed`). While
/// points are missing the path is the ghost line from the last placed point, except the
/// S-curve's: the line from its start, then the fillet through its start aim. `O(1)`, no
/// allocation once `path` holds three segments. Freeform drafts shape through
/// `freeform::Freeform::shape` (they keep handles); here they draw the ghost line.
pub fn shape(
    mode: DrawMode,
    placed: &[DVec2],
    live: DVec2,
    tangent: Option<DVec2>,
    path: &mut GuidePath,
) -> Result<(), GuideError> {
    let Some(&start) = placed.first() else {
        path.clear();
        return Ok(());
    };
    match mode {
        DrawMode::Straight => straight::shape(start, live, tangent, path),
        DrawMode::SimpleCurve => simple_curve::shape(placed, live, tangent, path),
        DrawMode::ComplexCurve => complex_curve::shape(placed, live, tangent, path),
        DrawMode::Freeform => ghost(*placed.last().expect("a started stroke"), live, path),
    }
}

// The ghost line from the last placed point to the cursor, or nothing when they coincide.
fn ghost(from: DVec2, to: DVec2, path: &mut GuidePath) -> Result<(), GuideError> {
    path.clear();
    if from == to {
        return Err(GuideError::Degenerate);
    }
    path.push(GuideSegment::Line(Line::new(from, to)));
    Ok(())
}
