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
pub(in crate::simulation::economy::agents::tick) struct LocalAccessPath {
    /// Ordered access points followed by the agent.
    pub(in crate::simulation::economy::agents::tick) points: [Vector2; 4],
    /// Number of valid entries in `points`.
    pub(in crate::simulation::economy::agents::tick) count: usize,
}

/// Builds a short local access path between a building door and a lane/curb handoff point.
pub(in crate::simulation::economy::agents::tick) fn local_access_path(
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
        points: [Vector2::ZERO; 4],
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
        let same_side_cross_point = edge_pos + normal * (edge.width * 0.5);
        let opposite_side_cross_point = edge_pos - normal * (edge.width * 0.5);
        path.points[0] = entrance.door_pos;
        path.points[1] = same_side_cross_point;
        path.points[2] = opposite_side_cross_point;
        path.points[3] = lane_point;
        path.count = 4;
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

/// Returns the next segment endpoint index the agent should move toward.
pub(in crate::simulation::economy::agents::tick) fn local_access_target_segment(
    current: Vector2,
    path: &LocalAccessPath,
) -> Option<usize> {
    if path.count <= 1 {
        return None;
    }
    if segment_distance(current, path.points[path.count - 1]) <= 0.05 {
        return None;
    }

    for idx in (1..path.count).rev() {
        let a = path.points[idx - 1];
        let b = path.points[idx];
        if segment_distance(current, b) <= 0.0001 {
            continue;
        }
        if point_on_segment(current, a, b) || segment_distance(current, a) <= 0.05 {
            return Some(idx);
        }
    }

    None
}

fn local_access_remaining_distance(current: Vector2, path: &LocalAccessPath) -> Option<f32> {
    let target_idx = local_access_target_segment(current, path)?;
    let mut distance = segment_distance(current, path.points[target_idx]);
    for idx in (target_idx + 1)..path.count {
        distance += segment_distance(path.points[idx - 1], path.points[idx]);
    }
    Some(distance)
}

/// Returns true when a traffic-debug local-access step carries new useful information.
pub(in crate::simulation::economy::agents::tick) fn local_access_should_log_step(
    current: Vector2,
    next: Vector2,
    path: &LocalAccessPath,
    reached_end: bool,
) -> bool {
    if reached_end {
        return true;
    }

    let seg_before = local_access_target_segment(current, path);
    let seg_after = local_access_target_segment(next, path);
    if seg_before != seg_after {
        return true;
    }

    let Some(before_remaining) = local_access_remaining_distance(current, path) else {
        return true;
    };
    let Some(after_remaining) = local_access_remaining_distance(next, path) else {
        return true;
    };

    before_remaining.floor() != after_remaining.floor()
}

/// Advances along a local access path by `step` meters.
pub(in crate::simulation::economy::agents::tick) fn advance_along_local_access_path(
    current: Vector2,
    path: &LocalAccessPath,
    step: f32,
) -> (Vector2, bool) {
    if path.count <= 1 {
        return (current, true);
    }

    let mut pos = current;
    let mut remaining = step.max(0.0);

    loop {
        while let Some(idx) = local_access_target_segment(pos, path) {
            let b = path.points[idx];
            if segment_distance(pos, b) > 0.0001 {
                break;
            }
            pos = b;
        }

        let Some(idx) = local_access_target_segment(pos, path) else {
            return (path.points[path.count - 1], true);
        };

        let target = path.points[idx];
        let dist = segment_distance(pos, target);
        if remaining < dist && dist > 1e-6 {
            let dir = (target - pos).normalized();
            return (pos + dir * remaining, false);
        }

        pos = target;
        if idx == path.count - 1 {
            return (pos, true);
        }
        remaining = (remaining - dist).max(0.0);
        if remaining <= 1e-6 {
            return (pos, false);
        }
    }
}
