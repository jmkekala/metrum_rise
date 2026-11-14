// SPDX-License-Identifier: GPL-2.0-only

//! Round HU (`ROAD-112`) tests: the footprint hue covers the draft's footprint once (no
//! overlap, area within 1 %) for a straight, a curve and a draft onto an existing road; it lies
//! on the higher of the road and the ground (a cut, a fill); a bridge's hue lies on the ground
//! under the deck; a tunnel's is outlined on the ground, not filled; the state
//! follows the preview; no hue after a cancel or a commit.

use super::*;
use crate::nodes::sim::road_tool::batch::{draft_requests, ghost_half_width_m};
use crate::nodes::sim::road_tool::height::{DraftSection, DraftStroke};
use crate::nodes::sim::road_tool::measure::RoadTypeSpec;
use crate::nodes::sim::road_tool::session::{
    Pointer, PointerButton, RoadToolAction, RoadToolEvent, RoadToolSession,
};
use crate::nodes::sim::road_tool::tool_tests::view as tool_tests_view;
use crate::simulation::network::road_edit::DraftGround;

fn flat(_: f32, _: f32) -> f32 {
    0.0
}

// A valley: 12 m deep at x = 50, varying across the road too.
fn valley(x: f32, z: f32) -> f32 {
    -12.0 * (1.0 - ((x - 50.0) / 50.0).powi(2)).max(0.0) + 0.05 * z
}

// The hue of one stroke of `kind`.
fn hue_of(
    points: &[Vector3],
    kind: HueKind,
    half_width_m: f32,
    ground: fn(f32, f32) -> f32,
) -> FootprintHue {
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(points, kind);
    hue.finish(half_width_m, ground, HueState::Valid);
    hue
}

fn plan_area(triangles: &[Vector3]) -> f32 {
    triangles
        .chunks(3)
        .map(|t| {
            let (ab, ac) = (t[1] - t[0], t[2] - t[0]);
            (ab.x * ac.z - ab.z * ac.x).abs() * 0.5
        })
        .sum()
}

fn plan_length(points: &[Vector3]) -> f32 {
    points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z))
        .sum()
}

// Whether plan point `(x, z)` lies strictly inside triangle `t` (in plan).
fn inside(t: &[Vector3], x: f32, z: f32) -> bool {
    let side = |a: Vector3, b: Vector3| (b.x - a.x) * (z - a.z) - (b.z - a.z) * (x - a.x);
    let (s0, s1, s2) = (side(t[0], t[1]), side(t[1], t[2]), side(t[2], t[0]));
    (s0 > 0.0 && s1 > 0.0 && s2 > 0.0) || (s0 < 0.0 && s1 < 0.0 && s2 < 0.0)
}

// Samples a 0.37 m lattice over the centreline's box: a point well inside the footprint (more
// than 5 cm from its sides and end caps) lies in exactly one fill triangle, one well outside in
// none. Proves cover and no overlap at every sample. Returns the samples checked inside.
fn assert_covers_once(triangles: &[Vector3], centreline: &[Vector3], half_width_m: f32) -> usize {
    const EPS: f32 = 0.05;
    let (mut min, mut max) = (Vector3::splat(f32::MAX), Vector3::splat(f32::MIN));
    for p in centreline {
        (min.x, min.z) = (min.x.min(p.x), min.z.min(p.z));
        (max.x, max.z) = (max.x.max(p.x), max.z.max(p.z));
    }
    let pad = half_width_m + 1.0;
    let end_cap = |end: Vector3, before: Vector3, x: f32, z: f32| {
        let out = Vector3::new(end.x - before.x, 0.0, end.z - before.z).normalized();
        (x - end.x) * out.x + (z - end.z) * out.z
    };
    let last = centreline.len() - 1;
    let mut checked = 0;
    let mut x = min.x - pad;
    while x < max.x + pad {
        let mut z = min.z - pad;
        while z < max.z + pad {
            let caps = end_cap(centreline[0], centreline[1], x, z).max(end_cap(
                centreline[last],
                centreline[last - 1],
                x,
                z,
            ));
            let distance = centreline
                .windows(2)
                .map(|pair| segment_distance(pair[0], pair[1], x, z))
                .fold(f32::MAX, f32::min);
            let count = triangles.chunks(3).filter(|t| inside(t, x, z)).count();
            if caps < -EPS && distance < half_width_m - EPS {
                assert_eq!(count, 1, "inside at ({x}, {z})");
                checked += 1;
            } else if caps > EPS || distance > half_width_m + EPS {
                assert_eq!(count, 0, "outside at ({x}, {z})");
            }
            z += 0.37;
        }
        x += 0.37;
    }
    checked
}

fn segment_distance(a: Vector3, b: Vector3, x: f32, z: f32) -> f32 {
    let (dx, dz) = (b.x - a.x, b.z - a.z);
    let t = (((x - a.x) * dx + (z - a.z) * dz) / (dx * dx + dz * dz)).clamp(0.0, 1.0);
    (a.x + dx * t - x).hypot(a.z + dz * t - z)
}

fn assert_area(triangles: &[Vector3], centreline: &[Vector3], half_width_m: f32) {
    let expected = plan_length(centreline) * 2.0 * half_width_m;
    let area = plan_area(triangles);
    assert!(
        (area - expected).abs() <= expected * 0.01,
        "area {area} against {expected}"
    );
}

// The road's height under plan point `(x, z)` of a straight `line`, linear along it.
fn road_at(line: [Vector3; 2], x: f32, z: f32) -> f32 {
    let along = Vector3::new(line[1].x - line[0].x, 0.0, line[1].z - line[0].z);
    let t = ((x - line[0].x) * along.x + (z - line[0].z) * along.z) / along.length_squared();
    line[0].y + (line[1].y - line[0].y) * t
}

// A straight above the ground: the footprint's area, covered once; on the road plus the lift.
#[test]
fn a_straights_hue_covers_its_footprint_once() {
    let line = [Vector3::new(0.0, 2.0, 0.0), Vector3::new(80.0, 4.0, 30.0)];
    let hue = hue_of(&line, HueKind::Road, 6.5, flat);
    assert_area(hue.filled(), &line, 6.5);
    assert!(assert_covers_once(hue.filled(), &line, 6.5) > 1_000);
    for vertex in hue.filled() {
        assert!((vertex.y - road_at(line, vertex.x, vertex.z) - HUE_LIFT_M).abs() < 1e-3);
    }
    // Padded to a power of two, collapsed.
    let used = hue.filled().len() / 3;
    assert_eq!(hue.fill.len(), used.max(64).next_power_of_two() * 3);
    assert_eq!(plan_area(&hue.fill), plan_area(hue.filled()));
    // The uploaded bytes are the fill's vertices.
    assert_eq!(hue.fill_bytes.len(), hue.fill.len() * 12);
    assert_eq!(hue.fill_bytes[4..8], hue.fill[0].y.to_le_bytes());
}

// A curve (a 90 degree arc of 60 m in 4.5 degree steps): covered once, area within 1 %.
#[test]
fn a_curves_hue_covers_its_footprint_once() {
    let arc: Vec<Vector3> = (0..=20)
        .map(|step| {
            let angle = (step as f32 / 20.0) * std::f32::consts::FRAC_PI_2;
            Vector3::new(60.0 * angle.cos(), 1.0, 60.0 * angle.sin())
        })
        .collect();
    let hue = hue_of(&arc, HueKind::Road, 8.0, flat);
    assert_area(hue.filled(), &arc, 8.0);
    assert!(assert_covers_once(hue.filled(), &arc, 8.0) > 1_000);
}

// A draft onto an existing road (the neighbourhood's west street at z = 0), as the preview
// prepares it: its footprint covered once, area within 1 % of its prepared length times width.
#[test]
fn a_junction_drafts_hue_covers_its_footprint_once() {
    let view = tool_tests_view();
    let y = 0.0;
    let stroke = DraftStroke {
        points: vec![Vector3::new(-150.0, y, 90.0), Vector3::new(-150.0, y, 0.0)],
        profile: RoadProfile::FollowGround,
    };
    let spec = RoadTypeSpec::default();
    let section = DraftSection {
        fwd_lanes: i32::from(spec.fwd_lanes),
        bkw_lanes: i32::from(spec.bkw_lanes),
        snap_to_existing_roads: true,
        median: spec.median,
        parking: spec.parking,
        attributes: spec.attributes,
    };
    let requests = draft_requests(&[stroke], section, view.surface_generation);
    let (prepared, _) = &view.validate_batch(&requests).expect("one stroke")[0];
    // The draft ends on the street's centreline.
    assert!(prepared.points.last().unwrap().z.abs() < 1e-3);
    let half_width_m = ghost_half_width_m(&requests[0]);
    let ground = DraftGround::new(&view.terrain, &view.roadbed);
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(
        &prepared.points,
        HueKind::of(RoadProfile::FollowGround, prepared.class),
    );
    hue.finish(half_width_m, |x, z| ground.height_at(x, z), HueState::Valid);
    assert_area(hue.filled(), &prepared.points, half_width_m);
    assert!(assert_covers_once(hue.filled(), &prepared.points, half_width_m) > 1_000);
}

// The hue lies on the higher of the road and the ground, plus the lift: a level road at -3 m
// across a valley is in a cut at its ends (the hue on the grass above it) and on a fill in the
// middle (the hue on the road); every vertex within 1 mm.
#[test]
fn the_hue_lies_on_the_higher_of_road_and_ground() {
    let line = [
        Vector3::new(-20.0, -3.0, 0.0),
        Vector3::new(120.0, -3.0, 0.0),
    ];
    let hue = hue_of(&line, HueKind::Road, 6.0, valley);
    let (mut cut, mut fill) = (0, 0);
    for vertex in hue.filled() {
        let ground = valley(vertex.x, vertex.z);
        assert!(
            (vertex.y - (-3.0_f32).max(ground) - HUE_LIFT_M).abs() < 1e-3,
            "{vertex:?}"
        );
        if ground > -3.0 { cut += 1 } else { fill += 1 }
    }
    assert!(cut > 50 && fill > 50, "cut {cut}, fill {fill}");
    assert!(assert_covers_once(hue.filled(), &line, 6.0) > 1_000);
}

// A bridge's hue lies on the ground under the deck (every vertex within 1 cm of the ground plus
// the lift), between ground roads on the road; the three strokes' hue still covers once.
#[test]
fn a_bridges_hue_lies_on_the_ground_under_it() {
    let deck = 4.0;
    let approach = [Vector3::new(-20.0, deck, 0.0), Vector3::new(0.0, deck, 0.0)];
    let bridge = [
        Vector3::new(0.0, deck, 0.0),
        Vector3::new(50.0, deck, 4.0),
        Vector3::new(100.0, deck, 0.0),
    ];
    let landing = [
        Vector3::new(100.0, deck, 0.0),
        Vector3::new(120.0, deck, 0.0),
    ];
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(&approach, HueKind::Road);
    hue.push_stroke(&bridge, HueKind::Bridge);
    hue.push_stroke(&landing, HueKind::Road);
    hue.finish(5.0, valley, HueState::Valid);
    let mut on_ground = 0;
    for vertex in hue.filled() {
        if vertex.x > 1.0 && vertex.x < 99.0 {
            assert!(
                (vertex.y - HUE_LIFT_M - valley(vertex.x, vertex.z)).abs() <= 0.01,
                "{vertex:?} off the ground"
            );
            on_ground += 1;
        }
    }
    assert!(on_ground > 100);
    // Cells of at most the step, along and across.
    let path = [approach[0], bridge[0], bridge[1], bridge[2], landing[1]];
    assert_area(hue.filled(), &path, 5.0);
    assert!(assert_covers_once(hue.filled(), &path, 5.0) > 1_000);
}

// A tunnel's stretch has no fill and one closed outline at the road; the ground roads either
// side are filled.
#[test]
fn a_tunnels_hue_is_an_outline() {
    let road_in = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(30.0, -2.0, 0.0)];
    let tunnel = [
        Vector3::new(30.0, -2.0, 0.0),
        Vector3::new(60.0, -12.0, 0.0),
        Vector3::new(90.0, -12.0, 0.0),
    ];
    let road_out = [
        Vector3::new(90.0, -12.0, 0.0),
        Vector3::new(110.0, -12.0, 0.0),
    ];
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(&road_in, HueKind::Road);
    hue.push_stroke(
        &tunnel,
        HueKind::of(RoadProfile::Tunnel, EdgeClass::Standard),
    );
    hue.push_stroke(&road_out, HueKind::Road);
    hue.finish(4.0, flat, HueState::Valid);
    for triangle in hue.filled().chunks(3) {
        let centre_x = (triangle[0].x + triangle[1].x + triangle[2].x) / 3.0;
        assert!(
            !(30.0..90.0).contains(&centre_x),
            "fill in the tunnel at x = {centre_x}"
        );
    }
    // The two ground roads' 30 m and 20 m, 8 m wide.
    assert!((plan_area(hue.filled()) - 50.0 * 8.0).abs() < 50.0 * 8.0 * 0.01);
    assert_eq!(hue.outline_starts, vec![0]);
    let outline = &hue.outline;
    // Left edge forward, right edge back (each 60 m in 2 m steps), closed; over the buried road
    // on the ground plus the lift.
    assert_eq!(outline.len(), 31 + 31 + 1);
    assert_eq!(outline[0], outline[62]);
    for point in outline {
        assert!((point.y - HUE_LIFT_M).abs() < 1e-5, "{point:?}");
    }
    assert!((outline[30].x - 90.0).abs() < 1e-3 && (outline[30].z - 4.0).abs() < 1e-3);
    assert!((outline[31].x - 90.0).abs() < 1e-3 && (outline[31].z + 4.0).abs() < 1e-3);
    assert!((outline[61].x - 30.0).abs() < 1e-3 && (outline[61].z + 4.0).abs() < 1e-3);
}

// The state follows the preview (pending first), a state change without a new footprint bumps
// the revision, and an equal state does not.
#[test]
fn the_hue_state_follows_the_preview() {
    assert_eq!(HueState::of_preview(true, false), HueState::Valid);
    assert_eq!(HueState::of_preview(true, true), HueState::Checking);
    assert_eq!(HueState::of_preview(false, true), HueState::Checking);
    assert_eq!(HueState::of_preview(false, false), HueState::Refused);
    let line = [Vector3::new(0.0, 0.0, 0.0), Vector3::new(40.0, 0.0, 0.0)];
    let mut hue = FootprintHue::default();
    // Hidden: a state change shows nothing.
    hue.set_state(HueState::Refused);
    assert_eq!((hue.state(), hue.revision()), (None, 0));
    hue.begin();
    hue.push_stroke(&line, HueKind::Road);
    hue.finish(4.0, flat, HueState::Checking);
    assert_eq!((hue.state(), hue.revision()), (Some(HueState::Checking), 1));
    hue.set_state(HueState::Checking);
    assert_eq!(hue.revision(), 1);
    hue.set_state(HueState::Valid);
    assert_eq!((hue.state(), hue.revision()), (Some(HueState::Valid), 2));
    // A refused graded draft without a prepared stroke: its points at the grade line.
    hue.begin();
    hue.push_graded(&line, [3.0, 7.0]);
    hue.finish(4.0, flat, HueState::Refused);
    assert_eq!(hue.state(), Some(HueState::Refused));
    let heights = hue.filled().iter().map(|vertex| vertex.y);
    let (low, high) = heights.fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
    assert!((low - 3.0 - HUE_LIFT_M).abs() < 1e-5 && (high - 7.0 - HUE_LIFT_M).abs() < 1e-5);
}

// The hue goes with a cancelled draft, stays while the sent stroke waits, and goes once the
// commit is acknowledged (the drawing side holds it only until the commit's chunks show).
#[test]
fn no_hue_after_a_cancel_or_a_commit() {
    let view = tool_tests_view();
    let at = |x: f32, z: f32| Pointer::Scripted(Vector3::new(x, 0.0, z));
    let pointer = |tool: &mut RoadToolSession, pointer: Pointer| {
        tool.handle_event(
            view,
            RoadToolEvent::Pointer {
                pointer,
                shift: false,
                ctrl: false,
            },
        );
    };
    let shown = |tool: &mut RoadToolSession, hue: &mut FootprintHue| {
        // The bridge's frame: a pose with a stroke gets a hue, then the rule.
        if tool.frame().stroke.points.len() >= 2 {
            hue.begin();
            hue.push_stroke(&tool.frame().stroke.points, HueKind::Road);
            hue.finish(4.0, flat, HueState::Valid);
        }
        hue.follow(tool.frame());
        hue.state().is_some()
    };
    let mut hue = FootprintHue::default();
    let mut tool = RoadToolSession::new();
    pointer(&mut tool, at(-280.0, 250.0));
    tool.handle_event(view, RoadToolEvent::Press(PointerButton::Left));
    pointer(&mut tool, at(-200.0, 250.0));
    assert!(shown(&mut tool, &mut hue));
    tool.handle_event(view, RoadToolEvent::Action(RoadToolAction::Cancel));
    assert!(!shown(&mut tool, &mut hue));
    // A committed draft.
    pointer(&mut tool, at(-280.0, 250.0));
    tool.handle_event(view, RoadToolEvent::Press(PointerButton::Left));
    pointer(&mut tool, at(-200.0, 250.0));
    assert!(shown(&mut tool, &mut hue));
    tool.handle_event(view, RoadToolEvent::Press(PointerButton::Left));
    assert!(tool.take_commit().is_some());
    assert!(
        shown(&mut tool, &mut hue),
        "the hue stays while the stroke waits"
    );
    tool.handle_event(view, RoadToolEvent::CommitAck { committed: true });
    // A chain continues from the committed end with the pointer still there: no stroke yet.
    assert!(!shown(&mut tool, &mut hue));
}
