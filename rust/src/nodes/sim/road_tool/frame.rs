// SPDX-License-Identifier: GPL-2.0-only

//! The road tool's output record (`ROAD-112` round B, plan 2.4): what `road_tool_frame` will
//! return once round C wires it. The session rebuilds it in place on every event; its buffers are
//! reused, so a pose allocates nothing once they have grown. Round B calls no preview: the frame
//! carries the stroke request the existing back end takes (`preview_road` / `add_road_with_snap`
//! points, lanes and type), which round C hands to the preview.

use super::height::DraftStroke;
use super::height::ground_marks::GroundMark;
use super::height::handles::HeightHandle;
use super::labels::ToolLabel;
use super::measure::{HudValues, RoadTypeSpec};
use super::modes::DrawMode;
use super::snap::SnapKind;
use crate::simulation::network::road_edit::RoadProfile;
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
    /// The grade line's world heights at the first and last point (round I): `Some` when a
    /// free end's handle is off its support or straight grade is on, and the points' heights
    /// are then that grade line, which the bridge segments into ground, bridge and tunnel
    /// strokes (`height::segment_draft`). `None`: the stroke follows the ground, sent as it is
    /// (`RoadProfile::FollowGround`).
    pub grade: Option<[f32; 2]>,
}

impl StrokeRequest {
    fn clear(&mut self) {
        self.points.clear();
        self.grade = None;
    }
}

/// One run of a draft's profile, in drawing order (round I, for the HUD's chips).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DraftRun {
    /// How the run is built: `FollowGround` for a draft without a grade line (one run), else
    /// the segmentation's class (held ground, bridge, tunnel).
    pub(crate) profile: RoadProfile,
    /// Horizontal length in metres.
    pub length_m: f64,
    /// Slope in percent along the drawing direction: the grade line's for a graded draft, the
    /// drawn ends' mean for a follow-ground one.
    pub slope_percent: f64,
}

/// The draft's heights as the frame's preview step reads them (round I): filled once a frame by
/// `RoadToolSession::update_draft`, never on a pointer move (segmentation reads the ground and
/// tests bore cover, which allocate).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DraftReport {
    /// The graded draft's strokes (`height::segment_draft`); empty for a follow-ground draft or
    /// a refused one.
    pub(crate) strokes: Vec<DraftStroke>,
    /// The segmentation's refusal (`cut_too_deep`).
    pub refusal: Option<&'static str>,
    /// The runs with their class, length and slope.
    pub runs: Vec<DraftRun>,
}

impl DraftReport {
    pub(crate) fn clear(&mut self) {
        self.strokes.clear();
        self.refusal = None;
        self.runs.clear();
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
    /// A curve's tangent leg (start to control point, control point to end), the S-curve's
    /// control polygon (start, first arc's control point, joint, second arc's control point,
    /// end) and a freeform node's handle stems.
    Tangent,
    /// A height handle's stem, from the end's support up to its grip (round I; drawn in the
    /// guide look, round V2).
    HeightStem,
    /// The zoning-depth guide's outside (round V items 4 and 9): every point whose distance to
    /// the stroke equals the depth the road's zoning reaches (the side offsets and the end
    /// circles' outer caps).
    ZoningOutside,
    /// The zoning-depth guide's inside: the parts of the end circles over the stroke (nearer to
    /// it than the depth).
    ZoningInside,
    /// A curb the zoning-grid snap aligned, and its tick on the end's cell line (round V item 8).
    GridCurb,
    /// An end's ground marker (round H2): the vertical line from the end down (or up) to the
    /// ground under it, and the ring on the ground around that point.
    GroundMarker,
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
    /// The live end at the cursor, and the freeform grip being dragged.
    Cursor,
    /// Where two pieces of the draft meet: the S-curve's joint between its arcs, a freeform
    /// draft's inner node.
    Joint,
    /// A control point the player does not place: an S-curve arc's control point (where its end
    /// tangents meet), a freeform node's Bezier handle.
    Handle,
    /// A free end's height grip (round I; drawn as a handle until the restyle gives it a look).
    HeightGrip,
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

    /// Starts a strip of `class`; [`Self::push_point`] appends its points.
    pub(crate) fn begin_line(&mut self, class: GizmoLineClass) {
        self.line_starts.push(self.line_points.len() as u32);
        self.line_classes.push(class);
    }

    /// Appends a point to the strip begun last.
    pub(crate) fn push_point(&mut self, point: Vector3) {
        self.line_points.push(point);
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
    /// A freeform draft's edit phase (round N): every node is placed; a press on a node or
    /// handle drags it, any other press or `confirm` sends the stroke.
    Editing,
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
    /// The draft's flattening passes its step cap (`FLATTEN_MAX_STEPS`: longer than a batch's
    /// 8 km at the finest step; `batch_too_large`).
    TooLong,
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
    /// The draft's pieces in order (its guide segments: a curve's lead and arc, an S-curve's two
    /// arcs, a freeform draft's spans): each one's length in metres (horizontal, along the
    /// guide), for labels on the geometry. Empty while no draft has a stroke.
    pub piece_lengths_m: Vec<f64>,
    /// Each piece's smallest radius in metres, infinite for a straight piece.
    pub piece_radii_m: Vec<f64>,
    /// Each piece's slope in percent along the drawing direction (round I): the grade line's
    /// for a graded draft; for a follow-ground draft the source terrain's between the piece's
    /// ends (the drawn ends' own heights at the draft's ends), which the solve follows.
    pub piece_slopes_percent: Vec<f64>,
    /// The draft's height handles (round I): the start's and the end's (the live end's while
    /// placing; before a stroke starts, the cursor's, whose offset a start placed there takes).
    pub height_handles: [Option<HeightHandle>; 2],
    /// Each end's ground marker (round H2: start, end), for an end off the ground under it or
    /// whose grip is hovered or dragged; its gizmos are in [`Self::gizmos`], its chip the
    /// bridge's ([`super::height::ground_marks::push_height_labels`]).
    pub ground_marks: [Option<GroundMark>; 2],
    /// The draft's heights, segmentation and slopes (`RoadToolSession::update_draft`).
    pub draft: DraftReport,
    /// The label chips on the geometry (round V): what each says, where it is anchored and to
    /// which side it is offset. Empty while no draft has a stroke.
    pub labels: Vec<ToolLabel>,
    /// The zoning depth of the drawn road type in metres (`None` for a type without zoning).
    pub zoning_depth_m: Option<f64>,
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
        self.piece_lengths_m.clear();
        self.piece_radii_m.clear();
        self.piece_slopes_percent.clear();
        self.height_handles = [None; 2];
        self.ground_marks = [None; 2];
        self.labels.clear();
        self.state.refusal = None;
    }
}
