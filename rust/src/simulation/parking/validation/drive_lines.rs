// SPDX-License-Identifier: GPL-3.0-or-later

//! Car routes along a lot's drive lines (`PARK-24`), tried only for a parking place whose
//! straight path from every Driveway fails.
//!
//! A drive line is a straight line the lot's own layout gives: the **gate line** through a
//! Driveway along its forward, the **row line** through a place's aisle point (one lead in front
//! of its entry edge) square to the place (and, for a place angled to its lot, along either lot
//! axis), and the **place line** through the place along its forward. A route bends only where
//! two of them cross, and keeps today's four points:
//!
//! - candidate 3, `[connection, J, aisle, stop]`: down the gate line to J, where it crosses a row
//!   line, along the row line to the aisle point, into the place;
//! - candidate 4, `[connection, J', P, stop]`: down the gate line to J', where it crosses the
//!   place line, along it into the place; P is the aisle point when J' lies before it, else
//!   halfway from J' to the stop. A place in line with the gate has no crossing: its two points
//!   lie at thirds of the straight way from the gate to the stop.
//!
//! A candidate is proven by the drive-through rules (`route::prove_way`) with the car row: a
//! corridor per leg, the turn envelope at every bend over 30°, the last leg along the place, legs
//! of at least 0.1 m that touch only their neighbours. No square is required just inside the gate
//! unless the route bends there: the car turns off the street on the road (owner decision 2).
//! The shortest valid route wins, lengths compared in whole millimetres; ties go to the lower
//! gate index, then the lower candidate index. Only `+ − × ÷` and square roots are used.

use super::super::geometry::rectangle;
use super::route::{MIN_LEG_M, Way, prove_way};
use super::{AnchorBay, Rejection, VehicleProof, turns_at};
use godot::prelude::Vector2;

/// The drive lines a car place may be routed along when its straight path fails (`PARK-24`).
/// Absent for freight and for a lot pinned to the straight path (`car_routes = "direct"`).
#[derive(Clone, Copy)]
pub(in crate::simulation::parking) struct DriveLines<'a> {
    /// The unit inward heading of each gate the straight path was tried through, in its order:
    /// the gate line runs along it through the gate's approach.
    pub inward: &'a [Vector2],
    /// The lot's two unit axes in the proof's frame, for the row lines of an angled place.
    pub axes: [Vector2; 2],
}

// Below this sine of the angle between two lines they count as parallel and never cross.
const PARALLEL_SIN: f32 = 1e-3;
// A place lies in line with a gate when its stop is within this of the gate line.
const IN_LINE_M: f32 = 0.01;

/// The shortest proven drive-line route to `bay` and the index of its gate in `paths`; when no
/// candidate proves, the rejection the shortest candidate failed on, carrying its route (`None`
/// when the lines give no candidate), for the editor to draw. `blocked` is the class's sweep test, as the
/// straight path uses it. O(gates × 4 candidates × (5 shape tests, each the local overlay work of
/// one shape)); a candidate no shorter than the best found so far is not proven.
pub(super) fn shortest_route(
    row: &VehicleProof,
    bay: &AnchorBay,
    paths: &[[Vector2; 2]],
    lines: DriveLines,
    blocked: &impl Fn(&[Vector2]) -> Option<&'static str>,
) -> Result<([Vector2; 4], usize), Option<Rejection>> {
    let mut best: Option<(i64, [Vector2; 4], usize)> = None;
    let mut failed: Option<(i64, Rejection)> = None;
    for (gate, (&[connection, approach], &inward)) in paths.iter().zip(lines.inward).enumerate() {
        for [first, second] in candidates(row, bay, approach, inward, lines.axes)
            .into_iter()
            .flatten()
        {
            let path = [connection, first, second, bay.pos];
            let length = millimetres(&path);
            if best.is_some_and(|(shortest, ..)| length >= shortest) {
                continue;
            }
            match prove_way(row, Way::In { derived: true }, &path, bay.forward, blocked) {
                Ok(()) => best = Some((length, path, gate)),
                Err(rejection) => {
                    if failed
                        .as_ref()
                        .is_none_or(|(shortest, ..)| length < *shortest)
                    {
                        let route = Some(path);
                        failed = Some((length, Rejection { route, ..rejection }));
                    }
                }
            }
        }
    }
    match best {
        Some((_, path, gate)) => Ok((path, gate)),
        None => Err(failed.map(|(_, rejection)| rejection)),
    }
}

/// A car bay neither its straight path nor any drive-line route reaches (`PARK-24`): the straight
/// path's failing `part` and `cause`, and the shape, leg and route of the shortest drive-line
/// candidate that failed (`tried`), or the straight path's `shape` when the lines give none.
/// O(1).
pub(super) fn no_way(
    row: &VehicleProof,
    part: &str,
    cause: &str,
    shape: [Vector2; 4],
    tried: Option<Rejection>,
) -> Rejection {
    let reason = format!(
        "no way to drive to this spot: straight from the Driveway, {part} {cause}, and no way along the row or straight down from the Driveway is clear (a {} needs a {} m strip of clear paving to drive on and about {} of it where it turns): clear the way or move the spot",
        row.noun, row.body_width, row.turn_room
    );
    match tried {
        Some(tried) => Rejection {
            reason,
            shape: tried.shape.or(Some(shape)),
            ..tried
        },
        None => Rejection {
            reason,
            shape: Some(shape),
            leg: None,
            route: None,
        },
    }
}

/// The turn envelopes a car spot's proof checked, for the editor to draw (`PARK-24`): the car's
/// square, squared to the spot, at each inner point of `path` bending by more than 30°, and at
/// the approach when `at_gate` (a straight path, whose car always turns there). O(1).
pub(super) fn car_envelopes(
    path: &[Vector2; 4],
    forward: Vector2,
    at_gate: bool,
) -> Vec<[Vector2; 4]> {
    let row = super::ProofClass::Car.proof();
    (1..3)
        .filter(|&j| {
            (j == 1 && at_gate)
                || turns_at(path[j - 1], path[j], path[j + 1], row.straight_through_cos)
        })
        .map(|j| rectangle(path[j], forward, row.turn_envelope, row.turn_envelope))
        .collect()
}

// A route's length in whole millimetres. O(1).
fn millimetres(path: &[Vector2; 4]) -> i64 {
    let metres: f32 = path.windows(2).map(|leg| leg[0].distance_to(leg[1])).sum();
    (metres * 1000.0).round() as i64
}

// Where the line through `p` along unit `u` crosses the line through `q` along unit `v`, or
// `None` when they are parallel. O(1).
fn crossing(p: Vector2, u: Vector2, q: Vector2, v: Vector2) -> Option<Vector2> {
    let sine = u.cross(v);
    (sine.abs() >= PARALLEL_SIN).then(|| p + u * ((q - p).cross(v) / sine))
}

// The two inner points of every drive-line candidate through one gate, in candidate order: the
// square row line, the lot axes' row lines of an angled place, then the place line. `None` where
// a candidate does not exist: its lines are parallel, or it would turn off the gate line before
// the gate. O(1).
fn candidates(
    row: &VehicleProof,
    bay: &AnchorBay,
    approach: Vector2,
    inward: Vector2,
    axes: [Vector2; 2],
) -> [Option<[Vector2; 2]>; 4] {
    let forward = bay.forward;
    let stop = bay.pos;
    let lead = bay.length * 0.5 + row.lead;
    let aisle = stop - forward * lead;
    let gate = approach - inward * row.lead;
    // A turn off the gate line lies inside the lot, past the gate.
    let off_gate_line = |through: Vector2, along: Vector2| {
        crossing(approach, inward, through, along).filter(|&turn| (turn - gate).dot(inward) > 0.0)
    };
    let row_line = |along: Vector2| off_gate_line(aisle, along).map(|turn| [turn, aisle]);
    // A lot axis is a further row line only for an angled place: for a square one it is the
    // square row line or the place line.
    let angled = |axis: Vector2| {
        (axis.cross(forward).abs() >= PARALLEL_SIN && axis.dot(forward).abs() >= PARALLEL_SIN)
            .then_some(axis)
    };
    let place_line = match crossing(approach, inward, stop, forward) {
        Some(turn) if (turn - gate).dot(inward) > 0.0 => {
            let behind = (stop - turn).dot(forward);
            (behind >= 2.0 * MIN_LEG_M).then(|| {
                let between = if behind >= lead + MIN_LEG_M {
                    aisle
                } else {
                    turn + forward * (behind * 0.5)
                };
                [turn, between]
            })
        }
        Some(_) => None,
        None => {
            // In line: the gate line runs through the place.
            let way = stop - gate;
            (way.cross(inward).abs() <= IN_LINE_M && way.dot(inward) > 0.0)
                .then(|| [gate + way / 3.0, gate + way * (2.0 / 3.0)])
        }
    };
    [
        row_line(Vector2::new(-forward.y, forward.x)),
        angled(axes[0]).and_then(row_line),
        angled(axes[1]).and_then(row_line),
        place_line,
    ]
}

#[cfg(test)]
mod tests;
