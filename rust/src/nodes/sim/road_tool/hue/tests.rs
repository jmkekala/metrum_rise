// SPDX-License-Identifier: GPL-3.0-or-later

//! Round HU (`ROAD-112`) tests: the footprint hue covers the draft's footprint once (no
//! overlap, area within 1 %) for a straight, a curve and a draft onto an existing road; it lies
//! on the higher of the road and the ground (a cut, a fill); a bridge's hue lies on the ground
//! under the deck; a tunnel's is outlined on the ground, not filled; the state
//! follows the preview; no hue after a cancel or a commit. Round HU2: over the preview's pieces
//! the hue is their drawn surface (a junction corner and an end cap included, an existing road's
//! span not), at least the lift above every drawn surface at every point.

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

// The drawn ground of a hue without pieces: never read.
fn no_pieces(_: f64, _: f64) -> f64 {
    f64::NEG_INFINITY
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
    hue.finish(half_width_m, ground, no_pieces, HueState::Valid);
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
    hue.finish(half_width_m, |x, z| ground.height_at(x, z), no_pieces, HueState::Valid);
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
    hue.finish(5.0, valley, no_pieces, HueState::Valid);
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
    hue.finish(4.0, flat, no_pieces, HueState::Valid);
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
    hue.finish(4.0, flat, no_pieces, HueState::Checking);
    assert_eq!((hue.state(), hue.revision()), (Some(HueState::Checking), 1));
    hue.set_state(HueState::Checking);
    assert_eq!(hue.revision(), 1);
    hue.set_state(HueState::Valid);
    assert_eq!((hue.state(), hue.revision()), (Some(HueState::Valid), 2));
    // A refused graded draft without a prepared stroke: its points at the grade line.
    hue.begin();
    hue.push_graded(&line, [3.0, 7.0]);
    hue.finish(4.0, flat, no_pieces, HueState::Refused);
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
            hue.finish(4.0, flat, no_pieces, HueState::Valid);
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

// Round HU2: a draft from the south onto a T junction's centre, over the sloped fixture's
// compiled pieces (two-way roads with raised sidewalks).
mod drawn {
    use super::*;
    use crate::simulation::network::roadbed::fixtures::{SLOPE, point, t_junction};
    use crate::simulation::network::roadbed::{PieceId, RoadVec3, RoadbedSystem};

    fn junction() -> RoadbedSystem {
        let mut roadbed = RoadbedSystem::new(64.0);
        roadbed.compile_dirty(&t_junction(true));
        assert!(roadbed.failed_nodes().is_empty());
        roadbed
    }

    // Every compiled piece with its mesh.
    fn all(roadbed: &RoadbedSystem) -> impl Iterator<Item = (PieceId, Option<&PieceMesh>)> {
        (roadbed.pieces_in_bounds([-1.0e6, -1.0e6, 1.0e6, 1.0e6]))
            .map(|(id, _)| (id, roadbed.piece_mesh(id)))
    }

    fn draft() -> Vec<Vector3> {
        vec![point(0.0, 60.0, true), point(0.0, 0.0, true)]
    }

    fn hue_over(roadbed: &RoadbedSystem, ground: impl Fn(f64, f64) -> f64) -> FootprintHue {
        let mut hue = FootprintHue::default();
        hue.begin();
        hue.push_stroke(&draft(), HueKind::Road);
        hue.push_pieces(
            all(roadbed)
                .filter_map(|(_, mesh)| mesh)
                .filter(|mesh| mesh.grounded()),
        );
        hue.finish(5.0, flat, ground, HueState::Valid);
        hue
    }

    fn top_area(roadbed: &RoadbedSystem, ids: &[PieceId]) -> f64 {
        let mut area = 0.0;
        for (id, mesh) in all(roadbed) {
            let Some(mesh) = mesh.filter(|_| ids.contains(&id)) else {
                continue;
            };
            for triangle in 0..mesh.top_triangle_count() {
                let [a, b, c] = mesh.triangle_points(triangle);
                area += ((b.x - a.x) * (c.z - a.z) - (b.z - a.z) * (c.x - a.x)).abs() * 0.5;
            }
        }
        area
    }

    // The highest drawn road top at plan point `(x, z)` among `roadbed`'s pieces, if any.
    fn road_at(tops: &[[RoadVec3; 3]], x: f64, z: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for t in tops {
            let det = (t[1].x - t[0].x) * (t[2].z - t[0].z) - (t[1].z - t[0].z) * (t[2].x - t[0].x);
            if det.abs() < 1.0e-9 {
                continue;
            }
            let u = ((x - t[0].x) * (t[2].z - t[0].z) - (z - t[0].z) * (t[2].x - t[0].x)) / det;
            let v = ((t[1].x - t[0].x) * (z - t[0].z) - (t[1].z - t[0].z) * (x - t[0].x)) / det;
            if u >= -1.0e-4 && v >= -1.0e-4 && u + v <= 1.0 + 1.0e-4 {
                let y = t[0].y + u * (t[1].y - t[0].y) + v * (t[2].y - t[0].y);
                best = Some(best.map_or(y, |b| b.max(y)));
            }
        }
        best
    }

    // A test ground: world XZ and the fixture's slope to height.
    type Ground = fn(f64, f64, f64) -> f64;

    // The drawn ground at `(x, z)`: `ground` at the `GL0` lattice points, linear on each
    // lattice cell's two triangles (split along its `(i, j)`-`(i + 1, j + 1)` diagonal).
    fn lattice_at(ground: impl Fn(f64, f64) -> f64, x: f64, z: f64) -> f64 {
        let step = surface::GROUND_LATTICE_M;
        let (i, j) = ((x / step).floor(), (z / step).floor());
        let (fx, fz) = (x / step - i, z / step - j);
        let g = |di: f64, dj: f64| ground((i + di) * step, (j + dj) * step);
        if fx >= fz {
            g(0.0, 0.0) + fx * (g(1.0, 0.0) - g(0.0, 0.0)) + fz * (g(1.0, 1.0) - g(1.0, 0.0))
        } else {
            g(0.0, 0.0) + fz * (g(0.0, 1.0) - g(0.0, 0.0)) + fx * (g(1.0, 1.0) - g(0.0, 1.0))
        }
    }

    #[test]
    fn hu2_hue_covers_the_drawn_pieces_once() {
        let roadbed = junction();
        let hue = hue_over(&roadbed, |_, _| f64::NEG_INFINITY);
        // The south span, the junction (its corners) and the south end cap; not the west or
        // east spans the draft does not run over.
        let ids = [PieceId::Span(2), PieceId::Node(0), PieceId::Node(3)];
        let expected = top_area(&roadbed, &ids);
        let corner = top_area(&roadbed, &[PieceId::Node(0)]);
        let cap = top_area(&roadbed, &[PieceId::Node(3)]);
        assert!(corner > 1.0 && cap > 1.0, "a junction and an end cap: {corner} {cap}");
        let area = f64::from(plan_area(hue.filled()));
        assert!(
            (area - expected).abs() <= 0.01 * expected,
            "hue {area} m² vs drawn pieces {expected} m²"
        );
        assert!(hue.state() == Some(HueState::Valid) && hue.fill.len() % 3 == 0);
    }

    #[test]
    fn hu2_hue_lies_above_every_drawn_surface() {
        let roadbed = junction();
        let tops: Vec<[RoadVec3; 3]> = all(&roadbed)
            .filter_map(|(_, mesh)| mesh)
            .flat_map(|mesh| (0..mesh.top_triangle_count()).map(|t| mesh.triangle_points(t)))
            .collect();
        let curbs = all(&roadbed)
            .filter_map(|(_, mesh)| mesh)
            .map(|mesh| mesh.curb_face_triangles().len())
            .sum::<usize>();
        assert!(curbs > 0, "the fixture has kerbs");
        let slope = f64::from(SLOPE);
        let cases: [(&str, Ground); 3] = [
            ("fill", |x, _, slope| slope * x - 4.0),
            ("cut", |x, z, slope| slope * x + 3.0 + 0.5 * (0.7 * x).sin() * (0.4 * z).cos()),
            ("graded neighbour", |x, z, slope| {
                slope * x + 0.4 * (1.7 * x).sin() * (1.3 * z).cos()
            }),
        ];
        let lift = f64::from(HUE_LIFT_M);
        for (name, ground) in cases {
            let hue = hue_over(&roadbed, |x, z| ground(x, z, slope));
            let mut checked = 0;
            for t in hue.filled().chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|p| [f64::from(p.x), f64::from(p.y), f64::from(p.z)]);
                let det = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
                if det.abs() < 1.0e-6 {
                    continue; // a curb face or a collapsed padding triangle
                }
                let hue_at = |x: f64, z: f64| -> Option<f64> {
                    let u = ((x - a[0]) * (c[2] - a[2]) - (z - a[2]) * (c[0] - a[0])) / det;
                    let v = ((b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0])) / det;
                    (u >= -1.0e-6 && v >= -1.0e-6 && u + v <= 1.0 + 1.0e-6)
                        .then(|| a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]))
                };
                // The hue's own vertices: over a piece, above the ground (the road under them is
                // their own fragment's plane, which they never go below: see the next test).
                for p in [a, b, c] {
                    assert!(road_at(&tops, p[0], p[2]).is_some(), "{name}: vertex off the pieces");
                    let drawn = lattice_at(|x, z| ground(x, z, slope), p[0], p[2]);
                    assert!(p[1] >= drawn + lift - 2.0e-3, "{name}: vertex under the ground");
                }
                // The drawn ground's vertices (its lattice points) inside the triangle.
                let step = surface::GROUND_LATTICE_M;
                let [x0, x1] = [a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0])];
                let [z0, z1] = [a[2].min(b[2]).min(c[2]), a[2].max(b[2]).max(c[2])];
                let mut i = (x0 / step).ceil() as i64;
                while (i as f64) * step <= x1 {
                    let mut j = (z0 / step).ceil() as i64;
                    while (j as f64) * step <= z1 {
                        let (x, z) = (i as f64 * step, j as f64 * step);
                        if let Some(h) = hue_at(x, z) {
                            assert!(
                                h >= ground(x, z, slope) + lift - 2.0e-3,
                                "{name}: hue {h} under the ground {} at ({x}, {z})",
                                ground(x, z, slope)
                            );
                            checked += 1;
                        }
                        j += 1;
                    }
                    i += 1;
                }
            }
            assert!(checked > 1000, "{name}: {checked} lattice points checked");
        }
    }

    // The neighbourhood's west street (published pieces, with sidewalks and junctions) under a
    // draft along it, over the view's real published graded surface: every lattice point inside
    // a hue triangle at least the lift above the graded ground, as the session reads it (one
    // gather over the pieces' bounds).
    #[test]
    fn hu2_hue_lies_above_the_published_graded_ground() {
        let view = tool_tests_view();
        let path = [Vector3::new(-190.0, 0.0, 0.0), Vector3::new(-110.0, 0.0, 0.0)];
        let mut hue = FootprintHue::default();
        hue.begin();
        hue.push_stroke(&path, HueKind::Road);
        hue.push_pieces(
            (view.roadbed.pieces_in_bounds([-200.0, -20.0, -100.0, 20.0]))
                .filter_map(|(id, _)| view.roadbed.piece_mesh(id))
                .filter(|mesh| mesh.grounded()),
        );
        let bounds = hue.surface_bounds().expect("the street's pieces");
        view.with_graded_surface(|surface, scratch| {
            let reach = (bounds[2] - bounds[0]).max(bounds[3] - bounds[1]) * 0.5 + 2.0;
            let (x, z) = ((bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5);
            surface.gather_near(x, z, reach, scratch);
            hue.finish(5.0, flat, |x, z| surface.ground_height_near(x, z, scratch), HueState::Valid);
        });
        let lift = f64::from(HUE_LIFT_M);
        let mut checked = 0;
        view.with_graded_surface(|surface, scratch| {
            for t in hue.filled().chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|p| [f64::from(p.x), f64::from(p.y), f64::from(p.z)]);
                let det = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
                if det.abs() < 1.0e-6 {
                    continue;
                }
                let step = surface::GROUND_LATTICE_M;
                let [x0, x1] = [a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0])];
                let [z0, z1] = [a[2].min(b[2]).min(c[2]), a[2].max(b[2]).max(c[2])];
                for i in (x0 / step).ceil() as i64..=(x1 / step).floor() as i64 {
                    for j in (z0 / step).ceil() as i64..=(z1 / step).floor() as i64 {
                        let (x, z) = (i as f64 * step, j as f64 * step);
                        let u = ((x - a[0]) * (c[2] - a[2]) - (z - a[2]) * (c[0] - a[0])) / det;
                        let v = ((b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0])) / det;
                        if u < -1.0e-6 || v < -1.0e-6 || u + v > 1.0 + 1.0e-6 {
                            continue;
                        }
                        let h = a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]);
                        let ground = surface.ground_height_at(x, z, scratch);
                        assert!(h >= ground + lift - 2.0e-3, "hue {h} under the graded {ground} at ({x}, {z})");
                        checked += 1;
                    }
                }
            }
        });
        assert!(checked > 1000, "{checked} lattice points checked");
    }

    #[test]
    fn hu2_hue_lies_on_the_road_above_low_ground() {
        // Over low ground every hue vertex is exactly the lift above the drawn road top there:
        // the camber and the raised sidewalks are followed, not the centreline.
        let roadbed = junction();
        let tops: Vec<[RoadVec3; 3]> = all(&roadbed)
            .filter_map(|(_, mesh)| mesh)
            .flat_map(|mesh| (0..mesh.top_triangle_count()).map(|t| mesh.triangle_points(t)))
            .collect();
        let hue = hue_over(&roadbed, |_, _| -100.0);
        let lift = f64::from(HUE_LIFT_M);
        let mut sidewalk = false;
        for t in hue.filled().chunks(3) {
            let det = (t[1].x - t[0].x) * (t[2].z - t[0].z) - (t[1].z - t[0].z) * (t[2].x - t[0].x);
            if det.abs() < 1.0e-6 {
                continue;
            }
            // The fragment's own plane: the triangle's centroid lies inside one top triangle.
            let (x, z) = (
                f64::from(t[0].x + t[1].x + t[2].x) / 3.0,
                f64::from(t[0].z + t[1].z + t[2].z) / 3.0,
            );
            let y = f64::from(t[0].y + t[1].y + t[2].y) / 3.0;
            let road = road_at(&tops, x, z).expect("over a piece");
            assert!((y - road - lift).abs() < 2.0e-3, "hue {y} vs road {road} + lift at ({x}, {z})");
            sidewalk |= road - f64::from(SLOPE) * x > 0.1;
        }
        assert!(sidewalk, "some hue lies on a raised sidewalk");
    }
}


// Round HU3: beside a committed level road that cuts through a bump and fills over a dip, a
// draft's hue (its preview's pieces, read through the session's graded reader and the view's
// shared index) lies at least the lift above the drawn graded surface: every vertex and every
// `GL0` lattice point inside a hue triangle, the neighbour's cut and fill slopes included.
#[test]
fn hu3_hue_lies_above_a_neighbours_cut_and_fill() {
    use crate::nodes::sim::core::{
        CityTreasury, RoadCommitRequest, RoadPreviewContext, RoadPreviewRequest, SimCore,
    };
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::graph::RoadParking;
    use crate::simulation::network::road_edit::RoadProfile;
    use crate::simulation::network::road_types::RoadAttributes;
    use crate::simulation::terrain::TerrainSystem;
    let mut core = SimCore {
        treasury: CityTreasury::new(1.0e9),
        ..SimCore::new(WorldConfig::new(256.0, 256.0, 40.0, 10.0))
    };
    core.heightmap = TerrainSystem::with_chunking(257, 257, 1.0, 129, 0.0);
    core.transit_network = TransitNetwork::new_with_surface_chunk_span(64.0);
    // A dip 3 m deep at x = -40 and a bump 3 m high at x = 40, each 40 m long, across z.
    for z in 0..257 {
        for x in 0..257 {
            let wx = x as f32 - 128.0;
            let bump = |centre: f32| (std::f32::consts::PI * (wx - centre + 20.0) / 40.0).sin();
            let height = match wx {
                -60.0..=-20.0 => -3.0 * bump(-40.0),
                20.0..=60.0 => 3.0 * bump(40.0),
                _ => 0.0,
            };
            core.heightmap.set_height(x, z, height);
        }
    }
    core.precompute_road_mesh_data();
    let request = |points: Vec<Vector3>, profile| RoadCommitRequest {
        points,
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile,
        median: None,
        parking: RoadParking::NONE,
        attributes: RoadAttributes::DEFAULT,
        preview: None,
    };
    let level = vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
    let outcome = core.commit_road(
        request(level, RoadProfile::HeldGround),
        std::time::Instant::now(),
        Default::default(),
    );
    assert!(outcome.committed, "{}", outcome.rejection);
    let view = core.publish_road_tool_view().expect("a current roadbed").view;
    // The draft beside it, following the source ground.
    let path = [-90.0f32, 90.0].map(|x| {
        let y = core.heightmap.sample_height_world(x, 14.0);
        Vector3::new(x, y, 14.0)
    });
    let mut context = RoadPreviewContext::capture(view.clone());
    let preview = context.preview_road(RoadPreviewRequest {
        surface_generation: view.surface_generation,
        points: path.to_vec(),
        fwd_lanes: 1,
        bkw_lanes: 1,
        snap_to_existing_roads: true,
        profile: RoadProfile::FollowGround,
        median: None,
        parking: RoadParking::NONE,
        attributes: RoadAttributes::DEFAULT,
    });
    let pieces = preview.pieces.as_ref().expect("the draft is valid");
    let mut hue = FootprintHue::default();
    hue.begin();
    hue.push_stroke(&preview.plan.prepared().points, HueKind::Road);
    hue.push_pieces(
        (pieces.delta.pieces())
            .filter_map(|(_, mesh)| mesh)
            .filter(|mesh| mesh.grounded() && !mesh.buried()),
    );
    view.with_graded_surface(|surface, scratch| {
        hue.finish(5.0, flat, super::graded_ground(surface, scratch), HueState::Valid);
    });
    let lift = f64::from(HUE_LIFT_M);
    let (mut checked, mut raised) = (0, 0.0f64);
    view.with_graded_surface(|surface, scratch| {
        let mut check = |x: f64, z: f64, h: f64| {
            let ground = surface.ground_height_at(x, z, scratch);
            assert!(h >= ground + lift - 2.0e-3, "hue {h} under the graded {ground} at ({x}, {z})");
            let source = f64::from(
                core.heightmap.sample_height_world(x as f32, z as f32),
            );
            raised = raised.max(ground - source);
            checked += 1;
        };
        for t in hue.filled().chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|p| [f64::from(p.x), f64::from(p.y), f64::from(p.z)]);
            for p in [a, b, c] {
                check(p[0], p[2], p[1]);
            }
            let det = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
            if det.abs() < 1.0e-6 {
                continue;
            }
            let step = surface::GROUND_LATTICE_M;
            let [x0, x1] = [a[0].min(b[0]).min(c[0]), a[0].max(b[0]).max(c[0])];
            let [z0, z1] = [a[2].min(b[2]).min(c[2]), a[2].max(b[2]).max(c[2])];
            for i in (x0 / step).ceil() as i64..=(x1 / step).floor() as i64 {
                for j in (z0 / step).ceil() as i64..=(z1 / step).floor() as i64 {
                    let (x, z) = (i as f64 * step, j as f64 * step);
                    let u = ((x - a[0]) * (c[2] - a[2]) - (z - a[2]) * (c[0] - a[0])) / det;
                    let v = ((b[0] - a[0]) * (z - a[2]) - (b[2] - a[2]) * (x - a[0])) / det;
                    if u < -1.0e-6 || v < -1.0e-6 || u + v > 1.0 + 1.0e-6 {
                        continue;
                    }
                    check(x, z, a[1] + u * (b[1] - a[1]) + v * (c[1] - a[1]));
                }
            }
        }
    });
    assert!(checked > 1000, "{checked} points checked");
    // The neighbour's fill reaches under the draft: the source terrain alone would not do.
    assert!(raised > 0.3, "the graded ground rises {raised} m above the source under the hue");
}
