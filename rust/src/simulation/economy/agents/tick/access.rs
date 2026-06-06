//! Local building access geometry, legality, and access-time helpers.

use super::lane_nav::{lane_origin_node, lane_terminal_node};
use super::{MODE_CAR, MODE_WALK};
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, AGENT_WALK_SPEED_MS};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use godot::prelude::Vector2;

/// Fixed-size polyline for door-to-lane or lane-to-door access movement.
#[derive(Clone, Copy)]
pub(super) struct LocalAccessPath {
    /// Ordered access points followed by the agent.
    pub(super) points: [Vector2; 4],
    /// Number of valid entries in `points`.
    pub(super) count: usize,
}

fn segment_distance(a: Vector2, b: Vector2) -> f32 {
    (a - b).length()
}

fn entrance_edge_pos(entrance: &BuildingEntrance, graph: &RegionGraph) -> Option<Vector2> {
    if entrance.edge_idx >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(entrance.edge_idx);
    if edge.deleted || edge.physical_length <= 1e-6 {
        return None;
    }
    Some(BuildingAllocator::sample_pos_on_edge(
        graph,
        entrance.edge_idx,
        entrance.entrance_s_m / edge.physical_length,
    ))
}

fn entrance_edge_normal(entrance: &BuildingEntrance, graph: &RegionGraph) -> Option<Vector2> {
    if entrance.edge_idx >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(entrance.edge_idx);
    if edge.deleted || edge.physical_length <= 1e-6 {
        return None;
    }
    let tangent = BuildingAllocator::sample_tangent_on_edge(
        graph,
        entrance.edge_idx,
        entrance.entrance_s_m / edge.physical_length,
    );
    Some(Vector2::new(tangent.y, -tangent.x) * entrance.side as f32)
}

/// Projects an entrance onto a lane and returns the lane distance in meters.
pub(super) fn projected_lane_distance_for_entrance(
    entrance: &BuildingEntrance,
    lane_id: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    Some(BuildingAllocator::project_point_to_polyline_s(
        &lane.geometry,
        edge_pos,
    ))
}

/// Returns the world-space access point on a lane or curb for a building entrance.
pub(super) fn local_access_point(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    transit_network: &TransitNetwork,
) -> Option<Vector2> {
    if mode == MODE_WALK {
        Some(entrance.curb_pos)
    } else {
        let lane = transit_network.lane_system.lanes.get(lane_id)?;
        Some(BuildingAllocator::sample_pos_on_lane(lane, lane_d))
    }
}

fn same_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_fwd
    } else {
        entrance.car_lane_bkw
    }
}

fn opposite_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_bkw
    } else {
        entrance.car_lane_fwd
    }
}

fn entrance_allows_lane(mode: u8, entrance: &BuildingEntrance, lane_id: usize) -> bool {
    if mode == MODE_CAR {
        lane_id == entrance.car_lane_fwd || lane_id == entrance.car_lane_bkw
    } else {
        lane_id == entrance.foot_lane_fwd || lane_id == entrance.foot_lane_bkw
    }
}

/// Returns a compact label for access-side debug logging.
pub(super) fn local_access_side_label(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
) -> &'static str {
    if mode != MODE_CAR {
        return "curb";
    }
    if lane_id == same_side_car_lane(entrance) {
        "same-side"
    } else if lane_id == opposite_side_car_lane(entrance) {
        "opposite-side"
    } else {
        "unknown-side"
    }
}

/// Returns whether a planned access attach still matches entrance and lane topology.
pub(super) fn planned_attach_is_legal(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    planned_attach_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if !entrance_allows_lane(mode, entrance, lane_id) {
        return false;
    }
    let Some(lane) = transit_network.lane_system.lanes.get(lane_id) else {
        return false;
    };
    if lane_d < 0.0 || lane_d > lane.length + 1e-4 {
        return false;
    }
    lane_terminal_node(lane_id, transit_network, graph) == Some(planned_attach_node)
}

/// Returns whether a planned access detach still matches entrance and lane topology.
pub(super) fn planned_detach_is_legal(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    planned_detach_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if !entrance_allows_lane(mode, entrance, lane_id) {
        return false;
    }
    let Some(lane) = transit_network.lane_system.lanes.get(lane_id) else {
        return false;
    };
    if lane_d < 0.0 || lane_d > lane.length + 1e-4 {
        return false;
    }
    lane_origin_node(lane_id, transit_network, graph) == Some(planned_detach_node)
}

/// Builds a short local access path between a building door and a lane/curb handoff point.
pub(super) fn local_access_path(
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
pub(super) fn local_access_target_segment(
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

/// Advances along a local access path by `step` meters.
pub(super) fn advance_along_local_access_path(
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

/// Returns the local door-to-lane or lane-to-door access distance.
pub(super) fn local_access_distance(
    mode: u8,
    entrance: &BuildingEntrance,
    lane_id: usize,
    lane_d: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    if mode == MODE_WALK {
        return Some(segment_distance(entrance.door_pos, entrance.curb_pos));
    }

    let chosen_lane_point = local_access_point(mode, entrance, lane_id, lane_d, transit_network)?;
    let same_side_lane = if entrance.side == -1 {
        entrance.car_lane_fwd
    } else {
        entrance.car_lane_bkw
    };
    let opposite_side_lane = if entrance.side == -1 {
        entrance.car_lane_bkw
    } else {
        entrance.car_lane_fwd
    };
    if entrance.vehicle_frontage_access == VehicleFrontageAccess::SameSideOnly
        || lane_id == same_side_lane
    {
        return Some(segment_distance(entrance.door_pos, chosen_lane_point));
    }
    if entrance.vehicle_frontage_access != VehicleFrontageAccess::BothSides
        || lane_id != opposite_side_lane
    {
        return None;
    }

    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let normal = entrance_edge_normal(entrance, graph)?;
    let edge = graph.edge(entrance.edge_idx);
    let same_side_cross_point = edge_pos + normal * (edge.width * 0.5);
    let opposite_side_cross_point = edge_pos - normal * (edge.width * 0.5);
    Some(
        segment_distance(entrance.door_pos, same_side_cross_point)
            + segment_distance(same_side_cross_point, opposite_side_cross_point)
            + segment_distance(opposite_side_cross_point, chosen_lane_point),
    )
}

/// Converts a local access distance into travel time for the current mode.
pub(super) fn local_access_time_s(distance: f32, mode: u8) -> f32 {
    let speed = if mode == MODE_CAR {
        AGENT_DRIVEWAY_SPEED_MS
    } else {
        AGENT_WALK_SPEED_MS
    };
    distance / speed
}

/// Returns the frontage travel time from or to a planned access handoff.
pub(super) fn frontage_time_s(
    mode: u8,
    lane_id: usize,
    lane_d: f32,
    from_attach_point: bool,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    let edge = graph.edge(lane.edge_id);
    let frontage_distance = if from_attach_point {
        (lane.length - lane_d).max(0.0)
    } else {
        lane_d.max(0.0)
    };
    if frontage_distance <= 1e-6 {
        return Some(0.0);
    }

    let travel_speed = if mode == MODE_CAR {
        edge.speed_limit
    } else {
        AGENT_WALK_SPEED_MS
    };
    if !travel_speed.is_finite() || travel_speed <= 1e-6 {
        return None;
    }

    let free_flow_time = frontage_distance / travel_speed;
    if mode == MODE_WALK || lane.length <= 1e-6 {
        return Some(free_flow_time);
    }
    let penalty_ratio = if from_attach_point {
        (lane.length - lane_d) / lane.length
    } else {
        lane_d / lane.length
    };
    Some(free_flow_time + penalty_ratio.max(0.0) * lane.frontage_delay_penalty_s)
}

/// Returns frontage travel time between two distances on one lane.
pub(super) fn direct_frontage_segment_time_s(
    mode: u8,
    lane_id: usize,
    start_lane_d: f32,
    end_lane_d: f32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> Option<f32> {
    let lane = transit_network.lane_system.lanes.get(lane_id)?;
    if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
        return None;
    }
    if end_lane_d + 1e-6 < start_lane_d {
        return None;
    }
    let segment_distance = (end_lane_d - start_lane_d).max(0.0);
    if segment_distance <= 1e-6 {
        return Some(0.0);
    }

    let edge = graph.edge(lane.edge_id);
    let travel_speed = if mode == MODE_CAR {
        edge.speed_limit
    } else {
        AGENT_WALK_SPEED_MS
    };
    if !travel_speed.is_finite() || travel_speed <= 1e-6 {
        return None;
    }

    let free_flow_time = segment_distance / travel_speed;
    if mode == MODE_WALK || lane.length <= 1e-6 {
        return Some(free_flow_time);
    }

    let penalty_ratio = segment_distance / lane.length;
    Some(free_flow_time + penalty_ratio.max(0.0) * lane.frontage_delay_penalty_s)
}

#[cfg(test)]
mod tests {
    use super::{LocalAccessPath, advance_along_local_access_path};
    use godot::prelude::Vector2;

    #[test]
    fn test_opposite_side_car_egress_finishes_when_already_at_lane_endpoint() {
        let path = LocalAccessPath {
            points: [
                Vector2::new(173.14, -47.41),
                Vector2::new(177.24, -47.41),
                Vector2::new(184.24, -47.41),
                Vector2::new(182.49, -47.41),
            ],
            count: 4,
        };

        let current = path.points[3];
        let (next, reached_handoff) = advance_along_local_access_path(current, &path, 0.05);

        assert!(
            reached_handoff,
            "opposite-side egress should complete at the exact lane endpoint instead of backtracking onto the crossover segment"
        );
        assert_eq!(next, path.points[3]);
    }
}
