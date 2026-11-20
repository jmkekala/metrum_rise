// SPDX-License-Identifier: GPL-3.0-or-later

//! Round GS (`ROAD-112`) tests: with the zoning grid on, a cursor on an existing road keeps the
//! road's centreline across it and takes `RoadGridSnap`'s grid-aligned position along it (a
//! branch's curbs on cell lines); a node still wins; grid off, the frames `dev` `73173102` gave
//! (a digest recorded there). Round GS2: the grid's one rule (`snap.rs` `grid_rule`) over every
//! start and drag, a road start's drag equal to a cell start's.

use super::labels::zoned_straight_road;
use super::{ack, click, move_to, ray, session, view};
use crate::nodes::sim::road_tool::frame::GizmoLineClass;
use crate::nodes::sim::road_tool::guides::grid_curb_offset_m;
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::pick::lane_width_m;
use crate::nodes::sim::road_tool::session::{RoadToolEvent, RoadToolOption, RoadToolSession};
use crate::nodes::sim::road_tool::snap::{
    SnapInput, SnapKind, SnapOptions, SnapScratch, edge_direction_at, network_snap, resolve, xz_of,
};
use crate::simulation::core::config::WorldConfig;
use crate::simulation::network::guide::DVec2;
use crate::simulation::network::interaction::{NetworkSnapTarget, StructureHit};
use godot::prelude::Vector3;

// Millimetre tolerance of the round's bars.
const MM: f64 = 1e-3;

fn grid_on() -> SnapOptions {
    SnapOptions {
        grid: true,
        ..SnapOptions::default()
    }
}

// The branch types: two-way, four-lane, one-way.
fn branch_types() -> [RoadTypeSpec; 3] {
    [(1, 1), (2, 2), (1, 0)].map(|(fwd_lanes, bkw_lanes)| RoadTypeSpec {
        fwd_lanes,
        bkw_lanes,
        ..RoadTypeSpec::default()
    })
}

// FNV-1a over the frames' Debug strings (every float printed exactly).
fn frames_digest(frames: &[String]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in frames.iter().flat_map(|frame| frame.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

// Hovers over edges, nodes and open ground, then strokes that start and end on roads (the
// commits acknowledged), recording every frame.
fn run_trail(tool: &mut RoadToolSession, frames: &mut Vec<String>) {
    let hovers = [
        (150.4, 1.0),
        (0.3, 0.4),
        (-247.3, 251.9),
        (-80.6, 161.2),
        (40.2, -61.3),
        (73.7, 3.9),
    ];
    for (x, z) in hovers {
        move_to(tool, ray(x, z));
        frames.push(format!("{:?}", tool.frame()));
    }
    let strokes = [
        [(100.3, -0.8), (100.6, -58.7)],
        [(-143.6, 2.1), (-150.2, 120.4)],
        [(213.4, -59.2), (211.9, -3.3)],
    ];
    for [start, end] in strokes {
        click(tool, ray(start.0, start.1));
        frames.push(format!("{:?}", tool.frame()));
        move_to(tool, ray(end.0 + 0.9, end.1 - 1.1));
        frames.push(format!("{:?}", tool.frame()));
        click(tool, ray(end.0, end.1));
        frames.push(format!("{:?}", tool.frame()));
        if tool.take_commit().is_some() {
            ack(tool, true);
            frames.push(format!("{:?}", tool.frame()));
        }
    }
}

// A cursor hovering a straight zoned road (0, 0)-(120, 0), cell columns on x = 10 k, with the
// grid on: the session's candidates (the network snap as the session takes it, the grid's point
// and the road holding it) resolve to that road's centreline (1 mm) at exactly the point
// `RoadGridSnap` gives the old cursor, for every branch type. One of the branch's curbs lies on a
// cell column (1 mm, the snap's rule: "either curb may bound a new corner"), both where the
// branch's curb-to-curb width is a whole number of cells (the two-way road: 10 m); a
// perpendicular branch drawn from the point keeps both curbs on those lines. Near the road's end
// node, the node wins.
#[test]
fn grid_snap_on_a_road_takes_the_centreline_at_the_grid_position() {
    let config = WorldConfig::default();
    let cell_m = f64::from(config.zone_cell_m);
    let (graph, store) = zoned_straight_road(&RoadTypeSpec::default(), &config);
    let grid = store.road_grid_snap(cell_m);
    let mut scratch = SnapScratch::default();
    let on_column = |x: f64| ((x / cell_m) - (x / cell_m).round()).abs() * cell_m < MM;
    let candidates = |raw: DVec2, width: f64, scratch: &mut SnapScratch| {
        let world = Vector3::new(raw.x as f32, 0.0, raw.y as f32);
        let snap = network_snap(
            &graph,
            world,
            StructureHit::Ground,
            None,
            0,
            8.0,
            5.0,
            scratch,
        )
        .expect("the road is in reach");
        let direction = match snap.target {
            NetworkSnapTarget::Edge(edge) => edge_direction_at(&graph, edge, xz_of(snap.position)),
            NetworkSnapTarget::Node(_) => None,
        };
        let (point, on) = grid.snap_road_cursor_on(&graph, raw, None, width);
        SnapInput {
            raw,
            anchor: None,
            reference: None,
            free: false,
            network: Some((snap, direction)),
            grid: Some((point, None)),
            grid_edge: on,
            square: None,
        }
    };
    for spec in branch_types() {
        let width = f64::from(lane_width_m(
            i32::from(spec.fwd_lanes),
            i32::from(spec.bkw_lanes),
        ));
        let offset = grid_curb_offset_m(&spec);
        for x in [23.3, 37.9, 51.2, 64.6, 88.4] {
            for z in [-3.1, 0.8, 2.4] {
                let raw = DVec2::new(x, z);
                let snapped = resolve(&candidates(raw, width, &mut scratch), &grid_on(), &[]);
                let label = format!("{spec:?} at {raw:?}: {snapped:?}");
                assert_eq!(snapped.kind, SnapKind::GridEdge, "{label}");
                assert!(
                    matches!(snapped.target, Some(NetworkSnapTarget::Edge(_))),
                    "{label}"
                );
                assert!(snapped.xz.y.abs() < MM, "on the centreline: {label}");
                let old = grid.snap_road_cursor(&graph, raw, None, width);
                assert!(
                    snapped.xz.distance(old) < MM,
                    "the old cursor's {old:?}: {label}"
                );
                let curbs = [snapped.xz.x - offset, snapped.xz.x + offset].map(on_column);
                assert!(curbs[0] || curbs[1], "a curb on a column: {label}");
                if (2.0 * offset / cell_m).fract() == 0.0 {
                    assert!(curbs[0] && curbs[1], "both curbs on columns: {label}");
                }
                // The perpendicular branch from it: its end on the start's axis, so its curbs run
                // along the same columns.
                let end = grid.snap_road_cursor(
                    &graph,
                    snapped.xz + DVec2::new(0.4, 63.0),
                    Some(snapped.xz),
                    width,
                );
                assert!((end.x - snapped.xz.x).abs() < MM, "{end:?}: {label}");
                assert!(end.y > 50.0, "{end:?}: {label}");
            }
        }
        // The end node wins.
        let snapped = resolve(
            &candidates(DVec2::new(0.6, 0.9), width, &mut scratch),
            &grid_on(),
            &[],
        );
        assert_eq!(snapped.kind, SnapKind::Node, "{spec:?}");
    }
}

// The session with the grid on, on the tests' neighbourhood: hovering the road along z = 0 the
// cursor is `RoadGridSnap`'s point on its centreline (kind `GridEdge`), not the edge's
// projection; at a junction the node wins; a stroke started there and drawn away from the road
// is the grid's (its curbs drawn); a stroke from open ground ending on the road along z = -60
// ends where the grid's axis through its start crosses that centreline.
#[test]
fn session_grid_snap_on_a_road() {
    let width = f64::from(lane_width_m(1, 1));
    let graph = &view().region_graph;
    let mut tool = session();
    tool.set_option(RoadToolOption::ZoningGrid(true));
    tool.set_option(RoadToolOption::GuideLines(false));
    let cursor = |tool: &RoadToolSession| tool.frame().cursor;
    let xz = |p: Vector3| DVec2::new(f64::from(p.x), f64::from(p.z));

    move_to(&mut tool, ray(150.4, 1.3));
    let hover = cursor(&tool);
    assert_eq!(hover.snap, SnapKind::GridEdge);
    let old = view()
        .zoning_grid
        .snap_road_cursor(graph, DVec2::new(150.4, 1.3), None, width);
    assert!(xz(hover.position).distance(old) < MM, "{hover:?} {old:?}");
    assert!(f64::from(hover.position.z).abs() < MM, "{hover:?}");
    assert!(
        (old.x - 150.4).abs() > MM,
        "the grid moves it along the road: {old:?}"
    );

    move_to(&mut tool, ray(0.6, 0.9));
    assert_eq!(cursor(&tool).snap, SnapKind::Node);

    click(&mut tool, ray(150.4, 1.3));
    move_to(&mut tool, ray(151.1, 41.7));
    let away = cursor(&tool);
    assert_eq!(away.snap, SnapKind::Grid);
    assert!((f64::from(away.position.x) - old.x).abs() < MM, "{away:?}");
    assert!(
        tool.frame()
            .gizmos
            .line_classes
            .contains(&GizmoLineClass::GridCurb)
    );

    let mut tool = session();
    tool.set_option(RoadToolOption::ZoningGrid(true));
    tool.set_option(RoadToolOption::GuideLines(false));
    click(&mut tool, ray(100.3, -30.2));
    let start = xz(cursor(&tool).position);
    move_to(&mut tool, ray(100.9, -58.6));
    let end = cursor(&tool);
    assert_eq!(end.snap, SnapKind::GridEdge, "{end:?}");
    assert!(
        (f64::from(end.position.x) - start.x).abs() < MM,
        "{end:?} {start:?}"
    );
    assert!((f64::from(end.position.z) + 60.0).abs() < MM, "{end:?}");
}

// Grid off (the default, and with an angle lock): the frames `dev` `73173102` gave, re-recorded
// by round UI1 (the round's four changes off, the old digest held): its strokes start and end on
// roads, so the frames gain the corner arcs and faint guides and the angle chips move to the
// arcs; with those drawing changes off too the digest still differs, as the 4 m guide reach and
// the square lock snap some of the trail's ends. Re-recorded by round UI2 (with the square forced
// off the old digest `0x9600_a727_6994_283d` held): its 90 degree corners draw the right-angle
// square in place of the arc, drawing only. Re-recorded 2026-11-18 (the user: guides 40 m past
// the cursor, faint from 24 m): drawing only, as the frames without their gizmos hash the same
// under the old and the new lengths (`0xb72e_a82f_7f69_a9a8`). Re-recorded by the icon UI merge
// (round U1): the frame's road type prints the type's pockets (`ROAD-128`), none here; with each
// frame's `pockets` field cut out, the old digest `0xa61a_7aa1_0ae5_fc7d` held.
#[test]
fn grid_off_frames_are_unchanged() {
    let mut frames = Vec::new();
    let mut tool = session();
    run_trail(&mut tool, &mut frames);
    let mut tool = session();
    tool.set_option(RoadToolOption::AngleStepDeg(Some(15.0)));
    run_trail(&mut tool, &mut frames);
    assert!(frames.iter().any(|frame| frame.contains("snap: Edge")));
    assert!(frames.iter().any(|frame| frame.contains("snap: Node")));
    assert_eq!(
        (frames.len(), frames_digest(&frames)),
        (36, 0x9ec0_7cae_572c_1b29)
    );
}

// Round GS2: with the grid on and the default options (guide lines on), a stroke started on the
// road along z = 0 snaps its end exactly as one started in a cell beside it at the same x. A
// sweep on both sides: perpendicular, inside the grid's 5 degree capture, and oblique (10-80
// degrees) and along the road. Wherever `RoadGridSnap` captures the end (the cell start's rule:
// the start's axis, the terminating corner on the lattice), the session's end is that point
// (kind `Grid`, 1 mm), and from either start the snapped end lies on the same lattice row.
#[test]
fn grid_snap_from_a_road_start_matches_a_cell_start() {
    let width = f64::from(lane_width_m(1, 1));
    let graph = &view().region_graph;
    let grid = &view().zoning_grid;
    let xz = |p: Vector3| DVec2::new(f64::from(p.x), f64::from(p.z));
    let start_at = |x: f64, z: f64| {
        let mut tool = session();
        tool.set_option(RoadToolOption::ZoningGrid(true));
        click(&mut tool, ray(x as f32, z as f32));
        let cursor = tool.frame().cursor;
        (tool, xz(cursor.position), cursor.snap)
    };
    let mut misses = Vec::new();
    let mut captured = 0;
    for x in [150.4, 183.7, 216.2] {
        for side in [1.0, -1.0] {
            let (mut road, start, kind) = start_at(x, 0.6 * side);
            assert_eq!(kind, SnapKind::GridEdge, "road start at {x}");
            assert!(start.y.abs() < MM, "{start:?}");
            let (mut cell, cell_start, kind) = start_at(start.x, 15.3 * side);
            assert_eq!(kind, SnapKind::Grid, "cell start at {x}");
            assert!(
                (cell_start.x - start.x).abs() < MM,
                "{cell_start:?} {start:?}"
            );
            for distance in [12.2, 17.9, 24.6, 33.1, 44.7] {
                for degrees in [
                    0.0, 1.3, -2.2, 3.4, -4.6, 10.0, -20.0, 35.0, -45.0, 60.0, -80.0, 90.0,
                ] {
                    let angle = f64::to_radians(degrees);
                    let raw = start + DVec2::new(angle.sin(), side * angle.cos()) * distance;
                    if raw.y * side > 50.0 {
                        continue;
                    }
                    let label = format!("x {x} side {side} at {distance} m, {degrees} deg");
                    let from_road = grid.snap_road_cursor(graph, raw, Some(start), width);
                    move_to(&mut road, ray(raw.x as f32, raw.y as f32));
                    let end = road.frame().cursor;
                    if from_road.distance(raw) > 0.0 && raw.y * side > 8.0 {
                        captured += 1;
                        if end.snap != SnapKind::Grid || xz(end.position).distance(from_road) > MM {
                            misses.push(format!("{label}: {:?} at {:?}", end.snap, end.position));
                        }
                        assert!((from_road.x - start.x).abs() < MM, "{label}");
                    }
                    // The cell start's own rule on the same cursor: the same lattice row.
                    let from_cell = grid.snap_road_cursor(graph, raw, Some(cell_start), width);
                    move_to(&mut cell, ray(raw.x as f32, raw.y as f32));
                    let cell_end = cell.frame().cursor;
                    if from_cell.distance(raw) > 0.0 && (raw.y - cell_start.y) * side > 8.0 {
                        assert_eq!(cell_end.snap, SnapKind::Grid, "cell {label}");
                        assert!(
                            xz(cell_end.position).distance(from_cell) < MM,
                            "cell {label}"
                        );
                        if from_road.distance(raw) > 0.0 {
                            assert!((from_road.y - from_cell.y).abs() < MM, "{label}");
                        }
                    }
                }
            }
        }
    }
    assert!(captured > 40, "{captured}");
    assert!(
        misses.is_empty(),
        "{} of {captured} missed:\n{}",
        misses.len(),
        misses.join("\n")
    );
}

// Where a GS2 case starts: in a cell beside the road along z = 0, on that road, on its dead-end
// node (300, 0), or chained on from a committed stroke drawn off the road.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Start {
    Cell,
    Road,
    Node,
    Chain,
}

// Where a GS2 case drags: 0.9 m off the start's perpendicular out to z = 41.3 on the start's
// side (open ground), 30 degrees off it (open ground), onto the road along z = -60, and the same
// perpendicular with the angle lock (15 degrees) or with Shift.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    Perpendicular,
    Oblique,
    OntoRoad,
    Locked,
    Shift,
}

// A GS2 case's expected end.
#[derive(Clone, Copy, Debug)]
enum Expect {
    // `RoadGridSnap`'s end for the stroke's start (whole cells, the terminating corner), on the
    // start's axis, of this kind.
    Grid(SnapKind),
    // The road along z = -60 where the start's axis crosses it.
    GridEdge,
    // This kind; with the grid on, exactly the result with it off for the drag.
    Kind(SnapKind),
}

// Runs one GS2 case, the grid set to `grid` for the start and to `drag_grid` for the drag: the
// session at its end pose, the stroke's start and the raw end.
fn grid_case(
    start: Start,
    side: f64,
    drag: Drag,
    grid: bool,
    drag_grid: bool,
) -> (RoadToolSession, DVec2, DVec2) {
    let xz = |p: Vector3| DVec2::new(f64::from(p.x), f64::from(p.z));
    let mut tool = session();
    tool.set_option(RoadToolOption::ZoningGrid(grid));
    if drag == Drag::Locked {
        tool.set_option(RoadToolOption::AngleStepDeg(Some(15.0)));
    }
    // The road start's column (the grid's position along the road, or the raw x with it off).
    let road_x = {
        let mut probe = session();
        probe.set_option(RoadToolOption::ZoningGrid(true));
        move_to(&mut probe, ray(183.7, (0.6 * side) as f32));
        probe.frame().cursor.position.x
    };
    match start {
        Start::Cell => click(&mut tool, ray(road_x, (15.3 * side) as f32)),
        Start::Road => click(&mut tool, ray(183.7, (0.6 * side) as f32)),
        Start::Node => click(&mut tool, ray(300.3, 0.4)),
        Start::Chain => {
            click(&mut tool, ray(183.7, (0.6 * side) as f32));
            click(&mut tool, ray(road_x + 0.4, (21.7 * side) as f32));
        }
    }
    // The stroke's start: the last click's cursor (a chain's start is the committed end).
    let from = xz(tool.frame().cursor.position);
    if start == Start::Chain {
        tool.take_commit().expect("the first stroke commits");
        ack(&mut tool, true);
    }
    tool.set_option(RoadToolOption::ZoningGrid(drag_grid));
    let raw = match drag {
        Drag::Oblique => from + DVec2::new(12.5, side * 21.65),
        Drag::OntoRoad => DVec2::new(from.x + 0.9, -59.1),
        _ => DVec2::new(from.x + 0.9, side * 41.3),
    };
    tool.handle_event(
        view(),
        RoadToolEvent::Pointer {
            pointer: ray(raw.x as f32, raw.y as f32),
            shift: drag == Drag::Shift,
            ctrl: false,
        },
    );
    let first = tool.frame().stroke.points[0];
    assert!(xz(first).distance(from) < MM, "{first:?} {from:?}");
    (tool, from, raw)
}

// Round GS2: the zoning grid's one rule over every start (cell, road, node, chained) and drag
// (open ground on the start's axis and oblique, onto a road, onto a node, the angle lock, Shift),
// grid on and off, guide lines on (the default). Grid on: on the start's axis the end is
// `RoadGridSnap`'s, from every start (a lock's ray along that axis keeps it); onto a road, where
// the start's axis crosses it; a node wins; oblique and with Shift exactly the grid-off result.
// A road start and a cell start in the same column give the same ends (1 mm).
#[test]
fn grid_rule_over_every_start_and_drag() {
    use Expect::{Grid, GridEdge, Kind};
    use SnapKind as K;
    let width = f64::from(lane_width_m(1, 1));
    let graph = &view().region_graph;
    let starts = [Start::Cell, Start::Road, Start::Node, Start::Chain];
    // (drag, grid, expected for each start).
    let cases = [
        (Drag::Perpendicular, true, [Grid(K::Grid); 4]),
        // Round UI1: the chain's first stroke ends 3.3 m off the road start's perpendicular, in
        // the guide's 4 m reach, so the chain runs along it and its drag 0.9 m off its axis
        // takes the axis (a guide, the square lock) as a road start's does.
        (
            Drag::Perpendicular,
            false,
            [
                Kind(K::Free),
                Kind(K::Guide),
                Kind(K::Guide),
                Kind(K::Guide),
            ],
        ),
        (Drag::Oblique, true, [Kind(K::Free); 4]),
        (Drag::Oblique, false, [Kind(K::Free); 4]),
        (
            Drag::OntoRoad,
            true,
            [GridEdge, GridEdge, Kind(K::Node), GridEdge],
        ),
        (
            Drag::OntoRoad,
            false,
            [Kind(K::Edge), Kind(K::Edge), Kind(K::Node), Kind(K::Edge)],
        ),
        (Drag::Locked, true, [Grid(K::AngleGrid); 4]),
        (Drag::Locked, false, [Kind(K::Angle); 4]),
        (Drag::Shift, true, [Kind(K::Free); 4]),
        (Drag::Shift, false, [Kind(K::Free); 4]),
    ];
    let xz = |p: Vector3| DVec2::new(f64::from(p.x), f64::from(p.z));
    for (drag, grid, expected) in cases {
        for (start, expect) in starts.into_iter().zip(expected) {
            // South only onto the road along z = -60.
            let sides: &[f64] = if drag == Drag::OntoRoad {
                &[-1.0]
            } else {
                &[1.0, -1.0]
            };
            for &side in sides {
                let (tool, from, raw) = grid_case(start, side, drag, grid, grid);
                let end = tool.frame().cursor;
                let at = xz(end.position);
                let label =
                    format!("{start:?} side {side} {drag:?} grid {grid}: {end:?} from {from:?}");
                match expect {
                    Grid(kind) => {
                        assert_eq!(end.snap, kind, "{label}");
                        let rule =
                            view()
                                .zoning_grid
                                .snap_road_cursor(graph, raw, Some(from), width);
                        assert!(rule.distance(raw) > MM, "the grid captures: {label}");
                        assert!(at.distance(rule) < MM, "{rule:?}: {label}");
                        assert!((at.x - from.x).abs() < MM, "on the start's axis: {label}");
                    }
                    GridEdge => {
                        assert_eq!(end.snap, K::GridEdge, "{label}");
                        assert!(at.distance(DVec2::new(from.x, -60.0)) < MM, "{label}");
                    }
                    Kind(kind) => {
                        assert_eq!(end.snap, kind, "{label}");
                        if grid {
                            // From the same start, the grid off for the drag.
                            let off = grid_case(start, side, drag, true, false).0.frame().cursor;
                            assert_eq!(end, off, "{label}");
                        }
                    }
                }
                if drag == Drag::Shift {
                    assert!(at.distance(raw) < MM, "{label}");
                }
            }
        }
    }
    // A road start and a cell start in the same column: the same ends.
    for drag in [Drag::Perpendicular, Drag::OntoRoad, Drag::Locked] {
        for side in [1.0, -1.0] {
            if drag == Drag::OntoRoad && side > 0.0 {
                continue;
            }
            let [road, cell] = [Start::Road, Start::Cell].map(|start| {
                let (tool, from, _) = grid_case(start, side, drag, true, true);
                (from.x, xz(tool.frame().cursor.position))
            });
            assert!((road.0 - cell.0).abs() < MM, "{drag:?} {side}");
            assert!(
                road.1.distance(cell.1) < MM,
                "{drag:?} {side}: {road:?} {cell:?}"
            );
        }
    }
}
