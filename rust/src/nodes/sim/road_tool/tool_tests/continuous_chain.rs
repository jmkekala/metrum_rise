// SPDX-License-Identifier: GPL-2.0-only

//! Round CT (`ROAD-112`) tests: with Continuous on a chain carries its direction into the
//! S-curve (its start aim: that step is skipped) and freeform (its start's handle direction),
//! across mode changes made while only the chained start is placed; every join is
//! tangent-continuous on the guide paths. Continuous off sends and shows what `dev` `8855c25f`
//! did (a frame digest recorded there).

use super::{ack, click, ground_at, move_to, ray, session, view};
use crate::nodes::sim::road_tool::frame::{GizmoHandleKind, Phase, RoadToolFrame};
use crate::nodes::sim::road_tool::labels::{HintStep, LabelKind};
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::SnapKind;
use crate::simulation::network::guide::DVec2;
use godot::prelude::Vector3;

// An exact (scripted) pointer on the ground at `(x, z)`.
fn at(x: f32, z: f32) -> Pointer {
    Pointer::Scripted(Vector3::new(x, ground_at(x, z), z))
}

fn act(tool: &mut RoadToolSession, action: RoadToolAction) {
    tool.handle_event(view(), RoadToolEvent::Action(action));
}

fn continuous() -> RoadToolSession {
    let mut tool = session();
    tool.set_option(RoadToolOption::Continuous(true));
    tool
}

// The unsigned angle between two directions, in radians.
fn heading_gap(a: DVec2, b: DVec2) -> f64 {
    a.perp_dot(b).atan2(a.dot(b)).abs()
}

fn hint_steps(frame: &RoadToolFrame) -> Vec<HintStep> {
    frame
        .labels
        .iter()
        .filter_map(|label| match label.kind {
            LabelKind::Hint(step) => Some(step),
            _ => None,
        })
        .collect()
}

// Sends the pose's stroke with the click (or `confirm`) that ends it, accepts it, and returns the
// sent draft's end tangent and its last point (where the chain starts).
fn build(tool: &mut RoadToolSession, end: Option<Pointer>) -> (DVec2, Vector3) {
    match end {
        Some(pointer) => click(tool, pointer),
        None => act(tool, RoadToolAction::Confirm),
    }
    let sent = tool.take_commit().expect("the stroke is sent");
    let tangent = tool.path().end_tangent().expect("a sent draft");
    ack(tool, true);
    (tangent, *sent.points.last().expect("two points"))
}

// The draft leaves the chain's start along the carried tangent: its path's start tangent within
// 1e-9 rad, its stroke from the start bit for bit, no refusal.
fn assert_joins(tool: &RoadToolSession, (tangent, end): (DVec2, Vector3), what: &str) {
    let frame = tool.frame();
    assert_eq!(frame.state.refusal, None, "{what}");
    assert_eq!(
        frame.stroke.points[0], end,
        "{what} starts at the chain's end"
    );
    let start = tool.path().start_tangent().expect("a draft");
    let gap = heading_gap(tangent, start);
    assert!(gap < 1e-9, "{what}: heading gap {gap:e} rad");
}

// Places freeform `nodes` and confirms the placing (the edit phase).
fn freeform_nodes(tool: &mut RoadToolSession, nodes: &[(f32, f32)]) {
    for &(x, z) in nodes {
        click(tool, at(x, z));
    }
    act(tool, RoadToolAction::Confirm);
    assert_eq!(tool.frame().state.phase, Phase::Editing);
}

// S-curve after S-curve: the second starts at the first's end with its start direction set
// (its hint is the end direction's), previews tangent from the first pose and, after its end aim
// and end, is sent tangent-continuous.
#[test]
fn chained_s_curves_carry_the_start_direction() {
    let mut tool = continuous();
    act(&mut tool, RoadToolAction::ModeComplexCurve);
    for (x, z) in [(-285.0, 150.0), (-265.0, 150.0), (-235.0, 185.0)] {
        click(&mut tool, at(x, z));
    }
    let join = build(&mut tool, Some(at(-215.0, 190.0)));
    move_to(&mut tool, at(-190.0, 215.0));
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 2 });
    assert_eq!(hint_steps(tool.frame()), [HintStep::EndDirection]);
    assert_joins(&tool, join, "before the end aim");
    click(&mut tool, at(-180.0, 250.0));
    assert_eq!(hint_steps(tool.frame()), [HintStep::PlaceEnd]);
    move_to(&mut tool, at(-160.0, 255.0));
    assert_eq!(tool.path().segments().len(), 2, "the biarc");
    assert_joins(&tool, join, "the S-curve");
    let second = build(&mut tool, Some(at(-160.0, 255.0)));
    // A third S-curve chains on as well.
    move_to(&mut tool, at(-150.0, 275.0));
    assert_joins(&tool, second, "the third");
}

// Freeform after freeform: the next draft's first span leaves along the carried tangent, its
// start handle keeps that direction when dragged (length free), Ctrl does not break it.
#[test]
fn chained_freeform_drafts_carry_the_start_direction() {
    let mut tool = continuous();
    act(&mut tool, RoadToolAction::ModeFreeform);
    click(&mut tool, at(-285.0, 140.0));
    freeform_nodes(&mut tool, &[(-255.0, 160.0), (-225.0, 150.0)]);
    let join = build(&mut tool, None);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
    move_to(&mut tool, at(-195.0, 135.0));
    assert_joins(&tool, join, "the live span");
    freeform_nodes(&mut tool, &[(-195.0, 135.0), (-165.0, 130.0)]);
    assert_joins(&tool, join, "the edit phase");
    // The start's handle (node 0 has no inward one: the first `Handle` gizmo) dragged further
    // along and sideways, without and with Ctrl: it moves, its direction stays.
    let start_handle = |tool: &RoadToolSession| {
        let gizmos = &tool.frame().gizmos;
        let index = gizmos
            .handle_kinds
            .iter()
            .position(|kind| *kind == GizmoHandleKind::Handle);
        gizmos.handles[index.expect("the start's handle")]
    };
    let (tangent, _) = join;
    for (step, ctrl) in [(1.0, false), (2.0, true)] {
        let grip = start_handle(&tool);
        move_to(&mut tool, Pointer::Scripted(grip));
        tool.handle_event(view(), RoadToolEvent::Press(PointerButton::Left));
        assert!(tool.take_commit().is_none(), "the press grabs the handle");
        let to = DVec2::new(f64::from(grip.x), f64::from(grip.z))
            + tangent * (6.0 * step)
            + tangent.perp() * 7.0;
        tool.handle_event(
            view(),
            RoadToolEvent::Pointer {
                pointer: at(to.x as f32, to.y as f32),
                shift: false,
                ctrl,
            },
        );
        tool.handle_event(view(), RoadToolEvent::Release(PointerButton::Left));
        assert_ne!(
            start_handle(&tool),
            grip,
            "the handle's length follows the drag"
        );
        assert_joins(&tool, join, if ctrl { "a Ctrl drag" } else { "a drag" });
    }
    build(&mut tool, None);
}

// Mixed chain: straight -> S-curve -> freeform -> simple curve, each mode chosen while only the
// chained start is placed; every join tangent-continuous.
#[test]
fn mixed_chain_is_tangent_continuous() {
    let mut tool = continuous();
    click(&mut tool, at(-285.0, 110.0));
    let join = build(&mut tool, Some(at(-260.0, 115.0)));

    act(&mut tool, RoadToolAction::ModeComplexCurve);
    move_to(&mut tool, at(-240.0, 135.0));
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 2 });
    assert_eq!(
        tool.frame().gizmos.handle_kinds[0],
        GizmoHandleKind::ChainStart
    );
    assert_joins(&tool, join, "straight -> S-curve preview");
    click(&mut tool, at(-225.0, 170.0));
    move_to(&mut tool, at(-205.0, 175.0));
    assert_joins(&tool, join, "straight -> S-curve");
    let join = build(&mut tool, Some(at(-205.0, 175.0)));

    act(&mut tool, RoadToolAction::ModeFreeform);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
    freeform_nodes(&mut tool, &[(-185.0, 200.0), (-170.0, 230.0)]);
    assert_joins(&tool, join, "S-curve -> freeform");
    let join = build(&mut tool, None);

    act(&mut tool, RoadToolAction::ModeSimpleCurve);
    click(&mut tool, at(-180.0, 260.0));
    move_to(&mut tool, at(-200.0, 280.0));
    assert_joins(&tool, join, "freeform -> curve");
    build(&mut tool, Some(at(-200.0, 280.0)));
}

// Round CT2: only a chained start carries a direction. With Continuous on, Straight and the
// simple curve started by a first click on an existing road mid-span, on the junction node or on
// the spur's dead end (not the chain's end) aim freely: the stroke to a cursor 90 degrees off the
// road (45 off the junction's arms) is previewed toward it, not held along the road nor refused.
#[test]
fn first_clicks_on_the_network_aim_freely() {
    let half = std::f64::consts::FRAC_1_SQRT_2;
    // (start, the road's direction there, the aim off it).
    let starts = [
        ((-150.0, 0.0), DVec2::X, DVec2::Y),
        ((0.0, 0.0), DVec2::new(half, -half), DVec2::new(half, half)),
        ((-40.0, 200.0), DVec2::new(half, half), DVec2::new(-half, half)),
    ];
    let toward = |tool: &RoadToolSession, to: Vector3, what: &str| {
        let frame = tool.frame();
        assert_eq!(frame.state.refusal, None, "{what}");
        let start = frame.stroke.points[0];
        let aim = DVec2::new(f64::from(to.x - start.x), f64::from(to.z - start.z));
        let leaves = tool.path().start_tangent().expect("a draft");
        let gap = heading_gap(aim.normalize(), leaves);
        assert!(gap < 1e-6, "{what}: leaves {gap:e} rad off its aim");
    };
    for ((x, z), road, aim) in starts {
        let start = DVec2::new(f64::from(x), f64::from(z));
        let point = |p: DVec2| at(p.x as f32, p.y as f32);
        let control = start + aim * 50.0;
        let end = start + aim * 90.0 + road * 40.0;
        for mode in [RoadToolAction::ModeStraight, RoadToolAction::ModeSimpleCurve] {
            let what = format!("{mode:?} from ({x}, {z})");
            let mut tool = continuous();
            act(&mut tool, mode);
            // A camera ray: the start takes the network snap (a scripted pointer is free).
            click(&mut tool, ray(x, z));
            assert!(
                matches!(tool.frame().cursor.snap, SnapKind::Edge | SnapKind::Node),
                "{what} starts on the network"
            );
            move_to(&mut tool, point(control));
            toward(&tool, tool.frame().cursor.position, &what);
            if mode == RoadToolAction::ModeSimpleCurve {
                click(&mut tool, point(control));
                let placed = tool.frame().cursor.position;
                move_to(&mut tool, point(end));
                toward(&tool, placed, &format!("{what}, its control"));
            }
        }
    }
}

// A cancel at the chained S-curve's or freeform draft's every step leaves nothing; a mode change
// once more than the chained start is placed drops the stroke and ends the chain (as before).
#[test]
fn chained_cancel_and_mode_change_leave_nothing() {
    let start = |action| {
        let mut tool = continuous();
        act(&mut tool, action);
        click(&mut tool, at(-285.0, 150.0));
        if action == RoadToolAction::ModeFreeform {
            freeform_nodes(&mut tool, &[(-255.0, 160.0)]);
            build(&mut tool, None);
        } else {
            for (x, z) in [(-265.0, 150.0), (-235.0, 185.0)] {
                click(&mut tool, at(x, z));
            }
            build(&mut tool, Some(at(-215.0, 190.0)));
        }
        tool
    };
    // The chained S-curve's end aim is its only click before the end.
    let nodes = [(-180.0, 240.0), (-160.0, 230.0)];
    for (action, most) in [
        (RoadToolAction::ModeComplexCurve, 1),
        (RoadToolAction::ModeFreeform, 2),
    ] {
        for clicks in (0..=most).map(|count| &nodes[..count]) {
            for ending in [RoadToolAction::Cancel, RoadToolAction::ModeStraight] {
                let mut tool = start(action);
                for &(x, z) in clicks {
                    click(&mut tool, at(x, z));
                }
                move_to(&mut tool, at(-150.0, 260.0));
                act(&mut tool, ending);
                move_to(&mut tool, at(-140.0, 265.0));
                let frame = tool.frame();
                let kept = ending == RoadToolAction::ModeStraight && clicks.is_empty();
                let phase = if kept {
                    Phase::Placing { placed: 1 }
                } else {
                    Phase::Idle
                };
                assert_eq!(frame.state.phase, phase, "{action:?} {clicks:?} {ending:?}");
                if !kept {
                    assert!(frame.stroke.points.is_empty() && frame.labels.is_empty());
                }
                assert!(tool.take_commit().is_none());
            }
        }
    }
}

// FNV-1a over the frames' Debug strings (every float printed exactly).
fn frames_digest(frames: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in frames.iter().flat_map(|frame| frame.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

// Runs `script` (a mode action or a click with a pointer move beside it first; the sent
// stroke accepted) and records every frame.
fn run_script(tool: &mut RoadToolSession, script: &[Step], frames: &mut Vec<String>) {
    for step in script {
        match *step {
            Step::Act(action) => act(tool, action),
            Step::Click(x, z) => {
                move_to(tool, at(x - 3.0, z + 4.0));
                frames.push(format!("{:?}", tool.frame()));
                click(tool, at(x, z));
            }
        }
        frames.push(format!("{:?}", tool.frame()));
        if tool.take_commit().is_some() {
            ack(tool, true);
            frames.push(format!("{:?}", tool.frame()));
        }
    }
}

#[derive(Clone, Copy)]
enum Step {
    Act(RoadToolAction),
    Click(f32, f32),
}

// Continuous off: chains in every mode, mode changes at a chained start, freeform placing and
// cancels give the frames `dev` `8855c25f` gave (digest recorded there); Continuous on, a
// straight chain likewise (the simple curve's now leaves along the tangent exactly). Re-recorded
// by round UI1 (the round's changes off, the old digest held; its drawing changes alone off, the
// 4 m reach and the square lock on, it held too): the chained starts gain their corner arcs and
// faint guides and the angle chips move to the arcs; no snapped point changed. Re-recorded by the
// icon UI merge (round U1): the frame's road type prints the type's pockets (`ROAD-128`), none
// here; with each frame's `pockets` field cut out, the old digest `0x59a9_8956_2034_85fa` held.
#[test]
fn continuous_off_and_straight_curve_chains_are_unchanged() {
    use RoadToolAction::{Cancel, Confirm, ModeComplexCurve, ModeFreeform, ModeSimpleCurve};
    use Step::{Act, Click};
    let off = [
        Click(-285.0, 110.0),
        Click(-260.0, 115.0),
        Act(ModeComplexCurve),
        Click(-240.0, 135.0),
        Click(-225.0, 150.0),
        Click(-205.0, 175.0),
        Click(-185.0, 180.0),
        Click(-170.0, 200.0),
        Act(ModeFreeform),
        Click(-160.0, 220.0),
        Click(-150.0, 240.0),
        Click(-170.0, 250.0),
        Act(Confirm),
        Act(Confirm),
        Click(-190.0, 265.0),
        Click(-210.0, 275.0),
        Act(Confirm),
        Act(Confirm),
        Act(ModeSimpleCurve),
        Click(-230.0, 285.0),
        Click(-250.0, 280.0),
        Click(-270.0, 260.0),
        Act(Cancel),
    ];
    let on = [
        Act(Cancel),
        Act(RoadToolAction::ModeStraight),
        Click(-285.0, 110.0),
        Click(-260.0, 115.0),
        Click(-240.0, 135.0),
        Click(-215.0, 150.0),
        Click(-190.0, 170.0),
        Act(Cancel),
    ];
    let mut frames = Vec::new();
    let mut tool = session();
    run_script(&mut tool, &off, &mut frames);
    tool.set_option(RoadToolOption::Continuous(true));
    run_script(&mut tool, &on, &mut frames);
    assert_eq!(
        (frames.len(), frames_digest(&frames)),
        (58, 0x53d2_088c_9ddb_29fd)
    );
}
