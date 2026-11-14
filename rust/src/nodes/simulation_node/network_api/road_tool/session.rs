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

use super::super::super::*;
use crate::nodes::sim::road_tool::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Gizmos, Phase, StrokeRequest,
};
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::modes::DrawMode;
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::SnapKind;

/// The road tool session of one `SimulationNode` and the preview the bridge last ran for it.
#[derive(Default)]
pub(crate) struct RoadToolBridge {
    session: RoadToolSession,
    // What the standing preview is for; `None` when none stands.
    previewed: Option<PreviewKey>,
    // That preview's dictionary: a click on the same stroke submits it.
    preview: Option<VarDictionary>,
}

// The inputs a preview depends on, as today's `_preview_is_for` compared them: the stroke, its
// cross-section, the snap and tunnel flags and the context generation the preview was made on.
#[derive(Clone, Debug, PartialEq)]
struct PreviewKey {
    points: Vec<Vector3>,
    spec: RoadTypeSpec,
    snap: bool,
    tunnel: bool,
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
    /// `confirm` (freeform: ends the placing, then sends the stroke), `mode_straight`,
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
            "committed" => RoadToolEvent::CommitAck { committed: true },
            "refused" => RoadToolEvent::CommitAck { committed: false },
            "forget_preview" => {
                self.road_tool_session.previewed = None;
                self.road_tool_session.preview = None;
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
    /// `continuous` (bool), `altitude_offset_m` (the keyboard height offset on the ground). The
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
            "altitude_offset_m" => {
                float().map(|offset| RoadToolOption::AltitudeOffsetM(offset as f32))
            }
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
    /// `gizmo_line_classes` (0 draft centreline, 1 guide line, 2 angle ray, 3 tangent leg),
    /// `gizmo_handles` with `gizmo_handle_kinds` (0 placed end, 1 control point, 2 chain start,
    /// 3 the live end or the grabbed freeform grip, 4 a joint: the S-curve's or a freeform inner
    /// node, 5 a handle: an S-curve arc's control point or a freeform Bezier handle). When the stroke, its type or the context changed since the last
    /// preview, the frame previews it: `preview` holds `preview_road`'s dictionary;
    /// `preview_cleared` is true when the stroke went away. No preview while a sent stroke waits
    /// for its acknowledgement.
    #[func]
    pub fn road_tool_frame(&mut self) -> VarDictionary {
        let mut dict = self.road_tool_frame_record();
        let frame = self.road_tool_session.session.frame();
        let (awaiting, has_stroke) = (
            frame.state.phase == Phase::AwaitingAck,
            frame.stroke.points.len() >= 2,
        );
        if awaiting {
            return dict;
        }
        if !has_stroke {
            if self.road_tool_session.previewed.take().is_some() {
                self.road_tool_session.preview = None;
                dict.set("preview_cleared", true);
            }
            return dict;
        }
        if !self.road_tool_preview_current() {
            let points = self.road_tool_session.session.frame().stroke.points.clone();
            dict.set("preview", self.road_tool_run_preview(points));
        }
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
                && key.tunnel == self.road_tool_tunnel
                && key.generation == self.road_tool_view_generation()
        })
    }

    fn road_tool_view_generation(&self) -> i64 {
        let generation = self.road_tool.view.read().unwrap().surface_generation;
        i64::try_from(generation).unwrap_or(i64::MAX)
    }

    // Previews the session's stroke `points` with the session's lanes and the bridge's median,
    // parking, attributes and tunnel, and keeps the result as the standing preview.
    fn road_tool_run_preview(&mut self, points: Vec<Vector3>) -> VarDictionary {
        let spec = self.road_tool_session.session.spec();
        let snap = self
            .road_tool_session
            .session
            .frame()
            .stroke
            .snap_to_existing_roads;
        let preview = self.preview_stroke(
            points.clone(),
            i32::from(spec.fwd_lanes),
            i32::from(spec.bkw_lanes),
            snap,
        );
        let generation = preview
            .get("surface_generation")
            .and_then(|value| value.try_to::<i64>().ok())
            .unwrap_or(-1);
        self.road_tool_session.previewed = Some(PreviewKey {
            points,
            spec,
            snap,
            tunnel: self.road_tool_tunnel,
            generation,
        });
        self.road_tool_session.preview = Some(preview.clone());
        preview
    }

    // Sends the confirmed stroke: the preview of exactly it (the standing one, else a fresh one),
    // then the commit when the preview accepts it, else the refusal's log line and the session's
    // refusal acknowledgement (the stroke stays editable).
    fn road_tool_submit(&mut self, stroke: StrokeRequest) -> VarDictionary {
        let fresh = !self.road_tool_preview_current();
        let preview = match &self.road_tool_session.preview {
            Some(preview) if !fresh => preview.clone(),
            _ => self.road_tool_run_preview(stroke.points.clone()),
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
        let request_id = if valid {
            self.add_road_with_snap(points.clone(), fwd, bkw, snap)
        } else {
            self.log_road_tool_rejection(points.clone(), fwd, bkw, snap, preview.clone());
            self.road_tool_handle(RoadToolEvent::CommitAck { committed: false });
            0
        };
        vdict! {
            "points": points,
            "preview": preview,
            "fresh": fresh,
            "request_id": request_id,
        }
    }
}

// The frame's gizmos as packed arrays (`road_tool_gizmos.gd` draws them): one copy each, once a
// frame.
fn set_gizmos(dict: &mut VarDictionary, gizmos: &Gizmos) {
    let line_class = |class: &GizmoLineClass| match class {
        GizmoLineClass::Draft => 0,
        GizmoLineClass::Guide => 1,
        GizmoLineClass::Angle => 2,
        GizmoLineClass::Tangent => 3,
    };
    let handle_kind = |kind: &GizmoHandleKind| match kind {
        GizmoHandleKind::Endpoint => 0,
        GizmoHandleKind::Control => 1,
        GizmoHandleKind::ChainStart => 2,
        GizmoHandleKind::Cursor => 3,
        GizmoHandleKind::Joint => 4,
        GizmoHandleKind::Handle => 5,
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
