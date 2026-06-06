//! Local access and frontage travel-time helpers.

use super::super::super::{MODE_CAR, MODE_WALK};
use super::geometry::{
    entrance_edge_normal, entrance_edge_pos, local_access_point, opposite_side_car_lane,
    same_side_car_lane, segment_distance,
};
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, AGENT_WALK_SPEED_MS};
use crate::simulation::buildings::allocator::BuildingEntrance;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;

/// Returns the local door-to-lane or lane-to-door access distance.
pub(in crate::simulation::economy::agents::tick) fn local_access_distance(
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
    let same_side_lane = same_side_car_lane(entrance);
    let opposite_side_lane = opposite_side_car_lane(entrance);
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
pub(in crate::simulation::economy::agents::tick) fn local_access_time_s(
    distance: f32,
    mode: u8,
) -> f32 {
    let speed = if mode == MODE_CAR {
        AGENT_DRIVEWAY_SPEED_MS
    } else {
        AGENT_WALK_SPEED_MS
    };
    distance / speed
}

/// Returns the frontage travel time from or to a planned access handoff.
pub(in crate::simulation::economy::agents::tick) fn frontage_time_s(
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
pub(in crate::simulation::economy::agents::tick) fn direct_frontage_segment_time_s(
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
