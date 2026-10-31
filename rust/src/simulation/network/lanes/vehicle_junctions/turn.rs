// SPDX-License-Identifier: GPL-2.0-only

//! The curve of one vehicle junction connector (`TRAFFIC-12`), from the incoming lane's mouth
//! frame to the outgoing lane's: one construction that always exists, built from straights and
//! circular arcs only, so its direction is continuous and its curvature piecewise constant.
//!
//! - **Tangent arc.** Where the two lane directions meet ahead of both ends, a straight along
//!   the entry direction, one arc and a straight along the exit direction: a fillet of the two
//!   tangent lines. The arc never reaches past the lines' intersection, so it neither overshoots
//!   nor counter-steers. Its radius is the largest that fits between the two ends, reduced where
//!   needed so the arc keeps half a lane from every kerb corner of the node: a large fillet cuts
//!   across the inside corner of a turn between long mouths (a skewed junction's obtuse corner);
//!   at a right-angle kerb lane turn the bound is the arc concentric with the kerb. The tangent
//!   arc and its kerb bound are Veeti's (road-chain branch, `1ae3b22f`).
//! - **Biarc.** Otherwise (straight on, a lateral shift, or directions that meet behind an
//!   end), two arcs with a common tangent at their joint. The joint is the deterministic
//!   equal-tangent choice: both ends' tangent lines are cut at the same distance `d`, and the
//!   joint is the midpoint of the two cut points, its tangent along the segment between them.
//!   `d > 0` exists for every pair of ends except parallel directions with the exit behind or
//!   beside the entry, which no junction produces (its mouths lie on opposite sides of the node
//!   piece; a degree-two node whose lane ends overlap links its lanes directly).
//!
//! Building is `O(c)` for `c` kerb corners, `O(c * RADIUS_BISECTION_STEPS)` when the radius is
//! reduced; a point is `O(1)`. Nothing is allocated but the returned polyline.

use super::super::super::roadbed::{Corner, RoadVec2};
use crate::config;
use godot::prelude::Vector3;

// Connectors are sampled at most this far apart along their path, and at most this turn apart
// along an arc.
const MAX_STEP_M: f64 = 1.0;
const MAX_STEP_RAD: f64 = 0.15;
// Turn arcs keep a kerb lane's own clearance, half a lane, from every kerb corner.
const KERB_CLEARANCE_M: f64 = config::LANE_WIDTH as f64 * 0.5;
// Halvings of the radius search: below a millimetre for any junction-sized fit.
const RADIUS_BISECTION_STEPS: usize = 24;
// Directions closer to parallel than this (the sine of the angle between them) have no
// intersection to fillet.
const PARALLEL_SINE: f64 = 1.0e-9;
// An arc turning less than this is a straight.
const STRAIGHT_RAD: f64 = 1.0e-9;
// Pieces shorter than this (a tangent arc's used-up lead) are not sampled, so the polyline has
// no zero-length segment.
const MIN_SAMPLED_PIECE_M: f64 = 1.0e-3;

/// Which construction a connector uses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TurnKind {
    /// Straight, arc, straight.
    TangentArc,
    /// Two arcs with a common tangent.
    Biarc,
}

// A straight (`curvature == 0`) or a circular arc, from `start` along unit `tangent`; positive
// curvature turns left, towards `tangent.perp()`.
#[derive(Clone, Copy, Debug)]
struct Piece {
    start: RoadVec2,
    tangent: RoadVec2,
    length: f64,
    curvature: f64,
}

impl Piece {
    fn straight(start: RoadVec2, tangent: RoadVec2, length: f64) -> Self {
        Self {
            start,
            tangent,
            length,
            curvature: 0.0,
        }
    }

    // The arc from `start` along `tangent` through `end`; a straight where they are aligned.
    fn to_point(start: RoadVec2, tangent: RoadVec2, end: RoadVec2) -> Self {
        let chord = end - start;
        let chord_m = chord.length();
        // The arc turns twice the angle between its tangent and its chord.
        let half_turn = tangent.perp_dot(chord).atan2(tangent.dot(chord));
        if half_turn.abs() < STRAIGHT_RAD {
            return Self::straight(start, tangent, chord_m);
        }
        Self {
            start,
            tangent,
            length: chord_m * half_turn / half_turn.sin(),
            curvature: 2.0 * half_turn.sin() / chord_m,
        }
    }

    fn turn_rad(&self) -> f64 {
        self.curvature * self.length
    }

    fn point(&self, distance_m: f64) -> RoadVec2 {
        if self.curvature == 0.0 {
            return self.start + self.tangent * distance_m;
        }
        let (sine, cosine) = (self.curvature * distance_m).sin_cos();
        self.start + (self.tangent * sine + self.tangent.perp() * (1.0 - cosine)) / self.curvature
    }

    // Segments this piece is sampled with.
    fn steps(&self) -> usize {
        let by_length = (self.length / MAX_STEP_M).ceil();
        let by_turn = (self.turn_rad().abs() / MAX_STEP_RAD).ceil();
        (by_length.max(by_turn) as usize).max(1)
    }

    // XZ distance from `point` to the piece.
    fn distance_to(&self, point: RoadVec2) -> f64 {
        let end = self.point(self.length);
        let to_ends = point.distance(self.start).min(point.distance(end));
        if self.curvature == 0.0 {
            let along = (point - self.start).dot(self.tangent);
            if (0.0..=self.length).contains(&along) {
                return (point - self.start).perp_dot(self.tangent).abs();
            }
            return to_ends;
        }
        let centre = self.start + self.tangent.perp() / self.curvature;
        let (from, offset) = (self.start - centre, point - centre);
        // The point's angle about the centre from the arc's start, in the arc's turning sense.
        let mut angle = from.perp_dot(offset).atan2(from.dot(offset)) * self.curvature.signum();
        if angle < 0.0 {
            angle += std::f64::consts::TAU;
        }
        if angle <= self.turn_rad().abs() {
            (offset.length() - 1.0 / self.curvature.abs()).abs()
        } else {
            to_ends
        }
    }
}

/// One connector curve.
pub(crate) struct VehicleTurn {
    pieces: [Piece; 3],
    count: usize,
    /// The construction used.
    pub(crate) kind: TurnKind,
}

impl VehicleTurn {
    /// The curve from `start`, entering along unit `incoming`, to `end`, leaving along unit
    /// `outgoing`, keeping clear of `kerb_corners` where a tangent arc's radius allows. `None`
    /// only for parallel directions with `end` not ahead of `start` (see the module notes).
    pub(crate) fn new(
        start: RoadVec2,
        incoming: RoadVec2,
        end: RoadVec2,
        outgoing: RoadVec2,
        kerb_corners: &[Corner],
    ) -> Option<Self> {
        Self::tangent_arc(start, incoming, end, outgoing, kerb_corners)
            .or_else(|| Self::biarc(start, incoming, end, outgoing))
    }

    fn tangent_arc(
        start: RoadVec2,
        incoming: RoadVec2,
        end: RoadVec2,
        outgoing: RoadVec2,
        kerb_corners: &[Corner],
    ) -> Option<Self> {
        let sine = incoming.perp_dot(outgoing);
        if sine.abs() <= PARALLEL_SINE {
            return None;
        }
        // The tangent lines meet `ahead` metres after the entry and `behind` before the exit.
        let delta = end - start;
        let ahead = delta.perp_dot(outgoing) / sine;
        let behind = incoming.perp_dot(delta) / sine;
        if ahead <= 0.0 || behind <= 0.0 {
            return None;
        }
        let turn = sine.atan2(incoming.dot(outgoing));
        let half_tan = (0.5 * turn.abs()).tan();
        let corner = start + incoming * ahead;
        let pieces = |radius: f64| {
            let setback = radius * half_tan;
            let arc_start = corner - incoming * setback;
            let arc = Piece {
                start: arc_start,
                tangent: incoming,
                length: radius * turn.abs(),
                curvature: turn.signum() / radius,
            };
            [
                Piece::straight(start, incoming, ahead - setback),
                arc,
                Piece::straight(corner + outgoing * setback, outgoing, behind - setback),
            ]
        };
        let clears = |radius: f64| {
            let arc = pieces(radius)[1];
            kerb_corners
                .iter()
                .all(|corner| arc.distance_to(corner.centre) >= corner.radius_m + KERB_CLEARANCE_M)
        };
        let mut radius = ahead.min(behind) / half_tan;
        if !clears(radius) {
            // A smaller fillet lies closer to the tangents' intersection, away from the inside
            // corner: the largest radius that clears every corner.
            let (mut low, mut high) = (0.0, radius);
            for _ in 0..RADIUS_BISECTION_STEPS {
                let mid = 0.5 * (low + high);
                if clears(mid) {
                    low = mid;
                } else {
                    high = mid;
                }
            }
            if low == 0.0 {
                // No arc clears the kerbs: the tangent arc does not fit.
                return None;
            }
            radius = low;
        }
        let [entry, arc, exit] = pieces(radius);
        // The longer lead keeps its straight; the shorter one is used up by the arc.
        let entry = Piece {
            length: entry.length.max(0.0),
            ..entry
        };
        let exit = Piece {
            length: exit.length.max(0.0),
            ..exit
        };
        Some(Self {
            pieces: [entry, arc, exit],
            count: 3,
            kind: TurnKind::TangentArc,
        })
    }

    fn biarc(
        start: RoadVec2,
        incoming: RoadVec2,
        end: RoadVec2,
        outgoing: RoadVec2,
    ) -> Option<Self> {
        // Equal tangent lengths `d`: |end - d * outgoing - (start + d * incoming)| = 2 d, the
        // positive root in its cancellation-free form.
        let delta = end - start;
        if delta == RoadVec2::ZERO {
            // Coincident ends (a user connection through a degree-two node): no path at all.
            let none = Piece::straight(start, incoming, 0.0);
            return Some(Self {
                pieces: [none; 3],
                count: 1,
                kind: TurnKind::Biarc,
            });
        }
        let along = delta.dot(incoming + outgoing);
        let spread = 2.0 * (1.0 - incoming.dot(outgoing)) * delta.length_squared();
        let denominator = along + (along * along + spread).sqrt();
        if denominator <= 0.0 {
            return None;
        }
        let d = delta.length_squared() / denominator;
        let (cut_in, cut_out) = (start + incoming * d, end - outgoing * d);
        let joint = cut_in.lerp(cut_out, 0.5);
        let joint_tangent = (cut_out - cut_in).normalize();
        let first = Piece::to_point(start, incoming, joint);
        // The second arc is built backwards from the exit so both ends are exact.
        let second = Piece::to_point(end, -outgoing, joint);
        let second = Piece {
            start: joint,
            tangent: joint_tangent,
            length: second.length,
            curvature: -second.curvature,
        };
        Some(Self {
            pieces: [first, second, second],
            count: 2,
            kind: TurnKind::Biarc,
        })
    }

    fn pieces(&self) -> &[Piece] {
        &self.pieces[..self.count]
    }

    /// Path length.
    pub(crate) fn length_m(&self) -> f64 {
        self.pieces().iter().map(|piece| piece.length).sum()
    }

    /// Smallest arc radius, `INFINITY` for a straight.
    #[cfg(test)]
    pub(crate) fn min_radius_m(&self) -> f64 {
        self.pieces()
            .iter()
            .filter(|piece| piece.curvature != 0.0)
            .map(|piece| 1.0 / piece.curvature.abs())
            .fold(f64::INFINITY, f64::min)
    }

    /// The smallest margin by which the curve stays outside the carriageway-edge circle of a
    /// kerb corner (`distance - radius`), `INFINITY` without corners: negative where it leaves
    /// the carriageway at a kerb return.
    pub(crate) fn kerb_margin_m(&self, kerb_corners: &[Corner]) -> f64 {
        kerb_corners
            .iter()
            .flat_map(|corner| {
                self.pieces()
                    .iter()
                    .map(|piece| piece.distance_to(corner.centre) - corner.radius_m)
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// The sampled polyline from `start` to `end` (their XZ are the curve's ends, bitwise), at
    /// most `MAX_STEP_M` and `MAX_STEP_RAD` apart, its height linear in path length.
    pub(crate) fn polyline(&self, start: Vector3, end: Vector3) -> Vec<Vector3> {
        let sampled = || {
            self.pieces()
                .iter()
                .filter(|piece| piece.length >= MIN_SAMPLED_PIECE_M)
        };
        let steps: usize = sampled().map(Piece::steps).sum();
        let total_m = self.length_m();
        let mut points = Vec::with_capacity(steps + 1);
        points.push(start);
        let mut walked_m = 0.0;
        for piece in self.pieces() {
            if piece.length < MIN_SAMPLED_PIECE_M {
                walked_m += piece.length;
                continue;
            }
            let count = piece.steps();
            for step in 1..=count {
                let distance_m = piece.length * step as f64 / count as f64;
                let xz = piece.point(distance_m);
                let fraction = if total_m > 0.0 {
                    (walked_m + distance_m) / total_m
                } else {
                    1.0
                };
                let height =
                    f64::from(start.y) + (f64::from(end.y) - f64::from(start.y)) * fraction;
                points.push(Vector3::new(xz.x as f32, height as f32, xz.y as f32));
            }
            walked_m += piece.length;
        }
        if points.len() == 1 {
            points.push(end);
        } else if let Some(last) = points.last_mut() {
            *last = end;
        }
        points
    }
}
