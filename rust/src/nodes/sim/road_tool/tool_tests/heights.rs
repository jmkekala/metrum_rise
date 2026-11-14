// SPDX-License-Identifier: GPL-2.0-only

//! Round I (`ROAD-112`) tests: the height handles (a grip's drag projects the pointer ray onto
//! its vertical line and steps by 0.5 m, Shift frees it; PageUp/PageDown give the same offsets
//! as a drag; a locked end does not move), offsets 0 keep today's stroke bit for bit, a graded
//! draft segments on the frame (`update_draft`) with its runs, and the HUD's heights and slopes.
//! The neighbourhood's ground is flat.

use super::{ack, click, ground_at, move_to, ray, session, view};
use crate::nodes::sim::road_tool::frame::GizmoHandleKind;
use crate::nodes::sim::road_tool::height::handles::{HANDLE_STEM_M, HEIGHT_STEP_M};
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::simulation::network::road_edit::RoadProfile;
use godot::prelude::Vector3;

// A free stretch of the neighbourhood: no road within 50 m.
const START: (f32, f32) = (-280.0, 250.0);
const END: (f32, f32) = (-180.0, 250.0);

// An exact (scripted) pointer on the ground at `(x, z)`.
fn at((x, z): (f32, f32)) -> Pointer {
    Pointer::Scripted(ground((x, z)))
}

fn ground((x, z): (f32, f32)) -> Vector3 {
    Vector3::new(x, ground_at(x, z), z)
}

// A camera ray from 50 m south of and 50 m above `target`, through it.
fn aim(target: Vector3) -> Pointer {
    let back = Vector3::new(0.0, 50.0, 50.0);
    Pointer::Ray {
        origin: target + back,
        dir: -back,
    }
}

fn event(tool: &mut RoadToolSession, event: RoadToolEvent) {
    tool.handle_event(view(), event);
}

fn key(tool: &mut RoadToolSession, action: RoadToolAction, presses: usize) {
    for _ in 0..presses {
        event(tool, RoadToolEvent::Action(action));
    }
}

fn shifted(tool: &mut RoadToolSession, pointer: Pointer) {
    event(
        tool,
        RoadToolEvent::Pointer {
            pointer,
            shift: true,
            ctrl: false,
        },
    );
}

// Grabs `end`'s grip with a camera ray through it, drags it to `rise_m` above its height at the
// press, Shift for a free drag, and lets go.
fn drag_grip(tool: &mut RoadToolSession, end: usize, rise_m: f32, free: bool) {
    let handle = tool.frame().height_handles[end].expect("a handle");
    move_to(tool, aim(handle.grip));
    event(tool, RoadToolEvent::Press(PointerButton::Left));
    let to = aim(handle.grip + Vector3::new(0.0, rise_m, 0.0));
    if free {
        shifted(tool, to);
    } else {
        move_to(tool, to);
    }
    event(tool, RoadToolEvent::Release(PointerButton::Left));
}

// A straight draft from START with its live end at END.
fn draft() -> RoadToolSession {
    let mut tool = session();
    click(&mut tool, at(START));
    move_to(&mut tool, at(END));
    tool
}

// A freeform draft START, a bend, END in its edit phase.
fn freeform() -> RoadToolSession {
    let mut tool = session();
    event(
        &mut tool,
        RoadToolEvent::Action(RoadToolAction::ModeFreeform),
    );
    for point in [START, (-230.0, 280.0), END] {
        click(&mut tool, at(point));
    }
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    tool
}

// A press on the start's grip grabs it (nothing placed, nothing sent); the drag rounds the
// offset to 0.5 m, Shift leaves it free; after the release the draft is the grade line from the
// raised start to the end.
#[test]
fn a_grip_drag_projects_the_ray_and_steps_by_half_metres() {
    let mut tool = draft();
    let start = ground(START);
    let handle = tool.frame().height_handles[0].unwrap();
    assert_eq!(handle.grip, start + Vector3::new(0.0, HANDLE_STEM_M, 0.0));
    assert!(!handle.locked && handle.offset_m == 0.0);
    assert_eq!(tool.frame().stroke.grade, None);
    let grips = |tool: &RoadToolSession| {
        let kinds = &tool.frame().gizmos.handle_kinds;
        kinds
            .iter()
            .filter(|kind| **kind == GizmoHandleKind::HeightGrip)
            .count()
    };
    assert_eq!(grips(&tool), 1, "the free start's grip");

    move_to(&mut tool, aim(handle.grip));
    event(&mut tool, RoadToolEvent::Press(PointerButton::Left));
    assert_eq!(tool.placed_count(), 1, "the press grabs, places nothing");
    assert!(tool.frame().commit.is_none());
    move_to(&mut tool, aim(handle.grip + Vector3::new(0.0, 2.3, 0.0)));
    assert_eq!(tool.frame().height_handles[0].unwrap().offset_m, 2.5);
    event(&mut tool, RoadToolEvent::Release(PointerButton::Left));
    move_to(&mut tool, at(END));
    let frame = tool.frame();
    let raised = start + Vector3::new(0.0, 2.5, 0.0);
    assert_eq!(frame.stroke.points, [raised, ground(END)]);
    assert_eq!(frame.stroke.grade, Some([raised.y, ground(END).y]));

    drag_grip(&mut tool, 0, -2.3, true);
    move_to(&mut tool, at(END));
    let offset = tool.frame().height_handles[0].unwrap().offset_m;
    assert!((offset - 0.2).abs() < 1e-4, "free: {offset}");
}

// PageUp/PageDown move the active handle by the drag's step: three presses on a freeform draft's
// end in its edit phase make the draft a drag of its grip to 1.5 m makes, bit for bit.
#[test]
fn keys_and_drags_give_identical_drafts() {
    let mut dragged = freeform();
    drag_grip(&mut dragged, 1, 1.4, false);
    let mut keyed = freeform();
    key(&mut keyed, RoadToolAction::RaiseHandle, 3);
    let (a, b) = (dragged.frame(), keyed.frame());
    assert_eq!(a.height_handles[1].unwrap().offset_m, 3.0 * HEIGHT_STEP_M);
    assert_eq!(a.stroke, b.stroke);
    assert_eq!(
        a.height_handles[1].unwrap().offset_m,
        b.height_handles[1].unwrap().offset_m
    );
    assert!(a.stroke.grade.is_some());
    // While drawing, the keys move the live end; a new start takes its offset.
    let mut tool = session();
    move_to(&mut tool, at(START));
    key(&mut tool, RoadToolAction::RaiseHandle, 4);
    assert_eq!(tool.frame().height_handles[1].unwrap().offset_m, 2.0);
    click(&mut tool, at(START));
    move_to(&mut tool, at(END));
    let frame = tool.frame();
    assert_eq!(
        frame.stroke.grade,
        Some([ground(START).y + 2.0, ground(END).y + 2.0])
    );
}

// Handles back at their supports and straight grade off: today's two-point stroke, bit for bit,
// with no grade line.
#[test]
fn offsets_back_at_zero_send_todays_stroke_bitwise() {
    let today = draft();
    let mut moved = draft();
    key(&mut moved, RoadToolAction::RaiseHandle, 3);
    drag_grip(&mut moved, 0, 1.0, false);
    assert!(moved.frame().stroke.grade.is_some());
    key(&mut moved, RoadToolAction::LowerHandle, 3);
    drag_grip(&mut moved, 0, -1.0, false);
    move_to(&mut moved, at(END));
    assert_eq!(today.frame().stroke, moved.frame().stroke);
    assert_eq!(today.frame().stroke.points, [ground(START), ground(END)]);
    assert_eq!(moved.frame().stroke.grade, None);
    // Straight grade alone grades the same points.
    moved.set_option(RoadToolOption::StraightGrade(true));
    move_to(&mut moved, at(END));
    let frame = moved.frame();
    assert_eq!(frame.stroke.points, today.frame().stroke.points);
    assert_eq!(frame.stroke.grade, Some([ground(START).y, ground(END).y]));
}

// An end on a road is locked to its graph height: no grip, the keys do not move it, and the
// offset they would have set never appears when the end leaves the road.
#[test]
fn a_locked_end_cannot_move() {
    let mut tool = session();
    click(&mut tool, ray(0.0, 0.0));
    move_to(&mut tool, at((-60.0, 250.0)));
    let start = tool.frame().height_handles[0].unwrap();
    assert!(start.locked, "the junction locks the start");
    let kinds = &tool.frame().gizmos.handle_kinds;
    assert!(!kinds.contains(&GizmoHandleKind::HeightGrip));
    // The keys move the live end, never the locked start.
    key(&mut tool, RoadToolAction::RaiseHandle, 2);
    assert_eq!(tool.frame().height_handles[0].unwrap(), start);

    let mut tool = draft();
    move_to(&mut tool, ray(-120.0, 120.0));
    let end = tool.frame().height_handles[1].unwrap();
    assert!(end.locked, "a dead end locks the live end");
    key(&mut tool, RoadToolAction::RaiseHandle, 2);
    assert_eq!(
        tool.frame().height_handles[1].unwrap().height_m,
        end.height_m
    );
    move_to(&mut tool, at(END));
    assert_eq!(tool.frame().height_handles[1].unwrap().offset_m, 0.0);
    assert_eq!(tool.frame().stroke.grade, None);
}

// A raised draft over flat ground is one bridge; a start raised 5 m and an end lowered 8 m make
// a bridge to the crossing and a tunnel from it (plan section 4's table), reported per run with
// the grade line's slope; the frame's report stands while the pose does.
#[test]
fn a_graded_draft_segments_on_the_frame_with_its_runs_and_slopes() {
    let mut tool = draft();
    drag_grip(&mut tool, 0, 5.0, false);
    key(&mut tool, RoadToolAction::RaiseHandle, 10);
    move_to(&mut tool, at(END));
    assert!(tool.update_draft(view()));
    assert!(
        !tool.update_draft(view()),
        "an unchanged pose keeps its report"
    );
    let report = &tool.frame().draft;
    assert_eq!(report.refusal, None);
    assert_eq!(report.strokes.len(), 1);
    assert_eq!(report.runs[0].profile, RoadProfile::Bridge);
    assert_eq!(report.runs[0].slope_percent, 0.0);
    assert_eq!(tool.frame().hud.start_height_m, Some(5.0));
    assert_eq!(tool.frame().hud.end_height_m, Some(5.0));

    key(&mut tool, RoadToolAction::LowerHandle, 26);
    move_to(&mut tool, at(END));
    assert!(tool.update_draft(view()));
    let frame = tool.frame();
    let profiles: Vec<_> = frame.draft.runs.iter().map(|run| run.profile).collect();
    assert_eq!(profiles, [RoadProfile::Bridge, RoadProfile::Tunnel]);
    assert_eq!(frame.hud.end_height_m, Some(-8.0));
    let slope = frame.hud.slope_percent.unwrap();
    assert!((slope + 13.0).abs() < 1e-3, "{slope}");
    assert_eq!(frame.piece_slopes_percent.len(), 1);
    assert!((frame.piece_slopes_percent[0] - slope).abs() < 1e-9);
    let length: f64 = frame.draft.runs.iter().map(|run| run.length_m).sum();
    assert!((length - 100.0).abs() < 1e-3, "{length}");
    for run in &frame.draft.runs {
        assert!((run.slope_percent + 13.0).abs() < 1e-3);
    }
    // The bridge ends where the grade line crosses the ground: 5 / 13 of the way.
    assert!((frame.draft.runs[0].length_m - 500.0 / 13.0).abs() < 0.5);

    // A follow-ground draft is one run of its own.
    let mut plain = draft();
    assert!(plain.update_draft(view()));
    let runs = &plain.frame().draft.runs;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].profile, RoadProfile::FollowGround);
    assert!(plain.frame().draft.strokes.is_empty());
}

// An accepted raised stroke chains from its end, locked at the raised height; the live end keeps
// its offset, so the chain stays up.
#[test]
fn the_chain_starts_locked_at_the_raised_end() {
    let mut tool = draft();
    key(&mut tool, RoadToolAction::RaiseHandle, 10);
    click(&mut tool, at(END));
    let sent = tool.take_commit().expect("the stroke is sent");
    assert_eq!(sent.grade, Some([ground(START).y, ground(END).y + 5.0]));
    ack(&mut tool, true);
    move_to(&mut tool, at((-120.0, 300.0)));
    let start = tool.frame().height_handles[0].unwrap();
    assert!(start.locked);
    assert_eq!(start.height_m, ground(END).y + 5.0);
    assert_eq!(tool.frame().height_handles[1].unwrap().offset_m, 5.0);
}
