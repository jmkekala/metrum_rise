// SPDX-License-Identifier: GPL-2.0-only

//! Guide kernel tests (`ROAD-112` round A): construction exactness, the flattening rule's sag
//! and step bounds, `f32` placement at +-10 km, offsets, splits, reversal and determinism.

use super::*;

const ANGLE_EPS: f64 = 1e-9;
// World positions the kernel must handle: the origin and the corners of a 20 km map.
const ORIGINS: [(f64, f64); 4] = [
    (0.0, 0.0),
    (10_000.0, -10_000.0),
    (-10_000.0, 10_000.0),
    (9_500.0, 9_700.0),
];

fn v(x: f64, z: f64) -> DVec2 {
    DVec2::new(x, z)
}

fn angle_between(a: DVec2, b: DVec2) -> f64 {
    a.perp_dot(b).atan2(a.dot(b)).abs()
}

fn fillet(a: DVec2, c: DVec2, b: DVec2) -> GuidePath {
    let mut path = GuidePath::new();
    path.set_fillet(a, c, b).expect("fillet");
    path
}

fn biarc(a: DVec2, ta: DVec2, b: DVec2, tb: DVec2) -> GuidePath {
    let mut path = GuidePath::new();
    path.set_biarc(a, ta, b, tb).expect("biarc");
    path
}

fn bezier(origin: DVec2, points: [(f64, f64); 4]) -> GuidePath {
    let [p0, p1, p2, p3] = points.map(|(x, z)| origin + v(x, z));
    GuidePath::from_segment(GuideSegment::Bezier(
        CubicBezier::new(p0, p1, p2, p3).expect("bezier"),
    ))
}

fn arc_path(centre: DVec2, radius: f64, start_angle: f64, sweep: f64) -> GuidePath {
    GuidePath::from_segment(GuideSegment::Arc(
        Arc::from_centre(centre, radius, start_angle, sweep).expect("arc"),
    ))
}

// The fillet legs of the tests: (a, c, b) relative to an origin.
const FILLET_LEGS: [[(f64, f64); 3]; 6] = [
    [(0.0, 0.0), (100.0, 0.0), (100.0, 60.0)], // left turn, a -> c longer
    [(0.0, 0.0), (50.0, 0.0), (150.0, 80.0)],  // a -> c shorter
    [(0.0, 0.0), (80.0, 0.0), (80.0, -80.0)],  // right turn, equal legs: no lead
    [(0.0, 0.0), (100.0, 5.0), (200.0, -3.0)], // shallow S of the legs
    [(0.0, 0.0), (100.0, 0.0), (10.0, 30.0)],  // deflection over 90 degrees
    [(0.0, 0.0), (300.0, 0.1), (600.0, 0.0)],  // nearly straight: huge radius
];

// Every path shape the tests exercise, at one origin.
fn sample_paths(origin: DVec2) -> Vec<GuidePath> {
    let mut paths: Vec<GuidePath> = FILLET_LEGS
        .iter()
        .map(|legs| {
            let [a, c, b] = legs.map(|(x, z)| origin + v(x, z));
            fillet(a, c, b)
        })
        .collect();
    paths.push(biarc(
        origin,
        v(1.0, 0.0),
        origin + v(100.0, 20.0),
        v(1.0, 0.0),
    ));
    paths.push(biarc(
        origin,
        v(1.0, 0.3),
        origin + v(-40.0, 90.0),
        v(-1.0, 0.2),
    ));
    paths.push(arc_path(origin + v(5.0, 7.0), 2.0, 0.4, 5.9));
    paths.push(arc_path(origin, 40.0, -1.0, -2.5));
    paths.push(arc_path(origin, 1.0e6, 0.2, 0.001));
    paths.push(GuidePath::from_segment(GuideSegment::Line(Line::new(
        origin,
        origin + v(1000.0, 3.0),
    ))));
    paths.push(GuidePath::from_segment(GuideSegment::Line(Line::new(
        origin,
        origin + v(4.0, 0.0),
    ))));
    // A gentle 1 km span and a tight one.
    paths.push(bezier(
        origin,
        [(0.0, 0.0), (350.0, 0.0), (650.0, 200.0), (950.0, 250.0)],
    ));
    paths.push(bezier(
        origin,
        [(0.0, 0.0), (40.0, 0.0), (60.0, 40.0), (20.0, 50.0)],
    ));
    paths
}

#[test]
fn fillet_passes_through_both_clicks_tangent_to_both_legs() {
    for (ox, oz) in ORIGINS {
        let origin = v(ox, oz);
        for legs in FILLET_LEGS {
            let [a, c, b] = legs.map(|(x, z)| origin + v(x, z));
            let path = fillet(a, c, b);
            assert_eq!(path.start(), Some(a), "starts at A bit for bit");
            assert_eq!(path.end(), Some(b), "ends at B bit for bit");
            let (u_in, u_out) = ((c - a).normalize(), (b - c).normalize());
            assert!(angle_between(path.start_tangent().unwrap(), u_in) < ANGLE_EPS);
            assert!(angle_between(path.end_tangent().unwrap(), u_out) < ANGLE_EPS);
            for pair in path.segments().windows(2) {
                assert_eq!(pair[0].end(), pair[1].start());
                assert!(angle_between(pair[0].end_tangent(), pair[1].start_tangent()) < ANGLE_EPS);
            }
            // The arc: tangent length T = min legs from the corner, radius T / tan(delta / 2).
            let tangent_length = (c - a).length().min((b - c).length());
            let deflection = angle_between(u_in, u_out);
            let arc = path
                .segments()
                .iter()
                .find_map(|segment| match segment {
                    GuideSegment::Arc(arc) => Some(*arc),
                    _ => None,
                })
                .expect("one arc");
            assert!(angle_between(arc.start_tangent(), u_in) < ANGLE_EPS);
            assert!(angle_between(arc.end_tangent(), u_out) < ANGLE_EPS);
            assert!(((arc.start() - c).length() - tangent_length).abs() < 1e-9);
            assert!(((arc.end() - c).length() - tangent_length).abs() < 1e-9);
            let radius = tangent_length / (deflection / 2.0).tan();
            assert!((arc.radius() - radius).abs() <= 1e-9 * radius);
            assert!((arc.length() - radius * deflection).abs() <= 1e-9 * arc.length());
        }
    }
}

#[test]
fn fillet_special_cases() {
    let mut path = GuidePath::new();
    path.set_fillet(v(0.0, 0.0), v(50.0, 0.0), v(120.0, 0.0))
        .unwrap();
    assert_eq!(
        path.segments(),
        &[GuideSegment::Line(Line::new(v(0.0, 0.0), v(120.0, 0.0)))]
    );
    assert_eq!(
        path.set_fillet(v(0.0, 0.0), v(50.0, 0.0), v(10.0, 0.0)),
        Err(GuideError::FoldBack)
    );
    assert_eq!(
        path.set_fillet(v(0.0, 0.0), v(0.0, 0.0), v(10.0, 5.0)),
        Err(GuideError::Degenerate)
    );
}

#[test]
fn tangent_to_leaves_along_the_tangent_and_reaches_the_end() {
    let start = v(10_000.0, -10_000.0);
    let tangent = v(0.6, 0.8);
    assert!(matches!(
        GuideSegment::tangent_to(start, tangent, start + tangent * 50.0),
        Ok(GuideSegment::Line(_))
    ));
    assert_eq!(
        GuideSegment::tangent_to(start, tangent, start - tangent * 50.0),
        Err(GuideError::FullTurn)
    );
    for end in [v(30.0, 70.0), v(-60.0, 10.0), v(5.0, -80.0)] {
        let segment = GuideSegment::tangent_to(start, tangent, start + end).unwrap();
        assert_eq!(segment.start(), start);
        assert_eq!(segment.end(), start + end);
        assert!(angle_between(segment.start_tangent(), tangent) < ANGLE_EPS);
    }
}

#[test]
fn biarc_joint_is_tangent_continuous() {
    let cases = [
        (v(1.0, 0.0), v(100.0, 20.0), v(1.0, 0.0)),
        (v(1.0, 0.0), v(100.0, 20.0), v(0.0, 1.0)),
        (v(1.0, 0.3), v(-40.0, 90.0), v(-1.0, 0.2)),
        (v(0.0, 1.0), v(30.0, 300.0), v(0.1, 1.0)),
        (v(1.0, 0.0), v(0.0, 100.0), v(-1.0, 0.0)), // a U-turn: two quarter circles
    ];
    for (ox, oz) in ORIGINS {
        let origin = v(ox, oz);
        for (ta, offset, tb) in cases {
            let b = origin + offset;
            let path = biarc(origin, ta, b, tb);
            assert_eq!(path.start(), Some(origin));
            assert_eq!(path.end(), Some(b));
            assert!(angle_between(path.start_tangent().unwrap(), ta) < ANGLE_EPS);
            assert!(angle_between(path.end_tangent().unwrap(), tb) < ANGLE_EPS);
            let [head, tail] = path.segments() else {
                panic!("two pieces")
            };
            assert_eq!(head.end(), tail.start());
            assert!(angle_between(head.end_tangent(), tail.start_tangent()) < ANGLE_EPS);
        }
    }
    let mut path = GuidePath::new();
    assert_eq!(
        path.set_biarc(v(0.0, 0.0), v(1.0, 0.0), v(-100.0, 10.0), v(1.0, 0.0)),
        Err(GuideError::NoBiarc)
    );
    // Straight behind with the tangent reversed: the joint falls on the start.
    assert_eq!(
        path.set_biarc(v(0.0, 0.0), v(1.0, 0.0), v(-100.0, 0.0), v(-1.0, 0.0)),
        Err(GuideError::Degenerate)
    );
}

#[test]
fn arc_forms_agree_and_length_matches_closed_form() {
    for (ox, oz) in ORIGINS {
        let centre = v(ox, oz);
        for radius in [1.5, 30.0, 400.0, 25_000.0] {
            for sweep in [1e-6_f64, 0.3, -1.7, 3.1, -4.4, 6.2] {
                // A chord under 1 cm at 10 km is below f64's relative resolution for 1e-9.
                if 2.0 * radius * (0.5 * sweep).abs().sin() < 0.01 {
                    continue;
                }
                let arc = Arc::from_centre(centre, radius, 0.9, sweep).unwrap();
                let closed_form = radius * sweep.abs();
                assert!((arc.length() - closed_form).abs() <= 1e-9 * closed_form);
                assert!((arc.radius() - radius).abs() <= 1e-9 * radius);
                assert!((arc.sweep() - sweep).abs() <= 1e-12);
                // The centre of a tiny sweep is ill-conditioned by nature (a chord of 1e-6 rad).
                if sweep.abs() > 0.1 {
                    assert!((arc.centre() - centre).length() <= 1e-9 * radius.max(1e4));
                    assert!((arc.start_angle() - 0.9).abs() <= 1e-9);
                }
            }
        }
    }
}

#[test]
fn flattened_arc_points_stay_on_the_circle_after_the_f32_cast() {
    // Half an f32 step per axis is the cast's own rounding: 0.49 mm at 10 km, so the 1e-4 m
    // bar is checked beyond it there and exactly within 1 km of the origin.
    let half_ulp = |x: f64| {
        let f = (x as f32).abs();
        f64::from(f32::from_bits(f.to_bits() + 1) - f) * 0.5
    };
    let mut points = Vec::new();
    let centres = [
        (10_000.0, 10_000.0),
        (-10_000.0, -10_000.0),
        (10_000.0, -10_000.0),
        (300.0, -400.0),
    ];
    for (cx, cz) in centres {
        let centre = v(cx, cz);
        let near_origin = cx.abs() < 1_000.0;
        for radius in [12.0, 250.0, if near_origin { 500.0 } else { 2_000.0 }] {
            for sweep in [1.3, -2.9, 5.5] {
                arc_path(centre, radius, -0.4, sweep).flatten_into(&mut points);
                assert!(points.len() > 2);
                for &p in &points {
                    assert!(((p - centre).length() - radius).abs() <= 1e-9, "f64 point");
                    let cast = v(f64::from(p.x as f32), f64::from(p.y as f32));
                    let error = ((cast - centre).length() - radius).abs();
                    let rounding = if near_origin {
                        0.0
                    } else {
                        half_ulp(p.x).hypot(half_ulp(p.y))
                    };
                    assert!(error <= 1e-4 + rounding, "f32 error {error} at {p}");
                }
            }
        }
    }
}

// Each flattened point against the path's own station, and each chord's step and sag.
fn assert_flattening_bounds(path: &GuidePath, points: &[DVec2]) {
    let mut index = 0;
    for segment in path.segments() {
        let count = segment.flatten_count();
        let step = segment.length() / count as f64;
        for j in 0..count {
            let (p, q) = (points[index + j], points[index + j + 1]);
            assert!((p - segment.point_at(step * j as f64)).length() <= 1e-9);
            assert!(
                (p - q).length() <= FLATTEN_MAX_STEP_M,
                "step {}",
                (p - q).length()
            );
            let chord = q - p;
            for k in 1..16 {
                let curve = segment.point_at(step * (j as f64 + k as f64 / 16.0));
                let along = ((curve - p).dot(chord) / chord.length_squared()).clamp(0.0, 1.0);
                let sag = (curve - (p + chord * along)).length();
                assert!(sag <= FLATTEN_MAX_SAG_M, "sag {sag}");
            }
        }
        index += count;
        assert_eq!(points[index], segment.end());
    }
    assert_eq!(index + 1, points.len());
}

#[test]
fn flattening_keeps_sag_and_step_bounds() {
    let mut points = Vec::new();
    for (ox, oz) in ORIGINS {
        for path in sample_paths(v(ox, oz)) {
            path.flatten_into(&mut points);
            assert_flattening_bounds(&path, &points);
        }
    }
}

#[test]
fn flatten_count_follows_the_rule() {
    assert_eq!(flatten_count(1000.0, f64::INFINITY), 250);
    assert_eq!(flatten_count(4.0, f64::INFINITY), 1);
    assert_eq!(flatten_count(4.0001, f64::INFINITY), 2);
    assert_eq!(flatten_count(0.0, 5.0), 1);
    // 8 R 0.05 = 0.8 m^2 at R = 2 m.
    assert_eq!(
        flatten_count(10.0, 2.0),
        (10.0 / 0.8_f64.sqrt()).ceil() as usize
    );
    // A zero radius counts as the floor.
    assert_eq!(
        flatten_count(100.0, 0.0),
        flatten_count(100.0, FLATTEN_RADIUS_FLOOR_M)
    );
}

#[test]
fn bezier_length_quadrature_matches_a_fine_reference() {
    for points in [
        [(0.0, 0.0), (350.0, 0.0), (650.0, 200.0), (950.0, 250.0)],
        [(0.0, 0.0), (40.0, 0.0), (60.0, 40.0), (20.0, 50.0)],
        [(0.0, 0.0), (1.0, 0.0), (99.0, 0.0), (100.0, 0.0)],
    ] {
        let path = bezier(v(10_000.0, -10_000.0), points);
        let GuideSegment::Bezier(curve) = path.segments()[0] else {
            unreachable!()
        };
        // Composite Simpson over 200,000 intervals.
        let n = 200_000;
        let h = 1.0 / n as f64;
        let mut reference = 0.0;
        for i in 0..=n {
            let weight = if i == 0 || i == n {
                1.0
            } else if i % 2 == 1 {
                4.0
            } else {
                2.0
            };
            reference += weight * curve.derivative(i as f64 * h).length();
        }
        reference *= h / 3.0;
        assert!((curve.length() - reference).abs() <= 1e-9 * reference);
        // The arc-length inversion: the reference length up to t(s) is s.
        let table = curve.arc_length_table();
        for s in [0.1, 0.33, 0.5, 0.71, 0.97].map(|f| f * reference) {
            let t = curve.parameter_at(&table, s);
            let m = 20_000;
            let mut partial = 0.0;
            for i in 0..=m {
                let weight = if i == 0 || i == m {
                    1.0
                } else if i % 2 == 1 {
                    4.0
                } else {
                    2.0
                };
                partial += weight * curve.derivative(t * i as f64 / m as f64).length();
            }
            partial *= t / m as f64 / 3.0;
            assert!((partial - s).abs() <= 1e-9 * reference, "{partial} vs {s}");
        }
    }
}

#[test]
fn arc_and_line_offsets_are_exact() {
    let centre = v(10_000.0, -10_000.0);
    for sweep in [1.2, -1.2] {
        let arc = Arc::from_centre(centre, 100.0, 0.3, sweep).unwrap();
        // The left offset of a left turn shrinks the radius.
        let inward = sweep.signum() * 10.0;
        let inner = arc.offset(inward).unwrap();
        let outer = arc.offset(-inward).unwrap();
        for (shifted, radius) in [(inner, 90.0), (outer, 110.0)] {
            assert!((shifted.centre() - centre).length() <= 1e-9);
            assert!((shifted.radius() - radius).abs() <= 1e-9);
            assert_eq!(shifted.bulge(), arc.bulge());
        }
        assert!(arc.offset(sweep.signum() * 100.0).is_err());
        assert!(arc.offset(sweep.signum() * 150.0).is_err());
        assert!(arc.offset(-sweep.signum() * 150.0).is_ok());
    }
    let line = Line::new(v(1.0, 2.0), v(41.0, 32.0));
    let shifted = line.offset(5.0);
    assert!(angle_between(shifted.direction(), line.direction()) < ANGLE_EPS);
    assert!(((shifted.a - line.a).length() - 5.0).abs() <= 1e-12);
    assert!((shifted.a - line.a).dot(line.direction()).abs() <= 1e-12);
    assert!(
        (shifted.a - line.a).perp_dot(line.direction()) < 0.0,
        "left of travel"
    );
}

#[test]
fn path_offset_keeps_joints_and_one_way_copy_reverses() {
    let origin = v(-10_000.0, 10_000.0);
    let path = fillet(origin, origin + v(100.0, 0.0), origin + v(100.0, 60.0));
    let shifted = path.offset(7.0).unwrap();
    assert_eq!(shifted.segments().len(), path.segments().len());
    for pair in shifted.segments().windows(2) {
        assert_eq!(pair[0].end(), pair[1].start());
    }
    for (original, piece) in path.segments().iter().zip(shifted.segments()) {
        assert!(angle_between(original.start_tangent(), piece.start_tangent()) < ANGLE_EPS);
        assert!(((piece.start() - original.start()).length() - 7.0).abs() <= 1e-9);
        if let (GuideSegment::Arc(a), GuideSegment::Arc(b)) = (original, piece) {
            assert!((a.centre() - b.centre()).length() <= 1e-9);
            assert!((a.radius() - 7.0 - b.radius()).abs() <= 1e-9);
        }
    }
    let copy = shifted.reversed();
    assert_eq!(copy.start(), shifted.end());
    assert_eq!(copy.end(), shifted.start());
    // The flattened offset is the exact offset's flattening.
    let (mut sampled, mut exact) = (Vec::new(), Vec::new());
    path.flatten_offset_into(7.0, &mut sampled).unwrap();
    shifted.flatten_into(&mut exact);
    assert_eq!(sampled, exact);
    // A corner refuses.
    let mut cornered = GuidePath::new();
    cornered.push(GuideSegment::Line(Line::new(v(0.0, 0.0), v(10.0, 0.0))));
    cornered.push(GuideSegment::Line(Line::new(v(10.0, 0.0), v(10.0, 10.0))));
    assert_eq!(
        cornered.offset(1.0),
        Err(OffsetRefusal::Corner { joint: 1 })
    );
    // A Bezier has no exact offset.
    let curve = bezier(
        origin,
        [(0.0, 0.0), (40.0, 0.0), (60.0, 40.0), (20.0, 50.0)],
    );
    assert_eq!(
        curve.offset(1.0),
        Err(OffsetRefusal::NotClosedForm { segment: 0 })
    );
}

#[test]
fn bezier_offset_is_refused_where_its_radius_is_under_the_offset() {
    let origin = v(10_000.0, 10_000.0);
    // A left-turning span.
    let path = bezier(
        origin,
        [(0.0, 0.0), (40.0, 0.0), (60.0, 40.0), (20.0, 50.0)],
    );
    let GuideSegment::Bezier(curve) = path.segments()[0] else {
        unreachable!()
    };
    let min_radius = curve.min_radius();
    assert!(min_radius > 1.0 && min_radius < 40.0, "{min_radius}");
    let mut points = Vec::new();
    // Towards the centre (left) beyond the radius: refused; the same distance outwards: fine.
    assert_eq!(
        path.flatten_offset_into(1.5 * min_radius, &mut points),
        Err(OffsetRefusal::RadiusUnderOffset { segment: 0 })
    );
    for d in [-1.5 * min_radius, 0.5 * min_radius, -3.0] {
        path.flatten_offset_into(d, &mut points).unwrap();
        assert!(((points[0] - curve.points()[0]).length() - d.abs()).abs() <= 1e-9);
        let last = *points.last().unwrap();
        assert!(((last - curve.points()[3]).length() - d.abs()).abs() <= 1e-9);
        for pair in points.windows(2) {
            assert!((pair[1] - pair[0]).length() <= FLATTEN_MAX_STEP_M);
        }
    }
}

#[test]
fn split_and_join_and_reversal_are_exact() {
    for (ox, oz) in ORIGINS {
        for path in sample_paths(v(ox, oz)) {
            let length = path.length();
            let first_joint = path.segments()[0].length();
            assert!(path.split_at(0.0).is_none());
            assert!(path.split_at(length).is_none());
            for s in [0.1 * length, 0.37 * length, first_joint, 0.9 * length] {
                if s >= length {
                    continue;
                }
                let (head, tail) = path.split_at(s).expect("inside");
                assert_eq!(head.start(), path.start());
                assert_eq!(tail.end(), path.end());
                assert_eq!(head.end(), tail.start());
                assert!((head.length() + tail.length() - length).abs() <= 1e-9 * length);
                assert!((head.length() - s).abs() <= 1e-9 * length);
                let mut joined = head.clone();
                joined.append(&tail);
                for f in [0.0, 0.05, 0.3, 0.5, 0.77, 1.0] {
                    let x = f * length;
                    let error = (joined.point_at(x).unwrap() - path.point_at(x).unwrap()).length();
                    assert!(error <= 1e-9, "join error {error}");
                }
                assert!(
                    angle_between(head.end_tangent().unwrap(), tail.start_tangent().unwrap())
                        < ANGLE_EPS
                );
            }
            let reversed = path.reversed();
            assert_eq!(reversed.reversed(), path);
            assert_eq!(reversed.start(), path.end());
            assert_eq!(reversed.end(), path.start());
            for f in [0.0, 0.2, 0.5, 0.9] {
                let x = f * length;
                let back = reversed.point_at(length - x).unwrap();
                assert!((back - path.point_at(x).unwrap()).length() <= 1e-9);
            }
        }
    }
}

#[test]
fn same_input_gives_bitwise_equal_points() {
    let bits = |points: &[DVec2]| -> Vec<(u64, u64)> {
        points
            .iter()
            .map(|p| (p.x.to_bits(), p.y.to_bits()))
            .collect()
    };
    for (ox, oz) in ORIGINS {
        let (first, second) = (sample_paths(v(ox, oz)), sample_paths(v(ox, oz)));
        // The second buffer starts dirty and larger: the output must not depend on it.
        let mut reused = vec![v(1.0, 2.0); 5000];
        let mut fresh = Vec::new();
        for (a, b) in first.iter().zip(&second) {
            a.flatten_into(&mut fresh);
            b.flatten_into(&mut reused);
            assert_eq!(bits(&fresh), bits(&reused));
            a.flatten_offset_into(-3.5, &mut fresh).unwrap();
            b.flatten_offset_into(-3.5, &mut reused).unwrap();
            assert_eq!(bits(&fresh), bits(&reused));
        }
    }
}
