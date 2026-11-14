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

use super::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Phase, RoadToolFrame, StrokeRequest,
};
use super::measure::{EndRoad, RoadTypeSpec, measure};
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
// Most points a mode places (the complex curve's four).
const MAX_POINTS: usize = 4;
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
    /// Places the stroke's confirming point at the cursor (the last click's meaning).
    Confirm,
    /// Switches to straight mode (drops the stroke).
    ModeStraight,
    /// Switches to the simple curve (drops the stroke).
    ModeSimpleCurve,
    /// Switches to the complex curve (drops the stroke).
    ModeComplexCurve,
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
    },
    /// A button went down.
    Press(PointerButton),
    /// A button went up (no meaning yet; drags come with the handles).
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
    continuous: bool,
    altitude_offset_m: f32,
    placed: [Vector3; MAX_POINTS],
    placed_len: usize,
    // How the road the stroke starts on leaves its start, or the carried tangent.
    start_road: Option<EndRoad>,
    start_on_border: bool,
    // The retained network target and the view generation it was taken at.
    sticky: Option<(NetworkSnapTarget, u64)>,
    cursor: Option<Cursor>,
    shift: bool,
    pending: Option<Pending>,
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
        Self {
            mode: DrawMode::Straight,
            snap: SnapOptions::default(),
            spec: RoadTypeSpec::default(),
            continuous: false,
            altitude_offset_m: 0.0,
            placed: [Vector3::ZERO; MAX_POINTS],
            placed_len: 0,
            start_road: None,
            start_on_border: false,
            sticky: None,
            cursor: None,
            shift: false,
            pending: None,
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

    /// Takes the stroke to commit, if the last confirming click produced one; the session keeps
    /// waiting for its [`RoadToolEvent::CommitAck`].
    pub fn take_commit(&mut self) -> Option<StrokeRequest> {
        self.frame.commit.take()
    }

    /// Sets one option. Snapping changes apply from the next pointer move; a road type change
    /// re-prices the draft at once.
    pub fn set_option(&mut self, option: RoadToolOption) {
        match option {
            RoadToolOption::RoadType(spec) => self.spec = spec,
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
            RoadToolEvent::Pointer { pointer, shift } => {
                self.shift = shift;
                self.cursor = self.resolve_cursor(view, pointer).or(self.cursor);
            }
            RoadToolEvent::Press(PointerButton::Left)
            | RoadToolEvent::Action(RoadToolAction::Confirm) => self.click(view),
            RoadToolEvent::Press(PointerButton::Right)
            | RoadToolEvent::Action(RoadToolAction::Cancel) => self.cancel(),
            RoadToolEvent::Release(_) => {}
            RoadToolEvent::Action(RoadToolAction::ModeStraight) => {
                self.set_mode(DrawMode::Straight)
            }
            RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve) => {
                self.set_mode(DrawMode::SimpleCurve)
            }
            RoadToolEvent::Action(RoadToolAction::ModeComplexCurve) => {
                self.set_mode(DrawMode::ComplexCurve)
            }
            RoadToolEvent::CommitAck { committed } => self.acknowledge(view, committed),
        }
        self.refresh(view);
    }

    fn set_mode(&mut self, mode: DrawMode) {
        self.cancel();
        self.mode = mode;
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
        self.sticky = None;
    }

    fn phase(&self) -> Phase {
        match (self.pending, self.placed_len) {
            (Some(_), _) => Phase::AwaitingAck,
            (None, 0) => Phase::Idle,
            (None, placed) => Phase::Placing {
                placed: placed as u8,
            },
        }
    }

    // A left click: start a stroke, place the next point, or send the stroke.
    fn click(&mut self, view: &RoadToolQuerySnapshot) {
        let Some(cursor) = self.cursor.filter(|_| self.pending.is_none()) else {
            return;
        };
        if self.placed_len == 0 {
            self.begin(view, cursor.position, cursor.snapped);
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
        if self.frame.stroke.points.len() < 2 || self.frame.state.refusal.is_some() {
            return;
        }
        let border = self.start_on_border || cursor.snapped.kind == SnapKind::Border;
        self.pending = Some(Pending {
            chain: cursor.snapped.target.is_none() && !border,
            cancelled: false,
            end: *self.frame.stroke.points.last().expect("two points"),
            end_tangent: self.path.end_tangent(),
        });
        self.frame.commit = Some(self.frame.stroke.clone());
    }

    // Starts a stroke at `position`, reading how the road there leaves it.
    fn begin(&mut self, view: &RoadToolQuerySnapshot, position: Vector3, snapped: Snapped) {
        self.placed[0] = position;
        self.placed_len = 1;
        self.start_on_border = snapped.kind == SnapKind::Border;
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
        self.sticky = None;
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
    // point of a straight stroke or the control point of a simple curve).
    fn held_tangent(&self, live: DVec2) -> Option<DVec2> {
        (self.continuous && self.placed_len == 1 && self.mode != DrawMode::ComplexCurve)
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
        let anchor = (self.placed_len > 0).then(|| xz_of(self.placed[self.placed_len - 1]));
        let reference = self.reference(raw_xz);
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
            snapped = self.border_or_self(view, snapped);
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
            .filter(|_| self.self_point(snapped.xz).is_none())
            .map(|target| (target, view.surface_generation));
        Some(Cursor { position, snapped })
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

    // Rule 7: map-edge snap, then self snap, for a cursor nothing else took (today's order).
    fn border_or_self(&self, view: &RoadToolQuerySnapshot, snapped: Snapped) -> Snapped {
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
        match self.self_point(snapped.xz) {
            Some(point) => Snapped {
                xz: xz_of(point),
                kind: SnapKind::SelfPoint,
                ..snapped
            },
            None => snapped,
        }
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
        let live = xz_of(cursor.position);
        let tangent = self.held_tangent(live);
        let mut placed_xz = [DVec2::ZERO; MAX_POINTS];
        for (xz, point) in placed_xz.iter_mut().zip(&self.placed[..self.placed_len]) {
            *xz = xz_of(*point);
        }
        let shaped = modes::shape(
            self.mode,
            &placed_xz[..self.placed_len],
            live,
            tangent,
            &mut self.path,
        );
        self.push_tangent_legs(cursor.position);
        if let Err(error) = shaped {
            self.frame.state.refusal = Some(refusal(error));
            return;
        }
        if !self.build_stroke(cursor.position) {
            self.frame.state.refusal = Some(DraftRefusal::TooShort);
            return;
        }
        let end_road = cursor
            .snapped
            .target
            .and_then(|target| road_start_direction(&view.region_graph, target, cursor.position))
            .map(end_road);
        self.frame.hud = measure(
            &self.path,
            &self.frame.stroke.points,
            &self.spec,
            self.start_road,
            end_road,
        );
        let frame = &mut self.frame;
        frame
            .gizmos
            .push_line(GizmoLineClass::Draft, frame.stroke.points.iter().copied());
        self.push_constraint_line(cursor);
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
        if let [GuideSegment::Line(_)] = self.path.segments() {
            stroke.points.push(start);
            stroke.points.push(end);
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
    fn push_tangent_legs(&mut self, live: Vector3) {
        let gizmos = &mut self.frame.gizmos;
        for (index, point) in self.placed[..self.placed_len].iter().enumerate() {
            let kind = if index == 0 {
                GizmoHandleKind::Endpoint
            } else {
                GizmoHandleKind::Control
            };
            gizmos.push_handle(kind, *point);
        }
        gizmos.push_handle(GizmoHandleKind::Endpoint, live);
        if self.mode != DrawMode::Straight && self.placed_len > 1 {
            let placed = &self.placed[..self.placed_len];
            gizmos.push_line(
                GizmoLineClass::Tangent,
                placed.iter().copied().chain(std::iter::once(live)),
            );
        }
    }

    // The guide line or angle ray that took the cursor, as a gizmo strip.
    fn push_constraint_line(&mut self, cursor: Cursor) {
        let anchor = self.placed[self.placed_len - 1];
        match cursor.snapped.kind {
            SnapKind::Angle | SnapKind::AngleEdge | SnapKind::AngleGuide | SnapKind::AngleGrid => {
                self.frame
                    .gizmos
                    .push_line(GizmoLineClass::Angle, [anchor, cursor.position]);
            }
            SnapKind::Guide | SnapKind::GuideCorner => {
                if let Some(guide) = nearest_guide(&self.scratch.guides, cursor.snapped.xz) {
                    let at = |offset: f64| {
                        let p = guide.origin
                            + guide.dir
                                * ((cursor.snapped.xz - guide.origin).dot(guide.dir) + offset);
                        Vector3::new(p.x as f32, cursor.position.y, p.y as f32)
                    };
                    let reach = super::snap::GUIDE_RADIUS_M;
                    self.frame
                        .gizmos
                        .push_line(GizmoLineClass::Guide, [at(-reach), at(reach)]);
                }
            }
            _ => {}
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
