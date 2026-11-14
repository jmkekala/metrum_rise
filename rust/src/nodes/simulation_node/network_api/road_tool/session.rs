// SPDX-License-Identifier: GPL-2.0-only

//! Road tool session Godot API (`ROAD-112` round C, plan 2.3 and 2.4): `road_tool.gd` forwards
//! the pointer, clicks and keys, sets options, and draws what `road_tool_frame` returns. The
//! session (`nodes::sim::road_tool::session`) decides the cursor, the stroke, the chain and the
//! commit; this bridge runs the existing preview (`preview_road`) for the session's stroke when
//! it changed and sends the confirmed stroke with `add_road_with_snap`. One session per
//! `SimulationNode`, used from the main thread only.
//!
//! Round C drives the straight mode; the curve modes stay on `road_tool.gd`'s own path until
//! round D. The pointer calls take typed arguments, so a pointer move allocates nothing once the
//! session's buffers have grown (the frame and the preview allocate, once a frame).

use super::super::super::*;
use crate::nodes::sim::road_tool::frame::{DraftRefusal, Phase, StrokeRequest};
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
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
    /// angle lock, guide lines, the zoning grid and the length step. Returns the resolved cursor,
    /// or `null` while no pointer has met a surface. Reads the published view only; allocates
    /// nothing once the session's buffers have grown.
    #[func]
    pub fn road_tool_pointer(&mut self, origin: Vector3, dir: Vector3, shift: bool) -> Variant {
        self.road_tool_handle(RoadToolEvent::Pointer {
            pointer: Pointer::Ray { origin, dir },
            shift,
        });
        self.road_tool_cursor_variant()
    }

    /// The road tool's pointer at a scripted world position (tests, benchmarks and probes, as
    /// `set_scripted_pointer`): the cursor exactly, neither picked nor snapped.
    #[func]
    pub fn road_tool_pointer_at(&mut self, position: Vector3) -> Variant {
        self.road_tool_handle(RoadToolEvent::Pointer {
            pointer: Pointer::Scripted(position),
            shift: false,
        });
        self.road_tool_cursor_variant()
    }

    /// One road tool event by name: `press` (left button: places a point; the stroke's last one
    /// sends it), `right` (right button), `release`, `cancel`, `confirm`, `mode_straight`,
    /// `mode_simple_curve`, `mode_complex_curve`, and the commit acknowledgement `committed` or
    /// `refused` (what `get_road_commit_result` said of the sent stroke; after a refusal the
    /// unchanged stroke on the same context stays refused, `road_commit_rejected`), and
    /// `forget_preview`
    /// (the bridge drops its standing preview, so the next frame previews the stroke afresh:
    /// measurements and another world). Returns `phase` (as
    /// `road_tool_frame` reports it, after the event), and for a click that sent the stroke:
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
        result.set("phase", self.road_tool_phase().0);
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
    /// 1 placing, 2 waiting for the commit's acknowledgement), `placed` (points down), `cursor`
    /// and `cursor_valid`, `snap` (what took the cursor: `free`, `node`, `edge`, `guide`,
    /// `guide_corner`, `grid`, `angle`, `angle_edge`, `angle_guide`, `angle_grid`, `border`,
    /// `self`), `points` (the stroke of this pose, empty without one), `refusal` (why the draft
    /// has no stroke: `too_short`, `fold_back`, `full_turn`, `no_biarc`, else ""), `hud`
    /// (`length_m`, `bearing_deg`, `start_angle_deg` and `end_angle_deg` (-1 for none),
    /// `build_cost` and `upkeep_per_day` of the drawn road type). When the stroke, its type or
    /// the context changed since the last preview, the frame previews it: `preview` holds
    /// `preview_road`'s dictionary; `preview_cleared` is true when the stroke went away. No
    /// preview while a sent stroke waits for its acknowledgement.
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
                "build_cost": hud.build_cost,
                "upkeep_per_day": hud.upkeep_per_day,
            },
        );
        dict
    }

    // The phase code (0 idle, 1 placing, 2 waiting for the acknowledgement) and points placed.
    fn road_tool_phase(&self) -> (i64, i64) {
        match self.road_tool_session.session.frame().state.phase {
            Phase::Idle => (0, 0),
            Phase::Placing { placed } => (1, i64::from(placed)),
            Phase::AwaitingAck => (2, 0),
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
    }
}
