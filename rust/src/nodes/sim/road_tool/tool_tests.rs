// SPDX-License-Identifier: GPL-2.0-only

//! Round B (`ROAD-112`) tests: the snapping precedence of plan section 3 (one test per row and
//! per tie, on the pure resolver), the allocation-free network snap against the acquisition,
//! scripted event sequences against the road tool's published view (the straight two-point
//! request's recorded bits, the curve modes, chain rules) and determinism.

use super::bench::{NEIGHBOURHOOD, neighbourhood_view};
use super::frame::{DraftRefusal, Phase};
use super::measure::{bearing_deg, design_speed_kmh};
use super::modes::DrawMode;
use super::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolOption, RoadToolSession,
};
use super::snap::{
    GuideLine, SnapInput, SnapKind, SnapOptions, SnapScratch, Snapped, apply_length_step,
    collect_guides, network_snap, resolve,
};
use crate::nodes::sim::core::RoadToolQuerySnapshot;
use crate::simulation::network::guide::{DVec2, GuidePath};
use crate::simulation::network::interaction::{
    NetworkSnap, NetworkSnapTarget, StructureHit, get_closest_network_snap_at_hit,
};
use godot::prelude::Vector3;
use std::sync::OnceLock;

mod continuous_chain;
mod grid_snap;
mod ground_marks;
mod heights;
mod labels;
mod scurve_freeform;
mod scurve_steps;
mod ui_guides;

// One small world shared by the scripted tests: the bench neighbourhood, no background.
pub(super) fn view() -> &'static RoadToolQuerySnapshot {
    static VIEW: OnceLock<RoadToolQuerySnapshot> = OnceLock::new();
    VIEW.get_or_init(|| neighbourhood_view(false, 0, 2_048.0))
}

fn d(x: f64, y: f64) -> DVec2 {
    DVec2::new(x, y)
}

fn input(raw: DVec2) -> SnapInput {
    SnapInput {
        raw,
        anchor: Some(DVec2::ZERO),
        reference: None,
        free: false,
        network: None,
        grid: None,
        grid_edge: None,
        square: None,
    }
}

fn node_snap(at: DVec2, id: u32) -> (NetworkSnap, Option<DVec2>) {
    let snap = NetworkSnap {
        position: Vector3::new(at.x as f32, 0.0, at.y as f32),
        target: NetworkSnapTarget::Node(id),
    };
    (snap, None)
}

fn edge_snap(at: DVec2, dir: DVec2) -> (NetworkSnap, Option<DVec2>) {
    let snap = NetworkSnap {
        position: Vector3::new(at.x as f32, 0.0, at.y as f32),
        target: NetworkSnapTarget::Edge(3),
    };
    (snap, Some(dir))
}

fn locked(step: f64) -> SnapOptions {
    SnapOptions {
        angle_step_deg: Some(step),
        ..SnapOptions::default()
    }
}

fn close(a: DVec2, b: DVec2) -> bool {
    a.distance(b) < 1e-9
}

// Row 1: a node wins outright over an edge, a guide line and the angle lock; the lock reports
// itself broken.
#[test]
fn precedence_row_1_node_wins_and_breaks_the_lock() {
    let guide = [GuideLine {
        origin: d(0.0, 10.0),
        dir: DVec2::X,
    }];
    let mut pose = input(d(50.0, 10.5));
    pose.network = Some(node_snap(d(51.0, 9.0), 7));
    let snapped = resolve(&pose, &locked(15.0), &guide);
    assert_eq!(snapped.kind, SnapKind::Node);
    assert_eq!(snapped.target, Some(NetworkSnapTarget::Node(7)));
    assert!(snapped.angle_broken);
    assert!(close(snapped.xz, d(51.0, 9.0)));
    // Without the lock nothing is broken.
    assert!(!resolve(&pose, &SnapOptions::default(), &guide).angle_broken);
}

// Rows 2-3: the angle ray meets an edge within the edge's reach: the intersection, on the edge.
#[test]
fn precedence_row_3_angle_meets_edge_within_reach() {
    // A road along x = 40; the cursor 2 m off the 0 degree ray, 1 m off the road.
    let mut pose = input(d(41.0, 2.0));
    pose.network = Some(edge_snap(d(40.0, 2.0), DVec2::Y));
    let snapped = resolve(&pose, &locked(15.0), &[]);
    assert_eq!(snapped.kind, SnapKind::AngleEdge);
    assert_eq!(snapped.target, Some(NetworkSnapTarget::Edge(3)));
    assert!(close(snapped.xz, d(40.0, 0.0)));
}

// Row 3, otherwise: the intersection out of the edge's reach, so the edge wins over the angle.
#[test]
fn precedence_row_3_edge_wins_over_angle_out_of_reach() {
    let mut pose = input(d(41.0, 9.0));
    pose.network = Some(edge_snap(d(40.0, 9.0), DVec2::Y));
    let snapped = resolve(&pose, &locked(90.0), &[]);
    assert_eq!(snapped.kind, SnapKind::Edge);
    assert!(close(snapped.xz, d(40.0, 9.0)));
}

// Row 3 with a guide line: the intersection within the guide's reach; out of it the angle wins.
#[test]
fn precedence_row_3_angle_and_guide() {
    // A guide line x = 30 (a perpendicular at some node); the 0 degree ray meets it at (30, 0).
    let guide = [GuideLine {
        origin: d(30.0, 50.0),
        dir: DVec2::Y,
    }];
    let near = resolve(&input(d(31.5, 0.5)), &locked(45.0), &guide);
    assert_eq!(near.kind, SnapKind::AngleGuide);
    assert!(close(near.xz, d(30.0, 0.0)));
    // 1.5 m from the guide (in its reach) but the intersection 4.7 m away (round UI1's 4 m
    // reach): the angle wins.
    let far = resolve(&input(d(31.5, 4.5)), &locked(45.0), &guide);
    assert_eq!(far.kind, SnapKind::Angle);
    assert!(close(far.xz, d(31.5, 0.0)));
}

// Row 3 with the zoning grid: the grid's axis through the anchor meets the ray only at the
// anchor; off its direction the angle wins over the grid line, along it they agree.
#[test]
fn precedence_row_3_angle_wins_over_grid() {
    let mut pose = input(d(40.0, 3.0));
    pose.grid = Some((d(40.0, 10.0), Some(d(4.0, 1.0).normalize())));
    let options = SnapOptions {
        grid: true,
        ..locked(15.0)
    };
    let snapped = resolve(&pose, &options, &[]);
    assert_eq!(snapped.kind, SnapKind::Angle);
    assert!(close(snapped.xz, d(40.0, 0.0)));
}

// Row 4: two guide lines in reach and no lock: their corner.
#[test]
fn precedence_row_4_guide_corner() {
    let guides = [
        GuideLine {
            origin: d(0.0, 20.0),
            dir: DVec2::X,
        },
        GuideLine {
            origin: d(35.0, 0.0),
            dir: DVec2::Y,
        },
    ];
    let snapped = resolve(&input(d(34.0, 21.0)), &SnapOptions::default(), &guides);
    assert_eq!(snapped.kind, SnapKind::GuideCorner);
    assert!(close(snapped.xz, d(35.0, 20.0)));
}

// Row 5: one constraint alone projects: a guide, the angle ray, an edge, the grid's point.
#[test]
fn precedence_row_5_single_constraints() {
    let guide = [GuideLine {
        origin: d(0.0, 20.0),
        dir: DVec2::X,
    }];
    let on_guide = resolve(&input(d(12.0, 21.5)), &SnapOptions::default(), &guide);
    assert_eq!(on_guide.kind, SnapKind::Guide);
    assert!(close(on_guide.xz, d(12.0, 20.0)));

    let on_ray = resolve(&input(d(20.0, 19.0)), &locked(45.0), &[]);
    assert_eq!(on_ray.kind, SnapKind::Angle);
    assert!(close(on_ray.xz, d(19.5, 19.5)));

    let mut pose = input(d(41.0, 9.0));
    pose.network = Some(edge_snap(d(40.0, 9.0), DVec2::Y));
    assert_eq!(
        resolve(&pose, &SnapOptions::default(), &[]).kind,
        SnapKind::Edge
    );

    let mut pose = input(d(41.0, 9.0));
    pose.grid = Some((d(40.0, 8.0), None));
    let grid = SnapOptions {
        grid: true,
        ..SnapOptions::default()
    };
    let snapped = resolve(&pose, &grid, &[]);
    assert_eq!(snapped.kind, SnapKind::Grid);
    assert!(close(snapped.xz, d(40.0, 8.0)));
    // Off, the grid is no constraint.
    assert_eq!(
        resolve(&pose, &SnapOptions::default(), &[]).kind,
        SnapKind::Free
    );
}

// Row 2's order: an edge in reach before the grid where it captured the cursor (round GS2),
// the grid before a guide; a grid that left the cursor where it was is no constraint.
#[test]
fn precedence_row_2_line_order() {
    let guide = [GuideLine {
        origin: d(0.0, 10.0),
        dir: DVec2::X,
    }];
    let options = SnapOptions {
        grid: true,
        ..SnapOptions::default()
    };
    let mut pose = input(d(20.0, 10.5));
    pose.network = Some(edge_snap(d(20.0, 11.0), DVec2::X));
    pose.grid = Some((d(24.0, 12.0), None));
    assert_eq!(resolve(&pose, &options, &guide).kind, SnapKind::Edge);
    pose.network = None;
    assert_eq!(resolve(&pose, &options, &guide).kind, SnapKind::Grid);
    pose.grid = Some((pose.raw, None));
    assert_eq!(resolve(&pose, &options, &guide).kind, SnapKind::Guide);
    assert_eq!(resolve(&pose, &options, &[]).kind, SnapKind::Free);
}

// Row 6: the length step rounds a free, angle or guide result from the anchor; a node, an edge
// or an intersection keeps its point.
#[test]
fn precedence_row_6_length_step() {
    let mut free = Snapped {
        xz: d(0.0, 10.7),
        kind: SnapKind::Free,
        target: None,
        stepped: false,
        angle_broken: false,
    };
    apply_length_step(&mut free, Some(DVec2::ZERO), Some(4.0), None);
    assert!(free.stepped && close(free.xz, d(0.0, 12.0)));

    let guide = GuideLine {
        origin: d(-100.0, 5.0),
        dir: DVec2::X,
    };
    let mut on_guide = Snapped {
        xz: d(9.1, 5.0),
        kind: SnapKind::Guide,
        ..free
    };
    apply_length_step(&mut on_guide, Some(DVec2::ZERO), Some(4.0), Some(guide));
    assert!(close(on_guide.xz, d(8.0, 5.0)));

    for kind in [
        SnapKind::Node,
        SnapKind::Edge,
        SnapKind::AngleGuide,
        SnapKind::GuideCorner,
    ] {
        let mut kept = Snapped {
            xz: d(9.1, 5.0),
            kind,
            stepped: false,
            ..free
        };
        apply_length_step(&mut kept, Some(DVec2::ZERO), Some(4.0), Some(guide));
        assert!(!kept.stepped && close(kept.xz, d(9.1, 5.0)), "{kind:?}");
    }
}

// Shift frees the cursor of the angle lock, guide lines and grid; network snaps stay.
#[test]
fn shift_frees_all_but_the_network() {
    let guide = [GuideLine {
        origin: d(0.0, 20.0),
        dir: DVec2::X,
    }];
    let mut pose = input(d(12.0, 21.0));
    pose.free = true;
    assert_eq!(resolve(&pose, &locked(15.0), &guide).kind, SnapKind::Free);
    pose.network = Some(edge_snap(d(12.0, 22.0), DVec2::X));
    assert_eq!(resolve(&pose, &locked(15.0), &guide).kind, SnapKind::Edge);
}

// Tie: two guide lines at exactly the same distance: the earlier candidate wins.
#[test]
fn tie_equal_guides_take_the_earlier_candidate() {
    let guides = [
        GuideLine {
            origin: d(0.0, 1.0),
            dir: DVec2::X,
        },
        GuideLine {
            origin: d(0.0, -1.0),
            dir: DVec2::X,
        },
    ];
    let snapped = resolve(&input(d(10.0, 0.0)), &SnapOptions::default(), &guides);
    assert_eq!(snapped.kind, SnapKind::Guide);
    assert!(close(snapped.xz, d(10.0, 1.0)));
}

// Tie: an angle lock exactly between two steps rounds half away from zero, the same every time.
#[test]
fn tie_angle_half_step_is_fixed() {
    let a = resolve(&input(d(10.0, 10.0)), &locked(90.0), &[]);
    let b = resolve(&input(d(10.0, 10.0)), &locked(90.0), &[]);
    assert_eq!(a, b);
    assert_eq!(a.kind, SnapKind::Angle);
    assert!(close(a.xz, d(0.0, 10.0)));
}

// The session's network snap equals the acquisition (`get_closest_network_snap_at_hit`) bitwise
// (including the lowest-id tie rule and node priority), over a lattice of probe points around the
// neighbourhood.
#[test]
fn network_snap_matches_the_acquisition() {
    let graph = &view().region_graph;
    let mut scratch = SnapScratch::default();
    let mut snapped = 0;
    for ix in -40..=40 {
        for iz in -40..=40 {
            let at = Vector3::new(ix as f32 * 7.7, 0.0, iz as f32 * 7.7);
            let ours = network_snap(
                graph,
                at,
                StructureHit::Ground,
                None,
                0,
                8.0,
                5.0,
                &mut scratch,
            );
            let theirs = get_closest_network_snap_at_hit(graph, at, 5.0, StructureHit::Ground);
            assert_eq!(ours.map(bits), theirs.map(bits), "at {at:?}");
            snapped += usize::from(theirs.is_some());
        }
    }
    assert!(snapped > 100, "the lattice must meet the roads ({snapped})");
}

fn bits(snap: NetworkSnap) -> ([u32; 3], NetworkSnapTarget) {
    let p = snap.position;
    ([p.x.to_bits(), p.y.to_bits(), p.z.to_bits()], snap.target)
}

// The guide lines of the spur's dead end: its extension and the perpendicular there.
#[test]
fn guides_include_dead_end_extension_and_perpendicular() {
    let mut scratch = SnapScratch::default();
    let end = d(-40.0, 200.0);
    collect_guides(
        &view().region_graph,
        end + d(10.0, 10.0),
        None,
        None,
        None,
        &mut scratch,
    );
    let diagonal = d(1.0, 1.0).normalize();
    let through = |dir: DVec2| {
        scratch
            .guides
            .iter()
            .any(|guide| guide.origin.distance(end) < 1e-3 && guide.dir.perp_dot(dir).abs() < 1e-6)
    };
    assert!(through(diagonal), "extension");
    assert!(through(diagonal.perp()), "perpendicular");
}

// --- Scripted sessions -------------------------------------------------------------------------

fn session() -> RoadToolSession {
    RoadToolSession::new()
}

// A vertical camera ray onto `(x, z)`.
fn ray(x: f32, z: f32) -> Pointer {
    Pointer::Ray {
        origin: Vector3::new(x, 300.0, z),
        dir: Vector3::new(0.0, -1.0, 0.0),
    }
}

fn move_to(session: &mut RoadToolSession, pointer: Pointer) {
    session.handle_event(
        view(),
        RoadToolEvent::Pointer {
            pointer,
            shift: false,
            ctrl: false,
        },
    );
}

fn click(session: &mut RoadToolSession, pointer: Pointer) {
    move_to(session, pointer);
    session.handle_event(view(), RoadToolEvent::Press(PointerButton::Left));
}

fn ack(session: &mut RoadToolSession, committed: bool) {
    session.handle_event(view(), RoadToolEvent::CommitAck { committed });
}

// The straight two-point request, recorded (round R2-7): for clicks where no snap differs (open
// ground, a junction node, an edge mid-span, a dead end) the session sends exactly these bits.
// Recorded on `dev` `17207c94`, where they equalled the old cursor query's (`road_tool_cursor`,
// removed in R2-7) two points bit for bit.
#[test]
fn straight_mode_sends_the_recorded_two_point_request() {
    const SENT: [[[u32; 3]; 2]; 4] = [
        [
            [0xc3480000, 0x00000000, 0x43160000],
            [0xc3160000, 0x00000000, 0x43820000],
        ],
        [
            [0x00000000, 0x00000000, 0x00000000],
            [0xc2a00000, 0x00000000, 0x42b40000],
        ],
        [
            [0xc3520000, 0x00000000, 0x42f00000],
            [0x43166666, 0x00000000, 0x00000000],
        ],
        [
            [0xc3480000, 0x00000000, 0x437a0000],
            [0xc2200000, 0x00000000, 0x43480000],
        ],
    ];
    let cases = [
        (ray(-200.0, 150.0), ray(-150.0, 260.0)), // free -> free
        (ray(0.5, 0.5), ray(-80.0, 90.0)),        // junction node -> free
        (ray(-210.0, 120.0), ray(150.4, 1.0)),    // free -> edge mid-span
        (ray(-200.0, 250.0), ray(-41.0, 199.0)),  // free -> the spur's dead end
    ];
    for ((a, b), sent) in cases.into_iter().zip(SENT) {
        let mut tool = session();
        click(&mut tool, a);
        click(&mut tool, b);
        let commit = tool.take_commit().expect("a stroke");
        let as_bits = |p: &[Vector3]| {
            p.iter()
                .map(|v| [v.x, v.y, v.z].map(f32::to_bits))
                .collect::<Vec<_>>()
        };
        assert_eq!(as_bits(&commit.points), sent, "{a:?} -> {b:?}");
        assert_eq!(tool.frame().state.phase, Phase::AwaitingAck);
    }
}

// The old soft 45 degree magnet goes under U2: a stroke 3 degrees off a dead end's direction
// stays where the pointer is with the lock and guide lines off (the recorded raw point; the
// magnet snapped it; round UI1's square lock takes it onto the dead end's axis with guide lines
// on), and the 45 degree lock gives the magnet's direction (its point recorded on `dev`
// `17207c94` from the old cursor query, round R2-7).
#[test]
fn straight_mode_magnet_cases_under_u2() {
    const RAW: [u32; 3] = [0x41587cb4, 0x00000000, 0x4381b9ce];
    const MAGNET: [u32; 3] = [0x41848c62, 0x00000000, 0x438048c6];
    let start_ray = ray(-41.0, 199.0); // the spur's dead end, continuing north-east
    let aim = |degrees: f32| {
        let angle = (45.0_f32 + degrees).to_radians();
        ray(-40.0 + 80.0 * angle.cos(), 200.0 + 80.0 * angle.sin())
    };
    let off = aim(3.0);
    let mut square = session();
    click(&mut square, start_ray);
    move_to(&mut square, off);
    assert_eq!(square.frame().cursor.snap, SnapKind::Guide);
    let held = square.frame().cursor.position;
    let axis = DVec2::new(1.0, 1.0).normalize();
    let first = square.frame().stroke.points[0];
    let along = DVec2::new(f64::from(held.x - first.x), f64::from(held.z - first.z));
    assert!(along.perp_dot(axis).abs() < 0.05, "{along:?}");
    let mut tool = session();
    tool.set_option(RoadToolOption::GuideLines(false));
    click(&mut tool, start_ray);
    move_to(&mut tool, off);
    let start = tool.frame().stroke.points[0];
    let free = tool.frame().cursor.position;
    assert_eq!(
        [free.x, free.y, free.z].map(f32::to_bits),
        RAW,
        "lock off: the raw cursor"
    );
    let [x, y, z] = MAGNET.map(f32::from_bits);
    let magnet = Vector3::new(x, y, z);
    tool.set_option(RoadToolOption::AngleStepDeg(Some(45.0)));
    move_to(&mut tool, off);
    let locked = tool.frame().cursor.position;
    let direction =
        |p: Vector3| DVec2::new(f64::from(p.x - start.x), f64::from(p.z - start.z)).normalize();
    assert!(direction(locked).distance(direction(magnet)) < 1e-5);
    assert_eq!(tool.frame().cursor.snap, SnapKind::Angle);
}

// Chain rules: an accepted stroke ending on open ground starts the next at its end with its
// end tangent; one ending on the network ends the chain; so does the map edge; a cancel while
// waiting ends it; a refusal keeps the stroke editable.
#[test]
fn chain_rules() {
    // Open ground: chains.
    let mut tool = session();
    click(&mut tool, ray(-200.0, 150.0));
    click(&mut tool, ray(-150.0, 260.0));
    let end = tool.take_commit().unwrap().points[1];
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
    move_to(&mut tool, ray(-100.0, 300.0));
    assert_eq!(tool.frame().stroke.points[0], end);
    // The carried tangent is the angle reference: a 90 degree lock along it gives straight on.
    tool.set_option(RoadToolOption::AngleStepDeg(Some(90.0)));
    move_to(&mut tool, ray(-120.0, 330.0));
    let next = tool.frame().stroke.points[1];
    let carried = DVec2::new(50.0, 110.0).normalize();
    let along = DVec2::new(f64::from(next.x - end.x), f64::from(next.z - end.z)).normalize();
    assert!(along.distance(carried) < 1e-4);

    // Ending on an edge: no chain.
    let mut tool = session();
    click(&mut tool, ray(-210.0, 120.0));
    click(&mut tool, ray(150.4, 1.0));
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Idle);

    // Ending on the map edge (the 2,048 m world's east edge): no chain.
    let mut tool = session();
    click(&mut tool, ray(900.0, 500.0));
    click(&mut tool, ray(1_015.0, 500.0));
    assert_eq!(tool.frame().cursor.snap, SnapKind::Border);
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Idle);

    // Cancel while waiting: the stroke lands, the chain ends.
    let mut tool = session();
    click(&mut tool, ray(-200.0, 150.0));
    click(&mut tool, ray(-150.0, 260.0));
    tool.handle_event(view(), RoadToolEvent::Press(PointerButton::Right));
    assert_eq!(tool.frame().state.phase, Phase::AwaitingAck);
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Idle);

    // A refusal keeps the stroke editable at its last point; clicks wait for the ack.
    let mut tool = session();
    click(&mut tool, ray(-200.0, 150.0));
    click(&mut tool, ray(-150.0, 260.0));
    let sent = tool.take_commit().unwrap();
    click(&mut tool, ray(-100.0, 250.0));
    assert_eq!(tool.take_commit(), None, "no second stroke while waiting");
    ack(&mut tool, false);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
    assert_eq!(tool.frame().stroke.points[0], sent.points[0]);
    tool.handle_event(view(), RoadToolEvent::Action(RoadToolAction::Cancel));
    assert_eq!(tool.frame().state.phase, Phase::Idle);
}

// Simple curve (U1): three clicks; the request is the fillet's flattening, ends bit for bit, and
// the HUD radius is T / tan(delta / 2).
#[test]
fn simple_curve_three_clicks() {
    let mut tool = session();
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
    );
    click(&mut tool, ray(-250.0, 100.0));
    click(&mut tool, ray(-150.0, 100.0));
    move_to(&mut tool, ray(-150.0, 160.0));
    let frame = tool.frame();
    assert_eq!(frame.state.mode, DrawMode::SimpleCurve);
    let points = &frame.stroke.points;
    assert!(points.len() > 10);
    // Tangent length 60 m, a quarter turn: R = 60 / tan(45 degrees).
    let radius = frame.hud.radius_m.unwrap();
    assert!((radius - 60.0).abs() < 1e-6, "{radius}");
    assert_eq!(frame.hud.design_speed_kmh, Some(design_speed_kmh(radius)));
    let mut path = GuidePath::new();
    path.set_fillet(d(-250.0, 100.0), d(-150.0, 100.0), d(-150.0, 160.0))
        .unwrap();
    let mut flat = Vec::new();
    path.flatten_into(&mut flat).unwrap();
    assert_eq!(points.len(), flat.len());
    for (point, expected) in points.iter().zip(&flat).skip(1).take(flat.len() - 2) {
        assert_eq!((point.x, point.z), (expected.x as f32, expected.y as f32));
    }
    let start = points[0];
    click(&mut tool, ray(-150.0, 160.0));
    let commit = tool.take_commit().unwrap();
    assert_eq!(commit.points[0], start);
    assert_eq!(commit.points.last(), tool.frame().stroke.points.last());
}

// Complex curve: four clicks make the biarc; parallel tangents pointing apart are refused.
#[test]
fn complex_curve_biarc_and_refusal() {
    let mut tool = session();
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeComplexCurve),
    );
    click(&mut tool, ray(-250.0, 100.0));
    click(&mut tool, ray(-220.0, 100.0));
    click(&mut tool, ray(-150.0, 160.0));
    move_to(&mut tool, ray(-120.0, 160.0));
    let frame = tool.frame();
    assert_eq!(frame.state.refusal, None);
    assert_eq!(frame.hud.end_angle_deg, None);
    assert!(frame.stroke.points.len() > 10);
    // The S-curve leaves east and arrives east.
    assert!((frame.hud.bearing_deg - 90.0).abs() < 1e-6);
    click(&mut tool, ray(-120.0, 160.0));
    assert!(tool.take_commit().is_some());

    let mut tool = session();
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeComplexCurve),
    );
    click(&mut tool, ray(-250.0, 100.0));
    click(&mut tool, ray(-270.0, 100.0));
    click(&mut tool, ray(-110.0, 100.0));
    move_to(&mut tool, ray(-130.0, 100.0));
    assert_eq!(tool.frame().state.refusal, Some(DraftRefusal::NoBiarc));
    click(&mut tool, ray(-130.0, 100.0));
    assert!(tool.take_commit().is_none());
}

// Continuous: the next stroke of a chain leaves along the carried tangent as an arc through the
// cursor, so its start tangent is the previous stroke's end tangent.
#[test]
fn continuous_chain_carries_the_tangent() {
    let mut tool = session();
    tool.set_option(RoadToolOption::Continuous(true));
    click(&mut tool, ray(-250.0, 100.0));
    click(&mut tool, ray(-200.0, 100.0));
    ack(&mut tool, true);
    move_to(&mut tool, ray(-150.0, 150.0));
    let frame = tool.frame();
    let points = &frame.stroke.points;
    let first = DVec2::new(
        f64::from(points[1].x - points[0].x),
        f64::from(points[1].z - points[0].z),
    );
    assert!(
        first.normalize().perp_dot(DVec2::X).abs() < 0.05,
        "leaves east"
    );
    assert!((frame.hud.radius_m.unwrap() - 50.0).abs() < 1e-3);
    assert_eq!(frame.hud.start_angle_deg, Some(0.0));
}

// Determinism: the same events twice give equal frames, bitwise.
#[test]
fn same_events_give_bitwise_equal_frames() {
    let run = || {
        let mut tool = session();
        tool.set_option(RoadToolOption::AngleStepDeg(Some(15.0)));
        tool.set_option(RoadToolOption::LengthStepM(Some(4.0)));
        tool.handle_event(
            view(),
            RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
        );
        let mut frames = Vec::new();
        for step in 0..60 {
            let t = step as f32 * 0.21;
            let pointer = ray(-150.0 + 120.0 * t.cos(), 40.0 + 120.0 * t.sin());
            if step % 20 == 0 {
                click(&mut tool, pointer);
            } else {
                move_to(&mut tool, pointer);
            }
            frames.push(format!("{:?}", tool.frame()));
        }
        frames
    };
    assert_eq!(run(), run());
}

#[test]
fn hud_closed_forms() {
    assert_eq!(design_speed_kmh(60.0), 35.0);
    assert_eq!(bearing_deg(d(0.0, -1.0)), 0.0);
    assert_eq!(bearing_deg(d(1.0, 0.0)), 90.0);
    assert_eq!(bearing_deg(d(0.0, 1.0)), 180.0);
    assert_eq!(bearing_deg(d(-1.0, 0.0)), 270.0);
    // A 1 + 1 standard road's cost and upkeep for a 100 m straight.
    let mut tool = session();
    click(&mut tool, ray(-250.0, 100.0));
    move_to(&mut tool, ray(-150.0, 100.0));
    let hud = tool.frame().hud;
    assert!((hud.length_m - 100.0).abs() < 1e-3);
    let weight = crate::simulation::network::road_types::cost::stroke_weight(
        (1, 1),
        None,
        Default::default(),
        crate::simulation::network::road_types::RoadAttributes::DEFAULT,
        crate::simulation::network::types::EdgeClass::Standard,
    );
    let per_m = crate::simulation::network::road_types::cost::build_per_m(weight);
    assert!((hud.build_cost - hud.length_m * per_m).abs() < 1e-9);
    assert_eq!(NEIGHBOURHOOD.len(), 7);
}

// Round C (`ROAD-112`): the HUD prices the selected road type (B9) as the preview prices the
// same stroke: the per-metre build price of the preview's prepared input (its median, parking,
// attributes and class) and the session's lanes, for a type that is not the default road.
#[test]
fn hud_prices_the_selected_type_as_the_preview() {
    use crate::simulation::network::graph::{RoadMedian, RoadParking};
    use crate::simulation::network::road_edit::plan_road_edit;
    use crate::simulation::network::road_types::RoadAttributes;
    use crate::simulation::network::road_types::cost::{build_per_m, stroke_weight};
    use super::measure::RoadTypeSpec;
    let spec = RoadTypeSpec {
        fwd_lanes: 2,
        bkw_lanes: 2,
        median: RoadMedian::from_tool(1, 2.0).ok().flatten(),
        parking: RoadParking::from_code(3).expect("both kerbs"),
        attributes: RoadAttributes::from_tool(70, 3, true, false).expect("attributes"),
    };
    assert!(spec.median.is_some());
    let mut tool = session();
    tool.set_option(RoadToolOption::RoadType(spec));
    click(&mut tool, ray(-250.0, 100.0));
    move_to(&mut tool, ray(-150.0, 120.0));
    let frame = tool.frame();
    // The preview's plan (`RoadEditPlan::compile_from`'s preparation): its prepared input prices
    // the preview (`preview.rs` `stroke_cost_weight`).
    let (prepared, validation, _) = plan_road_edit(
        &frame.stroke.points,
        spec.fwd_lanes,
        spec.bkw_lanes,
        (spec.median, spec.parking, spec.attributes),
        &view().terrain,
        &view().region_graph,
        &view().roadbed,
        &crate::simulation::network::road_edit::NoFrontages,
        frame.stroke.snap_to_existing_roads,
        crate::simulation::network::road_edit::RoadProfile::FollowGround,
    );
    assert!(validation.is_valid, "{}", validation.invalid_reason);
    let preview_per_m = build_per_m(stroke_weight(
        (spec.fwd_lanes, spec.bkw_lanes),
        prepared.median,
        prepared.parking,
        prepared.attributes,
        prepared.class,
    ));
    let hud = frame.hud;
    assert!(
        (hud.build_cost / hud.length_m - preview_per_m).abs() < 1e-9 * preview_per_m,
        "HUD {} / m, preview {} / m",
        hud.build_cost / hud.length_m,
        preview_per_m
    );
    // The default road's price differs, so the check above is the type's.
    let mut plain = session();
    click(&mut plain, ray(-250.0, 100.0));
    move_to(&mut plain, ray(-150.0, 120.0));
    let plain_hud = plain.frame().hud;
    assert!(plain_hud.build_cost / plain_hud.length_m < 0.9 * preview_per_m);
}

// Round C: a scripted pointer is the cursor exactly (today's `set_scripted_pointer`), even where
// a snap would take a picked or world pointer, and the stroke is its two points bit for bit.
#[test]
fn scripted_pointer_is_taken_exactly() {
    let mut tool = session();
    let start = Vector3::new(0.3, 0.7, 0.4); // inside the junction's node snap
    let end = Vector3::new(150.25, 1.5, 0.75); // on the east road
    click(&mut tool, Pointer::Scripted(start));
    click(&mut tool, Pointer::Scripted(end));
    let commit = tool.take_commit().expect("a stroke");
    assert_eq!(commit.points, vec![start, end]);
    // The chain goes on from the exact end: no network target was taken.
    ack(&mut tool, true);
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 1 });
}

// Round C: cancelling while the stroke waits for its acknowledgement ends it whatever the answer;
// a refusal without the cancel keeps it editable.
#[test]
fn cancel_while_pending_survives_a_refusal() {
    for cancel in [false, true] {
        let mut tool = session();
        click(&mut tool, ray(-250.0, 100.0));
        click(&mut tool, ray(-150.0, 120.0));
        assert!(tool.take_commit().is_some());
        if cancel {
            tool.handle_event(view(), RoadToolEvent::Action(RoadToolAction::Cancel));
        }
        ack(&mut tool, false);
        let expected = if cancel {
            Phase::Idle
        } else {
            Phase::Placing { placed: 1 }
        };
        assert_eq!(tool.frame().state.phase, expected, "cancel {cancel}");
    }
}

// --- Round D (`ROAD-112`): curves on the session, the radius rule, gizmos ----------------------

// The ground height under `(x, z)`, picked by a vertical ray on open ground.
fn ground_at(x: f32, z: f32) -> f32 {
    let mut probe = session();
    move_to(&mut probe, ray(x, z));
    probe.frame().cursor.position.y
}

// A simple curve placed with exact (scripted) clicks at `a`, `c`, `b` on the ground; the frame
// is left at the end's pose, not yet confirmed.
fn scripted_simple_curve(a: (f64, f64), c: (f64, f64), b: (f64, f64)) -> RoadToolSession {
    let y = ground_at(a.0 as f32, a.1 as f32);
    let at = |p: (f64, f64)| Pointer::Scripted(Vector3::new(p.0 as f32, y, p.1 as f32));
    let mut tool = session();
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
    );
    click(&mut tool, at(a));
    click(&mut tool, at(c));
    move_to(&mut tool, at(b));
    tool
}

// The HUD's radius is the constructed fillet's, `T / tan(delta / 2)`, within 1 mm: legs of 70 m
// and 50 m (T = 50 m) turning by atan(4 / 3), so R = 50 / tan(atan(1 / 2)) = 100 m exactly.
#[test]
fn drawn_curve_reports_its_constructed_radius() {
    let tool = scripted_simple_curve((-280.0, 200.0), (-210.0, 200.0), (-180.0, 240.0));
    let frame = tool.frame();
    assert_eq!(frame.state.refusal, None);
    let (a, c, b) = (d(-280.0, 200.0), d(-210.0, 200.0), d(-180.0, 240.0));
    let (leg_in, leg_out) = (c - a, b - c);
    let tangent = leg_in.length().min(leg_out.length());
    let delta = leg_in.perp_dot(leg_out).abs().atan2(leg_in.dot(leg_out));
    let constructed = tangent / (delta / 2.0).tan();
    assert!((constructed - 100.0).abs() < 1e-9, "{constructed}");
    let radius = frame.hud.radius_m.expect("an arc");
    assert!(
        (radius - constructed).abs() < 1e-3,
        "HUD {radius} m, constructed {constructed} m"
    );
    assert_eq!(frame.hud.design_speed_kmh, Some(design_speed_kmh(radius)));
    assert!(!frame.hud.tight_radius);
}

// The radius rule (plan section 5, `road_tight_curve`): the tool refuses a draft exactly when its
// arc's radius is under the illegal radius, and the radius check, which reads a flattened arc's
// radius exactly since round E, never refuses a draft the tool sends. Quarter turns (shorter
// than the check's window) and 170 degree turns (longer), with and without a lead, from 0.5 m
// below the limit to 0.5 m above it. The clicks are `f32`, so the drawn radius is the limit plus
// the offset within about 1e-5 relative: the HUD's radius decides.
#[test]
fn tight_curve_refusal_follows_the_radius_check() {
    use super::measure::{RoadTypeSpec, illegal_radius_m};
    use crate::simulation::network::road_edit::plan_road_edit;
    let spec = RoadTypeSpec::default();
    let limit = illegal_radius_m(&spec);
    assert!(limit > 1.0, "{limit}");
    let (x0, z0) = (-280.0, 220.0);
    for deflection_deg in [90.0f64, 170.0] {
        let deflection = deflection_deg.to_radians();
        for lead in [0.0, 3.0] {
            for offset in [-0.5, -1e-3, 0.0, 1e-6 * limit, 1e-3, 0.02, 0.5] {
                let radius = limit + offset;
                // Legs `T` and `T + lead`: `R = T / tan(delta / 2)`.
                let leg = radius * (0.5 * deflection).tan();
                let c = (x0 + leg + lead, z0);
                let b = (c.0 + leg * deflection.cos(), c.1 + leg * deflection.sin());
                let tool = scripted_simple_curve((x0, z0), c, b);
                let frame = tool.frame();
                let hud = frame.hud;
                let hud_radius = hud.radius_m.expect("an arc");
                assert!(
                    (hud_radius - radius).abs() < 1e-4 * radius,
                    "{hud_radius} vs {radius}"
                );
                assert_eq!(hud.illegal_radius_m, limit);
                let case = format!(
                    "{deflection_deg} degrees, lead {lead}, radius {radius} (drawn {hud_radius}, \
                     limit {limit})"
                );
                assert_eq!(hud.tight_radius, hud_radius < limit, "{case}");
                if offset < -1e-4 * limit {
                    assert!(hud.tight_radius, "{case}: below the limit is always refused");
                }
                if hud.tight_radius {
                    assert_eq!(
                        frame.state.refusal,
                        Some(DraftRefusal::TightCurve),
                        "{case}"
                    );
                    assert!(frame.stroke.points.is_empty(), "{case}");
                    continue;
                }
                assert_eq!(frame.state.refusal, None, "{case}");
                let (_, validation, _) = plan_road_edit(
                    &frame.stroke.points,
                    spec.fwd_lanes,
                    spec.bkw_lanes,
                    (spec.median, spec.parking, spec.attributes),
                    &view().terrain,
                    &view().region_graph,
                    &view().roadbed,
                    &crate::simulation::network::road_edit::NoFrontages,
                    frame.stroke.snap_to_existing_roads,
                    crate::simulation::network::road_edit::RoadProfile::FollowGround,
                );
                assert!(
                    validation.is_valid,
                    "{case}: {} measured {} limit {}",
                    validation.invalid_reason,
                    validation.geometry_measured,
                    validation.geometry_limit
                );
            }
        }
    }
}

// Gizmos: the placed start, the control point and the live end as handles; the draft's
// centreline and, for a curve, its two tangent legs; the free start's height grip on its stem
// (round I). A chained stroke's start is the chain start, locked: no grip.
#[test]
fn gizmos_name_the_handles_and_legs() {
    use super::frame::{GizmoHandleKind, GizmoLineClass};
    let tool = scripted_simple_curve((-280.0, 200.0), (-210.0, 200.0), (-180.0, 240.0));
    let gizmos = &tool.frame().gizmos;
    assert_eq!(
        gizmos.handle_kinds,
        [
            GizmoHandleKind::Endpoint,
            GizmoHandleKind::Control,
            GizmoHandleKind::Cursor,
            GizmoHandleKind::HeightGrip
        ]
    );
    // Round V: the zoning-depth guide's runs follow (outside and inside); round I: the start's
    // height stem.
    assert_eq!(
        gizmos.line_classes[..2],
        [GizmoLineClass::Tangent, GizmoLineClass::Draft]
    );
    assert!(gizmos.line_classes[2..].iter().all(|class| matches!(
        class,
        GizmoLineClass::ZoningOutside | GizmoLineClass::ZoningInside | GizmoLineClass::HeightStem
    )));
    assert_eq!(
        gizmos
            .line_classes
            .iter()
            .filter(|&&class| class == GizmoLineClass::HeightStem)
            .count(),
        1
    );
    assert_eq!(gizmos.line_starts[0], 0);
    assert_eq!(gizmos.line_starts[1], 3);
    assert_eq!(
        gizmos.line_starts[2] as usize - 3,
        tool.frame().stroke.points.len()
    );

    let y = ground_at(-280.0, 200.0);
    let at = |x: f32, z: f32| Pointer::Scripted(Vector3::new(x, y, z));
    let mut chain = session();
    click(&mut chain, at(-280.0, 200.0));
    click(&mut chain, at(-240.0, 200.0));
    ack(&mut chain, true);
    move_to(&mut chain, at(-220.0, 230.0));
    let gizmos = &chain.frame().gizmos;
    assert_eq!(
        gizmos.handle_kinds,
        [GizmoHandleKind::ChainStart, GizmoHandleKind::Cursor]
    );
    assert_eq!(gizmos.line_classes[0], GizmoLineClass::Draft);
    // Round UI1: the chained start's corner arc, then the zoning guide.
    assert_eq!(gizmos.line_classes[1], GizmoLineClass::CornerArc);
    assert!(gizmos.line_classes[2..].iter().all(|class| matches!(
        class,
        GizmoLineClass::ZoningOutside | GizmoLineClass::ZoningInside
    )));
}

// Continuous applies to the simple curve (B8): the control point is held on the carried tangent's
// forward ray, so the chained curve leaves along the previous stroke.
#[test]
fn continuous_simple_curve_holds_the_control_on_the_tangent() {
    let y = ground_at(-280.0, 200.0);
    let at = |x: f32, z: f32| Pointer::Scripted(Vector3::new(x, y, z));
    let mut tool = session();
    tool.set_option(RoadToolOption::Continuous(true));
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
    );
    click(&mut tool, at(-280.0, 200.0));
    click(&mut tool, at(-260.0, 200.0));
    click(&mut tool, at(-240.0, 220.0));
    ack(&mut tool, true);
    // The carried tangent leaves north-east (45 degrees); a control click off that ray lands on it.
    click(&mut tool, at(-200.0, 240.0));
    move_to(&mut tool, at(-190.0, 280.0));
    let frame = tool.frame();
    assert_eq!(frame.state.refusal, None);
    let points = &frame.stroke.points;
    let first = DVec2::new(
        f64::from(points[1].x - points[0].x),
        f64::from(points[1].z - points[0].z),
    )
    .normalize();
    let carried = DVec2::new(1.0, 1.0).normalize();
    assert!(
        first.perp_dot(carried).abs() < 1e-3,
        "leaves along {first:?}"
    );
    assert_eq!(frame.hud.start_angle_deg.map(|a| a < 0.1), Some(true));
    // The current mode again keeps the stroke (the mode buttons send it on every press).
    tool.handle_event(
        view(),
        RoadToolEvent::Action(RoadToolAction::ModeSimpleCurve),
    );
    assert_eq!(tool.frame().state.phase, Phase::Placing { placed: 2 });
}

// Round E: each snapping toggle, as the options row sends it, removes its constraint on the next
// pose and puts it back when switched on again. Network snap: an edge mid-span. Guide lines: the
// spur's dead-end extension, 1 m off it. Zoning grid: a stroke 0.7 m off its start's east axis on
// open ground (no road in reach: the grid's cardinal steps). Angle lock: a stroke 6.8 degrees off
// east from its start (`0` = free, as the UI sends).
// Length step: a free end 52.5 m from the start (`0` = off). Off, the cursor is the raw pick.
#[test]
fn each_snapping_toggle_off_removes_its_constraint() {
    use super::frame::GizmoLineClass;
    let posed = |options: &[RoadToolOption], start: Option<Pointer>, at: Pointer| {
        let mut tool = session();
        for option in options {
            tool.set_option(*option);
        }
        if let Some(start) = start {
            click(&mut tool, start);
        }
        move_to(&mut tool, at);
        tool
    };
    let raw = |at: Pointer| {
        let Pointer::Ray { origin, .. } = at else {
            unreachable!()
        };
        (origin.x, origin.z)
    };
    let xz = |p: Vector3| (p.x, p.z);
    let (edge, open) = (ray(150.4, 1.0), ray(-247.3, 251.9));
    let guide = ray(-18.8 + 0.707, 221.2 - 0.707);
    let (aim, far) = (ray(-197.3, 257.9), ray(-207.6, 289.0));
    let near_east = ray(-197.3, 252.6);
    let cases: [(RoadToolOption, RoadToolOption, Option<Pointer>, Pointer, SnapKind); 5] = [
        (
            RoadToolOption::NetworkSnap(true),
            RoadToolOption::NetworkSnap(false),
            None,
            edge,
            SnapKind::Edge,
        ),
        (
            RoadToolOption::GuideLines(true),
            RoadToolOption::GuideLines(false),
            Some(ray(-250.0, 150.0)),
            guide,
            SnapKind::Guide,
        ),
        (
            RoadToolOption::ZoningGrid(true),
            RoadToolOption::ZoningGrid(false),
            Some(open),
            near_east,
            SnapKind::Grid,
        ),
        (
            RoadToolOption::AngleStepDeg(Some(15.0)),
            RoadToolOption::AngleStepDeg(Some(0.0)),
            Some(open),
            aim,
            SnapKind::Angle,
        ),
        (
            RoadToolOption::LengthStepM(Some(5.0)),
            RoadToolOption::LengthStepM(Some(0.0)),
            Some(open),
            far,
            SnapKind::Free,
        ),
    ];
    for (on, off, start, at, kind) in cases {
        // The grid's row turns the guides off: on, the grid takes this cursor before the start's
        // own east-west guide (round GS2), but switched off that guide would take it.
        let base = match on {
            RoadToolOption::ZoningGrid(_) => RoadToolOption::GuideLines(false),
            _ => RoadToolOption::GuideLines(true),
        };
        let pose = |options: &[RoadToolOption], start, at| {
            let mut all = vec![base];
            all.extend_from_slice(options);
            posed(&all, start, at)
        };
        let tool = pose(&[on], start, at);
        let cursor = tool.frame().cursor;
        assert_eq!(cursor.snap, kind, "{on:?}");
        assert_ne!(xz(cursor.position), raw(at), "{on:?} moves the cursor");
        if let RoadToolOption::LengthStepM(_) = on {
            assert!(cursor.stepped, "{on:?}");
            let from = tool.frame().stroke.points[0];
            let length = f64::from((cursor.position - from).length());
            assert!((length / 5.0 - (length / 5.0).round()).abs() < 1e-4, "{length}");
        }
        // Switched off (after on, as a toggle does), the pick is the raw cursor.
        let tool = pose(&[on, off], start, at);
        let cursor = tool.frame().cursor;
        assert_eq!(cursor.snap, SnapKind::Free, "{off:?}");
        assert!(!cursor.stepped, "{off:?}");
        assert_eq!(xz(cursor.position), raw(at), "{off:?}");
        // On again.
        assert_eq!(pose(&[on, off, on], start, at).frame().cursor.snap, kind);
    }
    // The gizmos draw what took the cursor: the angle lock's ray from the start through the
    // cursor and on past it (round V: `GUIDE_PAST_M`); the guide line through the cursor.
    let locked = posed(&[RoadToolOption::AngleStepDeg(Some(15.0))], Some(open), aim);
    let gizmos = &locked.frame().gizmos;
    let ray_strip = gizmos
        .line_classes
        .iter()
        .position(|class| *class == GizmoLineClass::Angle)
        .expect("the angle ray");
    let ray_start = gizmos.line_starts[ray_strip] as usize;
    let cursor = locked.frame().cursor.position;
    let strip = &gizmos.line_points[ray_start..ray_start + 2];
    let past = super::guides::GUIDE_PAST_M as f32;
    assert!((strip[1].x - (cursor.x + past)).abs() < 1e-3 && (strip[1].z - cursor.z).abs() < 1e-3);
    let guided = posed(&[], Some(ray(-250.0, 150.0)), guide);
    assert!(
        guided
            .frame()
            .gizmos
            .line_classes
            .contains(&GizmoLineClass::Guide)
    );
}
