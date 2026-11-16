// SPDX-License-Identifier: GPL-2.0-only

//! Round UI1 (`ROAD-112`) tests: the square lock's capture and release angles (hysteresis) and
//! its place in the precedence (a node, an edge and the zoning grid where it captured still
//! first), the corner arc's geometry and chip for 90, 60 and 120 degree joins on both sides, the
//! guide lines' 4 m reach, the faint guides near the cursor, and the Guides option hiding both.

use super::{click, d, edge_snap, input, move_to, node_snap, ray, session};
use crate::nodes::sim::road_tool::frame::{GizmoLineClass, Gizmos, RoadToolFrame};
use crate::nodes::sim::road_tool::guides::Corner;
use crate::nodes::sim::road_tool::measure::EndRoad;
use crate::nodes::sim::road_tool::session::{RoadToolOption, RoadToolSession};
use crate::nodes::sim::road_tool::snap::{
    GUIDE_NEAR_M, GUIDE_REACH_M, GuideLine, SQUARE_CAPTURE_DEG, SQUARE_RELEASE_DEG, SnapKind,
    SnapOptions, SquareAxis, near_guides, resolve, square_lock,
};
use crate::simulation::network::guide::DVec2;

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

// The faint set: the nearest two within 12 m that do not hold the cursor, one line once.
#[test]
fn near_guides_are_the_nearest_lines_in_reach_not_holding() {
    let line = |y: f64| GuideLine {
        origin: d(0.0, y),
        dir: DVec2::X,
    };
    let raw = DVec2::ZERO;
    let guides = [
        line(13.0),
        line(5.0),
        line(1.0),
        // The 5 m line again from another origin and in the other sense.
        GuideLine {
            origin: d(40.0, 5.0),
            dir: -DVec2::X,
        },
        line(-11.0),
    ];
    // The faint reach: 12 m.
    assert_eq!(
        near_guides(&[line(11.99)], raw, [None; 2]),
        [Some(line(11.99)), None]
    );
    assert_eq!(near_guides(&[line(12.01)], raw, [None; 2]), [None; 2]);
    assert_eq!(GUIDE_NEAR_M, 12.0);
    let near = near_guides(&guides, raw, [Some(line(1.0)), None]);
    assert_eq!(near, [Some(line(5.0)), Some(line(-11.0))]);
    let all = near_guides(&guides, raw, [None; 2]);
    assert_eq!(all, [Some(line(1.0)), Some(line(5.0))]);
    assert_eq!(near_guides(&[], raw, [None; 2]), [None; 2]);
    assert_eq!(near_guides(&[line(13.0)], raw, [None; 2]), [None; 2]);
}

// The arc's corner for a draft leaving `leaving` against `road`: the angle and the ray chosen,
// the arc's points on the circle from the draft to the road, the chip at its middle pushed out
// along the bisector.
fn check_corner(leaving: DVec2, road: EndRoad, angle: f64, ray: DVec2) {
    let joint = d(10.0, -4.0);
    let corner = Corner::new(joint, leaving, road).expect("a corner");
    assert!((corner.angle_deg - angle).abs() < 1e-9, "{corner:?}");
    assert!((corner.road - ray).length() < 1e-9, "{corner:?} {ray:?}");
    let mut gizmos = Gizmos::default();
    let (anchor, side) = corner.push_arc(&mut gizmos, 12.0, 2.0);
    assert_eq!(gizmos.line_classes, [GizmoLineClass::CornerArc]);
    let points: Vec<DVec2> = gizmos
        .line_points
        .iter()
        .map(|p| d(f64::from(p.x), f64::from(p.z)))
        .collect();
    assert_eq!(points.len(), (angle / 5.0).ceil() as usize + 1);
    for point in &points {
        assert!(((*point - joint).length() - 12.0).abs() < 1e-4);
    }
    assert!((points[0] - (joint + leaving * 12.0)).length() < 1e-4);
    assert!((points[points.len() - 1] - (joint + ray * 12.0)).length() < 1e-4);
    // Monotone: each point turns the same way, no wider than the corner.
    let swept: f64 = points
        .windows(2)
        .map(|pair| {
            let (a, b) = ((pair[0] - joint).normalize(), (pair[1] - joint).normalize());
            a.perp_dot(b).atan2(a.dot(b)).to_degrees()
        })
        .sum();
    assert!((swept.abs() - angle).abs() < 1e-3, "{swept} {angle}");
    let half = |a: DVec2, b: DVec2| a.perp_dot(b).abs().atan2(a.dot(b)).to_degrees();
    assert!((half(leaving, side) - angle / 2.0).abs() < 1e-6);
    assert!((half(side, ray) - angle / 2.0).abs() < 1e-6);
    let anchor = d(f64::from(anchor.x), f64::from(anchor.z));
    assert!((anchor - (joint + side * 12.0)).length() < 1e-4);
    assert_eq!(gizmos.line_points[0].y, 2.0);
}

#[test]
fn corner_arcs_for_90_60_and_120_degree_joins_on_both_sides() {
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
