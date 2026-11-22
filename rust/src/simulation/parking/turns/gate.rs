// SPDX-License-Identifier: GPL-3.0-or-later

//! The gate turn (`PARK-31`): a car or truck leaving its lane for a lot or yard, or joining it
//! from one, is drawn on an arc between the lane and the hop (the manoeuvre's leg from the lane
//! point across the sidewalk), where it used to turn on the spot at the lane point.
//!
//! Presentation only, like the bends inside the lot ([`super`]). The lane's direction at the lane
//! point is the turn's virtual first (entering) or last (leaving) leg, and the hop is taken square
//! to it on the lot's side, so the arc is a quarter circle of the class's radius `R` tangent to
//! the lane `R` before the lane point (entering) or `R` past it (leaving). The `2R` of route round
//! the corner maps linearly onto it, as in the lot, which splits it at the lane point:
//!
//! - **On the lane** ([`on_lane`]): within `R` of the lane point the car is drawn on the arc's lane
//!   half, as an offset from its own lane pose in that pose's frame, so on a curved lane the arc
//!   bends with the lane and meets the lane pose exactly where the reach begins. At the lane
//!   point it is at the arc's middle, `(√2 - 1) R` from the lane point, turned 45°.
//! - **Off the lane** ([`GateLane::on_hop`]): the hop half runs from the arc's middle to the pose
//!   the lot's own rounded line ([`super::drawn_arc`]) gives `R` along the route from the lane
//!   point (or at the route's end, if nearer), on a cubic tangent to both, with a circle's handles:
//!   on a square hop at least `R` long it finishes the same quarter circle (within 0.1 % of `R`),
//!   and where the route bends sooner (a yard whose first leg turns at the kerb) it leads from the
//!   arc's middle straight onto the lot's line past that bend, which it does not draw, rather
//!   than doubling back to the hop. Either way it meets both neighbours in position and heading.
//!
//! Lanes to cross (not built: lot accesses are same-side only) fit the same function: the lane
//! point is then on the far lane, the hop crosses the near ones, `hand` names the side the lot
//! lies on, and `ρ` caps the arc to the last `R` of the crossing, so the vehicle runs straight
//! over the lanes it crosses and arcs only into its own: the hop half starts `R` before the lane
//! point along the hop.

use godot::prelude::Vector2;

use super::drawn_arc;

/// The radius a car is drawn joining or leaving its lane at a lot gate with, metres: the lot's
/// own bends' ([`CAR_TURN_RADIUS_M`](super::CAR_TURN_RADIUS_M)).
pub(crate) const CAR_GATE_RADIUS_M: f32 = 2.5;
/// The radius a freight carrier is drawn joining or leaving its lane with, metres: the yard's
/// bends' ([`FREIGHT_TURN_RADIUS_M`](super::FREIGHT_TURN_RADIUS_M)); its arc starts 6 m before
/// the gate and swings up to 1.76 m toward the kerb at the lane point.
pub(crate) const FREIGHT_GATE_RADIUS_M: f32 = 6.0;
/// The side of travel a lot lies on, +1 right: same-side access gives a lot on side -1 the
/// forward lane and one on side +1 the backward lane (`derive_car_lanes`), and either side's
/// outward normal is the right of its lane's travel.
pub(crate) const LOT_HAND: f32 = 1.0;
/// The farthest a lane car on its gate turn is drawn from its lane pose: `(√2 - 1) R` at the lane
/// point for the freight radius (2.485 m), rounded up.
pub(crate) const GATE_LANE_OFFSET_M: f32 = 2.5;
/// The farthest a vehicle off its lane on the hop half of a gate turn is drawn from its simulated
/// position, a loose bound for the camera cull (the stills and tests show under 3 m): the
/// vehicle within `R` of the lane point; the cubic within its control points: the arc's middle,
/// the lot's line `R` along (within twice the lot's longest tangent, 12 m, of the route there)
/// and handles of at most half the chord. `2.71 R + 18` for the freight radius, rounded up.
pub(crate) const GATE_HOP_OFFSET_M: f32 = 34.3;

/// A drawn pose: the ground point, the unit direction of travel, and the curvature, 1/m,
/// positive to the driver's left (`RENDER-66`).
pub(crate) type DrawnPose = (Vector2, Vector2, f32);

/// The offset of the drawn pose from a lane car's own lane pose, `x` metres from its lane point
/// (before it entering, past it leaving: `egress`), in that pose's frame: along the lane, toward
/// the lot, and the heading's turn toward the lot, radians; `None` beyond the arc's reach `radius`.
fn lane_offset(radius: f32, egress: bool, x: f32) -> Option<(f32, f32, f32)> {
    if !(0.0..=radius).contains(&x) {
        return None;
    }
    // Entering: the arc from `R` before the lane point round to `R` along the hop, centre `R`
    // before it and `R` toward the lot; `φ` is the way round, π/4 at the lane point.
    let phi = std::f32::consts::FRAC_PI_4 * (1.0 - x / radius);
    let (sin, cos) = phi.sin_cos();
    let along = x - radius + radius * sin;
    let toward = radius * (1.0 - cos);
    // Leaving is its mirror along the lane, walked the other way.
    Some(if egress {
        (-along, toward, -phi)
    } else {
        (along, toward, phi)
    })
}

/// The drawn pose of a lane car `x` metres from the lane point it leaves its lane at (`egress`
/// false: before it) or joined it at (past it), on the gate turn of `radius` toward `hand`, from
/// its lane pose `point`, `along` (unit). `None` beyond the arc's reach, where it is drawn at its
/// lane pose. O(1), no allocation.
pub(crate) fn on_lane(
    radius: f32,
    hand: f32,
    egress: bool,
    x: f32,
    point: Vector2,
    along: Vector2,
) -> Option<DrawnPose> {
    let (ds, dl, turn) = lane_offset(radius, egress, x)?;
    let side = Vector2::new(-along.y, along.x) * hand;
    let (sin, cos) = turn.sin_cos();
    // Toward `hand` is a right turn for a lot on the right: negative curvature.
    Some((
        point + along * ds + side * dl,
        along * cos + side * sin,
        -hand / radius,
    ))
}

/// Where a gate turn meets its lane (`PARK-31`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateLane {
    /// The lane point, as the lane pose draws it.
    pub(crate) point: Vector2,
    /// The lane's unit direction of travel there, in the ground plane.
    pub(crate) along: Vector2,
    /// The side of travel the lot lies on, +1 right ([`LOT_HAND`]).
    pub(crate) hand: f32,
    /// The arc's radius by body class ([`CAR_GATE_RADIUS_M`], [`FREIGHT_GATE_RADIUS_M`]).
    pub(crate) radius: f32,
    /// Leaving the lot onto the lane, rather than entering it.
    pub(crate) egress: bool,
}

impl GateLane {
    /// The drawn pose of a vehicle standing at the lane point, on or off its lane: the arc's
    /// middle, facing along it. O(1).
    pub(crate) fn at_lane_point(&self) -> DrawnPose {
        on_lane(
            self.radius,
            self.hand,
            self.egress,
            0.0,
            self.point,
            self.along,
        )
        .unwrap_or((self.point, self.along, 0.0))
    }

    /// The drawn pose of a vehicle off its lane at `current` on the leg
    /// `points[target - 1] → points[target]` of its manoeuvre `points` (the lane point first
    /// entering, last leaving), when it is within the arc's radius of the lane point along the
    /// route; `None` beyond, where the lot's own bends (`turn_radius`, `None` for none) draw it.
    /// O(the legs within the radius of the lane point), at most the manoeuvre's points; no
    /// allocation.
    pub(crate) fn on_hop(
        &self,
        points: &[Vector2],
        turn_radius: Option<f32>,
        current: Vector2,
        target: usize,
    ) -> Option<DrawnPose> {
        let n = points.len();
        if n < 2 || target == 0 || target >= n {
            return None;
        }
        // Outward order: from the lane point into the lot, whichever way it is walked; outward
        // leg `k` is the leg ending at `points[target_of(k)]` in walking order.
        let at = |j: usize| points[if self.egress { n - 1 - j } else { j }];
        let target_of = |k: usize| if self.egress { n - 1 - k } else { k + 1 };
        let leg = if self.egress {
            n - 1 - target
        } else {
            target - 1
        };
        // The vehicle's progress from the lane point, and the leg and point the hop half ends at.
        let mut start = 0.0;
        let mut travelled = None;
        let mut end = None;
        for k in 0..n - 1 {
            let (a, b) = (at(k), at(k + 1));
            let length = a.distance_to(b);
            let out = if length > 1e-6 {
                (b - a) / length
            } else {
                self.along_hop()
            };
            if k == leg {
                travelled = Some(start + (current - a).dot(out).clamp(0.0, length));
            }
            if end.is_none() && (start + length >= self.radius || k == n - 2) {
                end = Some((k, a + out * (self.radius - start).clamp(0.0, length)));
            }
            start += length;
            if travelled.is_some() && end.is_some() {
                break;
            }
            if travelled.is_none() && start >= self.radius {
                return None;
            }
        }
        let (travelled, (end_leg, end_point)) = (travelled?, end?);
        let reach = start.min(self.radius);
        if travelled >= reach || reach < 1e-4 {
            return None;
        }
        // The lot's own line there, its travel in walking order.
        let end_target = target_of(end_leg);
        let walked = points[end_target] - points[end_target - 1];
        let (far, far_travel) = turn_radius
            .and_then(|radius| drawn_arc(points, end_target, end_point, radius))
            .map(|(point, travel, _)| (point, travel))
            .unwrap_or((
                end_point,
                walked.try_normalized().unwrap_or(self.along_hop()),
            ));
        let (middle, middle_travel, _) = self.at_lane_point();
        let u = travelled / reach;
        Some(if self.egress {
            cubic(far, far_travel, middle, middle_travel, 1.0 - u)
        } else {
            cubic(middle, middle_travel, far, far_travel, u)
        })
    }

    // The unit direction from the lane point toward the lot, square to the lane.
    fn along_hop(&self) -> Vector2 {
        Vector2::new(-self.along.y, self.along.x) * self.hand
    }
}

// The pose at `u` (0 to 1) on the cubic from `start`, travelling `start_travel` (unit), to `end`,
// travelling `end_travel`, with handles a circle's arc of the same turn and chord would have
// (`4/3 tan(φ/4)` of its radius), at most half the chord.
fn cubic(
    start: Vector2,
    start_travel: Vector2,
    end: Vector2,
    end_travel: Vector2,
    u: f32,
) -> DrawnPose {
    let chord = start.distance_to(end);
    let turn = start_travel
        .cross(end_travel)
        .abs()
        .atan2(start_travel.dot(end_travel));
    let handle = if turn > 1e-3 {
        chord * (4.0 / 3.0) * (0.25 * turn).tan() / (2.0 * (0.5 * turn).sin())
    } else {
        chord / 3.0
    }
    .min(0.5 * chord);
    let (b0, b1, b2, b3) = (
        start,
        start + start_travel * handle,
        end - end_travel * handle,
        end,
    );
    let v = 1.0 - u;
    let point =
        b0 * (v * v * v) + b1 * (3.0 * v * v * u) + b2 * (3.0 * v * u * u) + b3 * (u * u * u);
    let first = ((b1 - b0) * (v * v) + (b2 - b1) * (2.0 * v * u) + (b3 - b2) * (u * u)) * 3.0;
    let second = ((b2 - b1 * 2.0 + b0) * v + (b3 - b2 * 2.0 + b1) * u) * 6.0;
    let speed = first.length();
    if speed < 1e-6 {
        return (point, if u < 0.5 { start_travel } else { end_travel }, 0.0);
    }
    // Turning toward the counter-clockwise normal of travel is a right turn in the ground plane's
    // (x, z) (`Fillet::curvature`): negative.
    (
        point,
        first / speed,
        -first.cross(second) / (speed * speed * speed),
    )
}

#[cfg(test)]
mod tests;
