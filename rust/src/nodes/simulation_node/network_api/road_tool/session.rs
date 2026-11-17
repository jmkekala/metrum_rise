// SPDX-License-Identifier: GPL-2.0-only

//! Road tool session Godot API (`ROAD-112` round C, plan 2.3 and 2.4): `road_tool.gd` forwards
//! the pointer, clicks and keys, sets options, and draws what `road_tool_frame` returns. The
//! session (`nodes::sim::road_tool::session`) decides the cursor, the stroke, the chain and the
//! commit; this bridge runs the existing preview (`preview_road`) for the session's stroke when
//! it changed, keeps its typed outcome (round R2-1, `PreviewOutcome`), sends the confirmed
//! stroke with `add_road_with_snap` and answers its commit (`commit.rs`). One session per
//! `SimulationNode`, used from the main thread only.
//!
//! Every drawing mode runs on the session (round D: the simple and complex curves and continuous
//! too); the frame carries the gizmos and the HUD numbers `road_tool.gd` draws. The pointer
//! calls take typed arguments, so a pointer move allocates nothing once the session's buffers
//! have grown (the frame and the preview allocate, once a frame).
//!
//! A graded stroke (round I: a height handle off its support, or straight grade) previews and
//! commits through its segmentation (`RoadToolSession::update_draft`, once a frame): one stroke
//! previews alone with its profile and commits as a batch of one (`add_road_with_profile`'s
//! path); several preview as a batch (round G1: tier 1 on each new pose, a tier-2 stroke on each
//! frame the pose rests, the exact overlay once accepted) and commit as one batch, one undo step,
//! with tier 2's plans when it finished; a refused segmentation (`cut_too_deep`) is the preview's
//! refusal.

use super::super::super::*;
use crate::nodes::sim::core::BATCH_TOO_LARGE_REASON;
use crate::nodes::sim::road_tool::batch::{draft_requests, section_half_width_m};
use crate::nodes::sim::road_tool::commit::CommitSequence;
use crate::nodes::sim::road_tool::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Gizmos, Phase, RoadToolFrame, StrokeRequest,
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

mod commit;
mod labels;

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
    // A batch ghost is drawn (`road_tool_gizmos.gd`'s `show_batch_ghost`).
    batch_ghost: bool,
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
}

// The standing preview: its typed outcome, which every decision on it reads, and the dictionary
// the frame handed `road_tool.gd` to draw (a click returns it; a refused commit's display is a
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
    /// The road tool's pointer moved: a camera ray (`ROAD-112`). Shift frees the cursor of the
    /// angle lock, guide lines, the zoning grid and the length step; Ctrl breaks the joint of a
    /// freeform handle being dragged. Returns the resolved cursor, or `null` while no pointer
    /// has met a surface. Reads the published view only; allocates nothing once the session's
    /// buffers have grown.
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

    /// One road tool event by name: `press` (left button: places a point; the stroke's last one
    /// sends it; in freeform's edit phase grabs the node or handle under the pointer, else sends
    /// the stroke), `right` (right button), `release` (ends a freeform drag), `cancel`,
    /// `confirm` (freeform: ends the placing, then sends the stroke), `raise_handle` and
    /// `lower_handle` (PageUp/PageDown: the active height handle by one 0.5 m step, as a drag
    /// sets it: the live end's while drawing, the hovered or last dragged one in an edit
    /// phase), `mode_straight`,
    /// `mode_simple_curve`, `mode_complex_curve`, `mode_freeform`, and `forget_preview`
    /// (the bridge drops its standing preview, so the next frame previews the stroke afresh:
    /// measurements and another world). The commit's answer is the bridge's own
    /// (`road_tool_commit_poll`). Returns `phase` and `placed` (as `road_tool_frame` reports
    /// them, after the event), and for a click that sent the stroke: `points` (the stroke),
    /// `preview` (the preview of exactly this stroke, the dictionary `preview_road` returns),
    /// `fresh` (whether the click had to preview it), `request_id` (the commit, which
    /// `road_tool_commit_poll` answers; 0 when the preview refused it: the click is then logged
    /// on `events` as the tool's refusal and the stroke stays editable). An unknown name returns
    /// `{"error": "road_tool_event_unknown"}`.
    #[func]
    pub fn road_tool_event(&mut self, name: GString) -> VarDictionary {
        let event = match name.to_string().as_str() {
            "press" => RoadToolEvent::Press(PointerButton::Left),
            "right" => RoadToolEvent::Press(PointerButton::Right),
            "release" => RoadToolEvent::Release(PointerButton::Left),
            "cancel" => RoadToolEvent::Action(RoadToolAction::Cancel),
            "confirm" => RoadToolEvent::Action(RoadToolAction::Confirm),
            "mode_straight" => RoadToolEvent::Action(RoadToolAction::ModeStraight),
            "mode_simple_curve" => RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
            "mode_complex_curve" => RoadToolEvent::Action(RoadToolAction::ModeComplexCurve),
            "mode_freeform" => RoadToolEvent::Action(RoadToolAction::ModeFreeform),
            "raise_handle" => RoadToolEvent::Action(RoadToolAction::RaiseHandle),
            "lower_handle" => RoadToolEvent::Action(RoadToolAction::LowerHandle),
            "forget_preview" => {
                self.road_tool_session.previewed = None;
                self.road_tool_session.preview = None;
                self.road_tool_session.batch_running = false;
                return vdict! { "phase": self.road_tool_phase().0 };
            }
            _ => return vdict! { "error": "road_tool_event_unknown" },
        };
        self.road_tool_handle(event);
        let mut result = match self.road_tool_session.session.take_commit() {
            Some(stroke) => self.road_tool_submit(stroke),
            None => VarDictionary::new(),
        };
        let (phase, placed) = self.road_tool_phase();
        result.set("phase", phase);
        result.set("placed", placed);
        result
    }

    /// Sets one road tool option: `lanes` (`Vector2i` forward, backward: 0..4 each), and the
    /// snapping toggles `network_snap`, `guide_lines`, `zoning_grid` (bool), `angle_step_deg`
    /// (15, 30, 45 or 90; anything else turns the lock off), `length_step_m` (0 turns it off),
    /// `continuous` (bool), `straight_grade` (bool, U3: the grade line between the ends' heights
    /// even with both handles on their supports). A road type sets the lanes with the median,
    /// parking and attributes (`road_tool_set_road_type`); `set_road_tool_median`,
    /// `set_road_tool_parking` and `set_road_tool_attributes` set one each. Returns "" when set,
    /// else `road_tool_option_unknown` or `road_tool_option_value`.
    #[func]
    pub fn road_tool_set_option(&mut self, name: GString, value: Variant) -> GString {
        let float = || value.try_to::<f64>().ok();
        let flag = || value.try_to::<bool>().ok();
        let option = match name.to_string().as_str() {
            "lanes" => {
                let Ok(lanes) = value.try_to::<Vector2i>() else {
                    return "road_tool_option_value".into();
                };
                let mut spec = self.road_tool_session.session.spec();
                spec.fwd_lanes = lanes.x.clamp(0, 4) as u8;
                spec.bkw_lanes = lanes.y.clamp(0, 4) as u8;
                Some(RoadToolOption::RoadType(spec))
            }
            "network_snap" => flag().map(RoadToolOption::NetworkSnap),
            "guide_lines" => flag().map(RoadToolOption::GuideLines),
            "zoning_grid" => flag().map(RoadToolOption::ZoningGrid),
            "continuous" => flag().map(RoadToolOption::Continuous),
            "angle_step_deg" => float().map(|step| RoadToolOption::AngleStepDeg(Some(step))),
            "length_step_m" => float().map(|step| RoadToolOption::LengthStepM(Some(step))),
            "straight_grade" => flag().map(RoadToolOption::StraightGrade),
            _ => return "road_tool_option_unknown".into(),
        };
        let Some(option) = option else {
            return "road_tool_option_value".into();
        };
        self.road_tool_session.session.set_option(option);
        GString::new()
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

    /// The road tool's frame, called once a frame while the tool is active: `phase` (0 idle,
    /// 1 placing, 2 waiting for the commit's acknowledgement, 3 a freeform draft's edit phase),
    /// `placed` (points down), `mode` (0 straight, 1 simple curve, 2 complex curve, 3 freeform),
    /// `continuous`, `cursor` and
    /// `cursor_valid`, `snap` (what took the cursor: `free`, `node`, `edge`, `guide`,
    /// `guide_corner`, `grid`, `angle`, `angle_edge`, `angle_guide`, `angle_grid`, `border`,
    /// `self`), `points` (the stroke of this pose, empty without one), `refusal` (why the draft
    /// has no stroke: `too_short`, `fold_back`, `full_turn`, `no_biarc`, `road_tight_curve`, else
    /// ""), `hud`
    /// (`length_m`, `bearing_deg`, `start_angle_deg` and `end_angle_deg` (-1 for none),
    /// `radius_m` (the smallest arc radius, -1 for a straight draft), `other_radius_m` (an
    /// S-curve's other arc radius, -1 for any other draft), `design_speed_kmh` (-1
    /// for none), `design_speed_warning`, `illegal_radius_m`, `tight_radius`, `build_cost` and
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
    /// one handle surface's UVs) and `zoning_depth_m` (-1 for a type that does not zone).
    /// Heights (round I): `grade` (the grade line's two end heights, empty when the stroke follows the ground), `height_handles`
    /// (one dictionary per end that has one: `end` 0 start or 1 end, `support`, `height_m`,
    /// `offset_m`, `grip`, `locked`, `active`), the HUD's `start_height_m`, `end_height_m`
    /// (above the support; a locked end's above the terrain) and `slope_percent` (-1000 for
    /// none), `piece_slopes_percent` (per guide piece), and the draft's runs: `draft_profiles`
    /// (`RoadProfile::code` each: 0 follow ground, 1 held ground, 2 bridge, 3 tunnel),
    /// `draft_lengths_m`, `draft_slopes_percent`, and `draft_reason` (the segmentation's
    /// refusal, else ""). When the stroke, its type, its grade or the context changed since the
    /// last preview, the frame previews it: `preview` holds `preview_road`'s dictionary (a batch
    /// pose's: `is_valid`, `is_pending`, `invalid_reason`, `build_cost`, `length_m`, then on the
    /// frame tier 2 accepts it the exact overlay with `preview_road`'s keys, or its refusal),
    /// with `pose`, the pose's number, which the hue built with it carries
    /// ([`Self::road_tool_hue`]'s `pose`, `ROAD-123` round HF2); `batch` holds a new batch pose's tier 1 (`preview_road_draft`'s keys: the ghost) and
    /// `batch_cleared` is true when the ghost should go; `preview_cleared` is true when the
    /// stroke went away. No preview while a sent stroke waits for its acknowledgement. The
    /// frame also builds the label chips, which the HUD fetches only when they changed
    /// (`road_tool_chips_update`, round R2-5).
    #[func]
    pub fn road_tool_frame(&mut self) -> VarDictionary {
        self.road_tool_update_draft();
        let mut dict = self.road_tool_frame_record();
        let frame = self.road_tool_session.session.frame();
        let (awaiting, has_stroke) = (
            frame.state.phase == Phase::AwaitingAck,
            frame.stroke.points.len() >= 2,
        );
        // No preview while the sent stroke waits; its chips show it pending.
        if !awaiting {
            if !has_stroke {
                if self.road_tool_session.previewed.take().is_some() {
                    self.road_tool_session.preview = None;
                    dict.set("preview_cleared", true);
                }
                self.road_tool_session.batch_running = false;
                self.road_tool_clear_batch_ghost(&mut dict);
            } else if !self.road_tool_preview_current() {
                let preview = self.road_tool_run_preview(Some(&mut dict));
                dict.set("preview", preview);
            } else if self.road_tool_session.batch_running
                && let Some(preview) = self.road_tool_step_batch(&mut dict)
            {
                dict.set("preview", preview);
            }
        }
        // A hue in flight stays in flight: its stroke stands, so `follow` would keep it.
        let bridge = &mut self.road_tool_session;
        if !(has_stroke || awaiting) {
            bridge.hue.hue().follow(bridge.session.frame());
        }
        dict.set("hue_revision", bridge.hue.revision() as i64);
        dict.set("zoning_depth_m", bridge.session.frame().zoning_depth_m.unwrap_or(-1.0));
        self.road_tool_set_labels(awaiting && has_stroke);
        dict
    }

    /// The draft's footprint hue (`ROAD-112` round HU; a stopgap marking the footprint while the
    /// preview terrain lags, T27 (d)), as of the last `road_tool_frame`, whose `hue_revision`
    /// changes whenever it does: `revision`, `pose` (the pose of the preview it was built with,
    /// in the same frame: the frame's `preview`'s `pose`), `state` (-1 hidden, 0 valid, 1 checking, 2
    /// refused, drawn hatched), the fill (a triangle list on the road, a bridge's on the ground
    /// under it, padded with collapsed triangles to a power-of-two count so a drag uploads in
    /// place) as `fill_vertices` (its count) and `fill_bytes` (the vertex buffer's bytes, three
    /// little-endian `f32` a vertex, uploaded as they are), and the tunnel runs' outlines as
    /// `road_tool_line.gdshader` quads (`outline_vertices`, `outline_attributes`, looks 8 to 10
    /// by state; empty without a tunnel), with `build_ms` (its build on its worker, off this
    /// thread) and `wait_ms` (what this call waited for it). Joins the hue's build when it is
    /// still running (`ROAD-123` round HF2: the frame's one wait for it, at its upload).
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

    /// The heights part of the session's last frame record (`road_tool_frame`'s round I keys:
    /// `grade`, `height_handles`, the HUD's heights and slopes, `piece_slopes_percent` and the
    /// draft's runs) as of the last `road_tool_frame`; previews nothing (tests and probes read
    /// it between the tool's frames).
    #[func]
    pub fn road_tool_heights(&self) -> VarDictionary {
        let mut dict = VarDictionary::new();
        set_heights(&mut dict, self.road_tool_session.session.frame());
        dict
    }

    // The road tool's frame record without the preview.
    fn road_tool_frame_record(&self) -> VarDictionary {
        let frame = self.road_tool_session.session.frame();
        let phase = self.road_tool_phase();
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

    // The phase code (0 idle, 1 placing, 2 waiting for the acknowledgement) and points placed.
    fn road_tool_phase(&self) -> (i64, i64) {
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
    fn road_tool_preview_current(&self) -> bool {
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
    // as a batch pose, a refusal as the preview's refusal). A batch pose's tier 1 goes into
    // `frame` as `batch` (the ghost) when given, and an earlier ghost is cleared. Returns the
    // preview's dictionary. The last outcome's and key's buffers are reused.
    fn road_tool_run_preview(&mut self, frame: Option<&mut VarDictionary>) -> VarDictionary {
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
        let mut tier1 = None;
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
                let (preview, pose) = self.road_tool_batch_pose(&mut outcome);
                tier1 = Some(pose);
                preview
            }
        };
        if let Some(frame) = frame {
            match tier1 {
                Some(pose) => {
                    frame.set("batch", pose);
                    self.road_tool_session.batch_ghost = true;
                }
                None => self.road_tool_clear_batch_ghost(frame),
            }
        }
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
            dictionary: preview.clone(),
        });
        preview
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

    // A new batch pose: tier 1 (`preview_road_draft`'s keys, the ghost) and the standing preview
    // it gives into `outcome`, pending while every stroke passes (tier 2 then runs on the
    // resting frames, on the requests kept here), else the first refusal with its stroke
    // (`draft_stroke`). The hue takes tier 1's prepared strokes (checking while tier 2 runs), or
    // the drawn stroke's over the caps.
    fn road_tool_batch_pose(
        &mut self,
        outcome: &mut PreviewOutcome,
    ) -> (VarDictionary, VarDictionary) {
        let mut pose = VarDictionary::new();
        let requests = self.road_tool_draft_requests();
        let mut hue = std::mem::take(self.road_tool_session.hue.hue());
        let tier1 = self.preview_road_batch_tier1(&requests, &mut pose, Some(&mut hue));
        *self.road_tool_session.hue.hue() = hue;
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
        (preview, pose)
    }

    // A resting frame of a running batch: one tier-2 step on the pose's requests. On the frame
    // tier 2 accepts the batch (its overlay whole) the exact preview, keyed as `preview_road`'s,
    // replaces the standing pending one and the ghost goes; on a refusal the refusal does. The
    // pose's cost, length, generation and number stay. `None` while it runs.
    fn road_tool_step_batch(&mut self, frame: &mut VarDictionary) -> Option<VarDictionary> {
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
        preview.set("is_valid", accepted);
        preview.set("is_pending", false);
        preview.set("invalid_reason", outcome.reason.as_ref());
        preview.set(
            "draft_stroke",
            step.stroke.map_or(-1, |stroke| stroke as i64),
        );
        if accepted {
            self.road_tool_clear_batch_ghost(frame);
        }
        self.road_tool_session.preview = Some(StandingPreview {
            outcome,
            dictionary: preview.clone(),
        });
        Some(preview)
    }

    fn road_tool_clear_batch_ghost(&mut self, frame: &mut VarDictionary) {
        if std::mem::take(&mut self.road_tool_session.batch_ghost) {
            frame.set("batch_cleared", true);
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

// The frame's heights (`road_tool_frame`'s round I keys).
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
    let handle_kind = |kind: &GizmoHandleKind| match kind {
        GizmoHandleKind::Endpoint => 0,
        GizmoHandleKind::Control => 1,
        GizmoHandleKind::ChainStart => 2,
        GizmoHandleKind::Cursor => 3,
        GizmoHandleKind::Joint => 4,
        // A height grip draws as a handle, in round V's look (rounds I, V2).
        GizmoHandleKind::Handle | GizmoHandleKind::HeightGrip => 5,
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
            .map(handle_kind)
            .collect::<PackedInt32Array>(),
    );
    // One point surface for every handle (round V2): each point's kind as its UV, the
    // attribute buffer `road_tool_handle.gdshader` reads its colour and size by.
    dict.set(
        "gizmo_handle_attributes",
        gizmos
            .handle_kinds
            .iter()
            .flat_map(|kind| [handle_kind(kind) as f32, 0.0])
            .collect::<PackedFloat32Array>(),
    );
}

// The quads of the road tool's lines (`road_tool_line.gdshader` widens them on the screen, as
// `route_ribbon.gdshader` does): four vertices a segment, the vertex buffer's positions and the
// attribute buffer's interleaved `[other end x, y, z, side * (dash metres + 1), style]` (UV, UV2
// and CUSTOM0's red, 20 bytes a vertex; the side is the sign, the dash station the magnitude less
// one; the style picks the shader's colour, width and dash: `LineStyle`), padded with collapsed
// quads to a power-of-two segment count (at least 16; none without strips) so a drag keeps the
// surface's layout and uploads in place. Every style in one surface (round V2: one upload a
// frame instead of one a style).
#[derive(Default)]
pub(super) struct LineQuads {
    pub(super) vertices: PackedVector3Array,
    pub(super) attributes: PackedFloat32Array,
}

// Minimum quads a line surface holds.
const LINE_QUADS_MIN: usize = 16;
// Floats a vertex's attributes take.
const LINE_ATTRIBUTES: usize = 5;

/// The line looks `road_tool_line.gdshader` draws, as the attributes number them (its palette
/// arrays in `road_tool_gizmos.gd`'s `PALETTE` follow this order).
#[derive(Clone, Copy)]
pub(super) enum LineStyle {
    /// The draft's centreline.
    Stroke = 0,
    /// Every guide line in one look (round V item 6; a height handle's stem too, round V2); since
    /// round UI1 the tangent legs, control polygons and stems, the alignment guides having
    /// their own.
    Guide = 1,
    /// The zoning guide's outside (item 9).
    ZoningOutside = 2,
    /// Its inside.
    ZoningInside = 3,
    /// The grid curbs (item 8, drawn to stand out on the preview road's own edges).
    GridCurb = 4,
    /// A batch ghost's edges, tier 1 accepting the stroke (round V2).
    GhostEdge = 5,
    /// A batch ghost's edges, tier 1 refusing the stroke.
    GhostRefusedEdge = 6,
    /// An end's ground marker: its vertical line and its ring on the ground (round H2).
    GroundMarker = 7,
    /// A tunnel run's outline in the footprint hue (round HU), the draft accepted.
    HueOutlineValid = 8,
    /// The same, the draft still checked.
    HueOutlineChecking = 9,
    /// The same, the draft refused.
    HueOutlineRefused = 10,
    /// The alignment guide that took the cursor and the angle lock's ray (round UI1: thicker and
    /// more opaque than the shared guide look).
    Alignment = 11,
    /// A guide line near the cursor that has not taken it (round UI1), faint.
    GuideNear = 12,
    /// An end's corner arc (round UI1), or its square at a right angle (round UI2).
    CornerArc = 13,
}

// A strip class's look.
fn line_style(class: GizmoLineClass) -> LineStyle {
    match class {
        GizmoLineClass::Draft => LineStyle::Stroke,
        GizmoLineClass::Guide | GizmoLineClass::Angle => LineStyle::Alignment,
        GizmoLineClass::GuideNear => LineStyle::GuideNear,
        GizmoLineClass::CornerArc => LineStyle::CornerArc,
        GizmoLineClass::Tangent | GizmoLineClass::HeightStem => LineStyle::Guide,
        GizmoLineClass::ZoningOutside => LineStyle::ZoningOutside,
        GizmoLineClass::ZoningInside => LineStyle::ZoningInside,
        GizmoLineClass::GridCurb => LineStyle::GridCurb,
        GizmoLineClass::GroundMarker => LineStyle::GroundMarker,
    }
}

// The gizmo strips as quads. `O(strip points)`.
fn line_quads(gizmos: &Gizmos) -> LineQuads {
    strip_quads(&gizmos.line_points, &gizmos.line_starts, |strip| {
        line_style(gizmos.line_classes[strip])
    })
}

// Strips (`points` back to back, `starts` each strip's first point) as quads, each strip in the
// look `style_of` its index; the batch ghost's edges use it too. `O(points)`.
pub(super) fn strip_quads(
    points: &[Vector3],
    starts: &[u32],
    style_of: impl Fn(usize) -> LineStyle,
) -> LineQuads {
    let strip = |index: usize| {
        let start = starts[index] as usize;
        let end = starts
            .get(index + 1)
            .map_or(points.len(), |&end| end as usize);
        &points[start..end]
    };
    let segments: usize = (0..starts.len())
        .map(|index| strip(index).len().saturating_sub(1))
        .sum();
    // Without strips no quads (the surface is left out).
    let capacity = if segments == 0 {
        0
    } else {
        segments.max(LINE_QUADS_MIN).next_power_of_two()
    };
    let mut quads = LineQuads::default();
    quads.vertices.resize(capacity * 4);
    quads.attributes.resize(capacity * 4 * LINE_ATTRIBUTES);
    let (vertices, attributes) = (
        quads.vertices.as_mut_slice(),
        quads.attributes.as_mut_slice(),
    );
    let mut quad = 0;
    for index in 0..starts.len() {
        let style = style_of(index) as u8 as f32;
        let mut dash = 0.0;
        for pair in strip(index).windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = a.distance_to(b);
            // Each end twice, one vertex per side; the side's sign is taken from the start's
            // view, so it flips at the far end (whose other end points back).
            for (corner, (point, other, side, at)) in [
                (a, b, 1.0, dash),
                (a, b, -1.0, dash),
                (b, a, -1.0, dash + length),
                (b, a, 1.0, dash + length),
            ]
            .into_iter()
            .enumerate()
            {
                let vertex = quad * 4 + corner;
                vertices[vertex] = point;
                attributes[vertex * LINE_ATTRIBUTES..(vertex + 1) * LINE_ATTRIBUTES]
                    .copy_from_slice(&[other.x, other.y, other.z, side * (at + 1.0), style]);
            }
            dash += length;
            quad += 1;
        }
    }
    quads
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
