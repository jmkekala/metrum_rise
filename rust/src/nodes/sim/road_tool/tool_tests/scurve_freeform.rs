// SPDX-License-Identifier: GPL-2.0-only

//! Round N (`ROAD-112`) tests: the S-curve's gizmos and its two radii with the tight-curve
//! verdict on the tighter; freeform placing, the edit phase (handle edits keep joints collinear
//! unless broken, nodes carry their handles), the committed polyline being the flattening rule's,
//! cancel leaving nothing, the chain, and determinism of both modes.

use super::{ack, click, ground_at, move_to, ray, session, view};
use crate::nodes::sim::road_tool::frame::{
    DraftRefusal, GizmoHandleKind, GizmoLineClass, Gizmos, Phase,
};
use crate::nodes::sim::road_tool::measure::design_speed_kmh;
use crate::nodes::sim::road_tool::modes::freeform::GRIP_REACH_M;
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::SnapKind;
use crate::simulation::network::guide::{
    DVec2, FLATTEN_MAX_SAG_M, FLATTEN_MAX_STEP_M, GuidePath, GuideSegment,
};
use godot::prelude::Vector3;

// An exact (scripted) pointer on the ground at `(x, z)`.
fn at(x: f32, z: f32) -> Pointer {
    Pointer::Scripted(Vector3::new(x, ground_at(x, z), z))
}

fn event(tool: &mut RoadToolSession, event: RoadToolEvent) {
    tool.handle_event(view(), event);
}

fn mode(tool: &mut RoadToolSession, action: RoadToolAction) {
    event(tool, RoadToolEvent::Action(action));
}

// A pointer move with Ctrl held.
fn move_ctrl(tool: &mut RoadToolSession, pointer: Pointer) {
    event(
        tool,
        RoadToolEvent::Pointer {
            pointer,
            shift: false,
            ctrl: true,
        },
    );
}

fn xz(point: Vector3) -> DVec2 {
    DVec2::new(f64::from(point.x), f64::from(point.z))
}

// The handles of `kind`, in the order the frame lists them.
fn handles(gizmos: &Gizmos, kind: GizmoHandleKind) -> Vec<Vector3> {
    gizmos
        .handles
        .iter()
        .zip(&gizmos.handle_kinds)
        .filter(|(_, k)| **k == kind)
        .map(|(point, _)| *point)
        .collect()
}

// A freeform draft of four nodes zigzagging east over open ground, in its edit phase.
fn freeform_draft() -> RoadToolSession {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeFreeform);
    for (x, z) in [
        (-280.0, 140.0),
        (-240.0, 175.0),
        (-200.0, 140.0),
        (-160.0, 175.0),
    ] {
        click(&mut tool, at(x, z));
    }
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 4 });
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    assert_eq!(tool.frame().state.phase, Phase::Editing);
    tool
}

// Drags the grip under `from` to `to` (Ctrl held while moving when `ctrl`), then releases.
fn drag(tool: &mut RoadToolSession, from: Vector3, to: (f32, f32), ctrl: bool) {
    move_to(tool, Pointer::Scripted(from));
    event(tool, RoadToolEvent::Press(PointerButton::Left));
    assert!(
        tool.take_commit().is_none(),
        "a press on a grip sends nothing"
    );
    let target = at(to.0, to.1);
    if ctrl {
        move_ctrl(tool, target);
    } else {
        move_to(tool, target);
    }
    event(tool, RoadToolEvent::Release(PointerButton::Left));
}

// The turn between the spans meeting at the path's inner joint `joint` (radians, signed).
fn joint_turn(path: &GuidePath, joint: usize) -> f64 {
    let (a, b) = (
        path.segments()[joint - 1].end_tangent(),
        path.segments()[joint].start_tangent(),
    );
    a.perp_dot(b).atan2(a.dot(b))
}

// Handle edits keep a joint collinear unless broken with Ctrl; a broken joint stays broken; a
// dragged node carries its handles.
#[test]
fn freeform_handle_edits_keep_joints_collinear_unless_broken() {
    let mut tool = freeform_draft();
    // Handles in frame order: node 0 out; node 1 in, out; node 2 in, out; node 3 in.
    let before = handles(&tool.frame().gizmos, GizmoHandleKind::Handle);
    assert_eq!(before.len(), 6);
    let node1 = Vector3::new(-240.0, 0.0, 175.0);
    let inward_length = xz(before[1]).distance(xz(node1));
    drag(&mut tool, before[2], (-226.0, 190.0), false);
    let after = handles(&tool.frame().gizmos, GizmoHandleKind::Handle);
    assert!(
        joint_turn(tool.path(), 1).abs() < 1e-9,
        "joint 1 stays collinear"
    );
    assert!((xz(after[1]).distance(xz(node1)) - inward_length).abs() < 1e-3);
    assert!(xz(after[2]).distance(DVec2::new(-226.0, 190.0)) < 1e-3);

    // Ctrl breaks node 2's joint: its inward handle moves alone, now and later.
    drag(&mut tool, after[3], (-215.0, 120.0), true);
    let broken = handles(&tool.frame().gizmos, GizmoHandleKind::Handle);
    assert!(joint_turn(tool.path(), 2).abs() > 0.1, "joint 2 broken");
    assert_eq!(broken[4], after[4], "the outward handle stays");
    drag(&mut tool, broken[4], (-185.0, 130.0), false);
    let later = handles(&tool.frame().gizmos, GizmoHandleKind::Handle);
    assert_eq!(later[3], broken[3], "a broken joint's handles move apart");

    // Dragging node 1 carries both its handles.
    let joints = handles(&tool.frame().gizmos, GizmoHandleKind::Joint);
    let offsets = [xz(later[1]) - xz(joints[0]), xz(later[2]) - xz(joints[0])];
    drag(&mut tool, joints[0], (-238.0, 168.0), false);
    let moved = handles(&tool.frame().gizmos, GizmoHandleKind::Handle);
    let node = handles(&tool.frame().gizmos, GizmoHandleKind::Joint)[0];
    assert!(xz(node).distance(DVec2::new(-238.0, 168.0)) < 1e-4);
    for (handle, offset) in [moved[1], moved[2]].into_iter().zip(offsets) {
        assert!((xz(handle) - xz(node) - offset).length() < 1e-3);
    }
    assert!(joint_turn(tool.path(), 1).abs() < 1e-9);
    assert_eq!(tool.frame().state.phase, Phase::Editing);
}

// The committed polyline is the flattening rule's: the stroke is the path's flattening, every
// node a point bit for bit, no step over 4 m and the true curve within 5 cm of the chords.
#[test]
fn freeform_commits_the_flattening_rule() {
    let mut tool = freeform_draft();
    let handle = handles(&tool.frame().gizmos, GizmoHandleKind::Handle)[2];
    // An edited joint at node 1.
    drag(&mut tool, handle, (-220.0, 178.0), false);
    assert_eq!(tool.frame().state.refusal, None, "{:?}", tool.frame().hud);
    let nodes: Vec<Vector3> = tool.frame().gizmos.handles[..4].to_vec();
    let path = tool.path().clone();
    assert!(matches!(path.segments()[0], GuideSegment::Bezier(_)));
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    let sent = tool.take_commit().expect("confirm sends the stroke");
    assert_eq!(sent.points, tool.frame().stroke.points);
    let mut flat = Vec::new();
    path.flatten_into(&mut flat).unwrap();
    assert_eq!(sent.points.len(), flat.len());
    for (point, expected) in sent.points.iter().zip(&flat) {
        assert_eq!((point.x, point.z), (expected.x as f32, expected.y as f32));
    }
    for node in &nodes {
        assert!(sent.points.contains(node), "node {node:?} bit for bit");
    }
    assert_eq!(sent.points[0], nodes[0]);
    assert_eq!(*sent.points.last().unwrap(), nodes[3]);
    let polyline: Vec<DVec2> = sent.points.iter().map(|p| xz(*p)).collect();
    for pair in polyline.windows(2) {
        let step = pair[0].distance(pair[1]);
        assert!(step <= FLATTEN_MAX_STEP_M + 1e-4, "step {step}");
    }
    // Every true point within the sag bound of the polyline (plus the f32 cast's rounding).
    let length = path.length();
    let samples = (length / 0.05).ceil() as usize;
    let mut worst: f64 = 0.0;
    for index in 0..=samples {
        let p = path
            .point_at(length * index as f64 / samples as f64)
            .unwrap();
        let distance = polyline
            .windows(2)
            .map(|pair| {
                let (a, b) = (pair[0], pair[1]);
                let t = ((p - a).dot(b - a) / (b - a).length_squared()).clamp(0.0, 1.0);
                p.distance(a + (b - a) * t)
            })
            .fold(f64::INFINITY, f64::min);
        worst = worst.max(distance);
    }
    assert!(worst <= FLATTEN_MAX_SAG_M + 1e-4, "sag {worst}");
}

// Cancel (Esc, right-click) leaves nothing: no stroke, gizmos, HUD or commit, and the next draft
// starts with automatic handles again.
#[test]
fn freeform_cancel_leaves_nothing() {
    for cancel in [
        RoadToolEvent::Action(RoadToolAction::Cancel),
        RoadToolEvent::Press(PointerButton::Right),
    ] {
        let mut tool = freeform_draft();
        let handle = handles(&tool.frame().gizmos, GizmoHandleKind::Handle)[0];
        drag(&mut tool, handle, (-262.0, 120.0), true);
        event(&mut tool, cancel);
        let frame = tool.frame();
        assert_eq!(frame.state.phase, Phase::Idle);
        assert!(frame.stroke.points.is_empty());
        assert!(frame.gizmos.handles.is_empty() && frame.gizmos.line_points.is_empty());
        assert_eq!(frame.hud, Default::default());
        assert_eq!(frame.state.refusal, None);
        assert!(tool.take_commit().is_none());
        // A two-node draft at the same start is the straight line again.
        click(&mut tool, at(-280.0, 140.0));
        move_to(&mut tool, at(-200.0, 140.0));
        assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
        assert!(tool.path().min_radius().is_infinite());
    }
}

// A node snapped to the network ends the placing; the stroke ends there, so no chain. On open
// ground the accepted stroke chains from its end.
#[test]
fn freeform_ends_on_the_network_and_chains_on_open_ground() {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeFreeform);
    click(&mut tool, ray(-220.0, 90.0));
    click(&mut tool, ray(-180.0, 50.0));
    click(&mut tool, ray(-150.0, 1.0));
    assert_eq!(tool.frame().cursor.snap, SnapKind::Edge);
    assert_eq!(tool.frame().state.phase, Phase::Editing);
    // A press off every grip sends the stroke.
    click(&mut tool, ray(-100.0, 150.0));
    assert!(tool.take_commit().is_some());
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Idle);

    let mut tool = freeform_draft();
    let end = *tool.frame().stroke.points.last().unwrap();
    // A press beside a grip (within reach) grabs instead of sending.
    let node = handles(&tool.frame().gizmos, GizmoHandleKind::Joint)[1];
    move_to(
        &mut tool,
        Pointer::Scripted(node + Vector3::new(GRIP_REACH_M as f32 - 0.5, 0.0, 0.0)),
    );
    event(&mut tool, RoadToolEvent::Press(PointerButton::Left));
    assert!(tool.take_commit().is_none());
    event(&mut tool, RoadToolEvent::Release(PointerButton::Left));
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    assert!(tool.take_commit().is_some());
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
    move_to(&mut tool, at(-120.0, 200.0));
    assert_eq!(tool.frame().stroke.points[0], end);
    let kinds = &tool.frame().gizmos.handle_kinds;
    assert_eq!(kinds[0], GizmoHandleKind::ChainStart);
    // A refusal keeps the draft editable.
    click(&mut tool, at(-120.0, 200.0));
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    assert_eq!(tool.frame().state.phase, Phase::Editing);
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    assert!(tool.take_commit().is_some());
    ack(&mut tool, false);
    assert_eq!(tool.frame().state.phase, Phase::Editing);
    assert_eq!(tool.frame().stroke.points[0], end);
}

// An S-curve from (-280, 150) heading east to `end` along `end_tangent`, at its last pose.
fn s_curve(end: (f32, f32), end_tangent: (f32, f32)) -> RoadToolSession {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeComplexCurve);
    click(&mut tool, at(-280.0, 150.0));
    click(&mut tool, at(-270.0, 150.0));
    click(
        &mut tool,
        at(end.0 - end_tangent.0 * 10.0, end.1 - end_tangent.1 * 10.0),
    );
    move_to(&mut tool, at(end.0, end.1));
    tool
}

fn arc_radii(path: &GuidePath) -> [f64; 2] {
    let radius = |segment: &GuideSegment| match segment {
        GuideSegment::Arc(arc) => arc.radius(),
        other => panic!("an arc, got {other:?}"),
    };
    [radius(&path.segments()[0]), radius(&path.segments()[1])]
}

// The S-curve's aim legs draw as tangent gizmos (and, since round SC, a road preview: see
// `scurve_steps`); at its end the HUD gives both arcs' radii, the design speed and the
// tight-curve verdict of the tighter; the gizmos show the arcs' control points and the joint.
#[test]
fn s_curve_reports_both_radii_and_judges_the_tighter() {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeComplexCurve);
    click(&mut tool, at(-280.0, 150.0));
    move_to(&mut tool, at(-260.0, 150.0));
    for placed in 1..3 {
        let frame = tool.frame();
        assert!(frame.gizmos.line_classes.contains(&GizmoLineClass::Tangent));
        assert_eq!(frame.state.phase, Phase::Placing { placed });
        click(&mut tool, at(-270.0 + 40.0 * placed as f32, 170.0));
    }

    let tool = s_curve((-180.0, 200.0), (1.0, 0.3));
    let frame = tool.frame();
    assert_eq!(frame.state.refusal, None);
    let [first, second] = arc_radii(tool.path());
    assert!(
        (first - second).abs() > 1.0,
        "unequal arcs {first} {second}"
    );
    assert_eq!(frame.hud.radius_m, Some(first.min(second)));
    assert_eq!(frame.hud.other_radius_m, Some(first.max(second)));
    assert_eq!(
        frame.hud.design_speed_kmh,
        Some(design_speed_kmh(first.min(second)))
    );
    // Each arc's length and radius, for labels on the geometry.
    assert_eq!(frame.piece_radii_m, vec![first, second]);
    let lengths: Vec<f64> = tool
        .path()
        .segments()
        .iter()
        .map(GuideSegment::length)
        .collect();
    assert_eq!(frame.piece_lengths_m, lengths);
    let joint = handles(&frame.gizmos, GizmoHandleKind::Joint);
    assert_eq!(joint.len(), 1);
    let expected = tool.path().segments()[0].end();
    assert!(xz(joint[0]).distance(expected) < 1e-3);
    let controls = handles(&frame.gizmos, GizmoHandleKind::Handle);
    assert_eq!(controls.len(), 2);
    // Each arc's control point lies on both its end tangents: the first on the start's ray east.
    assert!((controls[0].z - 150.0).abs() < 1e-3 && controls[0].x > -280.0);

    // An S whose first arc is under the default road's illegal radius and whose second is over
    // it: refused on the tighter, the HUD keeps both numbers.
    let limit = frame.hud.illegal_radius_m;
    let tight = (0..400)
        .map(|step| {
            let end = (-276.0 + (step % 40) as f32, 152.0 + (step / 40) as f32);
            s_curve(end, (1.0, 0.6))
        })
        .find(|tool| {
            let radii = arc_radii(tool.path());
            radii[0] < limit && radii[1] > limit
        })
        .expect("an S with one arc each side of the limit");
    let frame = tight.frame();
    assert!(frame.hud.tight_radius);
    assert!(frame.hud.other_radius_m.unwrap() > limit);
    assert_eq!(frame.state.refusal, Some(DraftRefusal::TightCurve));
    assert!(frame.stroke.points.is_empty());
}

// The same events give bitwise equal frames in both new modes, drags and Ctrl included.
#[test]
fn s_curve_and_freeform_frames_are_deterministic() {
    let run = || {
        let mut frames = Vec::new();
        let mut tool = s_curve((-180.0, 200.0), (1.0, 0.3));
        for step in 0..20 {
            move_to(
                &mut tool,
                ray(-190.0 + step as f32, 195.0 + 0.7 * step as f32),
            );
            frames.push(format!("{:?}", tool.frame()));
        }
        let mut tool = freeform_draft();
        let grips = tool.frame().gizmos.handles.clone();
        for (index, grip) in grips.into_iter().enumerate() {
            // A press that meets no grip any more sends the stroke; that is part of the run.
            move_to(&mut tool, Pointer::Scripted(grip));
            event(&mut tool, RoadToolEvent::Press(PointerButton::Left));
            let to = at(grip.x + 3.0, grip.z - 2.0 + index as f32 * 0.3);
            if index % 3 == 0 {
                move_ctrl(&mut tool, to);
            } else {
                move_to(&mut tool, to);
            }
            event(&mut tool, RoadToolEvent::Release(PointerButton::Left));
            let sent = tool.take_commit();
            frames.push(format!("{:?} {sent:?}", tool.frame()));
        }
        frames
    };
    assert_eq!(run(), run());
}

// Round FX: a handle turned through a full circle about its node, onto the exact fold (the
// handle straight back, away from the next node: a cusp) and ever nearer it (a tiny loop), poses
// in bounded work at every angle: the stroke, the guides and the labels stay small, and a pose
// the road cannot be built along shows a refusal. On `dev` `3a950b78` the zoning-depth guide's
// Bezier offset asked for unbounded points near the fold and the game ran out of memory. The
// drafts: node 0's outward handle on a two-node draft; node 1's on a three-node draft (its
// inward handle turning with it); and the user's second freeze, a four-span draft whose third
// node is a near reversal (the spans arrive heading down-right and leave heading back up), its
// outward handle pulled far out, collinear and Ctrl-broken.
#[test]
fn freeform_handle_turned_full_circle_stays_bounded() {
    let reversal: &[(f32, f32)] = &[
        (-420.0, 100.0),
        (-310.0, 130.0),
        (-200.0, 190.0),
        (-320.0, 210.0),
        (-390.0, 250.0),
    ];
    // The nodes, the grabbed node, its outward handle's index in the frame, Ctrl, handle length.
    type Draft<'a> = (&'a [(f32, f32)], usize, usize, bool, f64);
    let drafts: [Draft; 4] = [
        (&[(-280.0, 140.0), (-180.0, 140.0)], 0, 0, false, 33.0),
        (
            &[(-280.0, 140.0), (-230.0, 140.0), (-180.0, 140.0)],
            1,
            2,
            false,
            25.0,
        ),
        (reversal, 2, 4, false, 90.0),
        (reversal, 2, 4, true, 90.0),
    ];
    for (nodes, grabbed, handle_index, ctrl, length) in drafts {
        let mut tool = session();
        mode(&mut tool, RoadToolAction::ModeFreeform);
        for &(x, z) in nodes {
            click(&mut tool, at(x, z));
        }
        event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
        assert_eq!(tool.frame().state.phase, Phase::Editing);
        let handle = handles(&tool.frame().gizmos, GizmoHandleKind::Handle)[handle_index];
        let (node, next) = (nodes[grabbed], nodes[grabbed + 1]);
        let centre = DVec2::new(f64::from(node.0), f64::from(node.1));
        let away = centre - DVec2::new(f64::from(next.0), f64::from(next.1));
        // The fold: the handle pointing straight away from the next node.
        let fold = away.y.atan2(away.x).to_degrees();
        let mut angles: Vec<f64> = (0..360).map(|step| fold + f64::from(step)).collect();
        for exponent in 1..=7 {
            let near = 10f64.powi(-exponent);
            angles.extend([fold - near, fold + near]);
        }
        move_to(&mut tool, Pointer::Scripted(handle));
        event(&mut tool, RoadToolEvent::Press(PointerButton::Left));
        let mut at_fold = None;
        for &angle in &angles {
            let (sin, cos) = angle.to_radians().sin_cos();
            let target = centre + DVec2::new(cos, sin) * length;
            let started = std::time::Instant::now();
            // Shift frees the guide snaps: the pointer is the handle.
            event(
                &mut tool,
                RoadToolEvent::Pointer {
                    pointer: Pointer::Scripted(Vector3::new(
                        target.x as f32,
                        handle.y,
                        target.y as f32,
                    )),
                    shift: true,
                    ctrl,
                },
            );
            let elapsed = started.elapsed();
            let frame = tool.frame();
            let at_angle = format!("draft {grabbed}/{ctrl}, angle {angle}");
            assert!(
                elapsed.as_secs_f64() < 2.0,
                "{at_angle}: a pose took {elapsed:?}"
            );
            assert!(
                frame.stroke.points.len() < 2_000,
                "{at_angle}: {} stroke points",
                frame.stroke.points.len()
            );
            assert!(
                frame.gizmos.line_points.len() < 50_000,
                "{at_angle}: {} gizmo points",
                frame.gizmos.line_points.len()
            );
            assert!(frame.labels.len() < 64, "{at_angle}");
            assert!(
                !frame.stroke.points.is_empty() || frame.state.refusal.is_some(),
                "{at_angle}: no stroke and no refusal"
            );
            if angle == fold {
                at_fold = frame.state.refusal;
            }
        }
        event(&mut tool, RoadToolEvent::Release(PointerButton::Left));
        if ctrl || grabbed < 2 {
            // At the fold all four control points of the span leaving the node lie on one line
            // and the span runs back over itself: a cusp, refused as too tight.
            assert_eq!(
                at_fold,
                Some(DraftRefusal::TightCurve),
                "draft {grabbed}/{ctrl}"
            );
        }
    }
}

// Round FX: the near cusp a sweep steps over, built exactly. Node 0's handle flipped back to
// `(-200, 50)` and node 1's at `(-400, 50 + 1/1024)` put the span's speed within 1/1024 m of zero
// at parameter 1/2, one of the radius samples: a loop of about 10 nm radius, the rest of the
// span curving the same way. On `dev` `3a950b78` the zoning-depth guide's offset to the loop's
// outside multiplied the span's length by the stretch `1 + depth / radius` (about 1e11) and
// never finished the pose (it ran out of memory); now the pose ends at once, refused as too
// tight.
#[test]
fn freeform_near_cusp_on_a_radius_sample_is_refused() {
    let mut tool = session();
    mode(&mut tool, RoadToolAction::ModeFreeform);
    click(&mut tool, at(-280.0, 140.0));
    click(&mut tool, at(-180.0, 140.0));
    event(&mut tool, RoadToolEvent::Action(RoadToolAction::Confirm));
    let short = 1.0_f32 / 1024.0;
    // Node 1's inward handle first (it is automatic until dragged), then node 0's outward.
    for (index, target) in [(1, (-580.0, 190.0 + short)), (0, (-480.0, 190.0))] {
        let handle = handles(&tool.frame().gizmos, GizmoHandleKind::Handle)[index];
        move_to(&mut tool, Pointer::Scripted(handle));
        event(&mut tool, RoadToolEvent::Press(PointerButton::Left));
        let started = std::time::Instant::now();
        event(
            &mut tool,
            RoadToolEvent::Pointer {
                pointer: Pointer::Scripted(Vector3::new(target.0, handle.y, target.1)),
                shift: true,
                ctrl: false,
            },
        );
        assert!(
            started.elapsed().as_secs_f64() < 2.0,
            "{:?}",
            started.elapsed()
        );
        event(&mut tool, RoadToolEvent::Release(PointerButton::Left));
    }
    let GuideSegment::Bezier(curve) = tool.path().segments()[0] else {
        panic!("a Bezier span");
    };
    let [p0, p1, p2, p3] = curve.points();
    assert_eq!(p1 - p0, DVec2::new(-200.0, 50.0));
    assert_eq!(p2 - p3, DVec2::new(-400.0, 50.0 + f64::from(short)));
    let frame = tool.frame();
    assert_eq!(
        frame.state.refusal,
        Some(DraftRefusal::TightCurve),
        "{:?}",
        frame.hud
    );
    assert!(frame.stroke.points.is_empty());
    assert!(frame.gizmos.line_points.len() < 50_000);
}
