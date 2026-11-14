// SPDX-License-Identifier: GPL-2.0-only

//! Road tool session Godot API (`ROAD-112` round C, plan 2.3 and 2.4): `road_tool.gd` forwards
//! the pointer, clicks and keys, sets options, and draws what `road_tool_frame` returns. The
//! session (`nodes::sim::road_tool::session`) decides the cursor, the stroke, the chain and the
//! commit; this bridge runs the existing preview (`preview_road`) for the session's stroke when
//! it changed and sends the confirmed stroke with `add_road_with_snap`. One session per
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
use crate::nodes::sim::road_tool::batch::draft_requests;
use crate::nodes::sim::road_tool::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Gizmos, Phase, RoadToolFrame, StrokeRequest,
};
use crate::nodes::sim::road_tool::height::draft_batch;
use crate::nodes::sim::road_tool::labels::{
    LabelKind, LabelTone, PreviewFacts, apply_preview, hint_text, middle_label, place_chips,
};
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::modes::DrawMode;
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::SnapKind;
use crate::simulation::network::road_edit::RoadProfile;

/// The road tool session of one `SimulationNode` and the preview the bridge last ran for it.
#[derive(Default)]
pub(crate) struct RoadToolBridge {
    session: RoadToolSession,
    // What the standing preview is for; `None` when none stands.
    previewed: Option<PreviewKey>,
    // That preview's dictionary: a click on the same stroke submits it.
    preview: Option<VarDictionary>,
    // The standing preview is a batch's whose tier 2 still runs (each resting frame steps it).
    batch_running: bool,
    // A batch ghost is drawn (`road_tool_gizmos.gd`'s `show_batch_ghost`).
    batch_ghost: bool,
}

// The inputs a preview depends on, as today's `_preview_is_for` compared them: the stroke, its
// cross-section, the snap flag, the grade line (round I) and the context generation the preview
// was made on.
#[derive(Clone, Debug, PartialEq)]
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
    /// `mode_simple_curve`, `mode_complex_curve`, `mode_freeform`, and the commit acknowledgement `committed` or
    /// `refused` (what `get_road_commit_result` said of the sent stroke; after a refusal the
    /// unchanged stroke on the same context stays refused, `road_commit_rejected`), and
    /// `forget_preview`
    /// (the bridge drops its standing preview, so the next frame previews the stroke afresh:
    /// measurements and another world). Returns `phase` and `placed` (as
    /// `road_tool_frame` reports them, after the event), and for a click that sent the stroke:
    /// `points` (the stroke), `preview` (the preview of exactly this
    /// stroke, the dictionary `preview_road` returns), `fresh` (whether the click had to preview
    /// it), `request_id` (the commit to poll, 0 when the preview refused it: the click is then
    /// logged on `events` as the tool's refusal and the stroke stays editable). An unknown name
    /// returns `{"error": "road_tool_event_unknown"}`.
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
            "committed" => RoadToolEvent::CommitAck { committed: true },
            "refused" => RoadToolEvent::CommitAck { committed: false },
            "forget_preview" => {
                self.road_tool_session.previewed = None;
                self.road_tool_session.preview = None;
                self.road_tool_session.batch_running = false;
                return vdict! { "phase": self.road_tool_phase().0 };
            }
            _ => return vdict! { "error": "road_tool_event_unknown" },
        };
        self.road_tool_handle(event);
        if event == (RoadToolEvent::CommitAck { committed: false }) {
            self.road_tool_reject_standing_preview();
        }
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
    /// even with both handles on their supports). The
    /// median, parking and attributes come from `set_road_tool_median`, `set_road_tool_parking`
    /// and `set_road_tool_attributes`. Returns "" when set, else `road_tool_option_unknown` or
    /// `road_tool_option_value`.
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
    /// zoning-depth guide's outside, 5 its inside, 6 a grid curb), the same strips as quads for
    /// `road_tool_line.gdshader`, by style (round V): `gizmo_stroke_vertices` / `_attributes`
    /// (the centreline), `gizmo_guide_*` (guides, the angle ray, tangent legs),
    /// `gizmo_zoning_out_*` (the zoning guide's outside), `gizmo_zoning_in_*` (its inside) and
    /// `gizmo_curb_*` (the grid curbs), empty for a style without strips,
    /// `gizmo_handles` with `gizmo_handle_kinds` (0 placed end, 1 control point, 2 chain start,
    /// 3 the live end or the grabbed freeform grip, 4 a joint: the S-curve's or a freeform inner
    /// node, 5 a handle: an S-curve arc's control point, a freeform Bezier handle or a height
    /// handle's grip, whose stem is a guide strip), the label chips (`label_kinds`: 0 piece
    /// length, 1 total length, 2 radius, 3 angle, 4 slope, 5 cost, 6 bearing, 7 design speed, 8
    /// notice, 9 hint, 10 Build, 11 Cancel; `label_values`, `label_anchors`, `label_sides`,
    /// `label_tones`: 0 normal, 1 warning, 2 refused; `label_hint`, the hint's text) and
    /// `zoning_depth_m` (-1 for a type that does not zone). Heights (round I): `grade` (the grade
    /// line's two end heights, empty when the stroke follows the ground), `height_handles`
    /// (one dictionary per end that has one: `end` 0 start or 1 end, `support`, `height_m`,
    /// `offset_m`, `grip`, `locked`, `active`), the HUD's `start_height_m`, `end_height_m`
    /// (above the support; a locked end's above the terrain) and `slope_percent` (-1000 for
    /// none), `piece_slopes_percent` (per guide piece), and the draft's runs: `draft_profiles`
    /// (`RoadProfile::code` each: 0 follow ground, 1 held ground, 2 bridge, 3 tunnel),
    /// `draft_lengths_m`, `draft_slopes_percent`, and `draft_reason` (the segmentation's
    /// refusal, else ""). When the stroke, its type, its grade or the context changed since the
    /// last preview, the frame previews it: `preview` holds `preview_road`'s dictionary (a batch
    /// pose's: `is_valid`, `is_pending`, `invalid_reason`, `build_cost`, `length_m`, then on the
    /// frame tier 2 accepts it the exact overlay with `preview_road`'s keys, or its refusal);
    /// `batch` holds a new batch pose's tier 1 (`preview_road_draft`'s keys: the ghost) and
    /// `batch_cleared` is true when the ghost should go; `preview_cleared` is true when the
    /// stroke went away. No preview while a sent stroke waits for its acknowledgement.
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
        self.road_tool_set_labels(&mut dict, awaiting && has_stroke);
        dict
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

    /// Places the road tool's label chips on the screen (`road_tool_hud.gd`, round V): chip `i`
    /// of `sizes[i]` pixels, anchored at the projected `anchors[i]`, pushed along the projected
    /// unit direction `sides[i]`, `gap` pixels from its anchor, from every chip placed before
    /// it and from the squares of `obstacle_px` about each of `obstacles` (the projected
    /// handles) (`labels::place_chips`, the one placement rule). Returns each chip's top-left
    /// corner.
    #[func]
    pub fn road_tool_place_labels(
        &self,
        anchors: PackedVector2Array,
        sides: PackedVector2Array,
        sizes: PackedVector2Array,
        obstacles: PackedVector2Array,
        obstacle_px: f32,
        gap: f32,
    ) -> PackedVector2Array {
        let mut corners = Vec::with_capacity(anchors.len());
        place_chips(
            anchors.as_slice(),
            sides.as_slice(),
            sizes.as_slice(),
            obstacles.as_slice(),
            obstacle_px,
            gap,
            &mut corners,
        );
        PackedVector2Array::from(corners.as_slice())
    }

    // The frame's label chips with what the standing preview says (the class-priced cost, the
    // steepest grade, a refusal or a warning; `pending` while the sent stroke waits), as packed
    // arrays: `label_kinds`, `label_values`, `label_anchors`, `label_sides` (unit XZ, y 0) and
    // `label_tones`, with `label_hint` (the hint chip's text) and `zoning_depth_m`.
    fn road_tool_set_labels(&self, dict: &mut VarDictionary, pending: bool) {
        let frame = self.road_tool_session.session.frame();
        let mut labels = frame.labels.clone();
        let preview = self
            .road_tool_session
            .preview
            .as_ref()
            .filter(|_| frame.stroke.points.len() >= 2);
        if let Some(preview) = preview {
            let number = |key: &str| preview.get(key).and_then(|v| v.try_to::<f64>().ok());
            let text = |key: &str| {
                preview
                    .get(key)
                    .and_then(|v| v.try_to::<GString>().ok())
                    .map(|v| v.to_string())
                    .unwrap_or_default()
            };
            let valid = preview
                .get("is_valid")
                .and_then(|v| v.try_to::<bool>().ok())
                .unwrap_or(false);
            let plan_state = text("plan_state");
            let facts = PreviewFacts {
                refused: !valid,
                pending,
                build_cost: number("build_cost"),
                max_grade: number("max_grade"),
                warning: !text("warning_reason").is_empty()
                    || preview
                        .get("demolish_count")
                        .and_then(|v| v.try_to::<i64>().ok())
                        .is_some_and(|count| count > 0)
                    || !(plan_state.is_empty() || plan_state == "ready"),
            };
            let middle = middle_label(&labels);
            apply_preview(&mut labels, facts, middle);
        }
        let kind = |kind: LabelKind| match kind {
            LabelKind::PieceLength => 0,
            LabelKind::TotalLength => 1,
            LabelKind::Radius => 2,
            LabelKind::Angle => 3,
            LabelKind::Slope => 4,
            LabelKind::Cost => 5,
            LabelKind::Bearing => 6,
            LabelKind::DesignSpeed => 7,
            LabelKind::Notice => 8,
            LabelKind::Hint => 9,
            LabelKind::Build => 10,
            LabelKind::Cancel => 11,
        };
        let tone = |tone: LabelTone| match tone {
            LabelTone::Normal => 0,
            LabelTone::Warning => 1,
            LabelTone::Refused => 2,
        };
        dict.set(
            "label_kinds",
            labels
                .iter()
                .map(|l| kind(l.kind))
                .collect::<PackedInt32Array>(),
        );
        dict.set(
            "label_values",
            labels
                .iter()
                .map(|l| l.value)
                .collect::<PackedFloat64Array>(),
        );
        dict.set(
            "label_anchors",
            labels
                .iter()
                .map(|l| l.anchor)
                .collect::<PackedVector3Array>(),
        );
        dict.set(
            "label_sides",
            labels
                .iter()
                .map(|l| Vector3::new(l.side.x as f32, 0.0, l.side.y as f32))
                .collect::<PackedVector3Array>(),
        );
        dict.set(
            "label_tones",
            labels
                .iter()
                .map(|l| tone(l.tone))
                .collect::<PackedInt32Array>(),
        );
        dict.set("zoning_depth_m", frame.zoning_depth_m.unwrap_or(-1.0));
        // The hint chip's wording (`labels::hint_text`), "" without one.
        let hint = labels
            .iter()
            .find(|label| label.kind == LabelKind::Hint)
            .map_or("", |label| hint_text(label.value > 0.0));
        dict.set("label_hint", hint);
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

    // The core refused the sent stroke: its standing preview becomes the refusal
    // (`road_commit_rejected`, as `road_tool.gd` shows it), so another click on the unchanged
    // stroke on the same context is refused again instead of sent again (today's rule).
    fn road_tool_reject_standing_preview(&mut self) {
        let Some(preview) = &self.road_tool_session.preview else {
            return;
        };
        let mut rejected = preview.duplicate_shallow();
        rejected.set("is_valid", false);
        rejected.set("is_pending", false);
        rejected.set("invalid_reason", "road_commit_rejected");
        // Its graded tiles were handed over at the click.
        for key in ["graded_tiles", "graded_removed", "graded_pending"] {
            rejected.remove(key);
        }
        self.road_tool_session.preview = Some(rejected);
    }

    // The median, parking and attributes the bridge's setters hold, into the session's road type
    // (its stroke requests and HUD price the selected type, B9). Lanes stay the session's.
    pub(super) fn sync_road_tool_spec(&mut self) {
        let spec = RoadTypeSpec {
            median: self.road_tool_median,
            parking: self.road_tool_parking,
            attributes: self.road_tool_attributes,
            ..self.road_tool_session.session.spec()
        };
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

    // Previews the session's stroke as it stands, with the session's lanes and the bridge's
    // median, parking and attributes, and keeps the result as the standing preview: a
    // follow-ground stroke as today; a graded one through its segmentation (one stroke alone
    // with its profile, several as a batch pose, a refusal as the preview's refusal). A batch
    // pose's tier 1 goes into `frame` as `batch` (the ghost) when given, and an earlier ghost
    // is cleared.
    fn road_tool_run_preview(&mut self, frame: Option<&mut VarDictionary>) -> VarDictionary {
        self.road_tool_update_draft();
        self.road_tool_session.batch_running = false;
        let spec = self.road_tool_session.session.spec();
        let (fwd, bkw) = (i32::from(spec.fwd_lanes), i32::from(spec.bkw_lanes));
        let stroke = self.road_tool_session.session.frame().stroke.clone();
        let snap = stroke.snap_to_existing_roads;
        let draft = &self.road_tool_session.session.frame().draft;
        let mut tier1 = None;
        let preview = match (stroke.grade, draft.refusal, draft.strokes.as_slice()) {
            (None, ..) => self.preview_stroke(
                stroke.points.clone(),
                fwd,
                bkw,
                snap,
                RoadProfile::FollowGround,
            ),
            (Some(_), Some(reason), _) => vdict! {
                "is_valid": false,
                "is_pending": false,
                "invalid_reason": reason,
                "build_cost": 0.0,
                "surface_generation": self.road_tool_view_generation(),
            },
            (Some(_), None, [single]) => {
                self.preview_stroke(single.points.clone(), fwd, bkw, snap, single.profile)
            }
            (Some(_), None, _) => {
                let (preview, pose) = self.road_tool_batch_pose();
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
        let generation = preview
            .get("surface_generation")
            .and_then(|value| value.try_to::<i64>().ok())
            .unwrap_or(-1);
        self.road_tool_session.previewed = Some(PreviewKey {
            points: stroke.points,
            spec,
            snap,
            grade: stroke.grade,
            generation,
        });
        self.road_tool_session.preview = Some(preview.clone());
        preview
    }

    // A new batch pose: tier 1 (`preview_road_draft`'s keys, the ghost) and the standing preview
    // it gives, pending while every stroke passes (tier 2 then runs on the resting frames), else
    // the first refusal with its stroke (`draft_stroke`).
    fn road_tool_batch_pose(&mut self) -> (VarDictionary, VarDictionary) {
        let mut pose = VarDictionary::new();
        self.preview_road_batch_tier1(&self.road_tool_draft_requests(), &mut pose);
        let text = |key: &str| pose.get(key).map(|value| value.to::<GString>().to_string());
        let mut reason = text("reason").unwrap_or_default();
        let mut stroke = -1_i64;
        if reason.is_empty()
            && let Some(reasons) = pose
                .get("reasons")
                .and_then(|value| value.try_to::<PackedStringArray>().ok())
            && let Some(index) = reasons.as_slice().iter().position(|r| !r.is_empty())
        {
            reason = reasons.as_slice()[index].to_string();
            stroke = index as i64;
        }
        let valid = reason.is_empty();
        self.road_tool_session.batch_running = valid;
        let number = |key: &str| {
            pose.get(key)
                .and_then(|value| value.try_to::<f64>().ok())
                .unwrap_or(0.0)
        };
        let preview = vdict! {
            "is_valid": valid,
            "is_pending": valid,
            "invalid_reason": reason,
            "draft_stroke": stroke,
            "build_cost": number("build_cost"),
            "length_m": number("length_m"),
            "surface_generation": self.road_tool_view_generation(),
        };
        (preview, pose)
    }

    // A resting frame of a running batch: one tier-2 step. On the frame tier 2 accepts the batch
    // (its overlay whole) the exact preview, keyed as `preview_road`'s, replaces the standing
    // pending one and the ghost goes; on a refusal the refusal does. `None` while it runs.
    fn road_tool_step_batch(&mut self, frame: &mut VarDictionary) -> Option<VarDictionary> {
        let requests = self.road_tool_draft_requests();
        let mut step = VarDictionary::new();
        self.preview_road_batch_tier2(requests, &mut step);
        let text = |key: &str| {
            step.get(key)
                .map(|value| value.to::<GString>().to_string())
                .unwrap_or_default()
        };
        let state = text("tier2_state");
        if state == "running" {
            return None;
        }
        self.road_tool_session.batch_running = false;
        let standing = self.road_tool_session.preview.clone().unwrap_or_default();
        let mut preview = step.clone();
        for key in ["build_cost", "length_m", "surface_generation"] {
            if let Some(value) = standing.get(key) {
                preview.set(key, value);
            }
        }
        let accepted = state == "accepted";
        preview.set("is_valid", accepted);
        preview.set("is_pending", false);
        preview.set("invalid_reason", text("tier2_reason"));
        preview.set(
            "draft_stroke",
            step.get("tier2_stroke").unwrap_or((-1_i64).to_variant()),
        );
        if accepted {
            self.road_tool_clear_batch_ghost(frame);
        }
        self.road_tool_session.preview = Some(preview.clone());
        Some(preview)
    }

    fn road_tool_clear_batch_ghost(&mut self, frame: &mut VarDictionary) {
        if std::mem::take(&mut self.road_tool_session.batch_ghost) {
            frame.set("batch_cleared", true);
        }
    }

    // Sends the confirmed stroke: the preview of exactly it (the standing one, else a fresh one),
    // then the commit when the preview accepts it (a graded stroke's strokes as one batch), else
    // the refusal's log line and the session's refusal acknowledgement (the stroke stays
    // editable).
    fn road_tool_submit(&mut self, stroke: StrokeRequest) -> VarDictionary {
        let fresh = !self.road_tool_preview_current();
        let preview = match &self.road_tool_session.preview {
            Some(preview) if !fresh => preview.clone(),
            _ => self.road_tool_run_preview(None),
        };
        let points = PackedVector3Array::from(stroke.points.as_slice());
        let (fwd, bkw) = (
            i32::from(stroke.spec.fwd_lanes),
            i32::from(stroke.spec.bkw_lanes),
        );
        let snap = stroke.snap_to_existing_roads;
        let valid = preview
            .get("is_valid")
            .and_then(|value| value.try_to::<bool>().ok())
            .unwrap_or(false);
        let request_id = match (valid, stroke.grade) {
            (true, None) => self.add_road_with_snap(points.clone(), fwd, bkw, snap),
            (true, Some(_)) => self.road_tool_send_draft(),
            (false, _) => {
                self.road_tool_log_refusal(&stroke, &preview);
                self.road_tool_handle(RoadToolEvent::CommitAck { committed: false });
                0
            }
        };
        vdict! {
            "points": points,
            "preview": preview,
            "fresh": fresh,
            "request_id": request_id,
        }
    }

    // Queues the graded draft's strokes as one batch, one undo step (a single stroke as a batch
    // of one, `add_road_with_profile`'s path), with tier 2's plans and pieces when it finished
    // for exactly these strokes (`add_road_draft`'s rule).
    fn road_tool_send_draft(&mut self) -> i64 {
        let frame = self.road_tool_session.session.frame();
        let strokes = frame.draft.strokes.clone();
        let section = self.road_tool_draft_section(frame);
        let products = (strokes.len() > 1)
            .then(|| {
                let requests = self.road_tool_draft_requests();
                self.road_tool
                    .context
                    .lock()
                    .expect("road tool context lock poisoned")
                    .take_batch_products(&requests)
            })
            .flatten();
        let mut batch = draft_batch(strokes, section);
        batch.preview = products;
        self.send_road_batch(batch)
    }

    // The refusal's log line: a follow-ground stroke as today; a graded draft's refusing stroke
    // with its profile when the preview names one, else the draft as held ground.
    fn road_tool_log_refusal(&mut self, stroke: &StrokeRequest, preview: &VarDictionary) {
        let (fwd, bkw) = (
            i32::from(stroke.spec.fwd_lanes),
            i32::from(stroke.spec.bkw_lanes),
        );
        let snap = stroke.snap_to_existing_roads;
        let strokes = &self.road_tool_session.session.frame().draft.strokes;
        let named = preview
            .get("draft_stroke")
            .and_then(|value| value.try_to::<i64>().ok())
            .and_then(|index| usize::try_from(index).ok());
        let (points, profile) = match (stroke.grade, strokes.as_slice(), named) {
            (None, ..) => (stroke.points.clone(), RoadProfile::FollowGround),
            (Some(_), [single], _) => (single.points.clone(), single.profile),
            (Some(_), parts, Some(index)) if index < parts.len() => {
                (parts[index].points.clone(), parts[index].profile)
            }
            (Some(_), ..) => (stroke.points.clone(), RoadProfile::HeldGround),
        };
        self.log_road_tool_rejection_as(
            PackedVector3Array::from(points.as_slice()),
            [fwd, bkw],
            snap,
            preview.clone(),
            profile,
        );
    }
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
    };
    let handle_kind = |kind: &GizmoHandleKind| match kind {
        GizmoHandleKind::Endpoint => 0,
        GizmoHandleKind::Control => 1,
        GizmoHandleKind::ChainStart => 2,
        GizmoHandleKind::Cursor => 3,
        GizmoHandleKind::Joint => 4,
        // A height grip draws as a handle until the restyle gives it a look (round I).
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
    for ((vertices, attributes), quads) in LINE_STYLE_KEYS.into_iter().zip(line_quads(gizmos)) {
        dict.set(vertices, quads.vertices);
        dict.set(attributes, quads.attributes);
    }
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
}

// The quads one line style draws (`road_tool_line.gdshader` widens them on the screen, as
// `route_ribbon.gdshader` does): four vertices a segment, the vertex buffer's positions and the
// attribute buffer's interleaved `[other end x, y, z, side * (dash metres + 1)]` (UV then UV2, 16
// bytes a vertex; the side is the sign, the dash station the magnitude less one), padded with
// collapsed quads to a power-of-two segment count (at least 16; none for a style without
// strips) so a drag keeps the surfaces' layout and uploads in place.
#[derive(Default)]
struct LineQuads {
    vertices: PackedVector3Array,
    attributes: PackedFloat32Array,
}

// Minimum quads a line surface holds.
const LINE_QUADS_MIN: usize = 16;

// The line styles' frame keys (vertices, attributes), in style order (`line_style`).
const LINE_STYLE_KEYS: [(&str, &str); LINE_STYLES] = [
    ("gizmo_stroke_vertices", "gizmo_stroke_attributes"),
    ("gizmo_guide_vertices", "gizmo_guide_attributes"),
    ("gizmo_zoning_out_vertices", "gizmo_zoning_out_attributes"),
    ("gizmo_zoning_in_vertices", "gizmo_zoning_in_attributes"),
    ("gizmo_curb_vertices", "gizmo_curb_attributes"),
];
const LINE_STYLES: usize = 5;

// A strip class's style: the draft's centreline; every guide in one look (round V item 6, a
// height handle's stem too, round V2); the
// zoning guide's outside and inside (item 9); the grid curbs (item 8, drawn to stand out on the
// preview road's own edges).
fn line_style(class: GizmoLineClass) -> usize {
    match class {
        GizmoLineClass::Draft => 0,
        GizmoLineClass::Guide
        | GizmoLineClass::Angle
        | GizmoLineClass::Tangent
        | GizmoLineClass::HeightStem => 1,
        GizmoLineClass::ZoningOutside => 2,
        GizmoLineClass::ZoningInside => 3,
        GizmoLineClass::GridCurb => 4,
    }
}

// The gizmo strips as quads, one set per style (`line_style`). `O(strip points)`.
fn line_quads(gizmos: &Gizmos) -> [LineQuads; LINE_STYLES] {
    let mut out: [LineQuads; LINE_STYLES] = Default::default();
    let mut counts = [0usize; LINE_STYLES];
    let strips = gizmos.line_starts.len();
    let strip = |index: usize| {
        let start = gizmos.line_starts[index] as usize;
        let end = gizmos
            .line_starts
            .get(index + 1)
            .map_or(gizmos.line_points.len(), |&end| end as usize);
        &gizmos.line_points[start..end]
    };
    for index in 0..strips {
        let style = line_style(gizmos.line_classes[index]);
        counts[style] += strip(index).len().saturating_sub(1);
    }
    for (style, quads) in out.iter_mut().enumerate() {
        // A style without strips sends no quads (its surface is left out).
        let capacity = if counts[style] == 0 {
            0
        } else {
            counts[style].max(LINE_QUADS_MIN).next_power_of_two()
        };
        quads.vertices.resize(capacity * 4);
        quads.attributes.resize(capacity * 4 * 4);
    }
    let mut written = [0usize; LINE_STYLES];
    for index in 0..strips {
        let style = line_style(gizmos.line_classes[index]);
        let quads = &mut out[style];
        let (vertices, attributes) = (
            quads.vertices.as_mut_slice(),
            quads.attributes.as_mut_slice(),
        );
        let mut dash = 0.0;
        for pair in strip(index).windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let length = a.distance_to(b);
            let quad = written[style];
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
                attributes[vertex * 4..vertex * 4 + 4].copy_from_slice(&[
                    other.x,
                    other.y,
                    other.z,
                    side * (at + 1.0),
                ]);
            }
            dash += length;
            written[style] += 1;
        }
    }
    out
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
    }
}
