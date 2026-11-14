// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's output record (`ROAD-112` round B, plan 2.4): what `road_tool_frame` will
//! return once round C wires it. The session rebuilds it in place on every event; its buffers are
//! reused, so a pose allocates nothing once they have grown. Round B calls no preview: the frame
//! carries the stroke request the existing back end takes (`preview_road` / `add_road_with_snap`
//! points, lanes and type), which round C hands to the preview.

use super::measure::{HudValues, RoadTypeSpec};
use super::modes::DrawMode;
use super::snap::SnapKind;
use godot::prelude::Vector3;

/// One stroke as the existing back end takes it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StrokeRequest {
    /// The polyline: a straight stroke's two ends exactly; a curve flattened by the kernel's rule
    /// (4 m steps at most, sag at most 5 cm).
    pub points: Vec<Vector3>,
    /// The drawn road type.
    pub spec: RoadTypeSpec,
    /// Whether the plan snaps the stroke's ends to existing roads (the network snap toggle).
    pub snap_to_existing_roads: bool,
}

impl StrokeRequest {
    fn clear(&mut self) {
        self.points.clear();
    }
}

/// A gizmo line strip's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoLineClass {
    /// The draft's own centreline.
    Draft,
    /// The guide line that took the cursor.
    Guide,
    /// The angle lock's ray.
    Angle,
    /// A curve's tangent leg (start to control point, control point to end).
    Tangent,
}

/// A gizmo handle's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GizmoHandleKind {
    /// A placed end of the stroke.
    Endpoint,
    /// A placed control or tangent point.
    Control,
    /// The start of a stroke that continues the chain from the last accepted stroke's end.
    ChainStart,
    /// The live end at the cursor.
    Cursor,
}

/// Packed gizmos: line strips (`line_points[line_starts[i]..line_starts[i + 1]]`, the last to
/// the end) with a class each, and handles.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gizmos {
    /// All strip points, strips back to back.
    pub line_points: Vec<Vector3>,
    /// Index of each strip's first point.
    pub line_starts: Vec<u32>,
    /// Each strip's class.
    pub line_classes: Vec<GizmoLineClass>,
    /// Handle positions.
    pub handles: Vec<Vector3>,
    /// Each handle's kind.
    pub handle_kinds: Vec<GizmoHandleKind>,
}

impl Gizmos {
    fn clear(&mut self) {
        self.line_points.clear();
        self.line_starts.clear();
        self.line_classes.clear();
        self.handles.clear();
        self.handle_kinds.clear();
    }

    /// Appends one strip.
    pub(crate) fn push_line(
        &mut self,
        class: GizmoLineClass,
        points: impl IntoIterator<Item = Vector3>,
    ) {
        self.line_starts.push(self.line_points.len() as u32);
        self.line_classes.push(class);
        self.line_points.extend(points);
    }

    /// Appends one handle.
    pub(crate) fn push_handle(&mut self, kind: GizmoHandleKind, position: Vector3) {
        self.handles.push(position);
        self.handle_kinds.push(kind);
    }
}

/// The resolved cursor.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CursorState {
    /// Resolved position.
    pub position: Vector3,
    /// What took it.
    pub snap: SnapKind,
    /// Whether the length step rounded it.
    pub stepped: bool,
    /// Whether a node took the cursor off the angle lock.
    pub angle_broken: bool,
    /// Whether the pointer resolved to a point at all.
    pub valid: bool,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            position: Vector3::ZERO,
            snap: SnapKind::Free,
            stepped: false,
            angle_broken: false,
            valid: false,
        }
    }
}

/// The session's phase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    /// No stroke: the next click places a start.
    #[default]
    Idle,
    /// Placing the stroke's next point; `placed` points are down (the start included).
    Placing {
        /// Points placed so far.
        placed: u8,
    },
    /// The stroke was sent; clicks wait for its commit acknowledgement.
    AwaitingAck,
}

/// Why the draft has no stroke request this pose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DraftRefusal {
    /// The draft's points coincide or its ends are within 5 cm (today's `too_short`).
    TooShort,
    /// The simple curve's legs fold straight back.
    FoldBack,
    /// The end lies straight behind the start tangent.
    FullTurn,
    /// The biarc has no solution for these tangents.
    NoBiarc,
    /// An arc is tighter than the road type's illegal radius as the radius check reads it
    /// (`road_tight_curve`).
    TightCurve,
}

/// The state record.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ToolState {
    /// The drawing mode.
    pub mode: DrawMode,
    /// The phase.
    pub phase: Phase,
    /// Why the draft makes no stroke, if it makes none while placing.
    pub refusal: Option<DraftRefusal>,
}

/// One `road_tool_frame` record.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RoadToolFrame {
    /// The draft's stroke request for this pose (what the preview will be called with); empty
    /// points when there is none.
    pub stroke: StrokeRequest,
    /// The stroke to commit, set by the confirming click and kept until the acknowledgement.
    pub commit: Option<StrokeRequest>,
    /// Gizmos.
    pub gizmos: Gizmos,
    /// HUD numbers (zero while no draft has a stroke).
    pub hud: HudValues,
    /// The cursor.
    pub cursor: CursorState,
    /// Mode, phase, refusal.
    pub state: ToolState,
}

impl RoadToolFrame {
    // Clears the per-pose parts, keeping their buffers and the pending commit.
    pub(crate) fn clear_pose(&mut self) {
        self.stroke.clear();
        self.gizmos.clear();
        self.hud = HudValues::default();
        self.state.refusal = None;
    }
}
