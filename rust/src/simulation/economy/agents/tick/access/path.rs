// SPDX-License-Identifier: GPL-3.0-or-later

//! Local access path construction and walking.

use super::super::super::MODE_WALK;
use super::geometry::{
    entrance_edge_normal, entrance_edge_pos, local_access_point, opposite_side_car_lane,
    same_side_car_lane, segment_distance,
};
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use godot::prelude::Vector2;

/// Fixed-size polyline for door-to-lane or lane-to-door access movement.
#[derive(Clone, Copy)]
pub(crate) struct LocalAccessPath {
    /// Ordered access points followed by the agent.
    pub(crate) points: [Vector2; 5],
    /// Number of valid entries in `points`.
    pub(crate) count: usize,
}

impl LocalAccessPath {
    /// The valid points, in travel order: what the walking functions below take, so a longer
    /// polyline (a dock's authored route, `PARK-21`) walks by the same code. O(1).
    pub(crate) fn as_slice(&self) -> &[Vector2] {
        &self.points[..self.count]
    }
}

/// The most points a parking manoeuvre walks: a dock's authored way and the lane point
/// (`PARK-21`).
pub(crate) const MANEUVER_POINTS: usize = crate::assets::asset::validation::MAX_ROUTE_POINTS + 1;

/// A parking manoeuvre's polyline, lane point included (`ParkingSystem::maneuver_path`): a bay's
/// fixed five points, or a dock's authored way (`PARK-21`), built on the stack per access step,
/// so a car or a stage A truck copies the five points it always did.
#[derive(Clone, Copy)]
pub(crate) enum ManeuverPath {
    /// A bay's or a stage A dock's route and the lane point, in travel order.
    Bay(LocalAccessPath),
    /// A dock's authored way and the lane point, in travel order: `count` valid `points`.
    Way {
        /// The points, in travel order.
        points: [Vector2; MANEUVER_POINTS],
        /// Number of valid entries in `points`.
        count: usize,
    },
}

impl ManeuverPath {
    /// The valid points, in travel order. O(1).
    pub(crate) fn as_slice(&self) -> &[Vector2] {
        match self {
            Self::Bay(path) => path.as_slice(),
            Self::Way { points, count } => &points[..*count],
        }
    }
}

/// Builds a short local access path between a building door and a lane/curb handoff point.
pub(crate) fn local_access_path(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    reverse: bool,
) -> Option<LocalAccessPath> {
    let lane_point = local_access_point(mode, entrance, lane_id, lane_d, transit_network)?;
    let mut path = LocalAccessPath {
        points: [Vector2::ZERO; 5],
        count: 0,
    };

    if mode == MODE_WALK {
        path.points[0] = entrance.door_pos;
        path.points[1] = entrance.curb_pos;
        path.count = 2;
    } else if entrance.vehicle_frontage_access == VehicleFrontageAccess::BothSides
        && lane_id == opposite_side_car_lane(entrance)
        && lane_id != usize::MAX
    {
        let edge_pos = entrance_edge_pos(entrance, graph)?;
        let normal = entrance_edge_normal(entrance, graph)?;
        let edge = graph.edge(entrance.edge_idx);
        // The near kerb: a pocket edge's lies off its centreline (`ROAD-95`, D17). From there
        // the crossing runs straight over the near lane to the far lane point (`PARK-32`).
        // A door point on the road (a kerb stop) is reached straight from the lane point.
        let kerb = edge.frontage_half_width_m(entrance.side);
        let same_side_cross_point = if (entrance.door_pos - edge_pos).dot(normal) < kerb {
            entrance.door_pos
        } else {
            edge_pos + normal * kerb
        };
        path.points[0] = entrance.door_pos;
        path.points[1] = same_side_cross_point;
        path.points[2] = lane_point;
        path.count = 3;
    } else {
        if lane_id != same_side_car_lane(entrance)
            && entrance.vehicle_frontage_access == VehicleFrontageAccess::BothSides
        {
            return None;
        }
        path.points[0] = entrance.door_pos;
        path.points[1] = lane_point;
        path.count = 2;
    }

    if reverse {
        for idx in 0..(path.count / 2) {
            path.points.swap(idx, path.count - 1 - idx);
        }
    }

    Some(path)
}

fn point_on_segment(p: Vector2, a: Vector2, b: Vector2) -> bool {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 <= 1e-8 {
        return segment_distance(p, a) <= 0.05;
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    let proj = a + ab * t;
    segment_distance(p, proj) <= 0.05
}

/// How far `pos` is from the end of `path` along it: 0 at the end, infinite off it (a vehicle
/// not on this way). O(points).
pub(in crate::simulation::economy::agents::tick) fn left_along(path: &[Vector2], pos: Vector2) -> f32 {
    match local_access_target_segment(pos, path) {
        Some(k) => {
            pos.distance_to(path[k])
                + path[k..].windows(2).map(|w| w[0].distance_to(w[1])).sum::<f32>()
        }
        None if path.last().is_some_and(|end| end.distance_to(pos) <= 0.05) => 0.0,
        None => f32::INFINITY,
    }
}

/// The point `back` metres short of the end of `path` along it, its first point when the path is
/// shorter (`PARK-32`, the leaving line). O(points), no allocation.
pub(in crate::simulation::economy::agents::tick) fn point_back_along(
    path: &[Vector2],
    back: f32,
) -> Vector2 {
    let mut left = back.max(0.0);
    for w in path.windows(2).rev() {
        let seg = w[0].distance_to(w[1]);
        if left <= seg {
            return if seg > 1e-6 { w[1] + (w[0] - w[1]) * (left / seg) } else { w[1] };
        }
        left -= seg;
    }
    path.first().copied().unwrap_or(Vector2::ZERO)
}

/// Returns the next segment endpoint index the agent should move toward on the polyline
/// `points`. O(points).
pub(crate) fn local_access_target_segment(current: Vector2, points: &[Vector2]) -> Option<usize> {
    let count = points.len();
    if count <= 1 {
        return None;
    }
    if segment_distance(current, points[count - 1]) <= 0.05 {
        return None;
    }

    for idx in (1..count).rev() {
        let a = points[idx - 1];
        let b = points[idx];
        if segment_distance(current, b) <= 0.0001 {
            continue;
        }
        if point_on_segment(current, a, b) || segment_distance(current, a) <= 0.05 {
            return Some(idx);
        }
    }

    None
}

fn local_access_remaining_distance(current: Vector2, points: &[Vector2]) -> Option<f32> {
    let target_idx = local_access_target_segment(current, points)?;
    let mut distance = segment_distance(current, points[target_idx]);
    for idx in (target_idx + 1)..points.len() {
        distance += segment_distance(points[idx - 1], points[idx]);
    }
    Some(distance)
}

/// Returns true when a traffic-debug local-access step carries new useful information.
pub(in crate::simulation::economy::agents::tick) fn local_access_should_log_step(
    current: Vector2,
    next: Vector2,
    points: &[Vector2],
    reached_end: bool,
) -> bool {
    if reached_end {
        return true;
    }

    let seg_before = local_access_target_segment(current, points);
    let seg_after = local_access_target_segment(next, points);
    if seg_before != seg_after {
        return true;
    }

    let Some(before_remaining) = local_access_remaining_distance(current, points) else {
        return true;
    };
    let Some(after_remaining) = local_access_remaining_distance(next, points) else {
        return true;
    };

    before_remaining.floor() != after_remaining.floor()
}

/// Advances along the polyline `points` by `step` meters. O(points).
pub(in crate::simulation::economy::agents::tick) fn advance_along_local_access_path(
    current: Vector2,
    points: &[Vector2],
    step: f32,
) -> (Vector2, bool) {
    let count = points.len();
    if count <= 1 {
        return (current, true);
    }

    let mut pos = current;
    let mut remaining = step.max(0.0);

    loop {
        let Some(idx) = local_access_target_segment(pos, points) else {
            return (points[count - 1], true);
        };

        let target = points[idx];
        let dist = segment_distance(pos, target);
        if remaining < dist && dist > 1e-6 {
            let dir = (target - pos).normalized();
            return (pos + dir * remaining, false);
        }

        pos = target;
        if idx == count - 1 {
            return (pos, true);
        }
        remaining = (remaining - dist).max(0.0);
        if remaining <= 1e-6 {
            return (pos, false);
        }
    }
}

/// [`advance_along_local_access_path`] for a walker that may stand off its polyline, beside the
/// car it has just left (`PARK-35`): off it, the walker heads straight for the polyline's second
/// point (the first past the place it left) and follows the rest from there, where the plain
/// advance would put it at the end at once. On the polyline it is the plain advance. O(points).
pub(in crate::simulation::economy::agents::tick) fn step_out_along_local_access_path(
    current: Vector2,
    points: &[Vector2],
    step: f32,
) -> (Vector2, bool) {
    let count = points.len();
    if count < 2
        || local_access_target_segment(current, points).is_some()
        || segment_distance(current, points[count - 1]) <= 0.05
    {
        return advance_along_local_access_path(current, points, step);
    }
    let target = points[1];
    let dist = segment_distance(current, target);
    if step < dist && dist > 1e-6 {
        return (current + (target - current).normalized() * step.max(0.0), false);
    }
    (target, count == 2)
}
