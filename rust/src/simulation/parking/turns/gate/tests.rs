// SPDX-License-Identifier: GPL-3.0-or-later

//! Tests of the gate turn's geometry (`PARK-31`).

use super::*;
use crate::simulation::parking::turns::{CAR_TURN_RADIUS_M, FREIGHT_TURN_RADIUS_M, drawn_turn};

const SQRT_2: f32 = std::f32::consts::SQRT_2;

fn close(a: Vector2, b: Vector2, tolerance: f32) -> bool {
    a.distance_to(b) <= tolerance
}

// A lane along +x through the origin; the lot on its right, which is -z... in the ground plane's
// (x, z) the right of travel +x is the counter-clockwise normal (0, 1).
fn gate(radius: f32, egress: bool) -> GateLane {
    GateLane {
        point: Vector2::ZERO,
        along: Vector2::new(1.0, 0.0),
        hand: LOT_HAND,
        radius,
        egress,
    }
}

// The vehicle at `travelled` metres along `points` from the lane point (its first point entering,
// last leaving): its simulated position, and the leg it is on as the snapshot names it.
fn on_route(points: &[Vector2], egress: bool, travelled: f32) -> (Vector2, usize) {
    let outward: Vec<_> = if egress {
        points.iter().rev().copied().collect()
    } else {
        points.to_vec()
    };
    let mut left = travelled;
    for leg in 1..outward.len() {
        let (a, b) = (outward[leg - 1], outward[leg]);
        let length = a.distance_to(b);
        if left <= length || leg == outward.len() - 1 {
            let current = a + (b - a) / length * left.min(length);
            let target = if egress { points.len() - leg } else { leg };
            return (current, target);
        }
        left -= length;
    }
    unreachable!()
}

// The drawn pose of a vehicle `x` metres from the lane point: on the lane (negative `x`: before
// it entering, past it leaving, as its signed distance from the lane point along the route) or
// off it, as the snapshot would draw it on a straight lane along +x.
fn drawn(
    gate: &GateLane,
    points: &[Vector2],
    x: f32,
    turn_radius: Option<f32>,
) -> Option<DrawnPose> {
    if x < 0.0 {
        let along = if gate.egress { -x } else { x };
        return on_lane(
            gate.radius,
            gate.hand,
            gate.egress,
            -x,
            Vector2::new(along, 0.0),
            gate.along,
        );
    }
    let (current, target) = on_route(points, gate.egress, x);
    gate.on_hop(points, turn_radius, current, target)
}

// The lot side of the test lane is +z (`gate`); a square hop of `hop` metres, then a long lot leg
// on the same line.
fn square(hop: f32) -> Vec<Vector2> {
    vec![
        Vector2::ZERO,
        Vector2::new(0.0, hop),
        Vector2::new(0.0, hop + 20.0),
        Vector2::new(15.0, hop + 20.0),
    ]
}

fn reversed(points: &[Vector2]) -> Vec<Vector2> {
    points.iter().rev().copied().collect()
}

// The lane half is a quarter circle's half: tangent to the lane where the reach begins (the lane
// pose exactly), at the arc's middle facing 45° at the lane point, every point on the circle and
// facing along it; leaving is the mirror.
#[test]
fn the_lane_half_is_on_the_circle_and_meets_the_lane_pose() {
    for radius in [CAR_GATE_RADIUS_M, FREIGHT_GATE_RADIUS_M] {
        for egress in [false, true] {
            let gate = gate(radius, egress);
            let sign = if egress { 1.0 } else { -1.0 };
            let centre = Vector2::new(sign * radius, radius);
            let start = on_lane(
                radius,
                LOT_HAND,
                egress,
                radius,
                Vector2::new(sign * radius, 0.0),
                gate.along,
            )
            .unwrap();
            assert!(close(start.0, Vector2::new(sign * radius, 0.0), 1e-5));
            assert!(close(start.1, gate.along, 1e-6), "{radius} {egress}");
            assert!((start.2 + 1.0 / radius).abs() < 1e-6, "a right turn");
            for k in 0..=100 {
                let x = radius * k as f32 / 100.0;
                let (point, travel, _) = on_lane(
                    radius,
                    LOT_HAND,
                    egress,
                    x,
                    Vector2::new(sign * x, 0.0),
                    gate.along,
                )
                .unwrap();
                assert!((point.distance_to(centre) - radius).abs() < 1e-4, "{x}");
                assert!(travel.dot(point - centre).abs() < 1e-4, "{x}");
                assert!(point.distance_to(Vector2::new(sign * x, 0.0)) <= GATE_LANE_OFFSET_M);
            }
            let (middle, travel, _) = gate.at_lane_point();
            let inset = (SQRT_2 - 1.0) / SQRT_2 * radius;
            assert!(
                close(middle, Vector2::new(sign * inset, inset), 1e-5),
                "{middle:?}"
            );
            assert!(
                close(travel, Vector2::new(1.0, -sign).normalized(), 1e-5),
                "{travel:?}"
            );
            assert!(
                on_lane(
                    radius,
                    LOT_HAND,
                    egress,
                    radius + 0.01,
                    Vector2::ZERO,
                    gate.along
                )
                .is_none()
            );
        }
    }
}

// On a square hop as long as the radius, the hop half finishes the same quarter circle (a cubic
// within 0.1 % of the radius), meeting the lane half at the lane point exactly and the route
// where the run ends; farther in the lot's own bends draw the vehicle.
#[test]
fn a_long_square_hop_finishes_the_circle() {
    for radius in [CAR_GATE_RADIUS_M, FREIGHT_GATE_RADIUS_M] {
        for egress in [false, true] {
            let gate = gate(radius, egress);
            let forward = square(radius + 3.0);
            let points = if egress { reversed(&forward) } else { forward };
            let sign = if egress { 1.0 } else { -1.0 };
            let centre = Vector2::new(sign * radius, radius);
            let at_point = drawn(&gate, &points, 0.0, Some(CAR_TURN_RADIUS_M)).unwrap();
            let middle = gate.at_lane_point();
            assert_eq!(at_point.0, middle.0);
            assert_eq!(at_point.1, middle.1);
            for k in 0..100 {
                let x = radius * k as f32 / 100.0;
                let (point, travel, curvature) =
                    drawn(&gate, &points, x, Some(CAR_TURN_RADIUS_M)).unwrap();
                assert!(
                    (point.distance_to(centre) - radius).abs() < 1e-3 * radius,
                    "{x}: {point:?}"
                );
                assert!(travel.dot(point - centre).abs() < 2e-3 * radius, "{x}");
                assert!((curvature * radius + 1.0).abs() < 0.05, "{x}: {curvature}");
            }
            assert!(drawn(&gate, &points, radius, Some(CAR_TURN_RADIUS_M)).is_none());
            let (end, _) = on_route(&points, egress, radius - 1e-4);
            let (point, travel, _) =
                drawn(&gate, &points, radius - 1e-4, Some(CAR_TURN_RADIUS_M)).unwrap();
            assert!(close(point, end, 1e-3));
            assert!(close(travel, Vector2::new(0.0, -sign), 1e-3));
        }
    }
}

// Sampled finely from the lane through the gate into the lot (and out), position and heading
// never jump: no pop at the lane point, and none where either half meets the route.
fn assert_continuous(gate: &GateLane, points: &[Vector2], turn_radius: Option<f32>) {
    let step = 0.01;
    let mut previous: Option<(Vector2, Vector2)> = None;
    let reach = gate.radius + 2.0;
    let mut x = -reach;
    while x <= reach {
        let pose = drawn(gate, points, x, turn_radius).map_or_else(
            || {
                if x < 0.0 {
                    let along = if gate.egress { -x } else { x };
                    (Vector2::new(along, 0.0), gate.along)
                } else {
                    let (current, target) = on_route(points, gate.egress, x);
                    let (a, b) = (points[target - 1], points[target]);
                    turn_radius
                        .and_then(|radius| drawn_turn(points, target, current, radius))
                        .unwrap_or((current, (b - a).normalized()))
                }
            },
            |(point, travel, _)| (point, travel),
        );
        if let Some((point, travel)) = previous {
            assert!(
                point.distance_to(pose.0) < 3.0 * step,
                "{x}: {point:?} {:?}",
                pose.0
            );
            assert!(
                travel.dot(pose.1) > (2.0f32).to_radians().cos(),
                "{x}: {travel:?} {:?}",
                pose.1
            );
        }
        let (simulated, _) = if x < 0.0 {
            (Vector2::new(if gate.egress { -x } else { x }, 0.0), 0)
        } else {
            on_route(points, gate.egress, x)
        };
        // The bound the cull takes, and the offset these turns really show.
        assert!(simulated.distance_to(pose.0) <= GATE_HOP_OFFSET_M, "{x}");
        assert!(simulated.distance_to(pose.0) <= 2.6, "{x}: {:?}", pose.0);
        previous = Some(pose);
        x += step;
    }
}

// A truck on a two-lane street's 3.2 m hop whose first yard leg turns at once, a car on the same
// hop, a skewed hop (an authored way's first point off the square), and a hop continuing into a
// long first leg: every turn is continuous in position and heading both ways, within the bound.
#[test]
fn every_hop_meets_the_arc_without_a_jump() {
    let turning = vec![
        Vector2::ZERO,
        Vector2::new(0.0, 3.2),
        Vector2::new(8.0, 3.2),
        Vector2::new(8.0, 20.0),
    ];
    let skewed = vec![
        Vector2::ZERO,
        Vector2::new(1.5, 4.0),
        Vector2::new(1.5, 18.0),
    ];
    for (points, radius, turn_radius) in [
        (
            turning.clone(),
            FREIGHT_GATE_RADIUS_M,
            FREIGHT_TURN_RADIUS_M,
        ),
        (turning, CAR_GATE_RADIUS_M, CAR_TURN_RADIUS_M),
        (skewed, FREIGHT_GATE_RADIUS_M, FREIGHT_TURN_RADIUS_M),
        (square(3.2), FREIGHT_GATE_RADIUS_M, FREIGHT_TURN_RADIUS_M),
    ] {
        for egress in [false, true] {
            let points = if egress {
                reversed(&points)
            } else {
                points.clone()
            };
            assert_continuous(&gate(radius, egress), &points, Some(turn_radius));
        }
    }
}

// Where the route turns at the kerb (a yard's first leg along the road), a truck's hop half does
// not double back to the hop: it ends on the lot's line 6 m along the route, past that bend, and
// the lot's line draws it from there.
#[test]
fn the_hop_half_hands_over_to_the_lots_line() {
    let points = [
        Vector2::ZERO,
        Vector2::new(0.0, 3.2),
        Vector2::new(8.0, 3.2),
        Vector2::new(8.0, 20.0),
    ];
    let gate = gate(FREIGHT_GATE_RADIUS_M, false);
    let (point, travel, _) = drawn(&gate, &points, 5.999, Some(FREIGHT_TURN_RADIUS_M)).unwrap();
    assert!(close(point, Vector2::new(2.8, 3.2), 2e-3), "{point:?}");
    assert!(close(travel, Vector2::new(1.0, 0.0), 2e-3), "{travel:?}");
    assert!(drawn(&gate, &points, 6.001, Some(FREIGHT_TURN_RADIUS_M)).is_none());
    // Its depth into the hop only grows: it never turns back toward the road.
    let mut depth = gate.at_lane_point().0.y;
    for k in 1..=600 {
        let (point, _, _) = drawn(
            &gate,
            &points,
            k as f32 * 0.01 - 1e-4,
            Some(FREIGHT_TURN_RADIUS_M),
        )
        .unwrap();
        assert!(point.y >= depth - 1e-4, "{k}: {point:?}");
        depth = point.y;
    }
}

// Lanes to cross (not built): a 10 m hop over the near lanes is walked straight, the arc only in
// its last 6 m, into the far lane.
#[test]
fn a_crossing_runs_straight_and_arcs_at_the_end() {
    let gate = gate(FREIGHT_GATE_RADIUS_M, true);
    let points = reversed(&square(10.0));
    assert!(drawn(&gate, &points, 6.5, None).is_none());
    assert!(drawn(&gate, &points, 5.5, None).is_some());
    assert_continuous(&gate, &points, None);
}
