// SPDX-License-Identifier: GPL-2.0-only

//! Round SC (`ROAD-112`) tests: the S-curve previews its road from the first click (Straight's
//! request, then the simple curve's), each curve step shows its hint chip, a cancel leaves
//! nothing, and the finished S-curve's sent stroke is round N's bit for bit.

use super::{click, ground_at, move_to, ray, session};
use crate::nodes::sim::road_tool::frame::{Phase, RoadToolFrame, StrokeRequest};
use crate::nodes::sim::road_tool::labels::{HintStep, LabelKind};
use crate::nodes::sim::road_tool::session::{
    Pointer, RoadToolAction, RoadToolEvent, RoadToolSession,
};
use crate::nodes::sim::road_tool::wording::{NoticeFacts, label_text};
use godot::prelude::Vector3;

fn mode(tool: &mut RoadToolSession, action: RoadToolAction) {
    tool.handle_event(super::view(), RoadToolEvent::Action(action));
}

// An exact (scripted) pointer on the ground at `(x, z)`.
fn at(x: f32, z: f32) -> Pointer {
    Pointer::Scripted(Vector3::new(x, ground_at(x, z), z))
}

// A session in `action`'s mode with `clicks` placed and the pointer at `live`.
fn placed(action: RoadToolAction, clicks: &[(f32, f32)], live: (f32, f32)) -> RoadToolSession {
    let mut tool = session();
    mode(&mut tool, action);
    for &(x, z) in clicks {
        click(&mut tool, at(x, z));
    }
    move_to(&mut tool, at(live.0, live.1));
    tool
}

// The frame's hint chips: each one's step and its text as shown.
fn hints(frame: &RoadToolFrame) -> Vec<(HintStep, String)> {
    let notice = NoticeFacts::default();
    frame
        .labels
        .iter()
        .filter_map(|label| match label.kind {
            LabelKind::Hint(step) => {
                let mut text = String::new();
                label_text(label, &notice, &mut text);
                Some((step, text))
            }
            _ => None,
        })
        .collect()
}

// After the first click the S-curve's frame carries the preview request Straight mode carries
// for the same start and cursor; after the second, the simple curve's for the same three points.
#[test]
fn s_curve_previews_from_the_first_click() {
    let (start, start_aim, end_aim) = ((-280.0, 150.0), (-250.0, 150.0), (-205.0, 185.0));
    let s_curve = placed(RoadToolAction::ModeComplexCurve, &[start], start_aim);
    let straight = placed(RoadToolAction::ModeStraight, &[start], start_aim);
    let frame = s_curve.frame();
    assert_eq!(frame.state.phase, Phase::Placing { placed: 1 });
    assert_eq!(frame.state.refusal, None);
    assert_eq!(frame.stroke.points.len(), 2);
    assert_eq!(frame.stroke, straight.frame().stroke);
    assert_eq!(frame.hud, straight.frame().hud);

    let s_curve = placed(
        RoadToolAction::ModeComplexCurve,
        &[start, start_aim],
        end_aim,
    );
    let curve = placed(
        RoadToolAction::ModeSimpleCurve,
        &[start, start_aim],
        end_aim,
    );
    let frame = s_curve.frame();
    assert_eq!(frame.state.phase, Phase::Placing { placed: 2 });
    assert_eq!(frame.state.refusal, None);
    assert!(frame.stroke.points.len() > 3, "the fillet is flattened");
    assert_eq!(frame.stroke, curve.frame().stroke);
    assert_eq!(frame.hud, curve.frame().hud);
}

// One hint chip a step, worded in Rust: the S-curve's three steps, the simple curve's two, none
// for a straight stroke or before the start.
#[test]
fn curve_steps_show_their_hint() {
    let clicks = [(-280.0, 150.0), (-250.0, 150.0), (-205.0, 185.0)];
    let live = (-180.0, 200.0);
    for (action, steps) in [
        (
            RoadToolAction::ModeComplexCurve,
            &[
                HintStep::StartDirection,
                HintStep::EndDirection,
                HintStep::PlaceEnd,
            ][..],
        ),
        (
            RoadToolAction::ModeSimpleCurve,
            &[HintStep::ControlPoint, HintStep::PlaceEnd][..],
        ),
        (RoadToolAction::ModeStraight, &[][..]),
    ] {
        assert!(hints(placed(action, &[], live).frame()).is_empty());
        for (count, step) in steps.iter().enumerate() {
            let tool = placed(action, &clicks[..=count], live);
            let shown = hints(tool.frame());
            assert_eq!(shown.len(), 1, "{action:?} after {} clicks", count + 1);
            assert_eq!(shown[0].0, *step);
            assert!(
                shown[0].1.starts_with("Click to ") && shown[0].1.ends_with("Esc to cancel"),
                "{}",
                shown[0].1
            );
        }
        let after = placed(action, &clicks[..1], live);
        assert_eq!(hints(after.frame()).len(), usize::from(!steps.is_empty()));
    }
    let tool = placed(RoadToolAction::ModeComplexCurve, &clicks[..1], live);
    assert_eq!(
        hints(tool.frame())[0].1,
        "Click to set the start direction · Esc to cancel"
    );
}

// A cancel at any step drops the preview, the chips and the stroke: nothing is sent.
#[test]
fn cancel_leaves_nothing() {
    let clicks = [(-280.0, 150.0), (-250.0, 150.0), (-205.0, 185.0)];
    for count in 1..=3 {
        let mut tool = placed(
            RoadToolAction::ModeComplexCurve,
            &clicks[..count],
            (-180.0, 200.0),
        );
        assert!(!tool.frame().stroke.points.is_empty());
        mode(&mut tool, RoadToolAction::Cancel);
        move_to(&mut tool, at(-175.0, 205.0));
        let frame = tool.frame();
        assert_eq!(frame.state.phase, Phase::Idle);
        assert!(frame.stroke.points.is_empty() && frame.labels.is_empty());
        assert!(tool.take_commit().is_none());
    }
}

// FNV-1a over the stroke's point count, every coordinate's bits, its grade and snap flag.
fn digest(stroke: &StrokeRequest) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut eat = |word: u32| {
        for byte in word.to_le_bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    eat(stroke.points.len() as u32);
    for point in &stroke.points {
        eat(point.x.to_bits());
        eat(point.y.to_bits());
        eat(point.z.to_bits());
    }
    for height in stroke.grade.unwrap_or([f32::NAN; 2]) {
        eat(height.to_bits());
    }
    eat(u32::from(stroke.snap_to_existing_roads));
    hash
}

// The S-curve placed with pointer moves between its clicks (each move a preview pose since round
// SC) sends the stroke `dev` `c87eb234` sent for the same input (digest recorded there).
#[test]
fn finished_s_curve_is_unchanged() {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeComplexCurve);
    click(&mut tool, ray(-280.0, 150.0));
    move_to(&mut tool, ray(-262.0, 153.0));
    click(&mut tool, ray(-260.0, 150.0));
    move_to(&mut tool, ray(-216.0, 183.0));
    click(&mut tool, ray(-210.0, 190.0));
    move_to(&mut tool, ray(-190.0, 197.0));
    click(&mut tool, ray(-180.0, 200.0));
    let sent = tool.take_commit().expect("the end click sends the S-curve");
    assert!(sent.points.len() > 3);
    assert_eq!(
        digest(&sent),
        0xea43_b765_2ee6_bcb0,
        "{} points",
        sent.points.len()
    );
}
