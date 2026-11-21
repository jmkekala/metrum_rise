// SPDX-License-Identifier: GPL-3.0-or-later

//! Drawn turns of a lot or yard manoeuvre (`PARK-29`): each bend of the polyline a car or truck
//! walks is drawn as a circular arc tangent to both legs, where the vehicle used to pivot.
//!
//! Presentation only. The simulation walks the polyline as before
//! (`advance_along_local_access_path`); positions, arrival ticks and saves never read this. The
//! snapshot maps the vehicle's progress along its current leg onto the rounded line, and the
//! asset editor samples the same line.
//!
//! The bend at point `j`, turning by `θ` between its legs, gets the tangent length
//! `t = min(R tan(θ/2), half of either leg, MAX_TANGENT_M)` and the radius `t / tan(θ/2)`, `R` the
//! class's wanted radius. Half a leg at most, so two arcs never overlap and every leg's midpoint
//! stays on the stored route; a short leg shrinks the arc towards the old pivot. A bend under
//! `MIN_BEND_RAD` keeps its corner, as do the polyline's ends. The `2t` of route around the corner
//! maps linearly onto the arc (`θ / (2 tan(θ/2))` of the speed, 0.79 at 90°), so the vehicle is
//! drawn where it is on every straight part.

use godot::prelude::Vector2;

/// The radius a car's bends are drawn with, metres.
pub(crate) const CAR_TURN_RADIUS_M: f32 = 2.5;
/// The radius a freight carrier's bends are drawn with, metres: every vehicle in a dock
/// manoeuvre, a van included, is proven with the freight body.
pub(crate) const FREIGHT_TURN_RADIUS_M: f32 = 6.0;
/// The longest tangent length of a drawn turn, metres: a truck's at 90°. A sharper bend gets a
/// smaller radius, so the drawn pose keeps within [`MAX_DRAWN_OFFSET_M`] of the simulated one.
const MAX_TANGENT_M: f32 = 6.0;
/// The farthest the drawn position of a manoeuvring vehicle lies from its simulated one: both
/// lie within the tangent length of the corner, the simulated one up to the walker's 5 cm
/// corner cut off its leg (`local_access_target_segment`).
pub(crate) const MAX_DRAWN_OFFSET_M: f32 = 2.0 * MAX_TANGENT_M + 0.05;
// Bends below 5° keep their corner.
const MIN_BEND_RAD: f32 = 5.0 * std::f32::consts::PI / 180.0;
// Legs shorter than this are not rounded at either end.
const MIN_LEG_M: f32 = 1e-4;
// The arc of an editor line is sampled at least every 10° of turn.
const SAMPLE_STEP_RAD: f32 = 10.0 * std::f32::consts::PI / 180.0;

// One bend's arc: from `start` along `incoming`, turning towards `normal` by `angle` on a circle
// of `radius`; `tangent` is the length of route it replaces on either leg.
#[derive(Clone, Copy, Debug)]
struct Fillet {
    start: Vector2,
    incoming: Vector2,
    normal: Vector2,
    radius: f32,
    angle: f32,
    tangent: f32,
}

impl Fillet {
    // The arc's curvature, 1/m, positive turning to the driver's left (`RENDER-66`): in the
    // ground plane's (x, z) a turn toward `incoming`'s counter-clockwise normal is a turn to
    // the driver's right, as `+X` is the right of a car heading `-Z`.
    fn curvature(&self) -> f32 {
        let toward_ccw = self.incoming.cross(self.normal) > 0.0;
        if toward_ccw {
            -1.0 / self.radius
        } else {
            1.0 / self.radius
        }
    }

    // The point and unit travel direction `u` (0 to 1) of the way round the arc.
    fn at(&self, u: f32) -> (Vector2, Vector2) {
        let (sin, cos) = (self.angle * u).sin_cos();
        (
            self.start
                + self.incoming * (self.radius * sin)
                + self.normal * (self.radius * (1.0 - cos)),
            self.incoming * cos + self.normal * sin,
        )
    }
}

// The arc of the bend at point `j` of `points` drawn with the wanted `radius`, or `None` at an
// end, at a bend under 5°, at a reversal (which keeps its pivot) or beside a degenerate leg.
fn fillet(points: &[Vector2], j: usize, radius: f32) -> Option<Fillet> {
    if j == 0 || j + 1 >= points.len() {
        return None;
    }
    let (a, p, b) = (points[j - 1], points[j], points[j + 1]);
    let (into, out) = ((p - a).length(), (b - p).length());
    if into < MIN_LEG_M || out < MIN_LEG_M {
        return None;
    }
    let (incoming, outgoing) = ((p - a) / into, (b - p) / out);
    let (sine, cosine) = (incoming.cross(outgoing), incoming.dot(outgoing));
    let angle = sine.abs().atan2(cosine);
    if angle < MIN_BEND_RAD || 1.0 + cosine < 1e-6 {
        return None;
    }
    // tan(θ/2), finite below the reversal.
    let half_tan = sine.abs() / (1.0 + cosine);
    let tangent = (radius * half_tan)
        .min(0.5 * into)
        .min(0.5 * out)
        .min(MAX_TANGENT_M);
    let left = Vector2::new(-incoming.y, incoming.x);
    Some(Fillet {
        start: p - incoming * tangent,
        incoming,
        normal: if sine > 0.0 { left } else { -left },
        radius: tangent / half_tan,
        angle,
        tangent,
    })
}

/// The drawn position and unit travel direction of a vehicle at `current` on the leg
/// `points[target - 1] → points[target]` (the leg `local_access_target_segment` names), when its
/// progress along that leg lies on one of the leg's two arcs; `None` on a straight part, where it
/// is drawn at `current` facing `points[target]` as before. The progress is `current` projected
/// onto the leg. O(1): the bends at the leg's two ends, no allocation. The snapshot reads
/// [`drawn_arc`]; this is its tests' form.
#[cfg(test)]
pub(crate) fn drawn_turn(
    points: &[Vector2],
    target: usize,
    current: Vector2,
    radius: f32,
) -> Option<(Vector2, Vector2)> {
    drawn_arc(points, target, current, radius).map(|(point, travel, _)| (point, travel))
}

/// [`drawn_turn`] with the arc's curvature, 1/m, positive turning to the driver's left of
/// travel (`RENDER-66`: the steered wheels). O(1), no allocation.
pub(crate) fn drawn_arc(
    points: &[Vector2],
    target: usize,
    current: Vector2,
    radius: f32,
) -> Option<(Vector2, Vector2, f32)> {
    if target == 0 || target >= points.len() {
        return None;
    }
    let (a, b) = (points[target - 1], points[target]);
    let length = (b - a).length();
    if length < MIN_LEG_M {
        return None;
    }
    let along = ((current - a).dot(b - a) / length).clamp(0.0, length);
    let on = |arc: Fillet, u: f32| {
        let (point, travel) = arc.at(u);
        (point, travel, arc.curvature())
    };
    if let Some(end) = fillet(points, target, radius)
        && along > length - end.tangent
    {
        return Some(on(end, (along - (length - end.tangent)) / (2.0 * end.tangent)));
    }
    if let Some(start) = fillet(points, target - 1, radius)
        && along < start.tangent
    {
        return Some(on(start, 0.5 + along / (2.0 * start.tangent)));
    }
    None
}

/// The line a vehicle on `points` is drawn along, as a polyline: the ends, the corners kept and
/// each arc sampled at least every 10° of turn. For the asset editor; allocates.
pub(crate) fn rounded_line(points: &[Vector2], radius: f32) -> Vec<Vector2> {
    let mut line = Vec::with_capacity(points.len() * 4);
    for (j, &point) in points.iter().enumerate() {
        match fillet(points, j, radius) {
            Some(arc) => {
                // The epsilon keeps a whole number of steps (90° in 9) from rounding up.
                let steps = (arc.angle / SAMPLE_STEP_RAD - 1e-3).ceil().max(1.0) as usize;
                line.extend((0..=steps).map(|k| arc.at(k as f32 / steps as f32).0));
            }
            None => line.push(point),
        }
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEG: f32 = std::f32::consts::PI / 180.0;

    // The drawn pose at `fraction` (0 to 1) of the route's length, as the snapshot finds it: the
    // vehicle on the polyline at that distance, on the leg ending at the next point.
    fn drawn_at(points: &[Vector2], fraction: f32, radius: f32) -> (Vector2, Vector2, Vector2) {
        let total: f32 = points.windows(2).map(|w| w[0].distance_to(w[1])).sum();
        let mut left = total * fraction;
        for target in 1..points.len() {
            let (a, b) = (points[target - 1], points[target]);
            let length = a.distance_to(b);
            if left <= length || target == points.len() - 1 {
                let current = a + (b - a) / length * left.min(length);
                let pose = drawn_turn(points, target, current, radius)
                    .unwrap_or((current, (b - a) / length));
                return (current, pose.0, pose.1);
            }
            left -= length;
        }
        unreachable!()
    }

    fn close(a: Vector2, b: Vector2, tolerance: f32) -> bool {
        a.distance_to(b) <= tolerance
    }

    // A right-angled bend with long legs: the arc has the wanted radius, starts and ends `R` from
    // the corner, every drawn point lies on its circle and faces along it, and the ends, the leg
    // midpoints and the straight parts are drawn where the vehicle is.
    #[test]
    fn a_right_angle_is_drawn_on_a_circle_of_the_wanted_radius() {
        let points = [
            Vector2::new(0.0, 0.0),
            Vector2::new(10.0, 0.0),
            Vector2::new(10.0, 10.0),
        ];
        let arc = fillet(&points, 1, CAR_TURN_RADIUS_M).unwrap();
        assert!((arc.radius - 2.5).abs() < 1e-4 && (arc.tangent - 2.5).abs() < 1e-4);
        let centre = Vector2::new(7.5, 2.5);
        for k in 0..=200 {
            let (stored, drawn, heading) = drawn_at(&points, k as f32 / 200.0, CAR_TURN_RADIUS_M);
            assert!((heading.length() - 1.0).abs() < 1e-4);
            let along = k as f32 / 10.0;
            if (7.5..=12.5).contains(&along) {
                assert!(
                    (drawn.distance_to(centre) - 2.5).abs() < 1e-4,
                    "{along}: {drawn:?}"
                );
                assert!(
                    heading.dot(drawn - centre).abs() < 1e-3,
                    "tangent at {along}"
                );
                assert!(
                    heading.dot(Vector2::new(1.0, 1.0)) > 0.0,
                    "forward at {along}"
                );
            } else {
                assert!(close(drawn, stored, 1e-5), "straight at {along}");
            }
        }
        for fraction in [0.0, 0.25, 0.75, 1.0] {
            let (stored, drawn, _) = drawn_at(&points, fraction, CAR_TURN_RADIUS_M);
            assert!(close(drawn, stored, 1e-5), "{fraction}");
        }
        // The arc's middle is drawn when the vehicle stands at the corner.
        let (_, middle, heading) = drawn_at(&points, 0.5, CAR_TURN_RADIUS_M);
        let expected = centre + Vector2::new(1.0, -1.0).normalized() * 2.5;
        assert!(close(middle, expected, 1e-4), "{middle:?}");
        assert!(heading.dot(Vector2::new(1.0, 1.0).normalized()) > 0.9999);
    }

    // A truck's wanted radius; a bend under 5° keeps its corner; a short leg limits the arc to
    // half of it, so a leg between two bends keeps its midpoint, and a leg too short rounds
    // nothing more than the pivot it was.
    #[test]
    fn short_legs_shrink_the_arc_and_slight_bends_keep_their_corner() {
        let wide = [
            Vector2::new(0.0, 0.0),
            Vector2::new(20.0, 0.0),
            Vector2::new(20.0, 20.0),
        ];
        assert!((fillet(&wide, 1, FREIGHT_TURN_RADIUS_M).unwrap().radius - 6.0).abs() < 1e-4);
        let slight = [
            Vector2::new(0.0, 0.0),
            Vector2::new(10.0, 0.0),
            Vector2::new(20.0, 0.8),
        ];
        assert!(fillet(&slight, 1, FREIGHT_TURN_RADIUS_M).is_none(), "4.6°");
        // A 2 m leg between two right angles: each arc takes 1 m of it, radius 1 m.
        let jog = [
            Vector2::new(0.0, 0.0),
            Vector2::new(10.0, 0.0),
            Vector2::new(10.0, 2.0),
            Vector2::new(20.0, 2.0),
        ];
        for j in [1, 2] {
            let arc = fillet(&jog, j, CAR_TURN_RADIUS_M).unwrap();
            assert!((arc.tangent - 1.0).abs() < 1e-5 && (arc.radius - 1.0).abs() < 1e-5);
        }
        let (stored, drawn, heading) = drawn_at(&jog, 11.0 / 22.0, CAR_TURN_RADIUS_M);
        assert!(close(stored, Vector2::new(10.0, 1.0), 1e-5) && close(drawn, stored, 1e-5));
        assert!(close(heading, Vector2::new(0.0, 1.0), 1e-5));
        // The sharpest bends keep within the drawn offset bound.
        let hairpin = [
            Vector2::new(0.0, 0.0),
            Vector2::new(30.0, 0.0),
            Vector2::new(0.0, 1.0),
        ];
        let arc = fillet(&hairpin, 1, FREIGHT_TURN_RADIUS_M).unwrap();
        assert!(arc.tangent <= MAX_TANGENT_M);
        for k in 0..=100 {
            let (stored, drawn, _) = drawn_at(&hairpin, k as f32 / 100.0, FREIGHT_TURN_RADIUS_M);
            assert!(stored.distance_to(drawn) <= MAX_DRAWN_OFFSET_M);
        }
    }

    // A truck's way of authored 15° chords round a 12 m arc (`arc_waypoints`) looks as it did:
    // the drawn line keeps within 6 cm of the chords and within 8° of each chord's heading.
    #[test]
    fn a_chord_arc_way_looks_unchanged() {
        let points: Vec<_> = (0..=6)
            .map(|k| {
                let a = (k as f32 * 15.0) * DEG;
                Vector2::new(12.0 * a.sin(), 12.0 * (1.0 - a.cos()))
            })
            .collect();
        for k in 0..=600 {
            let fraction = k as f32 / 600.0;
            let (stored, drawn, heading) = drawn_at(&points, fraction, FREIGHT_TURN_RADIUS_M);
            assert!(
                stored.distance_to(drawn) < 0.06,
                "{fraction}: {stored:?} {drawn:?}"
            );
            let target = (1..points.len())
                .find(|&t| {
                    points[t].distance_to(stored) > 1e-4 && {
                        let (a, b) = (points[t - 1], points[t]);
                        (stored - a).dot(b - a) >= -1e-4 && (stored - b).dot(a - b) >= -1e-4
                    }
                })
                .unwrap_or(points.len() - 1);
            let chord = (points[target] - points[target - 1]).normalized();
            assert!(heading.dot(chord) > (8.0 * DEG).cos(), "{fraction}");
        }
    }

    // The same route walked backwards is drawn on the same line: a truck backing out along its
    // way in passes the points it was drawn at driving in.
    #[test]
    fn a_reversed_route_is_drawn_on_the_same_line() {
        let points = [
            Vector2::new(0.0, 0.0),
            Vector2::new(8.0, 0.0),
            Vector2::new(11.0, 6.0),
            Vector2::new(11.0, 14.0),
        ];
        let back: Vec<_> = points.iter().rev().copied().collect();
        for k in 0..=100 {
            let fraction = k as f32 / 100.0;
            let (_, forward, heading) = drawn_at(&points, fraction, FREIGHT_TURN_RADIUS_M);
            let (_, reverse, back_heading) = drawn_at(&back, 1.0 - fraction, FREIGHT_TURN_RADIUS_M);
            assert!(
                close(forward, reverse, 2e-4),
                "{fraction}: {forward:?} {reverse:?}"
            );
            assert!(heading.dot(back_heading) < -0.9999, "{fraction}");
        }
    }

    // The editor's line runs through the ends and the kept corners and along every arc.
    #[test]
    fn the_rounded_line_samples_every_arc() {
        let points = [
            Vector2::new(0.0, 0.0),
            Vector2::new(10.0, 0.0),
            Vector2::new(10.0, 10.0),
        ];
        let line = rounded_line(&points, CAR_TURN_RADIUS_M);
        assert_eq!(line.len(), 2 + 10, "9 steps of 10° and the ends");
        assert!(close(line[0], points[0], 0.0) && close(line[11], points[2], 1e-5));
        let centre = Vector2::new(7.5, 2.5);
        for p in &line[1..11] {
            assert!((p.distance_to(centre) - 2.5).abs() < 1e-4);
        }
    }
}
