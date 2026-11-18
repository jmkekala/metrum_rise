// SPDX-License-Identifier: GPL-2.0-only

//! Round UI1 (`ROAD-112`) tests: the square lock's capture and release angles (hysteresis) and
//! its place in the precedence (a node, an edge and the zoning grid where it captured still
//! first), the corner arc's geometry and chip for 90, 60 and 120 degree joins on both sides, the
//! guide lines' 4 m reach, the faint guides near the cursor, and the Guides option hiding both.
//! Round UI2: the right-angle square in place of the arc where the chip reads 90 degrees.

use super::{ack, click, d, edge_snap, ground_at, input, move_to, node_snap, ray, session, view};
use crate::nodes::sim::road_tool::frame::{GizmoLineClass, Gizmos, RoadToolFrame};
use crate::nodes::sim::road_tool::guides::{Corner, SQUARE_SIDE_FRACTION};
use crate::nodes::sim::road_tool::labels::{LabelKind, LabelTone, ToolLabel};
use crate::nodes::sim::road_tool::measure::EndRoad;
use crate::nodes::sim::road_tool::session::{
    Pointer, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use crate::nodes::sim::road_tool::snap::{
    GUIDE_NEAR_M, GUIDE_RADIUS_M, GUIDE_REACH_M, GuideLine, SQUARE_CAPTURE_DEG, SQUARE_RELEASE_DEG, SnapKind,
    SnapOptions, SquareAxis, near_guides, resolve, square_lock,
};
use crate::nodes::sim::road_tool::wording::{NoticeFacts, label_text};
use crate::simulation::network::guide::DVec2;
use godot::prelude::Vector3;

// The unit direction `deg` degrees counter-clockwise (in plan x, z) from `from`.
fn turned(from: DVec2, deg: f64) -> DVec2 {
    DVec2::from_angle(deg.to_radians()).rotate(from)
}

// A road along x through the origin; a leg 60 m long `deg` off the line across it.
fn across_leg(deg: f64) -> DVec2 {
    turned(DVec2::Y, deg) * 60.0
}

#[test]
fn square_lock_captures_inside_its_angle_and_holds_to_the_release_angle() {
    let (anchor, road) = (DVec2::ZERO, DVec2::X);
    let fresh = |deg: f64| square_lock(anchor, road, across_leg(deg), None).map(|(axis, _)| axis);
    let held = |deg: f64| {
        square_lock(anchor, road, across_leg(deg), Some(SquareAxis::Across)).map(|(axis, _)| axis)
    };
    for side in [1.0, -1.0] {
        assert_eq!(
            fresh(side * (SQUARE_CAPTURE_DEG - 0.1)),
            Some(SquareAxis::Across)
        );
        assert_eq!(fresh(side * (SQUARE_CAPTURE_DEG + 0.1)), None);
        // Captured, it holds past the capture angle up to the release angle.
        assert_eq!(
            held(side * (SQUARE_CAPTURE_DEG + 0.1)),
            Some(SquareAxis::Across)
        );
        assert_eq!(
            held(side * (SQUARE_RELEASE_DEG - 0.1)),
            Some(SquareAxis::Across)
        );
        assert_eq!(held(side * (SQUARE_RELEASE_DEG + 0.1)), None);
        // Behind the start: the same line.
        assert_eq!(fresh(180.0 + side * 5.0), Some(SquareAxis::Across));
    }
    // The road's own axis likewise; the leg between the two lines takes neither.
    let along = |deg: f64, held| {
        square_lock(anchor, road, turned(DVec2::X, deg) * 60.0, held).map(|(axis, _)| axis)
    };
    assert_eq!(
        along(SQUARE_CAPTURE_DEG - 0.1, None),
        Some(SquareAxis::Along)
    );
    assert_eq!(along(SQUARE_CAPTURE_DEG + 0.1, None), None);
    assert_eq!(
        along(SQUARE_RELEASE_DEG - 0.1, Some(SquareAxis::Along)),
        Some(SquareAxis::Along)
    );
    // Held across, a leg near the axis switches to the axis.
    assert_eq!(
        along(2.0, Some(SquareAxis::Across)),
        Some(SquareAxis::Along)
    );
    assert_eq!(along(45.0, None), None);
    assert_eq!(
        square_lock(anchor, road, anchor, Some(SquareAxis::Across)),
        None
    );
    // The line is the anchor's perpendicular `collect_guides` gathers.
    let (_, line) = square_lock(anchor, road, across_leg(3.0), None).unwrap();
    assert_eq!(line.origin, anchor);
    assert_eq!(line.dir, DVec2::X.perp());
}

// Within the precedence: the held square guide takes the cursor after a node, an edge and the
// zoning grid where it captured (GS2's `grid_rule`), and before any other guide line; a guide
// crossing it near the cursor's foot on it makes a corner; Shift and guides off drop it.
#[test]
fn square_lock_keeps_the_precedence() {
    let square = GuideLine {
        origin: DVec2::ZERO,
        dir: DVec2::Y,
    };
    let mut pose = input(across_leg(10.0));
    pose.square = Some(square);
    let snapped = resolve(&pose, &SnapOptions::default(), &[]);
    assert_eq!(snapped.kind, SnapKind::Guide);
    assert!(snapped.xz.x.abs() < 1e-9 && (snapped.xz.y - pose.raw.y).abs() < 1e-9);
    // A nearer guide line does not take it from the held square guide.
    let other = [GuideLine {
        origin: pose.raw + d(0.0, 1.0),
        dir: d(1.0, 1.0).normalize(),
    }];
    assert_eq!(
        resolve(&pose, &SnapOptions::default(), &other).xz,
        snapped.xz
    );
    // A guide across it near the foot: their corner.
    let across = [GuideLine {
        origin: d(-50.0, snapped.xz.y + 2.0),
        dir: DVec2::X,
    }];
    let corner = resolve(&pose, &SnapOptions::default(), &across);
    assert_eq!(corner.kind, SnapKind::GuideCorner);
    assert!((corner.xz - d(0.0, snapped.xz.y + 2.0)).length() < 1e-9);
    // A node and an edge come first.
    let mut node = pose;
    node.network = Some(node_snap(pose.raw, 4));
    assert_eq!(
        resolve(&node, &SnapOptions::default(), &[]).kind,
        SnapKind::Node
    );
    let mut edge = pose;
    edge.network = Some(edge_snap(pose.raw, DVec2::X));
    assert_eq!(
        resolve(&edge, &SnapOptions::default(), &[]).kind,
        SnapKind::Edge
    );
    // The grid where it captured.
    let mut grid = pose;
    grid.grid = Some((pose.raw + d(0.5, 0.5), None));
    let grid_on = SnapOptions {
        grid: true,
        ..SnapOptions::default()
    };
    assert_eq!(resolve(&grid, &grid_on, &[]).kind, SnapKind::Grid);
    // Shift and guide lines off.
    let mut free = pose;
    free.free = true;
    assert_eq!(
        resolve(&free, &SnapOptions::default(), &[]).kind,
        SnapKind::Free
    );
    let off = SnapOptions {
        guides: false,
        ..SnapOptions::default()
    };
    assert_eq!(resolve(&pose, &off, &[]).kind, SnapKind::Free);
}

fn cursor_xz(tool: &RoadToolSession) -> DVec2 {
    let p = tool.frame().cursor.position;
    d(f64::from(p.x), f64::from(p.z))
}

fn has_line(frame: &RoadToolFrame, class: GizmoLineClass) -> bool {
    frame.gizmos.line_classes.contains(&class)
}

// A branch from the road along z = 0 (started mid-span at x = 150): 7 degrees off square the
// end takes the perpendicular, 11 still holds it, 13 releases it and 10 does not take it again
// (it shows the perpendicular faint, the guide in reach before it captures).
#[test]
fn a_branch_from_a_road_locks_square_with_hysteresis() {
    let mut tool = session();
    click(&mut tool, ray(150.0, 0.5));
    let start = cursor_xz(&tool);
    assert!((start - d(150.0, 0.0)).length() < 1e-3, "{start:?}");
    let leg = |deg: f64| start + across_leg(deg);
    let at = |tool: &mut RoadToolSession, deg: f64| {
        let p = leg(deg);
        move_to(tool, ray(p.x as f32, p.y as f32));
        (tool.frame().cursor.snap, cursor_xz(tool))
    };
    for (deg, locked) in [
        (7.0, true),
        (11.0, true),
        (13.0, false),
        (10.0, false),
        (7.5, true),
    ] {
        let (kind, xz) = at(&mut tool, deg);
        if locked {
            assert_eq!(kind, SnapKind::Guide, "{deg}");
            assert!((xz.x - start.x).abs() < 1e-3, "{deg}: {xz:?}");
            assert!(has_line(tool.frame(), GizmoLineClass::Guide));
        } else {
            assert_eq!(kind, SnapKind::Free, "{deg}");
            assert!((xz.x - start.x).abs() > 1.0, "{deg}: {xz:?}");
        }
    }
    // Uncaptured at 10 degrees (10.4 m off the perpendicular): the perpendicular shows faint.
    at(&mut tool, 13.0);
    at(&mut tool, 10.0);
    assert!(has_line(tool.frame(), GizmoLineClass::GuideNear));
    assert!(!has_line(tool.frame(), GizmoLineClass::Guide));
    // Guide lines off: no lock, no guide and no faint guide.
    let mut off = session();
    off.set_option(RoadToolOption::GuideLines(false));
    click(&mut off, ray(150.0, 0.5));
    for deg in [7.0, 10.0] {
        let (kind, _) = at(&mut off, deg);
        assert_eq!(kind, SnapKind::Free, "{deg}");
        assert!(!has_line(off.frame(), GizmoLineClass::Guide));
        assert!(!has_line(off.frame(), GizmoLineClass::GuideNear));
    }
}

// The reach: a guide line takes a cursor 4 m from it, not one just beyond.
#[test]
fn guide_reach_is_four_metres() {
    assert_eq!(GUIDE_REACH_M, 4.0);
    let guide = [GuideLine {
        origin: d(0.0, 20.0),
        dir: DVec2::X,
    }];
    let options = SnapOptions::default();
    assert_eq!(
        resolve(&input(d(30.0, 23.95)), &options, &guide).kind,
        SnapKind::Guide
    );
    assert_eq!(
        resolve(&input(d(30.0, 24.05)), &options, &guide).kind,
        SnapKind::Free
    );
}

// The faint set: the nearest two within 24 m that do not hold the cursor, one line once.
#[test]
fn near_guides_are_the_nearest_lines_in_reach_not_holding() {
    let line = |y: f64| GuideLine {
        origin: d(0.0, y),
        dir: DVec2::X,
    };
    let raw = DVec2::ZERO;
    let guides = [
        line(25.0),
        line(5.0),
        line(1.0),
        // The 5 m line again from another origin and in the other sense.
        GuideLine {
            origin: d(40.0, 5.0),
            dir: -DVec2::X,
        },
        line(-11.0),
    ];
    // The faint reach: 24 m.
    assert_eq!(
        near_guides(&[line(23.99)], raw, [None; 2]),
        [Some(line(23.99)), None]
    );
    assert_eq!(near_guides(&[line(24.01)], raw, [None; 2]), [None; 2]);
    assert_eq!(GUIDE_NEAR_M, 24.0);
    let near = near_guides(&guides, raw, [Some(line(1.0)), None]);
    assert_eq!(near, [Some(line(5.0)), Some(line(-11.0))]);
    let all = near_guides(&guides, raw, [None; 2]);
    assert_eq!(all, [Some(line(1.0)), Some(line(5.0))]);
    assert_eq!(near_guides(&[], raw, [None; 2]), [None; 2]);
    assert_eq!(near_guides(&[line(25.0)], raw, [None; 2]), [None; 2]);
}

// The corner's mark for a draft leaving `leaving` against `road`: the angle and the ray chosen;
// round UI2: a corner reading 90 degrees draws the square (`check_square`), every other one the
// arc (`check_arc`).
fn check_corner(leaving: DVec2, road: EndRoad, angle: f64, ray: DVec2) {
    let joint = d(10.0, -4.0);
    let corner = Corner::new(joint, leaving, road).expect("a corner");
    assert!((corner.angle_deg - angle).abs() < 1e-9, "{corner:?}");
    assert!((corner.road - ray).length() < 1e-9, "{corner:?} {ray:?}");
    assert_eq!(corner.is_square(), angle.round() == 90.0, "{corner:?}");
    if corner.is_square() {
        check_square(&corner);
    } else {
        check_arc(&corner);
    }
}

fn plan(points: &[Vector3]) -> Vec<DVec2> {
    points
        .iter()
        .map(|p| d(f64::from(p.x), f64::from(p.z)))
        .collect()
}

// Half the corner each side of the chip's side: the bisector.
fn check_bisector(corner: &Corner, side: DVec2) {
    let half = |a: DVec2, b: DVec2| a.perp_dot(b).abs().atan2(a.dot(b)).to_degrees();
    assert!((half(corner.leaving, side) - corner.angle_deg / 2.0).abs() < 1e-6);
    assert!((half(side, corner.road) - corner.angle_deg / 2.0).abs() < 1e-6);
}

// The arc's points on the 12 m circle from the draft to the road, the chip at its middle pushed
// out along the bisector.
fn check_arc(corner: &Corner) {
    let (joint, angle) = (corner.joint, corner.angle_deg);
    let mut gizmos = Gizmos::default();
    let (anchor, side) = corner.push_mark(&mut gizmos, 12.0, 2.0);
    assert_eq!(gizmos.line_classes, [GizmoLineClass::CornerArc]);
    let points = plan(&gizmos.line_points);
    assert_eq!(points.len(), (angle / 5.0 - 1e-9).ceil() as usize + 1);
    for point in &points {
        assert!(((*point - joint).length() - 12.0).abs() < 1e-4);
    }
    assert!((points[0] - (joint + corner.leaving * 12.0)).length() < 1e-4);
    assert!((points[points.len() - 1] - (joint + corner.road * 12.0)).length() < 1e-4);
    // Monotone: each point turns the same way, no wider than the corner.
    let swept: f64 = points
        .windows(2)
        .map(|pair| {
            let (a, b) = ((pair[0] - joint).normalize(), (pair[1] - joint).normalize());
            a.perp_dot(b).atan2(a.dot(b)).to_degrees()
        })
        .sum();
    assert!((swept.abs() - angle).abs() < 1e-3, "{swept} {angle}");
    check_bisector(corner, side);
    let anchor = d(f64::from(anchor.x), f64::from(anchor.z));
    assert!((anchor - (joint + side * 12.0)).length() < 1e-4);
    assert_eq!(gizmos.line_points[0].y, 2.0);
}

// Round UI2: the square's open L, three points and no chord, side 12 / sqrt 2 m: on the draft,
// the outer corner `joint + (leaving + road) * s`, on the road; the chip anchored at the outer
// corner and pushed out along the bisector, so it sits past the square.
fn check_square(corner: &Corner) {
    let joint = corner.joint;
    let s = 12.0 * SQUARE_SIDE_FRACTION;
    assert!((s - 12.0 / 2f64.sqrt()).abs() < 1e-12);
    let mut gizmos = Gizmos::default();
    let (anchor, side) = corner.push_mark(&mut gizmos, 12.0, 2.0);
    assert_eq!(gizmos.line_classes, [GizmoLineClass::CornerArc]);
    assert_eq!(gizmos.line_starts, [0]);
    let points = plan(&gizmos.line_points);
    let outer = joint + (corner.leaving + corner.road) * s;
    let expected = [joint + corner.leaving * s, outer, joint + corner.road * s];
    assert_eq!(points.len(), 3);
    for (point, expected) in points.iter().zip(expected) {
        assert!(
            (*point - expected).length() < 1e-4,
            "{point:?} {expected:?}"
        );
    }
    assert!(gizmos.line_points.iter().all(|p| p.y == 2.0));
    check_bisector(corner, side);
    let anchor = d(f64::from(anchor.x), f64::from(anchor.z));
    assert!((anchor - outer).length() < 1e-4);
    // The outer corner on the bisector, within a hair of where the arc's middle was (exactly
    // there at 90 degrees); the whole square lies behind it along the side.
    assert!((anchor - joint).normalize().dot(side) > 1.0 - 1e-9);
    assert!(((anchor - joint).length() - 12.0).abs() < 0.1);
    for point in [expected[0], expected[2]] {
        assert!((point - joint).dot(side) < (anchor - joint).dot(side));
    }
}

#[test]
fn corner_marks_for_90_60_and_120_degree_joins_on_both_sides() {
    for sense in [1.0, -1.0] {
        let leaving = |deg: f64| turned(DVec2::X, sense * deg);
        // A road through the point: the smaller corner, against the road's nearer sense; square,
        // the corner on `leaving.perp()`'s side.
        check_corner(leaving(90.0), EndRoad::Along(DVec2::X), 90.0, {
            let l = leaving(90.0);
            if l.perp_dot(DVec2::X) > 0.0 {
                DVec2::X
            } else {
                -DVec2::X
            }
        });
        check_corner(leaving(60.0), EndRoad::Along(DVec2::X), 60.0, DVec2::X);
        check_corner(leaving(120.0), EndRoad::Along(DVec2::X), 60.0, -DVec2::X);
        // The road's sense does not matter.
        check_corner(leaving(60.0), EndRoad::Along(-DVec2::X), 60.0, DVec2::X);
        // Out of a dead end whose road runs on along +x: the corner against its ray back (-x).
        for deg in [90.0, 60.0, 120.0] {
            check_corner(
                turned(-DVec2::X, sense * deg),
                EndRoad::Continue(DVec2::X),
                deg,
                -DVec2::X,
            );
        }
    }
    assert!(Corner::new(DVec2::ZERO, DVec2::ZERO, EndRoad::Along(DVec2::X)).is_none());
}

// Round UI2: the square shows exactly where the chip reads "90°", by the chip's own rounding
// (`f64::round`, half away from zero: 89.5 up to below 90.5). So 89.6 and 90.4 show the square,
// 89.4 ("89°") and 90.6 ("91°") the arc, on both sides, out of a dead end and (below 90, the
// smaller corner) against a road through the point.
#[test]
fn the_square_shows_where_the_chip_reads_90() {
    let text = |corner: &Corner| {
        let label = ToolLabel {
            kind: LabelKind::Angle,
            value: corner.shown_deg(),
            anchor: Vector3::ZERO,
            side: DVec2::X,
            tone: LabelTone::Normal,
        };
        let mut out = String::new();
        label_text(&label, &NoticeFacts::default(), &mut out);
        out
    };
    for sense in [1.0, -1.0] {
        for (deg, square) in [(89.4, false), (89.6, true), (90.4, true), (90.6, false)] {
            let leaving = turned(-DVec2::X, sense * deg);
            let dead_end = Corner::new(DVec2::ZERO, leaving, EndRoad::Continue(DVec2::X)).unwrap();
            let mut corners = vec![dead_end];
            if deg < 90.0 {
                let along = turned(DVec2::X, sense * deg);
                corners.push(Corner::new(DVec2::ZERO, along, EndRoad::Along(DVec2::X)).unwrap());
            }
            for corner in corners {
                // Against a road through the point a tie (both corners read 90) takes either.
                let other = 180.0 - deg;
                assert!(
                    (corner.angle_deg - deg).abs() < 1e-9
                        || (corner.angle_deg - other).abs() < 1e-9,
                    "{corner:?}"
                );
                assert_eq!(corner.is_square(), square, "{deg} {corner:?}");
                assert_eq!(text(&corner) == "90°", square, "{deg} {}", text(&corner));
                if square {
                    check_square(&corner);
                } else {
                    check_arc(&corner);
                }
            }
        }
    }
    // The rounding's edges, half away from zero.
    let at = |angle_deg| Corner {
        joint: DVec2::ZERO,
        leaving: DVec2::X,
        road: DVec2::Y,
        angle_deg,
    };
    for (deg, square) in [
        (89.5, true),
        (89.499_999, false),
        (90.499_999, true),
        (90.5, false),
    ] {
        assert_eq!(at(deg).is_square(), square, "{deg}");
    }
}

// A chained start square to the stroke before it (Shift: off-square poses stay free): the
// square in the frame, three points in its strip and no chord, its chip "90°"; the arc at
// 60 degrees; and over the poses the gizmo buffers do not grow once they have (the square
// pushes into the reused buffers).
#[test]
fn a_chained_start_shows_the_square_and_reuses_the_buffers() {
    let scripted = |x: f64, z: f64| {
        Pointer::Scripted(Vector3::new(
            x as f32,
            ground_at(x as f32, z as f32),
            z as f32,
        ))
    };
    let mut tool = session();
    click(&mut tool, scripted(-280.0, 200.0));
    click(&mut tool, scripted(-200.0, 200.0));
    ack(&mut tool, true);
    // The corner against the ray back along -x: 180 - `deg` from +x.
    let pose = |tool: &mut RoadToolSession, deg: f64| {
        let leg = turned(DVec2::X, 180.0 - deg) * 80.0;
        tool.handle_event(
            view(),
            RoadToolEvent::Pointer {
                pointer: scripted(-200.0 + leg.x, 200.0 + leg.y),
                shift: true,
                ctrl: false,
            },
        );
        let frame = tool.frame();
        let gizmos = &frame.gizmos;
        let strip = gizmos
            .line_classes
            .iter()
            .position(|class| *class == GizmoLineClass::CornerArc)
            .expect("a corner strip");
        let first = gizmos.line_starts[strip] as usize;
        let end = gizmos
            .line_starts
            .get(strip + 1)
            .map_or(gizmos.line_points.len(), |next| *next as usize);
        let angles: Vec<_> = frame
            .labels
            .iter()
            .filter(|label| label.kind == LabelKind::Angle)
            .collect();
        assert_eq!(angles.len(), 1);
        let capacities = (
            gizmos.line_points.capacity(),
            gizmos.line_starts.capacity(),
            gizmos.line_classes.capacity(),
        );
        (end - first, angles[0].value, capacities)
    };
    for deg in [89.6, 90.4] {
        assert_eq!(pose(&mut tool, deg).0, 3, "{deg}");
        assert_eq!(pose(&mut tool, deg).1, 90.0, "{deg}");
    }
    let (arc, value, _) = pose(&mut tool, 60.0);
    assert!(arc >= 13 && value == 60.0, "{arc} {value}");
    let (_, _, grown) = pose(&mut tool, 89.6);
    for deg in [90.4, 60.0, 89.6, 90.0] {
        assert_eq!(pose(&mut tool, deg).2, grown, "{deg}");
    }
}

// The user, 2026-11-18: a guide drawn 40 m past the cursor took it only within 64 m of its
// source. Followed away from the spur's dead end along its perpendicular, 2 m off the line, the
// guide holds the cursor on the line to 150 m; jumped to directly that far out, nothing does.
#[test]
fn a_followed_guide_holds_past_the_gathering_radius() {
    let end = d(-40.0, 200.0);
    let along = d(-1.0, 1.0).normalize();
    let at = |m: f64| {
        let p = end + along * m + along.perp() * 2.0;
        ray(p.x as f32, p.y as f32)
    };
    let off_line = |tool: &RoadToolSession| {
        let p = tool.frame().cursor.position;
        along.perp_dot(d(f64::from(p.x), f64::from(p.z)) - end).abs()
    };
    let mut tool = session();
    for step in 4..=30 {
        let m = f64::from(step) * 5.0;
        move_to(&mut tool, at(m));
        assert_eq!(tool.frame().cursor.snap, SnapKind::Guide, "{m} m");
        assert!(off_line(&tool) < 1e-3, "{m} m: {}", off_line(&tool));
    }
    const { assert!(150.0 > GUIDE_RADIUS_M + GUIDE_REACH_M) };
    let mut fresh = session();
    move_to(&mut fresh, at(150.0));
    assert_eq!(fresh.frame().cursor.snap, SnapKind::Free);
    // Guides off drops the held line.
    tool.set_option(RoadToolOption::GuideLines(false));
    move_to(&mut tool, at(150.0));
    assert_eq!(tool.frame().cursor.snap, SnapKind::Free);
}
