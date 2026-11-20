// SPDX-License-Identifier: GPL-3.0-or-later

//! The segmentation table of `ROAD-112` plan section 4, one test per row, on analytic ground.

use super::*;

// A straight draft along +x from 0 to 100 m at z = 0.
fn straight() -> [Vector3; 2] {
    [Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)]
}

// The ground of `profile(x)` and a cover test answering `cover`.
fn segment(
    path: &[Vector3],
    heights: [f32; 2],
    ground: impl Fn(f32) -> f32,
    cover: bool,
) -> Result<Vec<DraftStroke>, &'static str> {
    let mut out = Vec::new();
    segment_grade_line(path, heights, |x, _| ground(x), |_| cover, &mut out).map(|()| out)
}

fn profiles(strokes: &[DraftStroke]) -> Vec<RoadProfile> {
    strokes.iter().map(|stroke| stroke.profile).collect()
}

// Consecutive strokes share their joint bit for bit, and the first and last points are the
// draft's ends at the grade line's end heights.
fn assert_chained(strokes: &[DraftStroke], path: &[Vector3], heights: [f32; 2]) {
    for pair in strokes.windows(2) {
        assert_eq!(pair[0].points.last(), pair[1].points.first(), "joint");
    }
    let (first, last) = (path[0], path[path.len() - 1]);
    assert_eq!(
        strokes[0].points[0],
        Vector3::new(first.x, heights[0], first.z)
    );
    assert_eq!(
        *strokes.last().unwrap().points.last().unwrap(),
        Vector3::new(last.x, heights[1], last.z)
    );
}

#[test]
fn a_grade_line_within_a_metre_of_the_ground_is_one_held_ground_stroke() {
    // Clearance between -0.9 and +0.9 m, changing sign twice.
    let ground = |x: f32| 0.9 * (x * std::f32::consts::PI / 50.0).sin();
    let strokes = segment(&straight(), [0.0, 0.0], ground, false).unwrap();
    assert_eq!(profiles(&strokes), [RoadProfile::HeldGround]);
    assert_eq!(
        strokes[0].points,
        [Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)]
    );
}

#[test]
fn raised_ends_over_flat_ground_are_one_bridge_of_the_draft_itself() {
    for raise in [2.5, 5.0] {
        let strokes = segment(&straight(), [raise, raise], |_| 0.0, false).unwrap();
        assert_eq!(profiles(&strokes), [RoadProfile::Bridge]);
        assert_eq!(
            strokes[0].points,
            [
                Vector3::new(0.0, raise, 0.0),
                Vector3::new(100.0, raise, 0.0)
            ]
        );
    }
}

#[test]
fn a_bridge_runs_landing_to_landing_with_crossings_interpolated() {
    // A valley 4 m deep between x = 30 and 70 under a level grade line at 0: the clearance
    // rises from 0 at x = 20 and falls back to 0 at x = 80, linearly.
    let ground = |x: f32| -((x - 20.0).clamp(0.0, 10.0) - (x - 70.0).clamp(0.0, 10.0)) * 0.4;
    let path = straight();
    let strokes = segment(&path, [0.0, 0.0], ground, false).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::HeldGround
        ]
    );
    let bridge = &strokes[1].points;
    assert!((bridge[0].x - 20.0).abs() < 1e-3, "landing {}", bridge[0].x);
    assert!((bridge[bridge.len() - 1].x - 80.0).abs() < 1e-3);
    assert_chained(&strokes, &path, [0.0, 0.0]);
}

#[test]
fn a_run_below_the_ground_reaching_bore_cover_is_a_tunnel() {
    // A hill 12 m high between x = 40 and 60 over a level grade line at 0, its feet at knots
    // (x = 28 and 72): the crossings are the last and first knots on the ground.
    let ground = |x: f32| (x - 28.0).clamp(0.0, 12.0) - (x - 60.0).clamp(0.0, 12.0);
    let path = straight();
    let strokes = segment(&path, [0.0, 0.0], ground, true).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround
        ]
    );
    let tunnel = &strokes[1].points;
    assert!((tunnel[0].x - 28.0).abs() < 1e-3, "{}", tunnel[0].x);
    assert!((tunnel[tunnel.len() - 1].x - 72.0).abs() < 1e-3);
    assert_chained(&strokes, &path, [0.0, 0.0]);
}

#[test]
fn a_shallow_run_below_the_ground_without_cover_is_a_cut() {
    // 3 m below a low hill: no bore cover, within the 5 m band: one held ground road.
    let ground = |x: f32| ((x - 30.0).clamp(0.0, 10.0) - (x - 60.0).clamp(0.0, 10.0)) * 0.3;
    let strokes = segment(&straight(), [0.0, 0.0], ground, false).unwrap();
    assert_eq!(profiles(&strokes), [RoadProfile::HeldGround]);
    assert_eq!(strokes[0].points.len(), 2, "the draft's own points");
}

#[test]
fn a_deep_run_below_the_ground_without_cover_is_refused() {
    // 8 m below the hill and no knot with bore cover (a section too wide, say).
    let ground = |x: f32| ((x - 30.0).clamp(0.0, 10.0) - (x - 60.0).clamp(0.0, 10.0)) * 0.8;
    assert_eq!(
        segment(&straight(), [0.0, 0.0], ground, false),
        Err(CUT_TOO_DEEP_REASON)
    );
}

// The arc length of a point on `path`, within 0.1 mm of it.
fn station_of(path: &[Vector3], point: Vector3) -> f32 {
    let mut station = 0.0;
    for pair in path.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let run = (b.x - a.x).hypot(b.z - a.z);
        let t = ((point.x - a.x) * (b.x - a.x) + (point.z - a.z) * (b.z - a.z)) / (run * run);
        let on = (a.x + (b.x - a.x) * t, a.z + (b.z - a.z) * t);
        if (-1e-4..=1.0 + 1e-4).contains(&t) && (on.0 - point.x).hypot(on.1 - point.z) < 1e-4 {
            return station + run * t;
        }
        station += run;
    }
    panic!("{point:?} is not on the path");
}

#[test]
fn a_bridge_then_a_tunnel_join_through_held_ground_at_exact_heights() {
    // A grade line rising at 10 % along a bent draft, over a valley 6 m deep between 10 % and
    // 40 % of its length, rippled level ground, then a hill 14 m high between 60 % and 90 %.
    let path = [
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(30.0, 0.0, 10.0),
        Vector3::new(70.0, 0.0, 10.0),
        Vector3::new(100.0, 0.0, 0.0),
    ];
    let length: f32 = path
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z))
        .sum();
    let heights = [0.0, length * 0.1];
    let bump = |u: f32| {
        let arch = |from: f32| ((u - from) * std::f32::consts::PI / 30.0).sin().max(0.0);
        if (10.0..40.0).contains(&u) {
            -6.0 * arch(10.0)
        } else if (60.0..90.0).contains(&u) {
            14.0 * arch(60.0)
        } else {
            // Level ground with a ripple of +-0.2 m: the clearance changes sign every 3 %.
            0.2 * (u * std::f32::consts::PI / 3.0).sin()
        }
    };
    let ground = |x: f32, z: f32| {
        let s = station_of(&path, Vector3::new(x, 0.0, z));
        0.1 * s + bump(s * 100.0 / length)
    };
    let mut strokes = Vec::new();
    segment_grade_line(&path, heights, ground, |_| true, &mut strokes).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::HeldGround,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround,
        ]
    );
    assert_chained(&strokes, &path, heights);
    // Every point stands on the grade line; the boundaries where the clearance crosses zero.
    let line = GradeLine::new(&path, heights);
    for point in strokes.iter().flat_map(|stroke| &stroke.points) {
        let s = station_of(&path, *point);
        assert!((point.y - line.height_at(s)).abs() < 1e-3, "{point:?}");
    }
    for (stroke, from, to) in [(1, 10.0, 40.0), (3, 60.0, 90.0)] {
        let points = &strokes[stroke].points;
        let at = |point: Vector3| station_of(&path, point) * 100.0 / length;
        // Within the ripple's half period and a lattice step of the feature: the landing is the
        // last sign change on the 4 m lattice.
        assert!((at(points[0]) - from).abs() <= 8.0, "{}", at(points[0]));
        assert!((at(points[points.len() - 1]) - to).abs() <= 8.0);
    }
    // The bends survive, each once, inside their runs.
    let all: Vec<_> = strokes
        .iter()
        .flat_map(|stroke| stroke.points.clone())
        .collect();
    for corner in &path[1..3] {
        let on = |p: &&Vector3| p.x == corner.x && p.z == corner.z;
        assert!(all.iter().filter(on).count() >= 1, "{corner:?}");
    }
}

// Ground interpolated linearly between `(x, height)` knots (constant beyond them).
fn table(knots: &'static [(f32, f32)]) -> impl Fn(f32) -> f32 {
    move |x| {
        let next = knots.partition_point(|&(at, _)| at <= x);
        match next {
            0 => knots[0].1,
            n if n == knots.len() => knots[n - 1].1,
            n => {
                let ((x0, h0), (x1, h1)) = (knots[n - 1], knots[n]);
                h0 + (h1 - h0) * (x - x0) / (x1 - x0)
            }
        }
    }
}

// A straight draft along +x from 0 to 100 m with path points every metre over 38-44 m, so the
// lattice reads the ground there at 1 m knots.
fn dense_path() -> Vec<Vector3> {
    let mut path = vec![Vector3::new(0.0, 0.0, 0.0)];
    path.extend((38..=44).map(|x| Vector3::new(x as f32, 0.0, 0.0)));
    path.push(Vector3::new(100.0, 0.0, 0.0));
    path
}

// Round I's short-run rule, row 1: a valley (a bridge), then the ground dipping and rising by
// 0.1 m, then a hill (a tunnel). The held run between them, 1.15 m from crossing to crossing,
// has its middle below the ground: it joins the tunnel, as more of its approach.
#[test]
fn a_short_held_run_below_the_ground_joins_the_tunnel_beside_it() {
    let ground = table(&[
        (8.0, 0.0),
        (12.0, -3.0),
        (36.0, -3.0),
        (40.0, 0.1),
        (41.0, -0.1),
        (42.0, 6.0),
        (68.0, 6.0),
        (72.0, 0.0),
    ]);
    let path = dense_path();
    let strokes = segment(&path, [0.0, 0.0], ground, true).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround
        ]
    );
    assert_chained(&strokes, &path, [0.0, 0.0]);
    // The bridge lands at its own crossing (between the 1 m knots at 39 m, +0.675 m, and 40 m,
    // -0.1 m); the tunnel starts there.
    let landing = 39.0 + 0.675 / 0.775;
    assert!((strokes[1].points.last().unwrap().x - landing).abs() < 1e-3);
}

// Row 2: the same with the rise to +0.3 m, so the short run's middle stands above the ground:
// it joins the bridge, whose deck holds the grade line through it.
#[test]
fn a_short_held_run_above_the_ground_joins_the_bridge_beside_it() {
    let ground = table(&[
        (8.0, 0.0),
        (12.0, -3.0),
        (36.0, -3.0),
        (40.0, 0.1),
        (41.0, -0.3),
        (42.0, 6.0),
        (68.0, 6.0),
        (72.0, 0.0),
    ]);
    let path = dense_path();
    let strokes = segment(&path, [0.0, 0.0], ground, true).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround
        ]
    );
    assert_chained(&strokes, &path, [0.0, 0.0]);
    // The tunnel starts at its own crossing (41 + 0.3 / 6.3 m): the bridge reaches it.
    let portal_side = 41.0 + 0.3 / 6.3;
    assert!((strokes[2].points[0].x - portal_side).abs() < 1e-3);
    for stroke in &strokes {
        assert!(stroke_length(stroke) >= 2.0, "{stroke:?}");
    }
}

// Row 3: a short held run at the draft's start, before a bridge (no structure before it): it
// joins the bridge after it, so the bridge starts at the draft's start.
#[test]
fn a_short_held_run_at_an_end_joins_its_one_neighbour() {
    let ground = table(&[(0.0, 0.3), (1.0, -3.0), (60.0, -3.0), (64.0, 0.0)]);
    let path = [
        Vector3::new(0.0, 0.0, 0.0),
        Vector3::new(1.0, 0.0, 0.0),
        Vector3::new(100.0, 0.0, 0.0),
    ];
    let strokes = segment(&path, [0.0, 0.0], ground, false).unwrap();
    assert_eq!(
        profiles(&strokes),
        [RoadProfile::Bridge, RoadProfile::HeldGround]
    );
    assert_eq!(strokes[0].points[0], Vector3::new(0.0, 0.0, 0.0));
    assert_chained(&strokes, &path, [0.0, 0.0]);
}

// Row 4: a held run of 2 m or more between two structures stays a ground road of its own.
#[test]
fn a_held_run_of_the_minimum_length_stays() {
    // As row 1, with the dip and rise spread over 4 m knots: the held run is over 4 m long.
    let ground = table(&[
        (8.0, 0.0),
        (12.0, -3.0),
        (36.0, -3.0),
        (40.0, 0.1),
        (44.0, -0.1),
        (48.0, 6.0),
        (68.0, 6.0),
        (72.0, 0.0),
    ]);
    let strokes = segment(&straight(), [0.0, 0.0], ground, true).unwrap();
    assert_eq!(
        profiles(&strokes),
        [
            RoadProfile::HeldGround,
            RoadProfile::Bridge,
            RoadProfile::HeldGround,
            RoadProfile::Tunnel,
            RoadProfile::HeldGround
        ]
    );
    assert!(stroke_length(&strokes[2]) >= 2.0);
}

fn stroke_length(stroke: &DraftStroke) -> f32 {
    stroke
        .points
        .windows(2)
        .map(|pair| (pair[1].x - pair[0].x).hypot(pair[1].z - pair[0].z))
        .sum()
}

#[test]
fn segmentation_is_deterministic() {
    let ground = |x: f32| 0.07 * x + 3.0 * (x * 0.13).sin();
    let a = segment(&straight(), [1.5, 4.0], ground, true).unwrap();
    let b = segment(&straight(), [1.5, 4.0], ground, true).unwrap();
    assert_eq!(a, b);
}

#[test]
fn profile_codes_round_trip() {
    for profile in [
        RoadProfile::FollowGround,
        RoadProfile::HeldGround,
        RoadProfile::Bridge,
        RoadProfile::Tunnel,
    ] {
        assert_eq!(RoadProfile::from_code(profile.code()), Some(profile));
    }
    assert_eq!(RoadProfile::from_code(4), None);
    assert_eq!(RoadProfile::from_tunnel(true), RoadProfile::Tunnel);
    assert_eq!(RoadProfile::from_tunnel(false), RoadProfile::FollowGround);
}
