// SPDX-License-Identifier: GPL-2.0-only

//! Round GS (`ROAD-112`) tests: with the zoning grid on, a cursor on an existing road keeps the
//! road's centreline across it and takes `RoadGridSnap`'s grid-aligned position along it (a
//! branch's curbs on cell lines); a node still wins; grid off, the frames `dev` `73173102` gave
//! (a digest recorded there).

use super::labels::zoned_straight_road;
use super::{ack, click, move_to, ray, session, view};
use crate::nodes::sim::road_tool::frame::GizmoLineClass;
use crate::nodes::sim::road_tool::guides::grid_curb_offset_m;
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::pick::lane_width_m;
use crate::nodes::sim::road_tool::session::{RoadToolOption, RoadToolSession};
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

// Grid off (the default, and with an angle lock): the frames `dev` `73173102` gave.
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
        (36, 0xa963_164f_3707_3a24)
    );
}
