// SPDX-License-Identifier: GPL-3.0-or-later

//! Shared local access geometry helpers.

use super::super::super::{MODE_CAR, MODE_WALK};
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

pub(super) fn segment_distance(a: Vector2, b: Vector2) -> f32 {
    (a - b).length()
}

pub(super) fn entrance_edge_pos(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
) -> Option<Vector2> {
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

pub(super) fn entrance_edge_normal(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
) -> Option<Vector2> {
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

/// How far inside the kerb a drop-off car's kerb stop puts the car's centre (`PARK-34`): half a
/// car's width and a little room.
pub(crate) const DROP_OFF_KERB_INSET_M: f32 = 1.0;

/// The kerb stop of a drop-off car at `entrance`'s door (`PARK-34`): the door's own entrance with
/// its door point moved to [`DROP_OFF_KERB_INSET_M`] inside the door side's kerb, on the door's
/// road connection. A car's access path to it is then the door's: from the lane point (across
/// the road from the far lane of a two-sided frontage) to the stop, and back. `None` when the
/// door's road is gone. O(edge points).
pub(crate) fn kerb_stop_entrance(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
) -> Option<BuildingEntrance> {
    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let normal = entrance_edge_normal(entrance, graph)?;
    let kerb = graph.edge(entrance.edge_idx).frontage_half_width_m(entrance.side);
    Some(BuildingEntrance {
        door_pos: edge_pos + normal * (kerb - DROP_OFF_KERB_INSET_M).max(0.0),
        ..entrance.clone()
    })
}

/// Projects an entrance onto a lane and returns the lane distance in meters.
pub(in crate::simulation::economy::agents::tick) fn projected_lane_distance_for_entrance(
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
pub(in crate::simulation::economy::agents::tick) fn local_access_point(
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

pub(super) fn same_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_fwd
    } else {
        entrance.car_lane_bkw
    }
}

pub(super) fn opposite_side_car_lane(entrance: &BuildingEntrance) -> usize {
    if entrance.side == -1 {
        entrance.car_lane_bkw
    } else {
        entrance.car_lane_fwd
    }
}

pub(super) fn entrance_allows_lane(mode: u8, entrance: &BuildingEntrance, lane_id: usize) -> bool {
    if mode == MODE_CAR {
        lane_id == entrance.car_lane_fwd || lane_id == entrance.car_lane_bkw
    } else {
        lane_id == entrance.foot_lane_fwd || lane_id == entrance.foot_lane_bkw
    }
}

/// Returns a compact label for access-side debug logging.
pub(in crate::simulation::economy::agents::tick) fn local_access_side_label(
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
