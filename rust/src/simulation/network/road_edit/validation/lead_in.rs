// SPDX-License-Identifier: GPL-2.0-only

//! Lead-ins (`ROAD-118` round JS5a): a join forms the junction; nothing the player drew moves.
//! Where a stroke end joins a road it touches or a node it is too close to, the stroke keeps
//! every drawn point and the junction adds a short piece of centreline from the drawn end to the
//! node the roads share: the lead-in. On a road's side that node is where the stroke's own line
//! meets the centreline, so the lead-in runs straight on. At an existing node off the stroke's
//! line it is the one circular arc tangent to the stroke that ends on the node, and the junction
//! may take the stroke's last metres, the trim, up to the footprints' reach (the user's decision
//! D3), to shape it:
//!
//! - the least trim that keeps the arc's radius at the road's illegal radius or wider
//!   (`ROAD-54`, [`ILLEGAL_RADIUS_HALF_WIDTH_FACTOR`] roadbed half widths);
//! - at a road end, the least trim that makes the arc arrive along the old road's line, so the
//!   two roads run on as one, where the reach allows it;
//! - at a node with roads, the least trim that keeps the new arm the junction arm limit (25
//!   degrees, `ROAD-54`) from every other arm there;
//! - where the node's layout refuses that (a corner that does not fit), the whole reach: the arc
//!   of least turn, its arm farthest from the arm it turns towards (one more plan build, refused
//!   poses only; `validation::Joins::widen`).
//!
//! In the frame of the drawn end `E` and its tangent `t`, the node lies `a` ahead and `b` to the
//! side. The arc tangent to the stroke `L` metres back from `E` turns `tau = 2 atan(|b| / (a + L))`
//! with radius `|b| / (2 sin^2(tau / 2))`, so each rule is a bound on the turn and the trim is
//! `L = |b| / tan(tau / 2) - a`, closed form. A stroke whose last metres curve gets the arc from
//! its actual point and tangent there; the rules' trim reads them as straight, and the geometry
//! checks and the node's layout judge the result as they judge any drawn road. A trim the reach
//! cannot satisfy is taken at the reach and refused by those checks, naming the node or the
//! curve. Pure functions of the stroke, the node and its arms: `O(points + degree^2)`.

use super::contact::MIN_CONTACT_ANGLE_DEG;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::guide::{DVec2, GuideSegment};
use crate::simulation::network::validate::ILLEGAL_RADIUS_HALF_WIDTH_FACTOR;
use godot::prelude::Vector3;

// The arm limit as the lead-in's turn bound keeps it: a hundredth of a degree over the limit,
// so the `f32` rounding of the built points never reads the arm under it.
const ARM_BOUND_DEG: f64 = MIN_CONTACT_ANGLE_DEG as f64 + 0.01;

// The radius bound's margin over the illegal radius, a thousandth: the flattened arc reads its
// circle's radius exactly (`validate::visit_turning_radii`), up to the rounding of its points.
const RADIUS_MARGIN: f64 = 1.001;

// A lead-in shorter than this replaces the drawn end instead of adding a segment: no segment
// shorter than the validator's least segment run (`validate::MIN_SEGMENT_RUN_M`).
const MIN_LEAD_IN_M: f64 = 0.05;

// The most arms a node's arm bound reads: the junction degree limit (6) and two to spare.
const MAX_ARMS: usize = 8;

/// One stroke end's lead-in (`ROAD-118` round JS5a): the stroke's last `trim_m` horizontal
/// metres become junction, and from there the arc tangent to the stroke (a line where it lies
/// ahead on the stroke's line) runs to `target`, the node the junction forms at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LeadIn {
    /// Where the lead-in ends: the node, or the point on a road's centreline the stroke's line
    /// meets.
    pub(crate) target: Vector3,
    /// The drawn stroke's horizontal metres before its end the lead-in replaces.
    pub(crate) trim_m: f32,
}

/// What shapes a lead-in at a node (module docs): the node's arms and whether it is a road end
/// (its one arm is the line to arrive along).
#[derive(Clone, Copy, Debug)]
pub(super) struct NodeArms {
    // The arms' unit directions leaving the node, horizontal.
    arms: [DVec2; MAX_ARMS],
    count: usize,
    road_end: bool,
}

impl NodeArms {
    /// No arms: a point on a road's centreline, where the lead-in runs straight on.
    pub(super) const NONE: Self = Self {
        arms: [DVec2::ZERO; MAX_ARMS],
        count: 0,
        road_end: false,
    };

    /// The live arms of `node` in `graph`, by their first segment's direction: `O(degree +
    /// points to the first distinct one)`.
    pub(super) fn of(graph: &RegionGraph, node: u32) -> Self {
        let mut arms = Self::NONE;
        for &edge in graph.node_adjacency(node) {
            let record = graph.edge(edge);
            if record.deleted || arms.count == MAX_ARMS {
                continue;
            }
            let at_start = graph.get_valid_node(record.start_node) == node;
            let at_end = graph.get_valid_node(record.end_node) == node;
            let points = &record.geometry;
            for direction in [
                at_start.then(|| leaving(points.iter())).flatten(),
                at_end.then(|| leaving(points.iter().rev())).flatten(),
            ]
            .into_iter()
            .flatten()
            {
                if arms.count < MAX_ARMS {
                    arms.arms[arms.count] = direction;
                    arms.count += 1;
                }
            }
        }
        arms.road_end = arms.count == 1;
        arms
    }
}

// The unit horizontal direction from the first of `points` to the first that stands apart from
// it.
fn leaving<'a>(mut points: impl Iterator<Item = &'a Vector3>) -> Option<DVec2> {
    let first = *points.next()?;
    points.find_map(|point| {
        let d = DVec2::new(f64::from(point.x - first.x), f64::from(point.z - first.z));
        (d.length() > 1.0e-3).then(|| d.normalize())
    })
}

/// The lead-in from end `end` (0 the start, 1 the end) of the drawn stroke `points` to `target`
/// (module docs): the trim the rules ask for, at most `cap_m` (the reach, and what the stroke's
/// other end leaves). `half_width_m` is the stroke's roadbed half width. `O(points + degree^2)`,
/// no allocation.
pub(super) fn shape(
    points: &[Vector3],
    end: usize,
    target: Vector3,
    arms: &NodeArms,
    half_width_m: f32,
    cap_m: f32,
) -> LeadIn {
    let lead_in = |trim_m: f64| LeadIn {
        target,
        trim_m: (trim_m.max(0.0) as f32).min(cap_m.max(0.0)),
    };
    let Some((tip, t)) = end_frame(points, end) else {
        return lead_in(0.0);
    };
    let rel = DVec2::new(
        f64::from(target.x) - tip.x,
        f64::from(target.z) - tip.y,
    );
    let (a, b) = (t.dot(rel), t.perp_dot(rel));
    if b.abs() <= 1.0e-4 * a.abs().max(1.0) {
        // Ahead on the stroke's line: a straight lead-in. Behind it, the stroke ends at the node.
        return lead_in(-a);
    }
    let side = b.signum();
    let b = b.abs();
    // The turn with no trim, and the most the radius bound allows (`r = b / (2 sin^2(tau/2))`),
    // short of a half turn.
    let r_min = f64::from(half_width_m * ILLEGAL_RADIUS_HALF_WIDTH_FACTOR) * RADIUS_MARGIN;
    let untrimmed = 2.0 * b.atan2(a);
    let by_radius = 2.0 * (b / (2.0 * r_min)).min(1.0).sqrt().asin();
    let mut tau = untrimmed
        .min(by_radius)
        .min(std::f64::consts::PI - 1.0e-6);
    if arms.road_end {
        // Arrive along the old road: travel on along its one arm.
        let align = angle_between(t, arms.arms[0]) * side;
        let align = align.rem_euclid(std::f64::consts::TAU);
        if align > 0.0 && align <= tau {
            tau = align;
        }
    } else {
        tau = clear_of_arms(tau, t, side, &arms.arms[..arms.count]);
    }
    if tau <= 0.0 {
        return lead_in(f64::from(cap_m));
    }
    lead_in(b / (tau / 2.0).tan() - a)
}

// The largest turn at most `tau` whose new arm, leaving the node opposite the arrival
// (`-rotate(t, side * turn)`), keeps the arm bound from every arm in `arms`; 0 when none does.
// Less turn turns the new arm by `-side`, so each step lowers the turn to the edge of the bound
// around the arm in the way: at most `arms^2` steps. `O(arms^2)`.
fn clear_of_arms(mut tau: f64, t: DVec2, side: f64, arms: &[DVec2]) -> f64 {
    let bound = ARM_BOUND_DEG.to_radians();
    for _ in 0..=arms.len() * arms.len() {
        let new_arm = -rotate(t, side * tau);
        let Some(blocking) = arms
            .iter()
            .map(|&other| angle_between(new_arm, other))
            .find(|delta| delta.abs() < bound)
        else {
            return tau;
        };
        // After `step` less turn the angle to that arm reads `blocking + side * step`.
        tau -= (bound - side * blocking).max(1.0e-9);
        if tau <= 0.0 {
            return 0.0;
        }
    }
    0.0
}

// The signed angle from `from` to `to`, radians in (-pi, pi].
fn angle_between(from: DVec2, to: DVec2) -> f64 {
    from.perp_dot(to).atan2(from.dot(to))
}

fn rotate(v: DVec2, angle: f64) -> DVec2 {
    let (sin, cos) = angle.sin_cos();
    DVec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

// End `end`'s point (x, z) and the unit horizontal direction the stroke leaves it in, from the
// nearest point back that stands apart from it. `None` for a stroke of one place.
fn end_frame(points: &[Vector3], end: usize) -> Option<(DVec2, DVec2)> {
    let flat = |point: &Vector3| DVec2::new(f64::from(point.x), f64::from(point.z));
    let tip = flat(if end == 0 { points.first()? } else { points.last()? });
    let away = |point: &Vector3| {
        let d = tip - flat(point);
        (d.length() > 1.0e-3).then(|| d.normalize())
    };
    let t = if end == 0 {
        points.iter().find_map(away)
    } else {
        points.iter().rev().find_map(away)
    }?;
    Some((tip, t))
}

/// `points` with the lead-in `lead_in` at end `end` (0 the start, 1 the end): its last `trim_m`
/// horizontal metres cut (the cut point interpolated, heights included), then the arc or line
/// tangent to the stroke there to the target, flattened by the guide kernel's rule
/// (`guide::flatten_count`, so the validator reads the arc's own radius), heights interpolated
/// along it, the target written bit for bit. Every point before the cut stays bit for bit. A
/// lead-in shorter than [`MIN_LEAD_IN_M`] replaces the end with the target. `O(points + arc
/// points)`; grows `points` by the arc's points.
pub(super) fn apply(points: &mut Vec<Vector3>, end: usize, lead_in: LeadIn) {
    if end == 0 {
        points.reverse();
        apply_at_end(points, lead_in);
        points.reverse();
    } else {
        apply_at_end(points, lead_in);
    }
}

fn apply_at_end(points: &mut Vec<Vector3>, LeadIn { target, trim_m }: LeadIn) {
    let mut remaining = trim_m;
    while remaining > 0.0 && points.len() >= 2 {
        let (prev, last) = (points[points.len() - 2], points[points.len() - 1]);
        let run = (last.x - prev.x).hypot(last.z - prev.z);
        if run <= remaining {
            remaining -= run;
            points.pop();
            continue;
        }
        let share = (run - remaining) / run;
        *points.last_mut().unwrap() = prev + (last - prev) * share;
        break;
    }
    let Some((start, tangent)) = end_frame(points, 1) else {
        return;
    };
    let flat_target = DVec2::new(f64::from(target.x), f64::from(target.z));
    let from = *points.last().unwrap();
    if flat_target.distance(start) < MIN_LEAD_IN_M {
        *points.last_mut().unwrap() = target;
        return;
    }
    if let Ok(segment) = GuideSegment::tangent_to(start, tangent, flat_target) {
        let (count, length) = (segment.flatten_count(), segment.length());
        for step in 1..count {
            let share = step as f64 / count as f64;
            let at = segment.point_at(length * share);
            let y = from.y + (target.y - from.y) * share as f32;
            points.push(Vector3::new(at.x as f32, y, at.y as f32));
        }
    }
    points.push(target);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(from: (f32, f32), to: (f32, f32), steps: usize) -> Vec<Vector3> {
        (0..=steps)
            .map(|step| {
                let share = step as f32 / steps as f32;
                Vector3::new(
                    from.0 + (to.0 - from.0) * share,
                    0.0,
                    from.1 + (to.1 - from.1) * share,
                )
            })
            .collect()
    }

    // The plan's road-end case (`ours-2`): the node 5.8 m ahead and 6.0 m beside the end of a
    // 2-lane stroke, the old road leaving it at 46 degrees to the stroke. The last 8.3 m become a
    // 19.6 m radius arc that arrives along the old road; every drawn point before the cut stays
    // bit for bit and the built road ends on the node.
    #[test]
    fn a_road_end_lead_in_arrives_along_the_old_road() {
        let drawn = line((-100.0, 0.0), (0.0, 0.0), 25);
        let target = Vector3::new(5.8, 0.0, 6.0);
        let old_road = DVec2::new(46f64.to_radians().cos(), 46f64.to_radians().sin());
        let mut arms = NodeArms::NONE;
        arms.arms[0] = old_road;
        arms.count = 1;
        arms.road_end = true;
        let lead_in = shape(&drawn, 1, target, &arms, 5.0, 10.0);
        let tan = 23f32.to_radians().tan();
        assert!((lead_in.trim_m - (6.0 / tan - 5.8)).abs() < 1.0e-3, "{lead_in:?}");
        let mut built = drawn.clone();
        apply(&mut built, 1, lead_in);
        let kept = drawn.iter().filter(|p| p.x < -lead_in.trim_m).count();
        assert_eq!(built[..kept], drawn[..kept], "drawn points stay bit for bit");
        assert_eq!(*built.last().unwrap(), target);
        let n = built.len();
        let arrive = DVec2::new(
            f64::from(built[n - 1].x - built[n - 2].x),
            f64::from(built[n - 1].z - built[n - 2].z),
        )
        .normalize();
        // The last chord turns half a step (about 4 degrees) short of the arc's end tangent.
        assert!(angle_between(arrive, old_road).abs() < 5f64.to_radians(), "{arrive:?}");
    }

    // A node ahead on the stroke's line: a straight lead-in, nothing trimmed, the drawn end kept
    // and the target appended. The start the same way round.
    #[test]
    fn a_target_on_the_line_runs_straight_on() {
        let drawn = line((0.0, -60.0), (0.0, -4.0), 14);
        let target = Vector3::new(0.0, 0.0, 0.0);
        let lead_in = shape(&drawn, 1, target, &NodeArms::NONE, 5.0, 10.0);
        assert_eq!(lead_in.trim_m, 0.0);
        let mut built = drawn.clone();
        apply(&mut built, 1, lead_in);
        assert_eq!(built[..drawn.len()], drawn[..]);
        assert_eq!(built.len(), drawn.len() + 1);
        assert_eq!(*built.last().unwrap(), target);

        let mut reversed: Vec<Vector3> = drawn.iter().rev().copied().collect();
        let lead_in = shape(&reversed, 0, target, &NodeArms::NONE, 5.0, 10.0);
        apply(&mut reversed, 0, lead_in);
        assert_eq!(reversed[0], target);
        assert_eq!(reversed[1..], drawn.iter().rev().copied().collect::<Vec<_>>()[..]);
    }

    // An end on a street 6 m from a T, the stroke square to the street: the arc's turn toward
    // the T stops where the new arm stands the arm limit from the street's arm, and its radius
    // stays legal.
    #[test]
    fn a_node_lead_in_keeps_the_arm_limit() {
        let drawn = line((6.0, -60.0), (6.0, 0.0), 15);
        let target = Vector3::ZERO;
        let mut arms = NodeArms::NONE;
        for (index, direction) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0)].into_iter().enumerate() {
            arms.arms[index] = DVec2::new(direction.0, direction.1);
        }
        arms.count = 3;
        let lead_in = shape(&drawn, 1, target, &arms, 5.0, 10.0);
        // The arc turns `tau` towards the T; the new arm leaves it at `tau - 90` degrees, so its
        // angle to the street's east arm is `90 - tau`.
        let tau = 2.0 * (6.0 / f64::from(lead_in.trim_m)).atan();
        let east = (std::f64::consts::FRAC_PI_2 - tau).to_degrees();
        assert!((east - ARM_BOUND_DEG).abs() < 1.0e-3, "{lead_in:?} {east}");
        assert!(lead_in.trim_m <= 10.0);
    }
}
