// SPDX-License-Identifier: GPL-2.0-only

//! The road tool session (`ROAD-112` round B, plan 2.1-2.4): the one owner of tool state that
//! decides outcomes. It consumes the event protocol of plan 2.3 as plain Rust
//! ([`RoadToolEvent`], [`RoadToolOption`]) and keeps the frame of plan 2.4 ([`RoadToolFrame`])
//! current. Not wired yet: round C adds the bridge calls and hands the frame's stroke to the
//! preview and its commit to `add_road_with_snap`.
//!
//! Per pointer move: pick ([`pick_surface`]), snap ([`super::snap`]), shape (the mode's draft on
//! the guide kernel), measure; no preview. It reads only the published road-tool view and the
//! existing indices, never the core, and allocates nothing once its buffers have grown.
//!
//! Chain rules (today's, workflow 3): an accepted stroke starts the next at its end, carrying its
//! end tangent, unless it ended on the network (a node or edge snap), either end lies on the map
//! edge, or the player cancelled meanwhile; right-click or `cancel` ends the chain. A refused
//! commit leaves the stroke editable at its last point. With Continuous on (round CT, the user's
//! decision of 2026-11-14 replacing B8 / N7) the chain carries its direction into every mode: the
//! S-curve takes it as its start aim (that step is skipped), freeform locks its start's handle to
//! it, and a mode change while only the chained start is placed keeps the chain in the new mode.
//! Only a chained start carries a direction (round CT2): a first click on an existing road, a
//! node or a dead end that is not the chain's end aims freely in every mode.
//!
//! Freeform (round N, `modes/freeform.rs`): clicks place nodes, then an edit phase drags nodes
//! and handles (press, pointer moves, release) until `confirm` or a press off every grip sends
//! the stroke. A drag takes the network snap but no anchor-bound constraint (angle lock, length
//! step, guide lines from the anchor) and no self snap.
//!
//! Heights (round I, `height/handles.rs`): each end has a height handle, free (an offset above
//! its support) or locked to the road it joins. A press on a free placed end's grip (the start;
//! a freeform draft's last node in the edit phase) drags it until the release; PageUp/PageDown
//! ([`RoadToolAction::RaiseHandle`], [`RoadToolAction::LowerHandle`]) move the active handle by
//! the same step: the live end's while drawing (a new start takes the live end's offset), the
//! hovered or last dragged one in the edit phase. A free end off its support, or straight grade,
//! gives the stroke a grade line ([`StrokeRequest::grade`]); [`RoadToolSession::update_draft`]
//! segments it once a frame.

use super::frame::{
    DraftRefusal, DraftRun, GizmoHandleKind, GizmoLineClass, Gizmos, Phase, RoadToolFrame,
    StrokeRequest,
};
use super::height::ground_marks::push_ground_mark;
use super::height::handles::{
    DraftEnd, EndHeights, HeightHandle, drag_offset, grip_at, stepped_by,
};
use super::height::{DraftSection, apply_grade_line, segment_draft};
use super::guides::{
    ZoningScratch, grid_curb_offset_m, push_alignment_guide, push_grid_curbs, push_near_guide,
    push_zoning_guide, zoning_depth_m,
};
use super::labels::{draft_hint, push_draft_labels, push_freeform_labels, push_hint};
use super::measure::{EndRoad, RoadTypeSpec, illegal_radius_m, measure};
use super::modes::freeform::{Freeform, Grip, MAX_NODES};
use super::modes::{self, DrawMode};
use super::pick::{
    NETWORK_SNAP_REACH_M, SELF_SNAP_M, SurfacePick, half_extents, is_near_border, lane_width_m,
    level_height, pick_surface, snap_to_border,
};
use super::snap::{
    GuideLine, SnapInput, SnapKind, SnapOptions, SnapScratch, Snapped, SquareAxis,
    apply_length_step, collect_guides, edge_direction_at, holding_guides, near_guides,
    nearest_guide, network_snap, resolve, square_lock, xz_of,
};
use crate::nodes::sim::core::RoadToolQuerySnapshot;
use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment};
use crate::simulation::network::interaction::{
    NetworkSnapTarget, StartDirection, StructureHit, road_start_direction,
};
use crate::simulation::network::road_edit::{DraftGround, PREVIEW_CLEARANCE_M, RoadProfile};
use godot::prelude::Vector3;

/// Today's map-edge snap distance (`MAP_BORDER_SNAP_DIST_M`).
pub const BORDER_SNAP_M: f32 = 25.0;
/// Today's release radius of a retained network snap (`ROAD_NETWORK_SNAP_RELEASE_DIST_M`).
pub const SNAP_RELEASE_M: f32 = 8.0;
// Today's `ROAD_SURFACE_POINT_EPS_M`: a straight stroke's ends must lie further apart.
const POINT_EPS_M: f32 = 0.05;
// Most points a mode places (freeform's nodes).
const MAX_POINTS: usize = MAX_NODES;
// Angle lock steps the option accepts, in degrees.
const ANGLE_STEPS_DEG: [f64; 4] = [15.0, 30.0, 45.0, 90.0];

/// Where a pointer event points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    /// A camera ray.
    Ray {
        /// Ray origin.
        origin: Vector3,
        /// Ray direction.
        dir: Vector3,
    },
    /// A world position: taken as a ground hit there, then snapped.
    World(Vector3),
    /// A scripted world position (tests, benchmarks and probes, today's `set_scripted_pointer`):
    /// the cursor exactly, neither picked nor snapped.
    Scripted(Vector3),
}

/// A pointer button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointerButton {
    /// Places points.
    Left,
    /// Cancels the stroke and ends the chain.
    Right,
}

/// A named action (key bindings stay in GDScript; meanings are Rust's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoadToolAction {
    /// Drops the stroke and ends the chain.
    Cancel,
    /// Places the stroke's confirming point at the cursor (the last click's meaning); in
    /// freeform it ends the placing (two nodes or more), and in the edit phase sends the stroke.
    Confirm,
    /// Switches to straight mode (drops the stroke; nothing when already straight).
    ModeStraight,
    /// Switches to the simple curve (drops the stroke; nothing when already in it).
    ModeSimpleCurve,
    /// Switches to the complex curve (drops the stroke; nothing when already in it).
    ModeComplexCurve,
    /// Switches to freeform (drops the stroke; nothing when already in it).
    ModeFreeform,
    /// Raises the active height handle by one step (PageUp): the live end's while drawing, the
    /// hovered or last dragged one in an edit phase. A locked end does not move.
    RaiseHandle,
    /// Lowers the active height handle by one step (PageDown).
    LowerHandle,
}

/// One input event (plan 2.3 `road_tool_event`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadToolEvent {
    /// The pointer moved.
    Pointer {
        /// Where it points.
        pointer: Pointer,
        /// Shift: frees the cursor of the angle lock, guide lines, grid and length step.
        shift: bool,
        /// Ctrl: a freeform handle drag breaks its node's joint (the handles move apart).
        ctrl: bool,
    },
    /// A button went down.
    Press(PointerButton),
    /// A button went up: ends a freeform drag.
    Release(PointerButton),
    /// A named action.
    Action(RoadToolAction),
    /// The commit acknowledgement of the sent stroke.
    CommitAck {
        /// Whether the core accepted it.
        committed: bool,
    },
}

/// One option (plan 2.3 `road_tool_set_option`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RoadToolOption {
    /// The drawn road type.
    RoadType(RoadTypeSpec),
    /// Snap to network nodes and edges.
    NetworkSnap(bool),
    /// Guide lines.
    GuideLines(bool),
    /// The zoning grid.
    ZoningGrid(bool),
    /// Angle lock step in degrees: 15, 30, 45 or 90; anything else turns it off.
    AngleStepDeg(Option<f64>),
    /// Length step in metres; zero, negative or `None` turns it off.
    LengthStepM(Option<f64>),
    /// Continuous: a chained start carries the previous stroke's end direction.
    Continuous(bool),
    /// Straight grade (U3): the grade line between the ends' heights even when both ends sit
    /// on their supports.
    StraightGrade(bool),
}

// The resolved cursor of the last pose.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cursor {
    position: Vector3,
    snapped: Snapped,
    // The pick before snapping (freeform grips are grabbed where the pointer is).
    raw: DVec2,
}

// What the session remembers of the stroke it sent, for the chain.
#[derive(Clone, Copy, Debug)]
struct Pending {
    chain: bool,
    // The player cancelled while it was pending: a refusal does not bring the stroke back.
    cancelled: bool,
    end: Vector3,
    end_tangent: Option<DVec2>,
}

// What the frame's draft report was made for (`RoadToolSession::update_draft`); its points
// buffer is reused.
#[derive(Debug, Default)]
struct DraftKey {
    made: bool,
    points: Vec<Vector3>,
    grade: Option<[f32; 2]>,
    spec: RoadTypeSpec,
    snap: bool,
    generation: u64,
}

impl DraftKey {
    fn matches(&self, stroke: &StrokeRequest, spec: RoadTypeSpec, generation: u64) -> bool {
        self.made
            && self.points == stroke.points
            && self.grade == stroke.grade
            && self.spec == spec
            && self.snap == stroke.snap_to_existing_roads
            && self.generation == generation
    }

    fn set(&mut self, stroke: &StrokeRequest, spec: RoadTypeSpec, generation: u64) {
        self.made = true;
        self.points.clear();
        self.points.extend_from_slice(&stroke.points);
        self.grade = stroke.grade;
        self.spec = spec;
        self.snap = stroke.snap_to_existing_roads;
        self.generation = generation;
    }
}

/// The road tool session: options, phase, the stroke's placed points, the draft and the frame.
#[derive(Debug)]
pub struct RoadToolSession {
    mode: DrawMode,
    snap: SnapOptions,
    spec: RoadTypeSpec,
    // The spec's illegal radius (`measure::illegal_radius_m`), kept with the spec: it allocates.
    illegal_radius_m: f64,
    continuous: bool,
    // The end heights (round I).
    heights: EndHeights,
    // The last pointer: a press grabs the height grip under it.
    pointer: Option<Pointer>,
    // What the draft report was made for (`update_draft`).
    draft_key: DraftKey,
    placed: [Vector3; MAX_POINTS],
    placed_len: usize,
    // How the road the stroke starts on leaves its start, or the carried tangent.
    start_road: Option<EndRoad>,
    start_on_border: bool,
    // The stroke starts where the last accepted one ended (the chain).
    chained: bool,
    // A continuous chain's S-curve: the carried unit direction standing for its start aim click
    // (round CT; not among the placed points).
    carried_aim: Option<DVec2>,
    // The retained network target and the view generation it was taken at.
    sticky: Option<(NetworkSnapTarget, u64)>,
    // The square lock's line held last pose (round UI1, `snap::square_lock`).
    square: Option<SquareAxis>,
    cursor: Option<Cursor>,
    shift: bool,
    ctrl: bool,
    pending: Option<Pending>,
    freeform: Freeform,
    path: GuidePath,
    flat: Vec<DVec2>,
    scratch: SnapScratch,
    zoning: ZoningScratch,
    frame: RoadToolFrame,
}

impl Default for RoadToolSession {
    fn default() -> Self {
        Self::new()
    }
}

impl RoadToolSession {
    /// A session in straight mode with the U2 defaults and the default two-lane road.
    pub fn new() -> Self {
        let spec = RoadTypeSpec::default();
        Self {
            mode: DrawMode::Straight,
            snap: SnapOptions::default(),
            spec,
            illegal_radius_m: illegal_radius_m(&spec),
            continuous: false,
            heights: EndHeights::default(),
            pointer: None,
            draft_key: DraftKey::default(),
            placed: [Vector3::ZERO; MAX_POINTS],
            placed_len: 0,
            start_road: None,
            start_on_border: false,
            chained: false,
            carried_aim: None,
            sticky: None,
            square: None,
            cursor: None,
            shift: false,
            ctrl: false,
            pending: None,
            freeform: Freeform::default(),
            path: GuidePath::new(),
            flat: Vec::new(),
            scratch: SnapScratch::default(),
            zoning: ZoningScratch::default(),
            frame: RoadToolFrame::default(),
        }
    }

    /// The current frame.
    pub fn frame(&self) -> &RoadToolFrame {
        &self.frame
    }

    /// The drawn road type.
    pub fn spec(&self) -> RoadTypeSpec {
        self.spec
    }

    /// The drawing mode.
    pub fn mode(&self) -> DrawMode {
        self.mode
    }

    // The draft's guide path (tests compare the stroke with its flattening).
    #[cfg(test)]
    pub(crate) fn path(&self) -> &GuidePath {
        &self.path
    }

    /// Points the stroke has placed (a freeform draft's nodes), the start included.
    pub fn placed_count(&self) -> usize {
        self.placed_len
    }

    /// Whether continuous (a chained start carries the previous stroke's direction) is on.
    pub fn continuous(&self) -> bool {
        self.continuous
    }

    /// Takes the stroke to commit, if the last confirming click produced one; the session keeps
    /// waiting for its [`RoadToolEvent::CommitAck`].
    pub fn take_commit(&mut self) -> Option<StrokeRequest> {
        self.frame.commit.take()
    }

    /// Sets one option. Snapping changes apply from the next pointer move; a road type change
    /// re-prices the draft at once.
    pub fn set_option(&mut self, option: RoadToolOption) {
        match option {
            RoadToolOption::RoadType(spec) => {
                if spec != self.spec {
                    self.illegal_radius_m = illegal_radius_m(&spec);
                }
                self.spec = spec;
            }
            RoadToolOption::NetworkSnap(on) => {
                self.snap.network = on;
                self.sticky = None;
            }
            RoadToolOption::GuideLines(on) => self.snap.guides = on,
            RoadToolOption::ZoningGrid(on) => self.snap.grid = on,
            RoadToolOption::AngleStepDeg(step) => {
                self.snap.angle_step_deg = step.filter(|step| ANGLE_STEPS_DEG.contains(step));
            }
            RoadToolOption::LengthStepM(step) => {
                self.snap.length_step_m = step.filter(|step| *step > 0.0);
            }
            RoadToolOption::Continuous(on) => {
                self.continuous = on;
                if self.only_chain_start() {
                    self.carry_chain();
                }
            }
            RoadToolOption::StraightGrade(on) => self.heights.straight_grade = on,
        }
    }

    /// Handles one event against the published view and brings the frame up to date.
    pub(crate) fn handle_event(&mut self, view: &RoadToolQuerySnapshot, event: RoadToolEvent) {
        match event {
            RoadToolEvent::Pointer {
                pointer,
                shift,
                ctrl,
            } => {
                self.shift = shift;
                self.ctrl = ctrl;
                self.pointer = Some(pointer);
                if let Some(end) = self.heights.grip {
                    // A height drag holds the cursor: the end stays where it is.
                    self.drag_height(end, pointer);
                } else {
                    self.cursor = self.resolve_cursor(view, pointer).or(self.cursor);
                    self.drag(view);
                    self.hover_height(pointer);
                }
            }
            RoadToolEvent::Press(PointerButton::Left) => {
                if !self.grab_height() {
                    self.click(view, false);
                }
            }
            RoadToolEvent::Action(RoadToolAction::Confirm) => self.click(view, true),
            RoadToolEvent::Press(PointerButton::Right)
            | RoadToolEvent::Action(RoadToolAction::Cancel) => self.cancel(),
            RoadToolEvent::Release(PointerButton::Left) => {
                self.heights.grip = None;
                self.freeform.release();
            }
            RoadToolEvent::Release(PointerButton::Right) => {}
            RoadToolEvent::Action(RoadToolAction::ModeStraight) => {
                self.set_mode(DrawMode::Straight)
            }
            RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve) => {
                self.set_mode(DrawMode::SimpleCurve)
            }
            RoadToolEvent::Action(RoadToolAction::ModeComplexCurve) => {
                self.set_mode(DrawMode::ComplexCurve)
            }
            RoadToolEvent::Action(RoadToolAction::ModeFreeform) => {
                self.set_mode(DrawMode::Freeform)
            }
            RoadToolEvent::Action(RoadToolAction::RaiseHandle) => self.step_height(1),
            RoadToolEvent::Action(RoadToolAction::LowerHandle) => self.step_height(-1),
            RoadToolEvent::CommitAck { committed } => self.acknowledge(view, committed),
        }
        self.refresh(view);
    }

    // A different mode drops the stroke; the current mode again changes nothing. With Continuous
    // on, a chain whose stroke has only its chained start placed keeps it (round CT).
    fn set_mode(&mut self, mode: DrawMode) {
        if mode == self.mode {
            return;
        }
        let keep = self.continuous && self.only_chain_start();
        if !keep {
            self.cancel();
        }
        self.mode = mode;
        if keep {
            self.heights.grip = None;
            self.sticky = None;
            self.carry_chain();
        }
    }

    // Whether the stroke is a chain's start alone: nothing else placed, nothing pending.
    fn only_chain_start(&self) -> bool {
        self.chained && self.pending.is_none() && self.placed_len == 1 && !self.freeform.editing()
    }

    // Continuous's one rule for every mode (rounds CT, CT2): only a chained start (the end of the
    // stroke the tool just built) carries a direction, the built stroke's end tangent. A first
    // click on an existing road, a node or a dead end that is not the chain's end carries none:
    // the stroke aims freely from it. Off, nothing is carried.
    fn carried_direction(&self) -> Option<DVec2> {
        match self.start_road {
            Some(EndRoad::Continue(direction)) if self.continuous && self.chained => {
                Some(direction)
            }
            _ => None,
        }
    }

    // A chained start carries `carried_direction` into the S-curve (its start aim) and freeform
    // (its start handle's direction). Straight and the simple curve read it live (`held_tangent`).
    fn carry_chain(&mut self) {
        let direction = self.carried_direction();
        self.carried_aim = direction.filter(|_| self.mode == DrawMode::ComplexCurve);
        self.freeform
            .lock_start(direction.filter(|_| self.mode == DrawMode::Freeform));
    }

    // The points the stroke's steps have set: the placed ones and a carried start aim.
    fn steps(&self) -> usize {
        self.placed_len + usize::from(self.carried_aim.is_some())
    }

    fn cancel(&mut self) {
        if let Some(pending) = &mut self.pending {
            // The sent stroke still lands; the chain does not continue from it.
            pending.chain = false;
            pending.cancelled = true;
            return;
        }
        self.placed_len = 0;
        self.start_road = None;
        self.start_on_border = false;
        self.chained = false;
        self.carried_aim = None;
        self.sticky = None;
        self.freeform.reset();
        self.heights.grip = None;
    }

    // Which height handle PageUp/PageDown move: the hovered or last dragged one in a freeform
    // draft's edit phase, else the live end's.
    fn active_end(&self) -> DraftEnd {
        if self.freeform.editing() {
            self.heights.active
        } else {
            DraftEnd::End
        }
    }

    // Which handles a press may grab: a placed start's, and a freeform draft's last node's in
    // the edit phase (the live end follows the cursor).
    fn grabbable(&self) -> [bool; 2] {
        let free = self.pending.is_none();
        [free && self.placed_len > 0, free && self.freeform.editing()]
    }

    // A press on a height grip grabs it (before any other meaning of the press).
    fn grab_height(&mut self) -> bool {
        let Some(pointer) = self.pointer else {
            return false;
        };
        let Some(end) = grip_at(&self.frame.height_handles, self.grabbable(), pointer) else {
            return false;
        };
        self.heights.grip = Some(end);
        self.heights.active = end;
        true
    }

    // A pointer move while a grip is held: the end's offset where the pointer puts the grip.
    fn drag_height(&mut self, end: DraftEnd, pointer: Pointer) {
        let Some(handle) = self.frame.height_handles[end as usize] else {
            return;
        };
        if let Some(offset_m) = drag_offset(&handle, pointer, self.shift) {
            self.heights.set_offset(end, offset_m);
        }
    }

    // In the edit phase the handle under the pointer becomes the one the keys move.
    fn hover_height(&mut self, pointer: Pointer) {
        if !self.freeform.editing() {
            return;
        }
        if let Some(end) = grip_at(&self.frame.height_handles, [true; 2], pointer) {
            self.heights.active = end;
        }
    }

    // PageUp/PageDown: the active handle by `steps` height steps, as a drag would set it; a
    // locked end does not move.
    fn step_height(&mut self, steps: i32) {
        let end = self.active_end();
        if self.frame.height_handles[end as usize].is_some_and(|handle| handle.locked) {
            return;
        }
        let offset_m = stepped_by(self.heights.offset(end), steps);
        self.heights.set_offset(end, offset_m);
    }

    // The pose's height handles for a draft from `start` to `end` (the ends on their supports),
    // into the frame.
    fn set_handles(&mut self, start: Vector3, end: Vector3, end_locked: bool) -> [HeightHandle; 2] {
        let mut handles = self.heights.handles(start, end, end_locked);
        let active = self.active_end();
        for handle in &mut handles {
            handle.active = handle.end == active;
        }
        self.frame.height_handles = handles.map(Some);
        handles
    }

    // A graded draft (a free end off its support, straight grade, or a locked end buried more
    // than 1.0 m under the terrain: a tunnel's buried end, from which a follow-ground stroke could
    // only climb through the ground): the stroke's heights become the grade line between the
    // handles' heights, and the request carries it. A locked end raised above the ground stays
    // today's follow-ground stroke, which infers its bridge.
    fn apply_grade(&mut self, view: &RoadToolQuerySnapshot, handles: &[HeightHandle; 2]) {
        let buried = |handle: &HeightHandle| {
            let ground = view
                .terrain
                .sample_height_world(handle.support.x, handle.support.z)
                * crate::config::HEIGHT_SCALE;
            handle.locked && handle.height_m < ground - PREVIEW_CLEARANCE_M
        };
        if !self.heights.graded(handles) && !handles.iter().any(buried) {
            return;
        }
        let heights = [handles[0].height_m, handles[1].height_m];
        apply_grade_line(&mut self.frame.stroke.points, heights);
        self.frame.stroke.grade = Some(heights);
    }

    // The HUD's heights and slopes of the pose: each end's height above its support (a locked
    // end's above the source terrain), the draft's slope between its ends over the guide's
    // length, and each piece's (`RoadToolFrame::piece_slopes_percent`). `O(pieces)`, no
    // allocation.
    fn measure_heights(&mut self, view: &RoadToolQuerySnapshot, handles: &[HeightHandle; 2]) {
        let terrain = |at: DVec2| {
            let height = view.terrain.sample_height_world(at.x as f32, at.y as f32);
            f64::from(height * crate::config::HEIGHT_SCALE)
        };
        let above = |handle: &HeightHandle| {
            if handle.locked {
                f64::from(handle.height_m) - terrain(xz_of(handle.support))
            } else {
                f64::from(handle.offset_m)
            }
        };
        let frame = &mut self.frame;
        frame.hud.start_height_m = Some(above(&handles[0]));
        frame.hud.end_height_m = Some(above(&handles[1]));
        let segments = self.path.segments();
        let length_m: f64 = segments.iter().map(GuideSegment::length).sum();
        if length_m <= 0.0 {
            return;
        }
        let ends = [f64::from(handles[0].height_m), f64::from(handles[1].height_m)];
        let grade = (ends[1] - ends[0]) / length_m * 100.0;
        frame.hud.slope_percent = Some(grade);
        let graded = frame.stroke.grade.is_some();
        let mut from = ends[0];
        for (index, segment) in segments.iter().enumerate() {
            let to = if index + 1 == segments.len() {
                ends[1]
            } else {
                terrain(segment.end())
            };
            let slope = match segment.length() {
                _ if graded => grade,
                run if run > 0.0 => (to - from) / run * 100.0,
                _ => 0.0,
            };
            frame.piece_slopes_percent.push(slope);
            from = to;
        }
    }

    // Each free handle's stem (support to grip) and grip, when
    // a press can grab it or it stands off its support. A locked end draws nothing of its own.
    fn push_height_gizmos(&mut self) {
        let grabbable = self.grabbable();
        let frame = &mut self.frame;
        for (handle, grabbable) in frame.height_handles.iter().zip(grabbable) {
            let Some(handle) =
                handle.filter(|handle| !handle.locked && (grabbable || handle.offset_m != 0.0))
            else {
                continue;
            };
            frame
                .gizmos
                .push_line(GizmoLineClass::HeightStem, [handle.support, handle.grip]);
            frame.gizmos.push_handle(GizmoHandleKind::HeightGrip, handle.grip);
        }
    }

    /// Brings the frame's draft report ([`RoadToolFrame::draft`]) up to date with the pose on
    /// `view`, once a frame (the bridge's `road_tool_frame`; never on a pointer move, since the
    /// segmentation allocates): a graded stroke's segmentation (`height::segment_draft`: its
    /// strokes or its refusal) and the runs' classes, lengths and slopes; a follow-ground
    /// stroke's one run. Returns whether the report changed (a new stroke, grade, road type,
    /// snap or view generation); otherwise it stands. `O(points)` to compare; a change costs
    /// the segmentation, `O(draft length / 4 m)` ground reads and one cover test per run below
    /// the ground.
    pub(crate) fn update_draft(&mut self, view: &RoadToolQuerySnapshot) -> bool {
        let stroke = &self.frame.stroke;
        if self.draft_key.matches(stroke, self.spec, view.surface_generation) {
            return false;
        }
        self.draft_key.set(stroke, self.spec, view.surface_generation);
        let draft = &mut self.frame.draft;
        draft.clear();
        let points = &stroke.points;
        if points.len() < 2 {
            return true;
        }
        let length_m: f64 = points
            .windows(2)
            .map(|pair| f64::from((pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z)))
            .sum();
        let (first, last) = (points[0], points[points.len() - 1]);
        let slope_percent = if length_m > 0.0 {
            f64::from(last.y - first.y) / length_m * 100.0
        } else {
            0.0
        };
        let Some(heights) = stroke.grade else {
            draft.runs.push(DraftRun {
                profile: RoadProfile::FollowGround,
                length_m,
                slope_percent,
            });
            return true;
        };
        let section = DraftSection {
            fwd_lanes: i32::from(self.spec.fwd_lanes),
            bkw_lanes: i32::from(self.spec.bkw_lanes),
            snap_to_existing_roads: stroke.snap_to_existing_roads,
            median: self.spec.median,
            parking: self.spec.parking,
            attributes: self.spec.attributes,
        };
        match segment_draft(
            points,
            heights,
            section,
            &view.terrain,
            &view.region_graph,
            &view.roadbed,
        ) {
            Ok(strokes) => {
                draft.runs.extend(strokes.iter().map(|part| DraftRun {
                    profile: part.profile,
                    length_m: part
                        .points
                        .windows(2)
                        .map(|pair| f64::from((pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z)))
                        .sum(),
                    slope_percent,
                }));
                draft.strokes = strokes;
            }
            Err(reason) => draft.refusal = Some(reason),
        }
        true
    }

    fn phase(&self) -> Phase {
        match (self.pending, self.placed_len) {
            (Some(_), _) => Phase::AwaitingAck,
            (None, 0) => Phase::Idle,
            _ if self.freeform.editing() => Phase::Editing,
            (None, _) => Phase::Placing {
                placed: self.steps() as u8,
            },
        }
    }

    // A left click (`confirm`: the confirming action): start a stroke, place the next point, or
    // send the stroke.
    fn click(&mut self, view: &RoadToolQuerySnapshot, confirm: bool) {
        let Some(cursor) = self.cursor.filter(|_| self.pending.is_none()) else {
            return;
        };
        if self.placed_len == 0 {
            if !(confirm && self.mode == DrawMode::Freeform) {
                self.begin(view, cursor.position, cursor.snapped);
            }
            return;
        }
        if self.mode == DrawMode::Freeform {
            self.freeform_click(cursor, confirm);
            return;
        }
        if self.steps() + 1 < self.mode.points() {
            let mut point = cursor.position;
            if self.mode == DrawMode::SimpleCurve
                && let Some(tangent) = self.held_tangent()
            {
                let held = modes::held_control(xz_of(self.placed[0]), xz_of(point), Some(tangent));
                point.x = held.x as f32;
                point.z = held.y as f32;
            }
            self.placed[self.placed_len] = point;
            self.placed_len += 1;
            // The control point is no connection target.
            self.sticky = None;
            return;
        }
        self.send(
            cursor.snapped.target.is_some(),
            cursor.snapped.kind == SnapKind::Border,
        );
    }

    // Sends the pose's stroke, unless the draft has none: the chain continues from its end
    // unless that end met the network or either end lies on the map edge.
    fn send(&mut self, end_on_network: bool, end_on_border: bool) {
        if self.frame.stroke.points.len() < 2 || self.frame.state.refusal.is_some() {
            return;
        }
        let border = self.start_on_border || end_on_border;
        self.pending = Some(Pending {
            chain: !end_on_network && !border,
            cancelled: false,
            end: *self.frame.stroke.points.last().expect("two points"),
            end_tangent: self.path.end_tangent(),
        });
        self.frame.commit = Some(self.frame.stroke.clone());
    }

    // A freeform click after the start. Placing: a click on the last node or `confirm` (two
    // nodes or more) starts the edit phase; any other click places a node, and a node on the
    // network or the map edge, or the last node there is room for, also starts it. Editing: a
    // press on a grip grabs it; any other press, or `confirm`, sends the stroke.
    fn freeform_click(&mut self, cursor: Cursor, confirm: bool) {
        if self.freeform.editing() {
            let nodes = self.placed_xz();
            let grip = self
                .freeform
                .grip_at(&nodes[..self.placed_len], cursor.raw, self.chained);
            match grip {
                Some(grip) if !confirm => self.freeform.grab(grip),
                _ => {
                    let (target, border) = self.freeform.snap_of(self.placed_len - 1);
                    self.send(target.is_some(), border);
                }
            }
            return;
        }
        let last = self.placed[self.placed_len - 1];
        let on_last = xz_of(last).distance(xz_of(cursor.position)) <= f64::from(POINT_EPS_M);
        if confirm || on_last {
            if self.placed_len >= 2 {
                self.freeform.start_editing();
            }
            return;
        }
        let index = self.placed_len;
        let border = cursor.snapped.kind == SnapKind::Border;
        self.placed[index] = cursor.position;
        self.placed_len += 1;
        self.freeform.place(index, cursor.snapped.target, border);
        if cursor.snapped.target.is_some() || border || self.placed_len == MAX_POINTS {
            self.freeform.start_editing();
        }
        self.sticky = None;
    }

    // A pointer move during a freeform drag: a node follows the cursor with its snap (a start
    // keeps its road's direction), a handle the cursor's ground position.
    fn drag(&mut self, view: &RoadToolQuerySnapshot) {
        let (Some(grip), Some(cursor)) = (self.freeform.grip(), self.cursor) else {
            return;
        };
        match grip {
            Grip::Node(index) => {
                self.placed[index] = cursor.position;
                self.freeform.place(
                    index,
                    cursor.snapped.target,
                    cursor.snapped.kind == SnapKind::Border,
                );
                if index == 0 {
                    self.heights.start_locked = cursor.snapped.target.is_some();
                    self.start_on_border = cursor.snapped.kind == SnapKind::Border;
                    self.start_road = cursor
                        .snapped
                        .target
                        .and_then(|target| {
                            road_start_direction(&view.region_graph, target, cursor.position)
                        })
                        .map(end_road);
                }
            }
            Grip::Inward(_) | Grip::Outward(_) => {
                let nodes = self.placed_xz();
                self.freeform.drag_handle(
                    &nodes[..self.placed_len],
                    xz_of(cursor.position),
                    self.ctrl,
                );
            }
        }
    }

    // Starts a stroke at `position`, reading how the road there leaves it.
    fn begin(&mut self, view: &RoadToolQuerySnapshot, position: Vector3, snapped: Snapped) {
        self.placed[0] = position;
        self.placed_len = 1;
        self.chained = false;
        self.carried_aim = None;
        // The start takes the live end's height: it was placed with it.
        self.heights.start_offset_m = self.heights.end_offset_m;
        self.heights.start_locked = snapped.target.is_some();
        self.heights.grip = None;
        self.start_on_border = snapped.kind == SnapKind::Border;
        self.freeform.reset();
        self.freeform.place(0, snapped.target, self.start_on_border);
        self.start_road = snapped
            .target
            .and_then(|target| road_start_direction(&view.region_graph, target, position))
            .map(end_road);
        self.sticky = None;
    }

    fn acknowledge(&mut self, view: &RoadToolQuerySnapshot, committed: bool) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        self.frame.commit = None;
        if !committed && !pending.cancelled {
            // The refusal keeps the stroke editable at its last point.
            return;
        }
        self.placed_len = 0;
        self.start_road = None;
        self.start_on_border = false;
        self.chained = false;
        self.sticky = None;
        self.freeform.reset();
        if pending.chain {
            self.begin(
                view,
                pending.end,
                Snapped {
                    xz: xz_of(pending.end),
                    kind: SnapKind::Free,
                    target: None,
                    stepped: false,
                    angle_broken: false,
                },
            );
            self.start_road = pending.end_tangent.map(EndRoad::Continue);
            self.chained = true;
            // The chain starts on the accepted stroke's end node: its graph height.
            self.heights.start_locked = true;
            self.carry_chain();
        }
    }

    // The stroke's start tangent toward `live` (an `Along` road takes the sense facing it).
    fn start_tangent(&self, live: DVec2) -> Option<DVec2> {
        let start = xz_of(self.placed[0]);
        match self.start_road? {
            EndRoad::Continue(direction) => Some(direction),
            EndRoad::Along(direction) if (live - start).dot(direction) < 0.0 => Some(-direction),
            EndRoad::Along(direction) => Some(direction),
        }
    }

    // The carried direction the continuous constraint holds, while it applies (before the second
    // point of a straight stroke or the control point of a simple curve; the S-curve and
    // freeform take a chain's direction once, `carry_chain`).
    fn held_tangent(&self) -> Option<DVec2> {
        let held = matches!(self.mode, DrawMode::Straight | DrawMode::SimpleCurve);
        (self.placed_len == 1 && held)
            .then(|| self.carried_direction())
            .flatten()
    }

    // The start direction the pose's draft leaves along: the held tangent; a simple curve's
    // once its control point is placed (held on the ray, whose stored `f32` point the shape
    // re-projects, so the curve leaves along the tangent exactly: round CT); or the S-curve's
    // carried start aim.
    fn draft_tangent(&self) -> Option<DVec2> {
        match (self.mode, self.placed_len) {
            (DrawMode::SimpleCurve, 2) => self.carried_direction(),
            _ => self.held_tangent().or(self.carried_aim),
        }
    }

    // Pick and snap one pointer pose. `None` when the pointer meets no surface.
    fn resolve_cursor(&mut self, view: &RoadToolQuerySnapshot, pointer: Pointer) -> Option<Cursor> {
        let (raw, hit_road, structure) = match pointer {
            Pointer::Ray { origin, dir } => match pick_surface(view, origin, dir, 0.0)? {
                SurfacePick::Surface {
                    position,
                    hit_road,
                    structure,
                } => (position, hit_road, structure.0),
                SurfacePick::Plane(position) => {
                    return Some(Cursor {
                        position,
                        raw: xz_of(position),
                        snapped: Snapped {
                            xz: xz_of(position),
                            kind: SnapKind::Border,
                            target: None,
                            stepped: false,
                            angle_broken: false,
                        },
                    });
                }
            },
            Pointer::World(position) => (position, false, StructureHit::Any),
            Pointer::Scripted(position) => {
                self.sticky = None;
                self.square = None;
                return Some(Cursor {
                    position,
                    raw: xz_of(position),
                    snapped: Snapped {
                        xz: xz_of(position),
                        kind: SnapKind::Free,
                        target: None,
                        stepped: false,
                        angle_broken: false,
                    },
                });
            }
        };
        let graph = &view.region_graph;
        let raw_xz = xz_of(raw);
        // A freeform drag has no anchor: the grip is not the stroke's next point.
        let dragging = self.freeform.grip().is_some();
        let anchor =
            (self.placed_len > 0 && !dragging).then(|| xz_of(self.placed[self.placed_len - 1]));
        let reference = self.reference(raw_xz).filter(|_| !dragging);
        let constrained = !self.shift;

        let network = self
            .snap
            .network
            .then(|| {
                network_snap(
                    graph,
                    raw,
                    structure,
                    self.sticky,
                    view.surface_generation,
                    SNAP_RELEASE_M,
                    NETWORK_SNAP_REACH_M,
                    &mut self.scratch,
                )
            })
            .flatten()
            .map(|snap| {
                let direction = match snap.target {
                    NetworkSnapTarget::Edge(edge) => {
                        edge_direction_at(graph, edge, xz_of(snap.position))
                    }
                    NetworkSnapTarget::Node(_) => None,
                };
                (snap, direction)
            });
        let grid = (constrained && self.snap.grid).then(|| {
            let start = (self.placed_len > 0).then(|| xz_of(self.placed[0]));
            let width = lane_width_m(
                i32::from(self.spec.fwd_lanes),
                i32::from(self.spec.bkw_lanes),
            );
            let (point, on) =
                view.zoning_grid
                    .snap_road_cursor_on(graph, raw_xz, start, f64::from(width));
            // With a start the grid snaps along an axis through it: that axis is the line.
            let dir = start.and_then(|start| (point - start).try_normalize());
            (point, dir, on)
        });
        if constrained && self.snap.guides {
            let first_axis = (self.steps() >= 2).then(|| {
                let a = xz_of(self.placed[0]);
                self.carried_aim
                    .or_else(|| (xz_of(self.placed[1]) - a).try_normalize())
                    .map(|dir| GuideLine { origin: a, dir })
            });
            collect_guides(
                graph,
                raw_xz,
                anchor,
                reference,
                first_axis.flatten(),
                &mut self.scratch,
            );
        } else {
            self.scratch.guides.clear();
        }
        // Round UI1: a first leg from a road or node takes the square lock, held across poses.
        let square = match (anchor, reference) {
            (Some(anchor), Some(reference))
                if constrained
                    && self.snap.guides
                    && self.placed_len == 1
                    && self.start_road.is_some() =>
            {
                square_lock(anchor, reference, raw_xz, self.square)
            }
            _ => None,
        };
        self.square = square.map(|(axis, _)| axis);
        let input = SnapInput {
            raw: raw_xz,
            anchor,
            reference,
            free: self.shift,
            network,
            grid: grid.map(|(point, dir, _)| (point, dir)),
            grid_edge: grid.and_then(|(_, _, on)| on),
            square: square.map(|(_, line)| line),
        };
        let mut snapped = resolve(&input, &self.snap, &self.scratch.guides);
        if snapped.kind == SnapKind::Free {
            snapped = self.border_or_self(view, snapped, !dragging);
        }
        if constrained {
            let guide = (snapped.kind == SnapKind::Guide)
                .then(|| {
                    input
                        .square
                        .or_else(|| nearest_guide(&self.scratch.guides, raw_xz))
                })
                .flatten();
            apply_length_step(&mut snapped, anchor, self.snap.length_step_m, guide);
        }

        let height = match (snapped.kind, network) {
            // An untouched pick keeps its hit, bit for bit (today's free cursor).
            (SnapKind::Free, _) if snapped.xz == raw_xz => raw.y,
            (SnapKind::Node | SnapKind::Edge, Some((snap, _))) => snap.position.y,
            (SnapKind::SelfPoint, _) => self.self_point(snapped.xz).map_or(raw.y, |p| p.y),
            _ => level_height(
                view,
                snapped.xz.x as f32,
                snapped.xz.y as f32,
                hit_road,
                raw.y,
                0.0,
            ),
        };
        let position = Vector3::new(snapped.xz.x as f32, height, snapped.xz.y as f32);
        // The retained target, unless it is the stroke's own point (today's sticky rule).
        self.sticky = snapped
            .target
            .filter(|_| dragging || self.self_point(snapped.xz).is_none())
            .map(|target| (target, view.surface_generation));
        Some(Cursor {
            position,
            snapped,
            raw: raw_xz,
        })
    }

    // The angle lock's and guides' reference at the anchor: the start tangent for the first
    // point, else the last placed leg's direction.
    fn reference(&self, live: DVec2) -> Option<DVec2> {
        match self.placed_len {
            0 => None,
            1 => self.start_tangent(live),
            n => (xz_of(self.placed[n - 1]) - xz_of(self.placed[n - 2])).try_normalize(),
        }
    }

    // Rule 7: map-edge snap, then self snap (unless `self_snap` is off: a freeform drag), for a
    // cursor nothing else took (today's order).
    fn border_or_self(
        &self,
        view: &RoadToolQuerySnapshot,
        snapped: Snapped,
        self_snap: bool,
    ) -> Snapped {
        let (half_w, half_h) = half_extents(view);
        let reach = BORDER_SNAP_M.min(half_w.min(half_h));
        let at = Vector3::new(snapped.xz.x as f32, 0.0, snapped.xz.y as f32);
        if is_near_border(at, half_w, half_h, reach) {
            return Snapped {
                xz: xz_of(snap_to_border(at, half_w, half_h)),
                kind: SnapKind::Border,
                ..snapped
            };
        }
        match self.self_point(snapped.xz).filter(|_| self_snap) {
            Some(point) => Snapped {
                xz: xz_of(point),
                kind: SnapKind::SelfPoint,
                ..snapped
            },
            None => snapped,
        }
    }

    // The placed points in XZ (the first `placed_len` entries; the rest zero).
    fn placed_xz(&self) -> [DVec2; MAX_POINTS] {
        let mut placed = [DVec2::ZERO; MAX_POINTS];
        for (xz, point) in placed.iter_mut().zip(&self.placed[..self.placed_len]) {
            *xz = xz_of(*point);
        }
        placed
    }

    // The placed point within the self-snap radius of `xz` (earliest first), if any.
    fn self_point(&self, xz: DVec2) -> Option<Vector3> {
        let radius = f64::from(SELF_SNAP_M);
        self.placed[..self.placed_len]
            .iter()
            .copied()
            .find(|point| xz_of(*point).distance_squared(xz) < radius * radius)
    }

    // Rebuilds the frame for the current pose: draft, stroke request, HUD, gizmos, state.
    fn refresh(&mut self, view: &RoadToolQuerySnapshot) {
        self.frame.state.mode = self.mode;
        self.frame.state.phase = self.phase();
        if let Some(cursor) = self.cursor {
            self.frame.cursor.position = cursor.position;
            self.frame.cursor.snap = cursor.snapped.kind;
            self.frame.cursor.stepped = cursor.snapped.stepped;
            self.frame.cursor.angle_broken = cursor.snapped.angle_broken;
            self.frame.cursor.valid = true;
        }
        if self.pending.is_some() {
            // The sent stroke stays drawn until its acknowledgement.
            return;
        }
        self.frame.clear_pose();
        self.refresh_draft(view);
        self.push_height_gizmos();
        self.push_ground_marks(view);
    }

    // Each end's ground marker (round H2) when it stands off the ground under it, or while its
    // grip is dragged or under the pointer. `O(ring points x pieces in a query cell)` an end, no
    // allocation once the gizmo buffers have grown.
    fn push_ground_marks(&mut self, view: &RoadToolQuerySnapshot) {
        let ground = DraftGround::new(&view.terrain, &view.roadbed);
        let hovered = self
            .pointer
            .and_then(|pointer| grip_at(&self.frame.height_handles, [true; 2], pointer));
        let frame = &mut self.frame;
        for (slot, handle) in frame.ground_marks.iter_mut().zip(frame.height_handles) {
            let Some(handle) = handle else {
                continue;
            };
            let held = self.heights.grip == Some(handle.end) || hovered == Some(handle.end);
            *slot = push_ground_mark(&handle, held, &ground, &mut frame.gizmos);
        }
    }

    // The pose's draft: shape, stroke request, heights, HUD and gizmos (all but the height
    // handles' gizmos).
    fn refresh_draft(&mut self, view: &RoadToolQuerySnapshot) {
        let Some(cursor) = self.cursor else {
            return;
        };
        if self.placed_len == 0 {
            // The cursor's handle: the height a start placed now takes.
            let mut handle = HeightHandle::new(
                DraftEnd::End,
                cursor.position,
                self.heights.end_offset_m,
                cursor.snapped.target.is_some(),
            );
            handle.active = true;
            self.frame.height_handles[1] = Some(handle);
            return;
        }
        if self.mode == DrawMode::Freeform {
            self.refresh_freeform(view, cursor);
            return;
        }
        let handles =
            self.set_handles(self.placed[0], cursor.position, cursor.snapped.target.is_some());
        let live = xz_of(cursor.position);
        let tangent = self.draft_tangent();
        let placed_xz = self.placed_xz();
        let shaped = modes::shape(
            self.mode,
            &placed_xz[..self.placed_len],
            live,
            tangent,
            &mut self.path,
        );
        self.push_tangent_legs(cursor.position, shaped.is_ok());
        // The next click's hint at the cursor (round SC), before the draft's chips.
        if let Some(step) = draft_hint(self.mode, self.steps()) {
            let last = self.placed[self.placed_len - 1];
            push_hint(step, cursor.position, Some(last), &mut self.frame.labels);
        }
        if let Err(error) = shaped {
            self.frame.state.refusal = Some(refusal(error));
            return;
        }
        if let Err(reason) = self.build_stroke(handles[0].point(), handles[1].point()) {
            self.frame.state.refusal = Some(reason);
            return;
        }
        self.apply_grade(view, &handles);
        self.measure_pose(view, cursor.snapped.target, cursor.position);
        self.measure_heights(view, &handles);
        self.push_constraint_line(cursor);
    }

    // A freeform pose: the spans through the placed nodes and, while placing, the cursor as the
    // provisional last node (unless it stands on the last one); the stroke, HUD and gizmos.
    fn refresh_freeform(&mut self, view: &RoadToolQuerySnapshot, cursor: Cursor) {
        let editing = self.freeform.editing();
        let mut nodes = [Vector3::ZERO; MAX_POINTS + 1];
        nodes[..self.placed_len].copy_from_slice(&self.placed[..self.placed_len]);
        let mut count = self.placed_len;
        let last = xz_of(self.placed[count - 1]);
        if !editing && last.distance(xz_of(cursor.position)) > f64::from(POINT_EPS_M) {
            nodes[count] = cursor.position;
            count += 1;
        }
        let mut nodes_xz = [DVec2::ZERO; MAX_POINTS + 1];
        for (xz, node) in nodes_xz.iter_mut().zip(&nodes[..count]) {
            *xz = xz_of(*node);
        }
        let shaped = self.freeform.shape(&nodes_xz[..count], &mut self.path);
        self.freeform
            .push_gizmos(&mut self.frame.gizmos, &nodes[..count], self.chained);
        let end = nodes[count - 1];
        let end_target = if count == self.placed_len {
            self.freeform.snap_of(count - 1).0
        } else {
            cursor.snapped.target
        };
        let handles = self.set_handles(nodes[0], end, end_target.is_some());
        push_freeform_labels(
            nodes[count - 1],
            count.checked_sub(2).map(|index| nodes[index]),
            editing,
            &mut self.frame.labels,
        );
        if let Err(error) = shaped {
            self.frame.state.refusal = Some(refusal(error));
            return;
        }
        if let Err(reason) = self.build_freeform_stroke(&nodes[..count]) {
            self.frame.state.refusal = Some(reason);
            return;
        }
        self.apply_grade(view, &handles);
        self.measure_pose(view, end_target, end);
        self.measure_heights(view, &handles);
        if !editing {
            self.push_constraint_line(cursor);
        }
    }

    // The pose's HUD (the end met `end_target` at `end`), the draft's centreline and the
    // tight-curve refusal.
    fn measure_pose(
        &mut self,
        view: &RoadToolQuerySnapshot,
        end_target: Option<NetworkSnapTarget>,
        end: Vector3,
    ) {
        let end_road = end_target
            .and_then(|target| road_start_direction(&view.region_graph, target, end))
            .map(end_road);
        self.frame.hud = measure(
            &self.path,
            &self.frame.stroke.points,
            &self.spec,
            self.illegal_radius_m,
            self.start_road,
            end_road,
        );
        let frame = &mut self.frame;
        for segment in self.path.segments() {
            frame.piece_lengths_m.push(segment.length());
            frame.piece_radii_m.push(segment.min_radius());
        }
        frame
            .gizmos
            .push_line(GizmoLineClass::Draft, frame.stroke.points.iter().copied());
        push_draft_labels(
            &self.path,
            &frame.stroke.points,
            &frame.hud,
            self.start_road,
            end_road,
            frame.hud.tight_radius,
            &mut frame.labels,
            &mut frame.gizmos,
        );
        frame.zoning_depth_m = zoning_depth_m(&self.spec, view.zoning_grid.cell_m());
        if let Some(depth) = frame.zoning_depth_m {
            push_zoning_guide(
                &mut frame.gizmos,
                &self.path,
                &frame.stroke.points,
                depth,
                &mut self.zoning,
            );
        }
        if frame.hud.tight_radius {
            // The radius check would refuse it: no stroke to preview or send; the HUD keeps the
            // numbers.
            frame.state.refusal = Some(DraftRefusal::TightCurve);
            frame.stroke.points.clear();
        }
    }

    // Fills the frame's stroke request for the freeform `nodes`: the kernel's flattening of the
    // spans, each span's heights interpolated by arc length between its two nodes', so every
    // node is a stroke point bit for bit. Refused when the ends are within 5 cm or the
    // flattening passes its step cap.
    fn build_freeform_stroke(&mut self, nodes: &[Vector3]) -> Result<(), DraftRefusal> {
        let stroke = &mut self.frame.stroke;
        stroke.spec = self.spec;
        stroke.snap_to_existing_roads = self.snap.network;
        if nodes[0].distance_to(nodes[nodes.len() - 1]) <= POINT_EPS_M {
            return Err(DraftRefusal::TooShort);
        }
        self.path
            .flatten_into(&mut self.flat)
            .map_err(|_| DraftRefusal::TooLong)?;
        // Each span's points are its flattening's steps from its start node (written once).
        let mut first = 0;
        for (span, segment) in self.path.segments().iter().enumerate() {
            let steps = segment.flatten_count();
            let points = &self.flat[first..=first + steps];
            let total: f64 = points
                .windows(2)
                .map(|pair| pair[0].distance(pair[1]))
                .sum();
            let (y0, y1) = (f64::from(nodes[span].y), f64::from(nodes[span + 1].y));
            let mut station = 0.0;
            for (index, point) in points.iter().enumerate() {
                if index > 0 {
                    station += point.distance(points[index - 1]);
                } else if span > 0 {
                    continue;
                }
                let y = if index == steps {
                    y1
                } else if total > 0.0 {
                    y0 + (y1 - y0) * (station / total)
                } else {
                    y0
                };
                stroke
                    .points
                    .push(Vector3::new(point.x as f32, y as f32, point.y as f32));
            }
            first += steps;
        }
        debug_assert_eq!(first + 1, self.flat.len(), "every span's steps counted");
        Ok(())
    }

    // Fills the frame's stroke request from the path from `start` to `end` (the placed start and
    // the cursor at their handles' heights): a single line's two ends exactly (today's straight
    // request), else the kernel's flattening with heights interpolated by arc length between the
    // two. Refused when the ends are within 5 cm or the flattening passes its step cap.
    fn build_stroke(&mut self, start: Vector3, end: Vector3) -> Result<(), DraftRefusal> {
        let stroke = &mut self.frame.stroke;
        stroke.spec = self.spec;
        stroke.snap_to_existing_roads = self.snap.network;
        if start.distance_to(end) <= POINT_EPS_M {
            return Err(DraftRefusal::TooShort);
        }
        if let [GuideSegment::Line(line)] = self.path.segments() {
            // A straight stroke is its start and the cursor bit for bit; a ghost leg (a curve's
            // line to its next point, or to the control point held on the tangent ray) runs
            // from the last placed point to the line's own end at the cursor's height.
            let last = self.placed[self.placed_len - 1];
            let from = if xz_of(start) == line.a { start } else { last };
            let to = if xz_of(end) == line.b {
                end
            } else {
                Vector3::new(line.b.x as f32, end.y, line.b.y as f32)
            };
            stroke.points.push(from);
            stroke.points.push(to);
            return Ok(());
        }
        self.path
            .flatten_into(&mut self.flat)
            .map_err(|_| DraftRefusal::TooLong)?;
        let total: f64 = self
            .flat
            .windows(2)
            .map(|pair| pair[0].distance(pair[1]))
            .sum();
        let (y0, y1) = (f64::from(start.y), f64::from(end.y));
        let mut station = 0.0;
        for (index, point) in self.flat.iter().enumerate() {
            if index > 0 {
                station += point.distance(self.flat[index - 1]);
            }
            let y = if total > 0.0 {
                y0 + (y1 - y0) * (station / total)
            } else {
                y0
            };
            stroke
                .points
                .push(Vector3::new(point.x as f32, y as f32, point.y as f32));
        }
        // The ends bit for bit: the placed start and the cursor.
        stroke.points[0] = start;
        *stroke
            .points
            .last_mut()
            .expect("flattened paths have two points") = end;
        Ok(())
    }

    // Handles at the placed points and the cursor; for curves the tangent legs through them.
    // The complex curve's legs are its aim legs (before the end aim, the simple curve's legs
    // through the start aim), and once `shaped` its control polygon (start, the arcs' control
    // points around the joint, end) with handles at the arcs' control points and a joint handle.
    fn push_tangent_legs(&mut self, live: Vector3, shaped: bool) {
        let gizmos = &mut self.frame.gizmos;
        for (index, point) in self.placed[..self.placed_len].iter().enumerate() {
            let kind = match index {
                0 if self.chained => GizmoHandleKind::ChainStart,
                0 => GizmoHandleKind::Endpoint,
                _ => GizmoHandleKind::Control,
            };
            gizmos.push_handle(kind, *point);
        }
        gizmos.push_handle(GizmoHandleKind::Cursor, live);
        if self.carried_aim.is_some() {
            // A carried start aim (round CT) is no point: before the end aim the leg runs from the
            // start to where the draft's tangents meet and on to the cursor; after it, the end
            // aim leg and the control polygon, which leaves the start along the direction.
            match self.placed[..self.placed_len] {
                [start] => match modes::single_control(&self.path).filter(|_| shaped) {
                    Some(aim) => {
                        let aim =
                            Vector3::new(aim.x as f32, (start.y + live.y) * 0.5, aim.y as f32);
                        gizmos.push_line(GizmoLineClass::Tangent, [start, aim, live]);
                    }
                    None => gizmos.push_line(GizmoLineClass::Tangent, [start, live]),
                },
                [start, end_aim, ..] => {
                    gizmos.push_line(GizmoLineClass::Tangent, [end_aim, live]);
                    push_control_polygon(gizmos, &self.path, start, live, shaped);
                }
                [] => {}
            }
        } else if self.mode == DrawMode::ComplexCurve {
            let placed = &self.placed[..self.placed_len];
            match *placed {
                [start] => gizmos.push_line(GizmoLineClass::Tangent, [start, live]),
                [start, start_aim] => {
                    gizmos.push_line(GizmoLineClass::Tangent, [start, start_aim, live]);
                }
                [start, start_aim, end_aim, ..] => {
                    gizmos.push_line(GizmoLineClass::Tangent, [start, start_aim]);
                    gizmos.push_line(GizmoLineClass::Tangent, [end_aim, live]);
                    push_control_polygon(gizmos, &self.path, start, live, shaped);
                }
                [] => {}
            }
        } else if self.mode != DrawMode::Straight && self.placed_len > 1 {
            let placed = &self.placed[..self.placed_len];
            gizmos.push_line(
                GizmoLineClass::Tangent,
                placed.iter().copied().chain(std::iter::once(live)),
            );
        }
    }

    // The constraints that took the cursor, as gizmo strips: the angle lock's ray from the anchor
    // through the cursor and on past it, the guide line (both at a corner) from where it comes
    // from through the cursor and on past it; then the guide lines near the cursor that did not
    // take it, faint (round UI1; none with guide lines off, whose candidates are empty).
    fn push_constraint_line(&mut self, cursor: Cursor) {
        let anchor = self.placed[self.placed_len - 1];
        let y = cursor.position.y;
        if self.mode == DrawMode::Straight
            && matches!(cursor.snapped.kind, SnapKind::Grid | SnapKind::AngleGrid)
        {
            // The grid snaps a stroke along an axis through its start (round V item 8).
            let offset = grid_curb_offset_m(&self.spec);
            push_grid_curbs(&mut self.frame.gizmos, self.placed[0], cursor.position, offset);
        }
        if matches!(
            cursor.snapped.kind,
            SnapKind::Angle | SnapKind::AngleEdge | SnapKind::AngleGuide | SnapKind::AngleGrid
        ) {
            let ray = GuideLine {
                origin: xz_of(anchor),
                dir: (cursor.snapped.xz - xz_of(anchor))
                    .try_normalize()
                    .unwrap_or(DVec2::X),
            };
            push_alignment_guide(&mut self.frame.gizmos, ray, cursor.snapped.xz, y);
            if let Some(line) = self.frame.gizmos.line_classes.last_mut() {
                *line = GizmoLineClass::Angle;
            }
        }
        let mut holding = [None; 2];
        if matches!(
            cursor.snapped.kind,
            SnapKind::Guide | SnapKind::GuideCorner | SnapKind::AngleGuide
        ) {
            let [first, second] = holding_guides(&self.scratch.guides, cursor.snapped.xz);
            let corner = cursor.snapped.kind == SnapKind::GuideCorner;
            holding = [first, second.filter(|_| corner)];
            for guide in holding.into_iter().flatten() {
                push_alignment_guide(&mut self.frame.gizmos, guide, cursor.snapped.xz, y);
            }
        }
        for guide in near_guides(&self.scratch.guides, cursor.raw, holding)
            .into_iter()
            .flatten()
        {
            push_near_guide(&mut self.frame.gizmos, guide, cursor.raw, y);
        }
    }
}

// The S-curve's control polygon (start, the arcs' control points around the joint, end) with
// handles at the arcs' control points and a joint handle, once `shaped` as a biarc.
fn push_control_polygon(
    gizmos: &mut Gizmos,
    path: &GuidePath,
    start: Vector3,
    live: Vector3,
    shaped: bool,
) {
    let Some(controls) = modes::biarc_controls(path).filter(|_| shaped) else {
        return;
    };
    // Drawn over everything, so one height serves: the ends' mean.
    let y = (start.y + live.y) * 0.5;
    let at = |p: DVec2| Vector3::new(p.x as f32, y, p.y as f32);
    let [first, joint, second] = controls.map(at);
    gizmos.push_line(GizmoLineClass::Tangent, [start, first, joint, second, live]);
    gizmos.push_handle(GizmoHandleKind::Handle, first);
    gizmos.push_handle(GizmoHandleKind::Joint, joint);
    gizmos.push_handle(GizmoHandleKind::Handle, second);
}

fn end_road(direction: StartDirection) -> EndRoad {
    match direction {
        StartDirection::Continue(d) => EndRoad::Continue(xz_of(d)),
        StartDirection::Along(d) => EndRoad::Along(xz_of(d)),
    }
}

fn refusal(error: GuideError) -> DraftRefusal {
    match error {
        GuideError::Degenerate => DraftRefusal::TooShort,
        GuideError::FoldBack => DraftRefusal::FoldBack,
        GuideError::FullTurn => DraftRefusal::FullTurn,
        GuideError::NoBiarc => DraftRefusal::NoBiarc,
    }
}
