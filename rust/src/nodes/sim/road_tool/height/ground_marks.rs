// SPDX-License-Identifier: GPL-3.0-or-later

//! A draft's height display (`ROAD-112` round H2, the user's 2026-11-14 test drive): at each end
//! off the ground, a ground marker (a vertical line from the end to the ground directly under or
//! over it, and a ring on that ground around the foot) and a height chip (the end's signed height
//! above that ground, "+6.5 m", and the part the end is on: ground, bridge, cut or tunnel).
//!
//! The ground is the one the draft's segmentation reads ([`DraftGround`]: the highest grounded
//! carriageway at the point, else the source terrain), so a marker's foot and a chip's height are
//! the clearance the segmentation classifies the end by. An end gets a marker when its height
//! differs from that ground by more than [`GROUND_MARK_MIN_M`], or while its grip is hovered or
//! dragged. The session pushes the markers on every pose ([`push_ground_mark`], `O(ring points x
//! pieces in a query cell)`, no allocation once the gizmo buffers have grown); the bridge adds
//! the chips once a frame ([`push_height_labels`]), after the frame's segmentation, whose end
//! runs name the parts.

use super::super::frame::{DraftRun, GizmoLineClass, Gizmos};
use super::super::labels::{EndPart, LabelKind, LabelTone, ToolLabel};
use super::handles::{DraftEnd, HeightHandle};
use crate::simulation::network::guide::DVec2;
use crate::simulation::network::road_edit::{DraftGround, PREVIEW_CLEARANCE_M, RoadProfile};
use godot::prelude::Vector3;
use std::f32::consts::TAU;

/// An end whose height differs from the ground under it by more than this many metres gets a
/// ground marker and a height chip: half a height step, so every stepped offset shows, while an
/// end on the ground (a picked point a few centimetres off the source terrain) shows none.
pub const GROUND_MARK_MIN_M: f32 = 0.25;
/// The ground ring's radius, metres.
pub const GROUND_RING_RADIUS_M: f32 = 2.0;
// Segments of the ground ring.
const GROUND_RING_SEGMENTS: usize = 24;

/// One end's ground marker.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroundMark {
    /// Which end.
    pub end: DraftEnd,
    /// The end at its height.
    pub point: Vector3,
    /// The ground's height under the end ([`DraftGround`]), metres.
    pub ground_m: f32,
}

impl GroundMark {
    /// The end's signed height above the ground, metres (negative below it).
    pub fn height_above_m(&self) -> f32 {
        self.point.y - self.ground_m
    }

    /// The marker's foot: the ground point directly under (or over) the end.
    pub fn foot(&self) -> Vector3 {
        Vector3::new(self.point.x, self.ground_m, self.point.z)
    }
}

/// The ground marker of `handle`'s end on `ground`, when the end is more than
/// [`GROUND_MARK_MIN_M`] off it or `held` (its grip hovered or dragged); its gizmos into
/// `gizmos`: the vertical line from the end to its foot and the ring around the foot, each ring
/// point on the ground at its own XZ. `O(ring points x pieces in a query cell)`, no allocation
/// beyond growing `gizmos`.
pub(crate) fn push_ground_mark(
    handle: &HeightHandle,
    held: bool,
    ground: &DraftGround,
    gizmos: &mut Gizmos,
) -> Option<GroundMark> {
    let point = handle.point();
    let mark = GroundMark {
        end: handle.end,
        point,
        ground_m: ground.height_at(point.x, point.z),
    };
    if !held && mark.height_above_m().abs() <= GROUND_MARK_MIN_M {
        return None;
    }
    gizmos.push_line(GizmoLineClass::GroundMarker, [point, mark.foot()]);
    gizmos.begin_line(GizmoLineClass::GroundMarker);
    for step in 0..=GROUND_RING_SEGMENTS {
        // The last point closes the ring on the first, bit for bit.
        let angle = (step % GROUND_RING_SEGMENTS) as f32 / GROUND_RING_SEGMENTS as f32 * TAU;
        let x = point.x + GROUND_RING_RADIUS_M * angle.cos();
        let z = point.z + GROUND_RING_RADIUS_M * angle.sin();
        gizmos.push_point(Vector3::new(x, ground.height_at(x, z), z));
    }
    Some(mark)
}

/// The part an end with this `clearance_m` above its ground is on, given the profile of the
/// draft's run at that end (`None`: no run, the segmentation refused the draft or there is no
/// draft yet): a bridge or tunnel run's own; a held ground run's cut below
/// [`PREVIEW_CLEARANCE_M`] under the ground, else ground; a follow-ground run's inferred bridge
/// more than [`PREVIEW_CLEARANCE_M`] above it (the solve's own rule), else ground; with no run,
/// a bridge above the ground and a cut below it.
pub(crate) fn end_part(profile: Option<RoadProfile>, clearance_m: f32) -> EndPart {
    match profile {
        Some(RoadProfile::Bridge) => EndPart::Bridge,
        Some(RoadProfile::Tunnel) => EndPart::Tunnel,
        Some(RoadProfile::HeldGround) if clearance_m < -PREVIEW_CLEARANCE_M => EndPart::Cut,
        Some(RoadProfile::FollowGround) if clearance_m > PREVIEW_CLEARANCE_M => EndPart::Bridge,
        Some(RoadProfile::HeldGround | RoadProfile::FollowGround) => EndPart::Ground,
        None if clearance_m > 0.0 => EndPart::Bridge,
        None => EndPart::Cut,
    }
}

/// Appends a height chip for each marked end (`marks`, start and end) of the draft whose stroke
/// points are `points` and whose runs, in drawing order, are `runs` (the frame's segmentation of
/// the same stroke): the end's signed height above its ground with its part ([`end_part`] of the
/// run at that end), anchored at the end and pushed to the left of travel there (the end's other
/// chips sit to its right). None without a stroke (fewer than two points: no draft, a placed
/// start with the cursor on it, a refused shape), whose ends have no part yet; their markers
/// still show. `O(1)`, no allocation beyond growing `out`.
pub fn push_height_labels(
    marks: &[Option<GroundMark>; 2],
    points: &[Vector3],
    runs: &[DraftRun],
    out: &mut Vec<ToolLabel>,
) {
    let along = |from: Option<&Vector3>, to: Option<&Vector3>| {
        from.zip(to)
            .and_then(|(from, to)| {
                DVec2::new(f64::from(to.x - from.x), f64::from(to.z - from.z)).try_normalize()
            })
            .unwrap_or(DVec2::X)
    };
    if points.len() < 2 {
        return;
    }
    for mark in marks.iter().flatten() {
        let (run, tangent) = match mark.end {
            DraftEnd::Start => (runs.first(), along(points.first(), points.get(1))),
            DraftEnd::End => (
                runs.last(),
                along(
                    points.len().checked_sub(2).and_then(|i| points.get(i)),
                    points.last(),
                ),
            ),
        };
        let clearance_m = mark.height_above_m();
        out.push(ToolLabel {
            kind: LabelKind::EndHeight(end_part(run.map(|run| run.profile), clearance_m)),
            value: f64::from(clearance_m),
            anchor: mark.point,
            side: tangent.perp(),
            tone: LabelTone::Normal,
        });
    }
}
