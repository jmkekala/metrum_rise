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
/// car's width (1.8 m, the car row of the lot proof) and a little room.
pub(crate) const DROP_OFF_KERB_INSET_M: f32 = 1.0;

/// How far inside the kerb a freight truck's kerb stop puts the truck's centre (`PARK-36`): half
/// a truck's width (2.6 m, the truck row of the lot proof) and the same room as a car's.
pub(crate) const FREIGHT_KERB_INSET_M: f32 = 1.4;

/// The kerb stop at `entrance`'s door (`PARK-34`, `PARK-36`): the door's own entrance with its
/// door point moved to `inset` inside the door side's kerb, on the door's road connection
/// ([`DROP_OFF_KERB_INSET_M`] for a drop-off car, [`FREIGHT_KERB_INSET_M`] for a truck). A
/// vehicle's access path to it is then the door's: from the lane point (across the road from
/// the far lane of a two-sided frontage) to the stop, and back. `None` when the door's road is
/// gone. O(edge points).
pub(crate) fn kerb_stop_entrance(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
    inset: f32,
) -> Option<BuildingEntrance> {
    let edge_pos = entrance_edge_pos(entrance, graph)?;
    let normal = entrance_edge_normal(entrance, graph)?;
    let kerb = graph.edge(entrance.edge_idx).frontage_half_width_m(entrance.side);
    Some(BuildingEntrance {
        door_pos: edge_pos + normal * (kerb - inset).max(0.0),
        ..entrance.clone()
    })
}

/// The spacing of the trucks waiting in a line at a kerb stop for a dock (`PARK-36`): a truck's
/// 8 m body and 1 m between bodies.
pub(crate) const KERB_QUEUE_SPACING_M: f32 = crate::config::TRUCK_BODY_LENGTH_M + 1.0;

/// Place `slot` of the line of trucks started together at `entrance`'s kerb stop, dispatched at
/// one moment from a building without a dock (`PARK-36`): the kerb stop moved `slot` spacings
/// ([`KERB_QUEUE_SPACING_M`]) back along the door side's kerb, against the near lane's travel. A
/// place would leave the road's edge: it stops half a body short of the edge's end, where every
/// further truck stands too. Slot 0 is the kerb stop. O(edge points).
pub(crate) fn kerb_queue_entrance(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
    slot: usize,
) -> Option<BuildingEntrance> {
    let edge = graph.get_edge(entrance.edge_idx)?;
    // The near lane runs along the edge for a door on its side -1 (`same_side_car_lane`).
    let back = if entrance.side == -1 { -1.0 } else { 1.0 };
    let half = crate::config::TRUCK_BODY_LENGTH_M * 0.5;
    let door = entrance.entrance_s_m;
    let s = door + back * slot as f32 * KERB_QUEUE_SPACING_M;
    let s = s.clamp(half.min(door), (edge.physical_length - half).max(door));
    kerb_stop_entrance(
        &BuildingEntrance {
            entrance_s_m: s,
            ..entrance.clone()
        },
        graph,
        FREIGHT_KERB_INSET_M,
    )
}

/// The lane distance, on a freight gate's near lane, of place `slot` of the line of trucks waiting
/// for a dock before the gate (`PARK-36`), for a gate whose lane point lies `gate_d` along that
/// lane: `slot + 1` spacings ([`KERB_QUEUE_SPACING_M`], a body and 1 m) back from the lane point,
/// against the lane's travel. The gate's mouth keeps the first spacing: the first waiting truck's
/// centre stands 9 m short of the lane point, its front 5 m short (9 m less half its 8 m body), so
/// a gate up to 10 m wide stays clear and a truck turning in has room. Places stop half a body
/// past the lane's start, where the road leaves its junction: every further truck stands there
/// too. O(1).
pub(crate) fn gate_line_d(gate_d: f32, slot: usize) -> f32 {
    let half = crate::config::TRUCK_BODY_LENGTH_M * 0.5;
    (gate_d - (slot + 1) as f32 * KERB_QUEUE_SPACING_M).max(half.min(gate_d))
}

/// The kerb stop beside `point`, a lane point of `entrance`'s road (`PARK-36`): the kerb stop of
/// `entrance` moved along its road to where `point` projects. A truck waiting for a dock stands
/// at the kerb beside its lane point in the line before the gate, and one sent to the line at a
/// kerb stop by a save from before the line moved there leaves its lane beside its place, so its
/// access path ends at it. O(edge points).
pub(crate) fn kerb_stop_beside(
    entrance: &BuildingEntrance,
    graph: &RegionGraph,
    point: Vector2,
) -> Option<BuildingEntrance> {
    let edge = graph.get_edge(entrance.edge_idx)?;
    let s = BuildingAllocator::project_point_to_polyline_s(&edge.physical_geometry, point);
    kerb_stop_entrance(
        &BuildingEntrance {
            entrance_s_m: s,
            ..entrance.clone()
        },
        graph,
        FREIGHT_KERB_INSET_M,
    )
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

pub(in crate::simulation::economy::agents::tick) fn same_side_car_lane(entrance: &BuildingEntrance) -> usize {
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
