// SPDX-License-Identifier: GPL-3.0-or-later

//! The dock proof with drive-through routes (`PARK-21`): a loading bay may name an exit gate and
//! carry waypoints, and then its way in and its way out are polylines proven leg by leg.
//!
//! A way in is the arrival gate's road connection, its approach (one lead inside the gate), the
//! waypoints (or, without any, the derived aisle one lead before the bay) and the stop at the
//! bay's centre. A way out is the stop, the waypoints, the exit gate's approach and its road
//! connection. Each way is proven by these rules, in this order, and the first that fails is the
//! rejection, naming the leg or bend:
//!
//! 1. at most `MAX_ROUTE_POINTS` points;
//! 2. no leg shorter than `MIN_LEG_M`;
//! 3. the leg into the stop, and the leg out of it, run along the bay's direction within the
//!    class's bend limit (15° for a truck): a truck stands in its bay and leaves it forward;
//! 4. simple: no two legs that do not share a point come within `MIN_LEG_M` of each other (the
//!    tick finds a truck's leg from its position); the way in and the way out may cross;
//! 5. every leg's corridor (the body's width, the leg plus one body length, centred on the leg)
//!    is paved, clear of the class's structures (passages cut) and of every other bay;
//! 6. at every inner point where the way bends by more than the class's limit, the class's turn
//!    envelope (a square of the body's diagonal, squared to the bay) is paved and clear the same
//!    way. A through route's stop is an inner point of the drive: the bend from the last leg in
//!    to the first leg out is checked there too.
//!
//! A dock without an exit and without waypoints is proven by the stage A rule
//! (`validate_bay`) unchanged.

use super::super::geometry::rectangle;
use super::{
    AnchorBay, BEND_LIMIT_DEG, ProofClass, Rejection, VehicleProof, check_bay, drive_rejection,
    meets_other_bay, sweep_blocked, turns_at, validate_bay,
};
use crate::assets::asset::validation::MAX_ROUTE_POINTS;
use godot::prelude::Vector2;

// The shortest leg a route may have, and the closest two of its legs that share no point may
// come: the tick's tolerance for finding a truck's leg from its position is 5 cm.
pub(super) const MIN_LEG_M: f32 = 0.1;

/// A loading bay's authored drive in the proof's frame (`PARK-21`).
#[derive(Default)]
pub(in crate::simulation::parking) struct DockWayInput {
    /// Waypoints between the arrival gate's approach and the stop.
    pub route_in: Vec<Vector2>,
    /// Waypoints between the stop and the exit gate's approach.
    pub route_out: Vec<Vector2>,
    /// The exit gate: none named, its index in the usable gates, or the name it gives when no
    /// usable gate has it.
    pub exit: Option<Result<usize, String>>,
}

impl DockWayInput {
    // Whether the bay carries anything of stage B; without, the stage A proof stands. O(1).
    fn authored(&self) -> bool {
        self.exit.is_some() || !self.route_in.is_empty() || !self.route_out.is_empty()
    }
}

/// A proven dock: the row's four points (gate connection, approach, the point before the stop,
/// the stop), its whole way in and way out when authored (`PARK-21`; both empty for a stage A
/// dock, the way out empty for one that backs out), and the gates it arrives and leaves by.
pub(in crate::simulation::parking) struct ProvenDock {
    pub path: [Vector2; 4],
    pub route_in: Vec<Vector2>,
    pub route_out: Vec<Vector2>,
    pub arrive: usize,
    pub leave: usize,
}

/// The dock proof of loading bay `slot` through gate `only`, or through the first of `gates`
/// that proves it when `None`. Stage A's proof for a bay without an exit or waypoints; the
/// module's rules for one with them. O(gates × (points² + points × local overlay work)), points
/// at most `MAX_ROUTE_POINTS`; an edit or load boundary.
#[allow(clippy::too_many_arguments)]
pub(in crate::simulation::parking) fn validate_dock(
    slot: usize,
    bays: &[AnchorBay],
    footprints: &[[Vector2; 4]],
    others: &[[Vector2; 4]],
    support: &[Vec<[f64; 2]>],
    has_gate: bool,
    gates: &[[Vector2; 2]],
    only: Option<usize>,
    way: &DockWayInput,
    collision: impl Fn(&[Vector2]) -> bool,
) -> Result<ProvenDock, Rejection> {
    let (offset, tried) = match only {
        Some(gate) => (gate, &gates[gate..=gate]),
        None => (0, gates),
    };
    if !way.authored() {
        let (path, gate) = validate_bay(
            ProofClass::Freight,
            slot,
            bays,
            footprints,
            others,
            support,
            has_gate,
            tried,
            None,
            None,
            collision,
        )?;
        // A dock never opens onto the street, so its route always names a gate.
        let gate = gate.map_or(offset, |gate| offset + gate);
        return Ok(ProvenDock {
            path,
            route_in: Vec::new(),
            route_out: Vec::new(),
            arrive: gate,
            leave: gate,
        });
    }
    let row = ProofClass::Freight.proof();
    let bay = &bays[slot];
    check_bay(
        row, slot, bays, footprints, others, support, has_gate, &collision,
    )?;
    let exit = match &way.exit {
        None => None,
        Some(Ok(gate)) => Some(*gate),
        Some(Err(name)) => {
            return Err(format!(
                "its exit '{name}' is not a usable gate: name a freight Driveway at least {} m wide on the street edge, pointing into the lot",
                row.min_gate
            )
            .into());
        }
    };
    let blocked = |foot: &[Vector2]| {
        sweep_blocked(row, foot, support, &collision, |foot| {
            meets_other_bay(foot, slot, footprints, others)
        })
    };
    let derived = way.route_in.is_empty();
    let aisle = bay.pos - bay.forward * (bay.length * 0.5 + row.lead);
    let middle = if derived {
        std::slice::from_ref(&aisle)
    } else {
        &way.route_in[..]
    };
    let mut first = None;
    let mut proven = None;
    for (gate, &[connection, approach]) in tried.iter().enumerate() {
        let mut route = Vec::with_capacity(middle.len() + 3);
        route.extend([connection, approach]);
        route.extend_from_slice(middle);
        route.push(bay.pos);
        match prove_way(row, Way::In { derived }, &route, bay.forward, &blocked) {
            Ok(()) => {
                proven = Some((offset + gate, route));
                break;
            }
            Err(rejection) => {
                first.get_or_insert(rejection);
            }
        }
    }
    let Some((arrive, route_in)) = proven else {
        return Err(first.unwrap_or_else(|| {
            "the Driveway must sit at the street edge and point into the lot".into()
        }));
    };
    let n = route_in.len();
    let route_out = match exit {
        None => Vec::new(),
        Some(exit) => {
            let [connection, approach] = gates[exit];
            let mut route = Vec::with_capacity(way.route_out.len() + 3);
            route.push(bay.pos);
            route.extend_from_slice(&way.route_out);
            route.extend([approach, connection]);
            prove_way(row, Way::Out, &route, bay.forward, &blocked)?;
            // The stop is an inner point of the drive through: the truck keeps its heading
            // there unless the envelope proves the change.
            if turns_at(route_in[n - 2], bay.pos, route[1], row.straight_through_cos) {
                let shape = rectangle(bay.pos, bay.forward, row.turn_envelope, row.turn_envelope);
                if let Some(cause) = blocked(&shape) {
                    let part = "the turn through the spot";
                    return Err(drive_rejection(row, "out", part, cause, shape, true));
                }
            }
            route
        }
    };
    Ok(ProvenDock {
        path: [route_in[0], route_in[1], route_in[n - 2], route_in[n - 1]],
        leave: exit.unwrap_or(arrive),
        route_in,
        route_out,
        arrive,
    })
}

/// One arrival gate and one departure gate per building (`PARK-19`, `PARK-21`): the first dock
/// takes the first gate, in anchor order, that `prove` proves its way in through, and leaves by
/// its exit, or by that gate when it names none; every later dock must arrive through that gate
/// and leave by that departure gate. A later dock that only another arrival gate proves is
/// rejected as differing, as is one that leaves by another gate. `prove(slot, only)` is the dock
/// proof of dock `slot` through gate `only`, or through the first gate that proves it when
/// `None`. Shared by the placed lot and the asset editor. O(docks × gates) proofs, two per later
/// dock that fails.
pub(in crate::simulation::parking) fn dock_ways(
    docks: usize,
    prove: impl Fn(usize, Option<usize>) -> Result<ProvenDock, Rejection>,
) -> Vec<Result<ProvenDock, Rejection>> {
    let mut building = None;
    (0..docks)
        .map(|slot| match building {
            None => prove(slot, None).inspect(|dock| building = Some((dock.arrive, dock.leave))),
            Some((arrive, leave)) => prove(slot, Some(arrive))
                .map_err(|rejection| {
                    if prove(slot, None).is_ok() {
                        "its Driveway differs from the building's first loading bay's".into()
                    } else {
                        rejection
                    }
                })
                .and_then(|dock| {
                    if dock.leave == leave {
                        Ok(dock)
                    } else {
                        Err("its exit differs from the building's first loading bay's: every loading bay of a building leaves by the same Driveway".into())
                    }
                }),
        })
        .collect()
}

/// The turn envelopes a proven drive-through needs, as its proof checked them, for the editor's
/// drawing (`PARK-21`): the class's square at every inner point of either way bending more than
/// the class's limit, and at the stop when the drive turns there. Empty for a stage A dock.
/// O(points).
pub(in crate::simulation::parking) fn drive_envelopes(
    dock: &ProvenDock,
    forward: Vector2,
) -> Vec<[Vector2; 4]> {
    let row = ProofClass::Freight.proof();
    let bends = |points: &[Vector2]| {
        (1..points.len().saturating_sub(1))
            .filter(|&j| turns_at(points[j - 1], points[j], points[j + 1], row.straight_through_cos))
            .map(|j| points[j])
            .collect::<Vec<_>>()
    };
    let mut corners = bends(&dock.route_in);
    corners.extend(bends(&dock.route_out));
    let n = dock.route_in.len();
    if n >= 2
        && dock.route_out.len() >= 2
        && turns_at(dock.route_in[n - 2], dock.route_in[n - 1], dock.route_out[1], row.straight_through_cos)
    {
        corners.push(dock.route_in[n - 1]);
    }
    corners
        .into_iter()
        .map(|p| rectangle(p, forward, row.turn_envelope, row.turn_envelope))
        .collect()
}

/// The waypoints of a turn the asset editor's arc helper lays (`PARK-21`): from `start`,
/// heading `heading`, an arc of `radius` metres turning `angle_deg` degrees to the left (as a
/// driver sees it, `+Y` up) or the right, as chords whose ends lie on the circle. A chord turns
/// by `angle / chords`, the first and the last half that against the straight before and after,
/// and `chords = floor(angle / 15°) + 1` keeps every bend under the freight class's 15° limit,
/// as the proof requires (13 chords for a half circle, 7 for a quarter turn). Points are rounded
/// to 0.1 mm, as an authored TOML holds them; the rounding moves a bend by far less than the
/// margin left under 15°. Returns the chord ends after `start`, at most `MAX_ROUTE_POINTS`.
/// O(chords).
pub(crate) fn arc_waypoints(
    start: Vector2,
    heading: Vector2,
    radius: f32,
    angle_deg: f32,
    left: bool,
) -> Result<Vec<Vector2>, String> {
    if !(radius.is_finite() && radius > 0.0 && angle_deg.is_finite() && angle_deg > 0.0 && angle_deg <= 360.0) {
        return Err("An arc needs a radius above 0 m and an angle above 0° and up to 360°".into());
    }
    if heading.length_squared() < 1e-8 {
        return Err("The arc's start has no heading: move its waypoint off the point before it".into());
    }
    let h = heading.normalized();
    // Toward the centre: a driver facing `h` with `+Y` up has `(-h.z, h.x)` on the right.
    let n = if left { Vector2::new(h.y, -h.x) } else { Vector2::new(-h.y, h.x) };
    let chords = (angle_deg / BEND_LIMIT_DEG as f32).floor() as usize + 1;
    if chords > MAX_ROUTE_POINTS - 3 {
        return Err(format!("A way holds at most {} waypoints", MAX_ROUTE_POINTS - 3));
    }
    let step = angle_deg.to_radians() / chords as f32;
    let round = |v: f32| (v * 10_000.0).round() / 10_000.0;
    Ok((1..=chords)
        .map(|k| {
            let phi = step * k as f32;
            // On the circle through `start` tangent to `h`: c - n r cos φ + h r sin φ, c = start + n r.
            let p = start + n * (radius * (1.0 - phi.cos())) + h * (radius * phi.sin());
            Vector2::new(round(p.x), round(p.y))
        })
        .collect())
}

#[derive(Clone, Copy)]
pub(super) enum Way {
    // A way in; `derived` when its one middle point is the aisle, not an authored waypoint.
    In { derived: bool },
    Out,
}

impl Way {
    fn word(self) -> &'static str {
        match self {
            Self::In { .. } => "in",
            Self::Out => "out",
        }
    }

    // The words for leg `i` (point i to point i + 1) of a way of `n` points. O(1).
    fn leg(self, i: usize, n: usize) -> String {
        match self {
            Self::In { .. } if i == 0 => "the way in from the Driveway".into(),
            Self::In { derived: true } if i == 1 => "the way to the front of the spot".into(),
            Self::In { derived: true } => "the way into the spot".into(),
            Self::In { .. } if i == n - 2 => {
                format!("the way into the spot from waypoint {}", n - 3)
            }
            Self::In { .. } if i == 1 => {
                "the way in from the Driveway's approach to waypoint 1".into()
            }
            Self::In { .. } => format!("the way in from waypoint {} to waypoint {i}", i - 1),
            Self::Out if i == n - 2 => "the way out through the exit Driveway".into(),
            Self::Out if i == 0 && n == 3 => {
                "the way out from the spot to the exit Driveway".into()
            }
            Self::Out if i == 0 => "the way out from the spot to waypoint 1".into(),
            Self::Out if i == n - 3 => {
                format!("the way out from waypoint {i} to the exit Driveway")
            }
            Self::Out => format!("the way out from waypoint {i} to waypoint {}", i + 1),
        }
    }

    // The words for the bend at inner point `j` of a way of `n` points. O(1).
    fn bend(self, j: usize, n: usize) -> String {
        match self {
            Self::In { .. } if j == 1 => "the turn off the Driveway".into(),
            Self::In { derived: true } => "the turn into the spot".into(),
            Self::In { .. } => format!("the bend at waypoint {} of the way in", j - 1),
            Self::Out if j == n - 2 => "the turn onto the exit Driveway".into(),
            Self::Out => format!("the bend at waypoint {j} of the way out"),
        }
    }
}

// Proves one way by the module's rules 1 to 6. O(points² + points × local overlay work).
pub(super) fn prove_way(
    row: &VehicleProof,
    way: Way,
    points: &[Vector2],
    forward: Vector2,
    blocked: &impl Fn(&[Vector2]) -> Option<&'static str>,
) -> Result<(), Rejection> {
    let n = points.len();
    let word = way.word();
    if n > MAX_ROUTE_POINTS {
        return Err(format!(
            "the way {word} has {n} points: at most {MAX_ROUTE_POINTS}, its Driveway's road connection and approach and the spot with at most {} waypoints",
            MAX_ROUTE_POINTS - 3
        )
        .into());
    }
    let on_leg = |rejection: Rejection, i: usize| Rejection {
        leg: Some([points[i], points[i + 1]]),
        ..rejection
    };
    if let Some(i) = (0..n - 1).find(|&i| points[i].distance_to(points[i + 1]) < MIN_LEG_M) {
        let reason = format!(
            "{} is shorter than {MIN_LEG_M} m: remove one of its ends",
            way.leg(i, n)
        );
        return Err(on_leg(reason.into(), i));
    }
    // The leg at the bay, and the heading it must keep: into the stop for a way in, out of it
    // for a way out, both along the bay.
    let (at_bay, heading) = match way {
        Way::In { .. } => (n - 2, points[n - 1] - points[n - 2]),
        Way::Out => (0, points[1] - points[0]),
    };
    let off = heading.normalized().dot(forward);
    if off < row.straight_through_cos {
        let degrees = off.clamp(-1.0, 1.0).acos().to_degrees().round();
        let fix = match way {
            Way::In { .. } => {
                "a truck drives into its spot along it, so put the last waypoint in line behind the spot"
            }
            Way::Out => {
                "a truck leaves its spot driving forward, so put the first waypoint in line ahead of the spot"
            }
        };
        let reason = format!(
            "{} runs {degrees}° off the spot's direction: {fix} (within {BEND_LIMIT_DEG}°)",
            way.leg(at_bay, n)
        );
        return Err(on_leg(reason.into(), at_bay));
    }
    for a in 0..n - 1 {
        for b in a + 2..n - 1 {
            if segment_gap(points[a], points[a + 1], points[b], points[b + 1]) < MIN_LEG_M {
                let reason = format!(
                    "the way {word} touches itself: {} comes within {MIN_LEG_M} m of {} (a way may not cross or touch itself; the way in and the way out may cross each other)",
                    way.leg(b, n),
                    way.leg(a, n)
                );
                return Err(on_leg(reason.into(), b));
            }
        }
    }
    let corridors = (0..n - 1).map(|i| {
        let delta = points[i + 1] - points[i];
        let shape = rectangle(
            (points[i] + points[i + 1]) * 0.5,
            delta.normalized(),
            row.body_width,
            delta.length() + row.body_length,
        );
        (way.leg(i, n), shape, false)
    });
    // Squared to the bay, as stage A squares its envelopes.
    let bends = (1..n - 1)
        .filter(|&j| {
            turns_at(
                points[j - 1],
                points[j],
                points[j + 1],
                row.straight_through_cos,
            )
        })
        .map(|j| {
            let shape = rectangle(points[j], forward, row.turn_envelope, row.turn_envelope);
            (way.bend(j, n), shape, true)
        });
    for (part, shape, bend) in corridors.chain(bends) {
        if let Some(cause) = blocked(&shape) {
            return Err(drive_rejection(row, word, &part, cause, shape, bend));
        }
    }
    Ok(())
}

// The shortest distance between segments a0-a1 and b0-b1: zero when they cross. O(1).
fn segment_gap(a0: Vector2, a1: Vector2, b0: Vector2, b1: Vector2) -> f32 {
    let cross = |o: Vector2, p: Vector2, q: Vector2| (p - o).cross(q - o);
    let (d1, d2) = (cross(b0, b1, a0), cross(b0, b1, a1));
    let (d3, d4) = (cross(a0, a1, b0), cross(a0, a1, b1));
    if d1 * d2 < 0.0 && d3 * d4 < 0.0 {
        return 0.0;
    }
    let to_segment = |p: Vector2, s0: Vector2, s1: Vector2| {
        let along = s1 - s0;
        let length = along.length_squared();
        let t = if length <= 1e-12 {
            0.0
        } else {
            ((p - s0).dot(along) / length).clamp(0.0, 1.0)
        };
        p.distance_to(s0 + along * t)
    };
    to_segment(a0, b0, b1)
        .min(to_segment(a1, b0, b1))
        .min(to_segment(b0, a0, a1))
        .min(to_segment(b1, a0, a1))
}

#[cfg(test)]
mod tests;
