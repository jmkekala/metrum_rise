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
//! commit leaves the stroke editable at its last point.
//!
//! Freeform (round N, `modes/freeform.rs`): clicks place nodes, then an edit phase drags nodes
//! and handles (press, pointer moves, release) until `confirm` or a press off every grip sends
//! the stroke. A drag takes the network snap but no anchor-bound constraint (angle lock, length
//! step, guide lines from the anchor) and no self snap.

use super::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Phase, RoadToolFrame, StrokeRequest,
};
use super::measure::{EndRoad, RoadTypeSpec, illegal_radius_m, measure};
use super::modes::freeform::{Freeform, Grip, MAX_NODES};
use super::modes::{self, DrawMode};
use super::pick::{
    NETWORK_SNAP_REACH_M, SELF_SNAP_M, SurfacePick, half_extents, is_near_border, lane_width_m,
    level_height, pick_surface, snap_to_border,
};
use super::snap::{
    GuideLine, SnapInput, SnapKind, SnapOptions, SnapScratch, Snapped, apply_length_step,
    collect_guides, edge_direction_at, nearest_guide, network_snap, resolve, xz_of,
};
use crate::nodes::sim::core::RoadToolQuerySnapshot;
use crate::simulation::network::guide::{DVec2, GuideError, GuidePath, GuideSegment};
use crate::simulation::network::interaction::{
    NetworkSnapTarget, StartDirection, StructureHit, road_start_direction,
};
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
    /// Continuous: the tangent constraint at a stroke's start.
    Continuous(bool),
    /// The keyboard height offset added to a ground pick (PageUp/PageDown, until round I's
    /// handles), in metres.
    AltitudeOffsetM(f32),
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

/// The road tool session: options, phase, the stroke's placed points, the draft and the frame.
#[derive(Debug)]
pub struct RoadToolSession {
    mode: DrawMode,
    snap: SnapOptions,
    spec: RoadTypeSpec,
    // The spec's illegal radius (`measure::illegal_radius_m`), kept with the spec: it allocates.
    illegal_radius_m: f64,
    continuous: bool,
    altitude_offset_m: f32,
    placed: [Vector3; MAX_POINTS],
    placed_len: usize,
    // How the road the stroke starts on leaves its start, or the carried tangent.
    start_road: Option<EndRoad>,
    start_on_border: bool,
    // The stroke starts where the last accepted one ended (the chain).
    chained: bool,
    // The retained network target and the view generation it was taken at.
    sticky: Option<(NetworkSnapTarget, u64)>,
    cursor: Option<Cursor>,
    shift: bool,
    ctrl: bool,
    pending: Option<Pending>,
    freeform: Freeform,
    path: GuidePath,
    flat: Vec<DVec2>,
    scratch: SnapScratch,
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
            altitude_offset_m: 0.0,
            placed: [Vector3::ZERO; MAX_POINTS],
            placed_len: 0,
            start_road: None,
            start_on_border: false,
            chained: false,
            sticky: None,
            cursor: None,
            shift: false,
            ctrl: false,
            pending: None,
            freeform: Freeform::default(),
            path: GuidePath::new(),
            flat: Vec::new(),
            scratch: SnapScratch::default(),
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

    /// Whether continuous (the tangent constraint at a stroke's start) is on.
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
            RoadToolOption::Continuous(on) => self.continuous = on,
            RoadToolOption::AltitudeOffsetM(offset) => self.altitude_offset_m = offset,
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
                self.cursor = self.resolve_cursor(view, pointer).or(self.cursor);
                self.drag(view);
            }
            RoadToolEvent::Press(PointerButton::Left) => self.click(view, false),
            RoadToolEvent::Action(RoadToolAction::Confirm) => self.click(view, true),
            RoadToolEvent::Press(PointerButton::Right)
            | RoadToolEvent::Action(RoadToolAction::Cancel) => self.cancel(),
            RoadToolEvent::Release(PointerButton::Left) => self.freeform.release(),
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
            RoadToolEvent::CommitAck { committed } => self.acknowledge(view, committed),
        }
        self.refresh(view);
    }

    // A different mode drops the stroke; the current mode again changes nothing.
    fn set_mode(&mut self, mode: DrawMode) {
        if mode != self.mode {
            self.cancel();
            self.mode = mode;
        }
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
        self.sticky = None;
        self.freeform.reset();
    }

    fn phase(&self) -> Phase {
        match (self.pending, self.placed_len) {
            (Some(_), _) => Phase::AwaitingAck,
            (None, 0) => Phase::Idle,
            _ if self.freeform.editing() => Phase::Editing,
            (None, placed) => Phase::Placing {
                placed: placed as u8,
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
        if self.placed_len + 1 < self.mode.points() {
            let mut point = cursor.position;
            if self.mode == DrawMode::SimpleCurve
                && let Some(tangent) = self.held_tangent(xz_of(point))
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

    // The start tangent the continuous constraint holds, while it applies (before the second
    // point of a straight stroke or the control point of a simple curve; B8: not the complex
    // curve or freeform, which set their own).
    fn held_tangent(&self, live: DVec2) -> Option<DVec2> {
        let held = matches!(self.mode, DrawMode::Straight | DrawMode::SimpleCurve);
        (self.continuous && self.placed_len == 1 && held)
            .then(|| self.start_tangent(live))
            .flatten()
    }

    // Pick and snap one pointer pose. `None` when the pointer meets no surface.
    fn resolve_cursor(&mut self, view: &RoadToolQuerySnapshot, pointer: Pointer) -> Option<Cursor> {
        let (raw, hit_road, structure) = match pointer {
            Pointer::Ray { origin, dir } => match pick_surface(view, origin, dir, self.altitude_offset_m)? {
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
            let point = view
                .zoning_grid
                .snap_road_cursor(graph, raw_xz, start, f64::from(width));
            // With a start the grid snaps along an axis through it: that axis is the line.
            let dir = start.and_then(|start| (point - start).try_normalize());
            (point, dir)
        });
        if constrained && self.snap.guides {
            let first_axis = (self.placed_len >= 2).then(|| {
                let (a, b) = (xz_of(self.placed[0]), xz_of(self.placed[1]));
                (b - a)
                    .try_normalize()
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
        let input = SnapInput {
            raw: raw_xz,
            anchor,
            reference,
            free: self.shift,
            network,
            grid,
        };
        let mut snapped = resolve(&input, &self.snap, &self.scratch.guides);
        if snapped.kind == SnapKind::Free {
            snapped = self.border_or_self(view, snapped, !dragging);
        }
        if constrained {
            let guide = (snapped.kind == SnapKind::Guide)
                .then(|| nearest_guide(&self.scratch.guides, raw_xz))
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
                self.altitude_offset_m,
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
        let Some(cursor) = self.cursor.filter(|_| self.placed_len > 0) else {
            return;
        };
        if self.mode == DrawMode::Freeform {
            self.refresh_freeform(view, cursor);
            return;
        }
        let live = xz_of(cursor.position);
        let tangent = self.held_tangent(live);
        let placed_xz = self.placed_xz();
        let shaped = modes::shape(
            self.mode,
            &placed_xz[..self.placed_len],
            live,
            tangent,
            &mut self.path,
        );
        self.push_tangent_legs(cursor.position, shaped.is_ok());
        // The complex curve's aim legs are no road (round N): nothing to preview before its end.
        if self.mode == DrawMode::ComplexCurve && self.placed_len < 3 {
            return;
        }
        if let Err(error) = shaped {
            self.frame.state.refusal = Some(refusal(error));
            return;
        }
        if !self.build_stroke(cursor.position) {
            self.frame.state.refusal = Some(DraftRefusal::TooShort);
            return;
        }
        self.measure_pose(view, cursor.snapped.target, cursor.position);
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
        if let Err(error) = shaped {
            self.frame.state.refusal = Some(refusal(error));
            return;
        }
        if !self.build_freeform_stroke(&nodes[..count]) {
            self.frame.state.refusal = Some(DraftRefusal::TooShort);
            return;
        }
        let end = nodes[count - 1];
        let end_target = if count == self.placed_len {
            self.freeform.snap_of(count - 1).0
        } else {
            cursor.snapped.target
        };
        self.measure_pose(view, end_target, end);
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
        if frame.hud.tight_radius {
            // The radius check would refuse it: no stroke to preview or send; the HUD keeps the
            // numbers.
            frame.state.refusal = Some(DraftRefusal::TightCurve);
            frame.stroke.points.clear();
        }
    }

    // Fills the frame's stroke request for the freeform `nodes`: the kernel's flattening of the
    // spans, each span's heights interpolated by arc length between its two nodes', so every
    // node is a stroke point bit for bit. False when the ends are within 5 cm.
    fn build_freeform_stroke(&mut self, nodes: &[Vector3]) -> bool {
        let stroke = &mut self.frame.stroke;
        stroke.spec = self.spec;
        stroke.snap_to_existing_roads = self.snap.network;
        if nodes[0].distance_to(nodes[nodes.len() - 1]) <= POINT_EPS_M {
            return false;
        }
        self.path.flatten_into(&mut self.flat);
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
        true
    }

    // Fills the frame's stroke request from the path: a single line's two ends exactly (today's
    // straight request), else the kernel's flattening with heights interpolated by arc length
    // between the start's and the cursor's. False when the ends are within 5 cm.
    fn build_stroke(&mut self, end: Vector3) -> bool {
        let start = self.placed[0];
        let stroke = &mut self.frame.stroke;
        stroke.spec = self.spec;
        stroke.snap_to_existing_roads = self.snap.network;
        if start.distance_to(end) <= POINT_EPS_M {
            return false;
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
            return true;
        }
        self.path.flatten_into(&mut self.flat);
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
        true
    }

    // Handles at the placed points and the cursor; for curves the tangent legs through them.
    // The complex curve's legs are its two aim legs, and once `shaped` its control polygon
    // (start, the arcs' control points around the joint, end) with handles at the arcs'
    // control points and a joint handle.
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
        if self.mode == DrawMode::ComplexCurve {
            let placed = &self.placed[..self.placed_len];
            match *placed {
                [start] => gizmos.push_line(GizmoLineClass::Tangent, [start, live]),
                [start, start_aim] => {
                    gizmos.push_line(GizmoLineClass::Tangent, [start, start_aim]);
                }
                [start, start_aim, end_aim, ..] => {
                    gizmos.push_line(GizmoLineClass::Tangent, [start, start_aim]);
                    gizmos.push_line(GizmoLineClass::Tangent, [end_aim, live]);
                    if let Some(controls) = modes::biarc_controls(&self.path).filter(|_| shaped) {
                        // Drawn over everything, so one height serves: the ends' mean.
                        let y = (start.y + live.y) * 0.5;
                        let at = |p: DVec2| Vector3::new(p.x as f32, y, p.y as f32);
                        let [first, joint, second] = controls.map(at);
                        gizmos.push_line(
                            GizmoLineClass::Tangent,
                            [start, first, joint, second, live],
                        );
                        gizmos.push_handle(GizmoHandleKind::Handle, first);
                        gizmos.push_handle(GizmoHandleKind::Joint, joint);
                        gizmos.push_handle(GizmoHandleKind::Handle, second);
                    }
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
    // through the cursor and on past it by the guides' radius, the guide line through the cursor
    // either way (with the ray where the two meet).
    fn push_constraint_line(&mut self, cursor: Cursor) {
        let anchor = self.placed[self.placed_len - 1];
        let reach = super::snap::GUIDE_RADIUS_M;
        if matches!(
            cursor.snapped.kind,
            SnapKind::Angle | SnapKind::AngleEdge | SnapKind::AngleGuide | SnapKind::AngleGrid
        ) {
            let beyond = (cursor.snapped.xz - xz_of(anchor))
                .try_normalize()
                .map_or(cursor.position, |dir| {
                    let p = cursor.snapped.xz + dir * reach;
                    Vector3::new(p.x as f32, cursor.position.y, p.y as f32)
                });
            self.frame
                .gizmos
                .push_line(GizmoLineClass::Angle, [anchor, cursor.position, beyond]);
        }
        if matches!(
            cursor.snapped.kind,
            SnapKind::Guide | SnapKind::GuideCorner | SnapKind::AngleGuide
        ) && let Some(guide) = nearest_guide(&self.scratch.guides, cursor.snapped.xz)
        {
            let at = |offset: f64| {
                let p = guide.origin
                    + guide.dir * ((cursor.snapped.xz - guide.origin).dot(guide.dir) + offset);
                Vector3::new(p.x as f32, cursor.position.y, p.y as f32)
            };
            self.frame
                .gizmos
                .push_line(GizmoLineClass::Guide, [at(-reach), at(reach)]);
        }
    }
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
