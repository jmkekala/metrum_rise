// SPDX-License-Identifier: GPL-2.0-only

//! Road tool session Godot API (`ROAD-112` round C, plan 2.3 and 2.4): `road_tool.gd` forwards
//! the clicks and keys as typed calls, sets options, ticks once a frame (`tick.rs`, round R2-2)
//! and draws what the tick's flags say changed. The session (`nodes::sim::road_tool::session`) decides the cursor, the stroke, the chain and the
//! commit; this bridge runs the existing preview (`preview_road`) for the session's stroke when
//! it changed, keeps its typed outcome (round R2-1, `PreviewOutcome`), sends the confirmed
//! stroke with `add_road_with_snap` and answers its commit (`commit.rs`). One session per
//! `SimulationNode`, used from the main thread only.
//!
//! Every drawing mode runs on the session (round D: the simple and complex curves and continuous
//! too). The pointer and the tick take typed arguments, so a pointer move allocates nothing once
//! the session's buffers have grown, outside the preview of a changed stroke.
//!
//! A graded stroke (round I: a height handle off its support, or straight grade) previews and
//! commits through its segmentation (`RoadToolSession::update_draft`, once a frame): one stroke
//! previews alone with its profile and commits as a batch of one (`add_road_with_profile`'s
//! path); several preview as a batch (round G1: tier 1 on each new pose, a tier-2 stroke on each
//! tick the pose rests, the exact overlay once accepted) and commit as one batch, one undo step,
//! with tier 2's plans when it finished; a refused segmentation (`cut_too_deep`) is the preview's
//! refusal.

use super::super::super::*;
use crate::nodes::sim::core::BATCH_TOO_LARGE_REASON;
use crate::nodes::sim::road_tool::batch::{draft_requests, section_half_width_m};
use crate::nodes::sim::road_tool::commit::CommitSequence;
use crate::nodes::sim::road_tool::draw::engine::GodotBackend;
use crate::nodes::sim::road_tool::draw::lines::{
    LineQuadBuffers, LineStyle, handle_kind_code, line_style,
};
use crate::nodes::sim::road_tool::draw::{Ghost, ToolDraw};
use crate::nodes::sim::road_tool::frame::{
    DraftRefusal, GizmoLineClass, Gizmos, Phase, RoadToolFrame, StrokeRequest,
};
use crate::nodes::sim::road_tool::height::draft_batch;
use crate::nodes::sim::road_tool::hue::{HueState, OffFrameHue};
use crate::nodes::sim::road_tool::intent::RoadTypePick;
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::modes::DrawMode;
use crate::nodes::sim::road_tool::outcome::{PreviewOutcome, Tier2State};
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::SnapKind;
use crate::simulation::network::graph::RoadMedian;
use crate::simulation::network::road_edit::RoadProfile;
use crate::simulation::network::road_types::RoadAttributes;
use labels::Chips;
use tick::TickState;

mod commit;
mod draw;
mod labels;
mod overlay;
mod tick;

/// The road tool session of one `SimulationNode` and the preview the bridge last ran for it.
#[derive(Default)]
pub(crate) struct RoadToolBridge {
    session: RoadToolSession,
    // What the standing preview is for; `None` when none stands.
    previewed: Option<PreviewKey>,
    // That preview (round R2-1): a click on the same stroke submits it.
    preview: Option<StandingPreview>,
    // The standing preview is a batch's whose tier 2 still runs (each resting frame steps it),
    // and that batch's preview requests, made at its pose (round R2-1: a resting frame reuses
    // them instead of making them again).
    batch_running: bool,
    batch_requests: Vec<RoadPreviewRequest>,
    // A batch ghost is drawn, and the last tier 1's ghost (round R2-3: drawn by `draw`).
    batch_ghost: bool,
    pub(super) ghost: Ghost,
    // The tool's drawing (round R2-3, `nodes::sim::road_tool::draw`), once its setup attached.
    draw: Option<ToolDraw<GodotBackend>>,
    // What the tick keeps between frames (round R2-2, `tick.rs`).
    tick: TickState,
    // The draft's footprint hue (round HU), built with each preview of a new pose, off the
    // frame's thread until the frame uploads it (`ROAD-123` round HF2).
    hue: OffFrameHue,
    // The previewed poses' count, the standing preview's `pose`, and the pose the hue was last
    // built for (`ROAD-123` round HF2: the two always arrive together, tests compare them).
    pose: i64,
    hue_pose: i64,
    // The HUD's label chips (rounds V2, R2-5): their font, this frame's and the HUD's copy, and
    // their placement (`labels.rs`).
    chips: Chips,
    // The road type the next stroke draws and the one R draws again (round R2-6).
    road_type: RoadTypePick,
    // The sent stroke waiting for the core's answer and the preview dictionary it was sent with,
    // whose copy shows a refusal (round R2-1, `commit.rs`).
    commit: CommitSequence,
    commit_preview: Option<VarDictionary>,
    // The preview's overlay chunks and their source (round R2-4a, `overlay.rs`).
    chunks: overlay::PreviewChunks,
}

// The standing preview: its typed outcome, which every decision on it reads, and the dictionary
// the tick hands `road_tool.gd` to draw (a click returns it; a refused commit's display is a
// copy of it). The bridge never reads the dictionary.
struct StandingPreview {
    outcome: PreviewOutcome,
    dictionary: VarDictionary,
}

// The inputs a preview depends on: the stroke, its cross-section, the snap flag, the grade line
// (round I) and the context generation the preview was made on.
#[derive(Clone, Debug, Default, PartialEq)]
struct PreviewKey {
    points: Vec<Vector3>,
    spec: RoadTypeSpec,
    snap: bool,
    grade: Option<[f32; 2]>,
    generation: i64,
}

#[godot_api(secondary)]
impl SimulationNode {
    /// The road tool's pointer moved: a camera ray (`ROAD-112`; a click's, which can arrive before
    /// the frame's tick, and tests). Shift frees the cursor of the angle lock, guide lines, the
    /// zoning grid and the length step; Ctrl breaks the joint of a freeform handle being dragged.
    /// Returns the resolved cursor, or `null` while no pointer has met a surface. Reads the
    /// published view only; allocates nothing once the session's buffers have grown.
    #[func]
    pub fn road_tool_pointer(
        &mut self,
        origin: Vector3,
        dir: Vector3,
        shift: bool,
        #[opt(default = false)] ctrl: bool,
    ) -> Variant {
        self.road_tool_handle(RoadToolEvent::Pointer {
            pointer: Pointer::Ray { origin, dir },
            shift,
            ctrl,
        });
        self.road_tool_cursor_variant()
    }

    /// The road tool's pointer at a scripted world position (tests, benchmarks and probes, as
    /// `set_scripted_pointer`): the cursor exactly, neither picked nor snapped. `ctrl` as for
    /// `road_tool_pointer`.
    #[func]
    pub fn road_tool_pointer_at(
        &mut self,
        position: Vector3,
        #[opt(default = false)] ctrl: bool,
    ) -> Variant {
        self.road_tool_handle(RoadToolEvent::Pointer {
            pointer: Pointer::Scripted(position),
            shift: false,
            ctrl,
        });
        self.road_tool_cursor_variant()
    }

    /// A press of the road tool's pointer (round R2-2, F19): `button` 0 the left (places a point;
    /// the stroke's last one sends it; in freeform's edit phase grabs the node or handle under
    /// the pointer, else sends the stroke), 1 the right. Returns `phase` and `placed` after it,
    /// and for a click that sent the stroke `points` (the stroke), `preview` (the preview of
    /// exactly this stroke, the dictionary `preview_road` returns, numbered as `sequence`),
    /// `fresh` (whether the click had to preview it), `request_id` (the commit, which
    /// `road_tool_commit_poll` answers; 0 when the preview refused it: the click is then logged
    /// on `events` as the tool's refusal and the stroke stays editable).
    #[func]
    pub fn road_tool_press(&mut self, button: i64) -> VarDictionary {
        let button = if button == 1 {
            PointerButton::Right
        } else {
            PointerButton::Left
        };
        self.road_tool_input(RoadToolEvent::Press(button))
    }

    /// The left button's release: ends a freeform drag.
    #[func]
    pub fn road_tool_release(&mut self) {
        self.road_tool_handle(RoadToolEvent::Release(PointerButton::Left));
    }

    /// One road tool action by code (round R2-2, F19): 0 cancel, 1 confirm (freeform: ends the
    /// placing, then sends the stroke; the click modes place the next point at the cursor), 2-5
    /// the modes straight, simple curve, complex curve and freeform, 6 and 7 raise and lower the
    /// active height handle by one 0.5 m step (PageUp/PageDown, as a drag sets it: the live
    /// end's while drawing, the hovered or last dragged one in an edit phase), 8 forget the
    /// standing preview (the next tick previews the stroke afresh: measurements and another
    /// world). Returns what `road_tool_press` returns (a confirm can send the stroke); an unknown
    /// code does nothing.
    #[func]
    pub fn road_tool_action(&mut self, code: i64) -> VarDictionary {
        let action = match code {
            0 => RoadToolAction::Cancel,
            1 => RoadToolAction::Confirm,
            2 => RoadToolAction::ModeStraight,
            3 => RoadToolAction::ModeSimpleCurve,
            4 => RoadToolAction::ModeComplexCurve,
            5 => RoadToolAction::ModeFreeform,
            6 => RoadToolAction::RaiseHandle,
            7 => RoadToolAction::LowerHandle,
            8 => {
                let bridge = &mut self.road_tool_session;
                bridge.previewed = None;
                bridge.preview = None;
                bridge.batch_running = false;
                return VarDictionary::new();
            }
            _ => return VarDictionary::new(),
        };
        self.road_tool_input(RoadToolEvent::Action(action))
    }

    /// Sets the next strokes' lanes, forward and backward (0..4 each; 0 + 0 is a walkway). A
    /// road type sets them with the median, parking and attributes (`road_tool_set_road_type`).
    #[func]
    pub fn road_tool_set_lanes(&mut self, fwd: i64, bkw: i64) {
        let mut spec = self.road_tool_session.session.spec();
        spec.fwd_lanes = fwd.clamp(0, 4) as u8;
        spec.bkw_lanes = bkw.clamp(0, 4) as u8;
        self.set_road_tool_spec(spec);
    }

    /// Turns one road tool option on or off by code (round R2-2, F19): 0 network snap, 1 guide
    /// lines, 2 the zoning grid, 3 continuous, 4 straight grade (U3: the grade line between the
    /// ends' heights even with both handles on their supports). False for an unknown code.
    #[func]
    pub fn road_tool_set_flag(&mut self, code: i64, enabled: bool) -> bool {
        let option = match code {
            0 => RoadToolOption::NetworkSnap(enabled),
            1 => RoadToolOption::GuideLines(enabled),
            2 => RoadToolOption::ZoningGrid(enabled),
            3 => RoadToolOption::Continuous(enabled),
            4 => RoadToolOption::StraightGrade(enabled),
            _ => return false,
        };
        self.road_tool_session.session.set_option(option);
        true
    }

    /// Sets one of the road tool's steps by code (round R2-2, F19): 0 the angle lock's step in
    /// degrees (15, 30, 45 or 90; anything else turns the lock off), 1 the length step in metres
    /// (0 turns it off). False for an unknown code.
    #[func]
    pub fn road_tool_set_step(&mut self, code: i64, value: f64) -> bool {
        let option = match code {
            0 => RoadToolOption::AngleStepDeg(Some(value)),
            1 => RoadToolOption::LengthStepM(Some(value)),
            _ => return false,
        };
        self.road_tool_session.session.set_option(option);
        true
    }

    /// Draws the next strokes with road type `id` (`ROAD-101`): the type's lanes, median,
    /// kerbside parking, speed, sidewalk sides, zoning and street lights as the library lists
    /// them, set at once (a walkway takes the built-ins' speed and sides). Picking any type but
    /// the Walkway also makes it the one R draws again. Returns "" when picked, else the refusal
    /// key: `road_type_unknown` when no type has `id`, the type's `invalid_reason` when it is
    /// unusable; a refused pick changes nothing of the intent (round R2-6). `O(types)` on a UI
    /// action.
    #[func]
    pub fn road_tool_set_road_type(&mut self, id: GString) -> GString {
        let id = id.to_string();
        let mut pick = self.road_tool_session.road_type.clone();
        match self.with_road_types(|library| pick.pick(library.types(), &id)) {
            Ok(spec) => {
                self.road_tool_session.road_type = pick;
                self.set_road_tool_spec(spec);
                GString::new()
            }
            Err(reason) => reason.into(),
        }
    }

    /// Picks the road tool's type again after the library changed (`ROAD-101`): an edited
    /// type's new layout draws from the next stroke on (built roads keep theirs). A type that is
    /// gone or no longer usable falls back to the last type R draws, and that one to the 2-Lane
    /// Road (the Walkway stays the Walkway). Returns the id now drawn, "" before the first pick.
    /// `O(types)`.
    #[func]
    pub fn road_tool_reload_road_type(&mut self) -> GString {
        let mut pick = self.road_tool_session.road_type.clone();
        let reloaded = self.with_road_types(|library| pick.reload(library.types()));
        self.road_tool_session.road_type = pick;
        if let Some(Ok(spec)) = reloaded {
            self.set_road_tool_spec(spec);
        }
        self.road_tool_session
            .road_type
            .current
            .as_deref()
            .unwrap_or_default()
            .into()
    }

    /// The road tool's intent as the UI shows it (round R2-6; read when a menu opens or a key
    /// acts, never per frame): `road_type_id` ("" before the first pick), `last_road_type_id`
    /// (the type R draws), `fwd_lanes`, `bkw_lanes`, `median_style` (0 none, 1 painted, 2 kerbed
    /// grass, 3 kerbed paving), `median_width` (metres, 0 without), `parking` (bit 0 the left
    /// kerb, bit 1 the right), `speed_kmh`, `sidewalks` (0 none, 1 left, 2 right, 3 both),
    /// `zoning`, `street_lights`, the snapping options `network_snap`, `guide_lines`,
    /// `zoning_grid`, `angle_step_deg` and `length_step_m` (0 for off), `continuous` and
    /// `straight_grade`.
    #[func]
    pub fn road_tool_intent(&self) -> VarDictionary {
        let session = &self.road_tool_session.session;
        let (spec, snap) = (session.spec(), session.snap_options());
        let pick = &self.road_tool_session.road_type;
        vdict! {
            "road_type_id": pick.current.as_deref().unwrap_or_default(),
            "last_road_type_id": pick.last.as_str(),
            "fwd_lanes": i64::from(spec.fwd_lanes),
            "bkw_lanes": i64::from(spec.bkw_lanes),
            "median_style": RoadMedian::style_code(spec.median),
            "median_width": spec.median.map_or(0.0, |median| median.width_m),
            "parking": spec.parking.code(),
            "speed_kmh": i64::from(RoadAttributes::speed_kmh_of(spec.attributes.speed_ms)),
            "sidewalks": i64::from(spec.attributes.sidewalks.bits()),
            "zoning": spec.attributes.zoning,
            "street_lights": spec.attributes.street_lights,
            "network_snap": snap.network,
            "guide_lines": snap.guides,
            "zoning_grid": snap.grid,
            "angle_step_deg": snap.angle_step_deg.unwrap_or(0.0),
            "length_step_m": snap.length_step_m.unwrap_or(0.0),
            "continuous": session.continuous(),
            "straight_grade": session.straight_grade(),
        }
    }

    /// The road tool's frame record on demand (round R2-2: tests and probes; the tool's frame is
    /// `road_tool_tick`, which builds none of it): brings the draft up to date with the published
    /// view and previews nothing. `phase` (0 idle, 1 placing, 2 waiting for the commit's
    /// acknowledgement, 3 a freeform draft's edit phase), `placed` (points down), `mode` (0
    /// straight, 1 simple curve, 2 complex curve, 3 freeform), `continuous`, `cursor` and
    /// `cursor_valid`, `snap` (what took the cursor: `free`, `node`, `edge`, `guide`,
    /// `guide_corner`, `grid`, `angle`, `angle_edge`, `angle_guide`, `angle_grid`, `border`,
    /// `self`), `points` (the stroke of this pose, empty without one), `refusal` (why the draft
    /// has no stroke: `too_short`, `fold_back`, `full_turn`, `no_biarc`, `road_tight_curve`, else
    /// ""), `hud` (`length_m`, `bearing_deg`, `start_angle_deg` and `end_angle_deg` (-1 for
    /// none), `radius_m` (the smallest arc radius, -1 for a straight draft), `other_radius_m` (an
    /// S-curve's other arc radius, -1 for any other draft), `design_speed_kmh` (-1 for none),
    /// `design_speed_warning`, `illegal_radius_m`, `tight_radius`, `build_cost` and
    /// `upkeep_per_day` of the drawn road type), `piece_lengths_m` and `piece_radii_m` (each
    /// guide piece's length and smallest radius, -1 for a straight piece: a curve's lead and
    /// arc, an S-curve's two arcs, a freeform draft's spans), and the gizmos as packed arrays:
    /// `gizmo_line_points` with `gizmo_line_starts` (each strip's first point) and
    /// `gizmo_line_classes` (0 draft centreline, 1 guide line, 2 angle ray, 3 tangent leg, 4 the
    /// zoning-depth guide's outside, 5 its inside, 6 a grid curb, 7 an end's ground marker: its
    /// vertical line and its ring, round H2), the same strips as quads for
    /// `road_tool_line.gdshader` (rounds V, V2): `gizmo_line_vertices` and
    /// `gizmo_line_attributes` (five floats a vertex, the last its look: 0 the centreline, 1 a
    /// guide, the angle ray, a tangent leg or a height stem, 2 and 3 the zoning guide's outside
    /// and inside, 4 a grid curb, 7 a ground marker), empty without strips,
    /// `gizmo_handles` with `gizmo_handle_kinds` (0 placed end, 1 control point, 2 chain start,
    /// 3 the live end or the grabbed freeform grip, 4 a joint: the S-curve's or a freeform inner
    /// node, 5 a handle: an S-curve arc's control point, a freeform Bezier handle or a height
    /// handle's grip, whose stem is a guide strip; `gizmo_handle_attributes`: the kinds as the
    /// one handle surface's UVs) and `zoning_depth_m` (-1 for a type that does not zone),
    /// `hue_revision` (the footprint hue's), and the heights (`road_tool_heights`' keys).
    #[func]
    pub fn road_tool_snapshot(&mut self) -> VarDictionary {
        self.road_tool_update_draft();
        let mut dict = self.road_tool_frame_record();
        let bridge = &self.road_tool_session;
        dict.set("hue_revision", bridge.hue.revision() as i64);
        dict.set("zoning_depth_m", bridge.session.frame().zoning_depth_m.unwrap_or(-1.0));
        dict
    }

    /// The draft's footprint hue (`ROAD-112` round HU; a stopgap marking the footprint while the
    /// preview terrain lags, T27 (d)), as of the last tick, whose hue flag says when it changed
    /// (tests and probes; the tool draws it through `road_tool_draw_hue`): `revision`, `pose`
    /// (the pose of the preview it was built with, in the same frame: that preview's `pose`), `state` (-1 hidden, 0 valid, 1 checking, 2
    /// refused, drawn hatched), the fill (a triangle list on the road, a bridge's on the ground
    /// under it, padded with collapsed triangles to a power-of-two count so a drag uploads in
    /// place) as `fill_vertices` (its count) and `fill_bytes` (the vertex buffer's bytes, three
    /// little-endian `f32` a vertex, uploaded as they are), and the tunnel runs' outlines as
    /// `road_tool_line.gdshader` quads (`outline_vertices`, `outline_attributes`, looks 8 to 10
    /// by state; empty without a tunnel), with `build_ms` (its build on its worker, off this
    /// thread) and `wait_ms` (what this call waited for it). Joins the hue's build when it is
    /// still running (`ROAD-123` round HF2).
    #[func]
    pub fn road_tool_hue(&mut self) -> VarDictionary {
        let (hue_pose, offframe) = (self.road_tool_session.hue_pose, &mut self.road_tool_session.hue);
        offframe.join();
        let (build_ms, wait_ms) = offframe.last_times();
        let hue = &*offframe.hue();
        let state = hue.state();
        let style = match state {
            Some(HueState::Checking) => LineStyle::HueOutlineChecking,
            Some(HueState::Refused) => LineStyle::HueOutlineRefused,
            _ => LineStyle::HueOutlineValid,
        };
        let outline = strip_quads(&hue.outline, &hue.outline_starts, |_| style);
        vdict! {
            "revision": hue.revision() as i64,
            "pose": hue_pose,
            "build_ms": build_ms,
            "wait_ms": wait_ms,
            "state": state.map_or(-1, |state| state as i64),
            "fill_vertices": hue.fill.len() as i64,
            "fill_bytes": PackedByteArray::from(hue.fill_bytes.as_slice()),
            "outline_vertices": outline.vertices,
            "outline_attributes": outline.attributes,
        }
    }

    /// The joined hue's build on its worker and this frame's wait for it, in milliseconds
    /// (`x`, `y`; the preview metrics).
    #[func]
    pub fn road_tool_hue_times(&self) -> Vector2 {
        let (build_ms, wait_ms) = self.road_tool_session.hue.last_times();
        Vector2::new(build_ms as f32, wait_ms as f32)
    }

    /// The heights part of the session's frame record (`road_tool_snapshot`'s round I keys:
    /// `grade`, `height_handles`, the HUD's heights and slopes, `piece_slopes_percent` and the
    /// draft's runs) as of the last tick; previews nothing (tests and probes read it between the
    /// tool's frames).
    #[func]
    pub fn road_tool_heights(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        set_heights(&mut dict, self.road_tool_session.session.frame());
        dict
    }

    // The road tool's frame record without the preview.
    fn road_tool_frame_record(&self) -> VarDictionary {
        let frame = self.road_tool_session.session.frame();
        let phase = self.road_tool_phase_placed();
        let hud = frame.hud;
        let angle = |angle: Option<f64>| angle.unwrap_or(-1.0);
        let mut dict = VarDictionary::new();
        dict.set("phase", phase.0);
        dict.set("placed", phase.1);
        dict.set("mode", mode_code(frame.state.mode));
        dict.set("continuous", self.road_tool_session.session.continuous());
        dict.set("cursor", frame.cursor.position);
        dict.set("cursor_valid", frame.cursor.valid);
        dict.set("snap", snap_key(frame.cursor.snap));
        dict.set(
            "points",
            PackedVector3Array::from(frame.stroke.points.as_slice()),
        );
        dict.set("refusal", frame.state.refusal.map_or("", refusal_key));
        dict.set(
            "hud",
            vdict! {
                "length_m": hud.length_m,
                "bearing_deg": hud.bearing_deg,
                "start_angle_deg": angle(hud.start_angle_deg),
                "end_angle_deg": angle(hud.end_angle_deg),
                "radius_m": hud.radius_m.unwrap_or(-1.0),
                "other_radius_m": hud.other_radius_m.unwrap_or(-1.0),
                "design_speed_kmh": hud.design_speed_kmh.unwrap_or(-1.0),
                "design_speed_warning": hud.design_speed_warning,
                "illegal_radius_m": hud.illegal_radius_m,
                "tight_radius": hud.tight_radius,
                "build_cost": hud.build_cost,
                "upkeep_per_day": hud.upkeep_per_day,
            },
        );
        dict.set(
            "piece_lengths_m",
            PackedFloat64Array::from(frame.piece_lengths_m.as_slice()),
        );
        dict.set(
            "piece_radii_m",
            frame
                .piece_radii_m
                .iter()
                .map(|radius| if radius.is_finite() { *radius } else { -1.0 })
                .collect::<PackedFloat64Array>(),
        );
        set_gizmos(&mut dict, &frame.gizmos);
        set_heights(&mut dict, frame);
        dict
    }

    // The phase code (0 idle, 1 placing, 2 waiting for the acknowledgement, 3 editing) and the
    // points placed.
    fn road_tool_phase_placed(&self) -> (i64, i64) {
        match self.road_tool_session.session.frame().state.phase {
            Phase::Idle => (0, 0),
            Phase::Placing { placed } => (1, i64::from(placed)),
            Phase::AwaitingAck => (2, 0),
            Phase::Editing => (3, self.road_tool_session.session.placed_count() as i64),
        }
    }

    // The road tool's road type as the session holds it: lanes, median, parking and attributes
    // (the one copy of the intent, round R2-6); every request the tool sends reads it here.
    pub(in super::super) fn road_tool_spec(&self) -> RoadTypeSpec {
        self.road_tool_session.session.spec()
    }

    // Sets the session's road type in one option (its stroke requests and HUD price it, B9).
    pub(super) fn set_road_tool_spec(&mut self, spec: RoadTypeSpec) {
        self.road_tool_session
            .session
            .set_option(RoadToolOption::RoadType(spec));
    }

    // One event against the published view.
    fn road_tool_handle(&mut self, event: RoadToolEvent) {
        let view = self.road_tool.view.read().unwrap();
        self.road_tool_session.session.handle_event(&view, event);
    }

    // A press or an action, then the commit when it sent the stroke: `road_tool_press`'s result.
    fn road_tool_input(&mut self, event: RoadToolEvent) -> VarDictionary {
        self.road_tool_handle(event);
        let mut result = match self.road_tool_session.session.take_commit() {
            Some(stroke) => self.road_tool_submit(stroke),
            None => VarDictionary::new(),
        };
        let (phase, placed) = self.road_tool_phase_placed();
        result.set("phase", phase);
        result.set("placed", placed);
        result
    }

    fn road_tool_cursor_variant(&self) -> Variant {
        let cursor = self.road_tool_session.session.frame().cursor;
        if cursor.valid {
            cursor.position.to_variant()
        } else {
            Variant::nil()
        }
    }

    // Whether the standing preview is for the session's stroke as it is now, on the current
    // context. Compares in place: no allocation.
    fn road_tool_preview_is_current(&self) -> bool {
        let bridge = &self.road_tool_session;
        let stroke = &bridge.session.frame().stroke;
        bridge.previewed.as_ref().is_some_and(|key| {
            key.points == stroke.points
                && key.spec == bridge.session.spec()
                && key.snap == stroke.snap_to_existing_roads
                && key.grade == stroke.grade
                && key.generation == self.road_tool_view_generation()
        })
    }

    fn road_tool_view_generation(&self) -> i64 {
        let generation = self.road_tool.view.read().unwrap().surface_generation;
        i64::try_from(generation).unwrap_or(i64::MAX)
    }

    // Brings the session's draft report up to date with the published view: once a frame, and
    // before a click's preview or commit reads it.
    fn road_tool_update_draft(&mut self) {
        let view = self.road_tool.view.read().unwrap();
        self.road_tool_session.session.update_draft(&view);
    }

    // The graded draft's strokes as preview requests on the view's generation.
    fn road_tool_draft_requests(&self) -> Vec<RoadPreviewRequest> {
        let session = &self.road_tool_session.session;
        let frame = session.frame();
        let section = self.road_tool_draft_section(frame);
        let generation = self.road_tool.view.read().unwrap().surface_generation;
        draft_requests(&frame.draft.strokes, section, generation)
    }

    fn road_tool_draft_section(
        &self,
        frame: &RoadToolFrame,
    ) -> crate::nodes::sim::road_tool::height::DraftSection {
        let spec = self.road_tool_session.session.spec();
        self.road_draft_section(
            i32::from(spec.fwd_lanes),
            i32::from(spec.bkw_lanes),
            frame.stroke.snap_to_existing_roads,
        )
    }

    // Previews the session's stroke as it stands, with the session's road type, and keeps the
    // result as the standing preview (its outcome typed, round R2-1): a follow-ground stroke as
    // today; a graded one through its segmentation (one stroke alone with its profile, several
    // as a batch pose, a refusal as the preview's refusal; its dictionary stands with it).
    // Returns whether the pose is a batch's (its tier 1 ghost stands in `ghost`). The last
    // outcome's and key's buffers are reused.
    fn road_tool_run_preview(&mut self) -> bool {
        self.road_tool_update_draft();
        self.road_tool_session.pose += 1;
        self.road_tool_session.batch_running = false;
        let mut outcome = (self.road_tool_session.preview.take())
            .map_or_else(PreviewOutcome::default, |standing| standing.outcome.recycled());
        let spec = self.road_tool_session.session.spec();
        let stroke = &self.road_tool_session.session.frame().stroke;
        let (grade, snap) = (stroke.grade, stroke.snap_to_existing_roads);
        let draft = &self.road_tool_session.session.frame().draft;
        let pose = match (grade, draft.refusal, draft.strokes.as_slice()) {
            (None, ..) => Pose::Stroke(None),
            (Some(_), Some(reason), _) => Pose::Refused(reason),
            (Some(_), None, [single]) => Pose::Stroke(Some(single.profile)),
            (Some(_), None, _) => Pose::Batch,
        };
        let mut tier1 = false;
        let mut preview = match pose {
            Pose::Stroke(profile) => self.road_tool_preview_with_hue(profile, &mut outcome),
            Pose::Refused(reason) => {
                self.road_tool_hue_of_stroke();
                outcome.reason = reason.into();
                outcome.surface_generation = self.road_tool_view_generation();
                vdict! {
                    "is_valid": false,
                    "is_pending": false,
                    "invalid_reason": reason,
                    "build_cost": 0.0,
                    "surface_generation": outcome.surface_generation,
                }
            }
            Pose::Batch => {
                tier1 = true;
                self.road_tool_batch_pose(&mut outcome)
            }
        };
        let bridge = &mut self.road_tool_session;
        outcome.pose = bridge.pose;
        preview.set("pose", outcome.pose);
        let stroke = &bridge.session.frame().stroke;
        let key = bridge.previewed.get_or_insert_with(PreviewKey::default);
        key.points.clear();
        key.points.extend_from_slice(&stroke.points);
        key.spec = spec;
        key.snap = snap;
        key.grade = grade;
        key.generation = outcome.surface_generation;
        bridge.preview = Some(StandingPreview {
            outcome,
            dictionary: preview,
        });
        tier1
    }

    // `preview_stroke` of one stroke (the session's, else the graded draft's single stroke of
    // `profile`) with its footprint hue, launched where the preview is built (`ROAD-123` round
    // HF2: off this thread, on the snapshot the preview read) and joined where the frame uploads
    // it (`road_tool_hue`), so the two arrive together in this frame. Takes the stroke by
    // reference (round R2-1, F18).
    fn road_tool_preview_with_hue(
        &mut self,
        profile: Option<RoadProfile>,
        outcome: &mut PreviewOutcome,
    ) -> VarDictionary {
        let spec = self.road_tool_session.session.spec();
        let half_width_m = self.road_tool_hue_half_width_m();
        let mut hue = std::mem::take(&mut self.road_tool_session.hue);
        let frame = self.road_tool_session.session.frame();
        let (points, profile) = match profile {
            None => (&frame.stroke.points, RoadProfile::FollowGround),
            Some(profile) => (&frame.draft.strokes[0].points, profile),
        };
        let preview = self.preview_stroke(
            points,
            [i32::from(spec.fwd_lanes), i32::from(spec.bkw_lanes)],
            frame.stroke.snap_to_existing_roads,
            profile,
            Some((&mut hue, half_width_m)),
            outcome,
        );
        self.road_tool_session.hue = hue;
        self.road_tool_session.hue_pose = self.road_tool_session.pose;
        preview
    }

    // The hue of a refused graded draft that has no prepared stroke (its segmentation refused
    // it): the stroke's points at its grade line, refused.
    fn road_tool_hue_of_stroke(&mut self) {
        let half_width_m = self.road_tool_hue_half_width_m();
        let frame = self.road_tool_session.session.frame();
        let heights = frame.stroke.grade.unwrap_or_default();
        let hue = self.road_tool_session.hue.hue();
        hue.begin();
        hue.push_graded(&frame.stroke.points, heights);
        // A graded stroke's road holds the grade line: no ground is read.
        hue.finish(
            half_width_m,
            |_, _| 0.0,
            |_, _| f64::NEG_INFINITY,
            HueState::Refused,
        );
        self.road_tool_session.hue_pose = self.road_tool_session.pose;
    }

    // The hue's half width: the session's section's road, a function of the section alone.
    fn road_tool_hue_half_width_m(&self) -> f32 {
        section_half_width_m(&self.road_tool_draft_section(self.road_tool_session.session.frame()))
    }

    // A new batch pose: tier 1 (the ghost, typed in `ghost`) and the standing preview
    // it gives into `outcome`, pending while every stroke passes (tier 2 then runs on the
    // resting frames, on the requests kept here), else the first refusal with its stroke
    // (`draft_stroke`). The hue takes tier 1's prepared strokes (checking while tier 2 runs), or
    // the drawn stroke's over the caps.
    fn road_tool_batch_pose(&mut self, outcome: &mut PreviewOutcome) -> VarDictionary {
        let requests = self.road_tool_draft_requests();
        let mut hue = std::mem::take(self.road_tool_session.hue.hue());
        let mut ghost = std::mem::take(&mut self.road_tool_session.ghost);
        let tier1 = self.preview_road_batch_tier1(&requests, None, Some(&mut hue), &mut ghost);
        *self.road_tool_session.hue.hue() = hue;
        self.road_tool_session.ghost = ghost;
        self.road_tool_session.hue_pose = self.road_tool_session.pose;
        if tier1.is_none() {
            self.road_tool_hue_of_stroke();
        }
        let (reason, stroke) = match &tier1 {
            None => (BATCH_TOO_LARGE_REASON, None),
            Some(tier1) => tier1
                .refusal
                .map_or(("", None), |(stroke, reason)| (reason, Some(stroke))),
        };
        let valid = reason.is_empty();
        self.road_tool_session.batch_running = valid;
        self.road_tool_session.batch_requests = requests;
        outcome.valid = valid;
        outcome.pending = valid;
        outcome.reason = reason.into();
        outcome.draft_stroke = stroke;
        (outcome.build_cost, outcome.length_m) = tier1
            .as_ref()
            .map_or((0.0, 0.0), |tier1| (tier1.build_cost, tier1.length_m));
        outcome.surface_generation = self.road_tool_view_generation();
        let preview = vdict! {
            "is_valid": valid,
            "is_pending": valid,
            "invalid_reason": reason,
            "draft_stroke": stroke.map_or(-1, |stroke| stroke as i64),
            "build_cost": outcome.build_cost,
            "length_m": outcome.length_m,
            "surface_generation": outcome.surface_generation,
        };
        preview
    }

    // A resting frame of a running batch: one tier-2 step on the pose's requests. On the frame
    // tier 2 accepts the batch (its overlay whole) the exact preview, keyed as `preview_road`'s,
    // replaces the standing pending one and the ghost goes; on a refusal the refusal does. The
    // pose's cost, length, generation and number stay. `None` while it runs, else the tick's
    // ghost-cleared flag (0 when no ghost goes).
    fn road_tool_step_batch(&mut self) -> Option<i64> {
        let requests = std::mem::take(&mut self.road_tool_session.batch_requests);
        let mut preview = VarDictionary::new();
        let step = self.preview_road_batch_tier2(&requests, &mut preview);
        self.road_tool_session.batch_requests = requests;
        if step.state == Tier2State::Running {
            return None;
        }
        self.road_tool_session.batch_running = false;
        let accepted = step.state == Tier2State::Accepted;
        self.road_tool_session.hue.hue().set_state(if accepted {
            HueState::Valid
        } else {
            HueState::Refused
        });
        let standing = self.road_tool_session.preview.take();
        if let Some(standing) = &standing {
            let pose = &standing.outcome;
            preview.set("build_cost", pose.build_cost);
            preview.set("length_m", pose.length_m);
            preview.set("surface_generation", pose.surface_generation);
            preview.set("pose", pose.pose);
        }
        let mut outcome = standing.map(|standing| standing.outcome).unwrap_or_default();
        outcome.valid = accepted;
        outcome.pending = false;
        outcome.reason = step.reason;
        outcome.draft_stroke = step.stroke;
        outcome.demolish_count = step.demolish_count;
        outcome.overlay = step.overlay;
        preview.set("is_valid", accepted);
        preview.set("is_pending", false);
        preview.set("invalid_reason", outcome.reason.as_ref());
        preview.set(
            "draft_stroke",
            step.stroke.map_or(-1, |stroke| stroke as i64),
        );
        let cleared = if accepted {
            self.road_tool_clear_batch_ghost()
        } else {
            0
        };
        self.road_tool_session.preview = Some(StandingPreview {
            outcome,
            dictionary: preview,
        });
        Some(cleared)
    }

    // The tick's ghost-cleared flag when a ghost is drawn, which goes.
    fn road_tool_clear_batch_ghost(&mut self) -> i64 {
        if std::mem::take(&mut self.road_tool_session.batch_ghost) {
            tick::TICK_BATCH_CLEARED
        } else {
            0
        }
    }
}

// What a new pose previews: one stroke (the session's, or the graded draft's single stroke of
// its profile), the segmentation's refusal, or a batch.
enum Pose {
    Stroke(Option<RoadProfile>),
    Refused(&'static str),
    Batch,
}

// The frame's heights (`road_tool_snapshot`'s round I keys).
fn set_heights(dict: &mut VarDictionary, frame: &RoadToolFrame) {
    dict.set(
        "grade",
        frame
            .stroke
            .grade
            .map_or(PackedFloat32Array::new(), |grade| {
                PackedFloat32Array::from(grade.as_slice())
            }),
    );
    let mut handles = VarArray::new();
    for handle in frame.height_handles.iter().flatten() {
        handles.push(
            &vdict! {
                "end": handle.end as i64,
                "support": handle.support,
                "height_m": handle.height_m,
                "offset_m": handle.offset_m,
                "grip": handle.grip,
                "locked": handle.locked,
                "active": handle.active,
            }
            .to_variant(),
        );
    }
    dict.set("height_handles", handles);
    let hud = &frame.hud;
    // No value: -1000 (a slope or height never reads it).
    let none = -1000.0;
    dict.set("start_height_m", hud.start_height_m.unwrap_or(none));
    dict.set("end_height_m", hud.end_height_m.unwrap_or(none));
    dict.set("slope_percent", hud.slope_percent.unwrap_or(none));
    dict.set(
        "piece_slopes_percent",
        PackedFloat64Array::from(frame.piece_slopes_percent.as_slice()),
    );
    let draft = &frame.draft;
    dict.set(
        "draft_profiles",
        draft
            .runs
            .iter()
            .map(|run| run.profile.code())
            .collect::<PackedInt32Array>(),
    );
    dict.set(
        "draft_lengths_m",
        draft
            .runs
            .iter()
            .map(|run| run.length_m)
            .collect::<PackedFloat64Array>(),
    );
    dict.set(
        "draft_slopes_percent",
        draft
            .runs
            .iter()
            .map(|run| run.slope_percent)
            .collect::<PackedFloat64Array>(),
    );
    dict.set("draft_reason", draft.refusal.unwrap_or(""));
}

// The frame's gizmos as packed arrays (`road_tool_gizmos.gd` draws them): one copy each, once a
// frame.
fn set_gizmos(dict: &mut VarDictionary, gizmos: &Gizmos) {
    let line_class = |class: &GizmoLineClass| match class {
        GizmoLineClass::Draft => 0,
        GizmoLineClass::Guide => 1,
        GizmoLineClass::Angle => 2,
        // A height stem draws in the guide look (round V2).
        GizmoLineClass::Tangent | GizmoLineClass::HeightStem => 3,
        GizmoLineClass::ZoningOutside => 4,
        GizmoLineClass::ZoningInside => 5,
        GizmoLineClass::GridCurb => 6,
        GizmoLineClass::GroundMarker => 7,
        GizmoLineClass::GuideNear => 8,
        GizmoLineClass::CornerArc => 9,
    };
    dict.set(
        "gizmo_line_points",
        PackedVector3Array::from(gizmos.line_points.as_slice()),
    );
    dict.set(
        "gizmo_line_starts",
        gizmos
            .line_starts
            .iter()
            .map(|&start| start as i32)
            .collect::<PackedInt32Array>(),
    );
    dict.set(
        "gizmo_line_classes",
        gizmos
            .line_classes
            .iter()
            .map(line_class)
            .collect::<PackedInt32Array>(),
    );
    let quads = line_quads(gizmos);
    dict.set("gizmo_line_vertices", quads.vertices);
    dict.set("gizmo_line_attributes", quads.attributes);
    dict.set(
        "gizmo_handles",
        PackedVector3Array::from(gizmos.handles.as_slice()),
    );
    dict.set(
        "gizmo_handle_kinds",
        gizmos
            .handle_kinds
            .iter()
            .map(|&kind| handle_kind_code(kind))
            .collect::<PackedInt32Array>(),
    );
    // One point surface for every handle (round V2): each point's kind as its UV, the
    // attribute buffer `road_tool_handle.gdshader` reads its colour and size by.
    dict.set(
        "gizmo_handle_attributes",
        gizmos
            .handle_kinds
            .iter()
            .flat_map(|&kind| [handle_kind_code(kind) as f32, 0.0])
            .collect::<PackedFloat32Array>(),
    );
}

// The road tool's line quads as packed arrays (snapshots and tests; the layout is
// `draw::lines`').
#[derive(Default)]
pub(super) struct LineQuads {
    pub(super) vertices: PackedVector3Array,
    pub(super) attributes: PackedFloat32Array,
}

// The gizmo strips as quads. `O(strip points)`.
fn line_quads(gizmos: &Gizmos) -> LineQuads {
    strip_quads(&gizmos.line_points, &gizmos.line_starts, |strip| {
        line_style(gizmos.line_classes[strip])
    })
}

// Strips (`points` back to back, `starts` each strip's first point) as quads, each strip in the
// look `style_of` its index, as packed arrays (snapshots, the hue's outlines, the batch ghost's
// edges). `O(points)`.
pub(super) fn strip_quads(
    points: &[Vector3],
    starts: &[u32],
    style_of: impl Fn(usize) -> LineStyle,
) -> LineQuads {
    let mut buffers = LineQuadBuffers::default();
    buffers.fill(points, starts, style_of);
    LineQuads {
        vertices: PackedVector3Array::from(buffers.vertices.as_slice()),
        attributes: PackedFloat32Array::from(buffers.attributes.as_slice()),
    }
}

fn mode_code(mode: DrawMode) -> i64 {
    match mode {
        DrawMode::Straight => 0,
        DrawMode::SimpleCurve => 1,
        DrawMode::ComplexCurve => 2,
        DrawMode::Freeform => 3,
    }
}

fn snap_key(kind: SnapKind) -> &'static str {
    match kind {
        SnapKind::Free => "free",
        SnapKind::Node => "node",
        SnapKind::Edge => "edge",
        SnapKind::Guide => "guide",
        SnapKind::GuideCorner => "guide_corner",
        SnapKind::Grid => "grid",
        SnapKind::Angle => "angle",
        SnapKind::AngleEdge => "angle_edge",
        SnapKind::AngleGuide => "angle_guide",
        SnapKind::AngleGrid => "angle_grid",
        SnapKind::GridEdge => "grid_edge",
        SnapKind::Border => "border",
        SnapKind::SelfPoint => "self",
    }
}

fn refusal_key(refusal: DraftRefusal) -> &'static str {
    match refusal {
        DraftRefusal::TooShort => "too_short",
        DraftRefusal::FoldBack => "fold_back",
        DraftRefusal::FullTurn => "full_turn",
        DraftRefusal::NoBiarc => "no_biarc",
        DraftRefusal::TightCurve => "road_tight_curve",
        DraftRefusal::TooLong => "batch_too_large",
    }
}
