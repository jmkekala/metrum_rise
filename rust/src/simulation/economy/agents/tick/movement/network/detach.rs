// SPDX-License-Identifier: GPL-3.0-or-later

//! Network-lane detach into local access ingress movement.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_INGRESS};
use super::super::super::access::{
    CROSSING_GIVE_UP_S, crossing_at, crossing_clear, gate_line_d, local_access_point,
    local_access_side_label,
};
use super::super::super::claims::LaneClaimContext;
use super::super::super::lane_nav::{
    lane_hosts_turn_preparation, lane_terminal_node, route_continues_past_lane,
};
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::{claim_lane_entry, lane_half_length};
use super::apply_network_replan;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use crate::simulation::parking::ParkingTripPhase;
use crate::traffic_log;
use std::sync::atomic::AtomicU32;

/// Attempts to detach from the current network lane into the target building's access path.
///
/// The detach rule (`TRAFFIC-15`, `TRAFFIC-13`): a car detaches once its movement step has
/// reached `reached_d` at or past the planned detach distance on the planned detach lane, and
/// only on the route's last visit to that lane, when no route node is left beyond the lane's
/// terminal node. A step that ends inside the lane reaches its `lane_d`; a step that reaches the
/// lane's end reaches `length + ACCESS_LANE_D_TOLERANCE_M`, the planner's legality bound, so every
/// legal detach point is reachable. A truck bound for a freight gate with no dock granted yet
/// (`PARK-36`) first stops on the lane at the end of the line of trucks waiting before the gate
/// (`gate_line_d` of the number waiting at its building at the last commit; never past the gate's
/// lane point, nor back from `start_d`, where its step began) and stands `ready` for the gate
/// commit: a dock free, it drives on holding it and detaches at the gate's lane point as any
/// truck; none, the commit takes it off the lane there, into the line. O(1), plus O(log W) for a
/// truck on its gate's lane with W trucks waiting in all lines; no allocation.
///
/// Returns `true` when movement should stop for this tick.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
// Keep frontage geometry and logging out of the frequently executed road movement loop.
#[inline(never)]
pub(super) unsafe fn try_network_detach(
    i: usize,
    lane_id: usize,
    start_d: f32,
    reached_d: f32,
    delta: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    lane_buckets: &[Vec<LaneEntry>],
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;
        let s_cur_e = &slices.cur_e;

        let access_plan_valid = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
        let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        if !access_plan_valid
            || planned_detach_lane_id == usize::MAX
            || lane_id != planned_detach_lane_id
        {
            return false;
        }
        let planned_d = *s_plan_detach_lane_d.get(i);
        let trip = *slices.parking_trip.get(i);
        let line_end = (trip.freight_gate
            && trip.phase == crate::simulation::parking::ParkingTripPhase::None
            && trip.destination.is_none()
            && slices.parking.is_some())
        .then(|| {
            let lines = slices.kerb_lines;
            let waiting = lines.partition_point(|&(b, _)| b <= trip.target)
                - lines.partition_point(|&(b, _)| b < trip.target);
            gate_line_d(planned_d, waiting)
        });
        if reached_d < line_end.unwrap_or(planned_d)
            || route_continues_past_lane(
                s_path.get(i),
                *s_path_idx.get(i),
                lane_terminal_node(lane_id, transit_network, graph),
            )
        {
            return false;
        }
        if let Some(end) = line_end {
            // At the end of the line before its gate: it stands on the lane there, keeping its
            // speed, for the commit's decision; where it began the step if that was past the end
            // already (it joined the road within the line, or the line grew under it), never
            // back.
            *s_lane_d.get_mut(i) = end
                .max(start_d)
                .min(planned_d)
                .min(transit_network.lane_system.lanes[lane_id].length);
            slices.parking_trip.get_mut(i).ready = true;
            return true;
        }

        let detach_d = planned_d;
        // A far-side access (`PARK-32`): the car waits here, in its lane, for a gap in the lane it
        // crosses, and after `CROSSING_GIVE_UP_S` drives on to come back on the near side.
        let car = *s_tmode.get(i) == MODE_CAR;
        let crossing = if car {
            detach_access(i, allocator, transit_network, graph, slices).and_then(|access| {
                crossing_at(planned_detach_lane_id, detach_d, access.side, transit_network)
            })
        } else {
            None
        };
        if let Some((crossed, crossed_d)) = crossing {
            let half = lane_half_length(MODE_CAR, *slices.vehicle_type.get(i));
            if !crossing_clear(crossed, crossed_d, half, transit_network, graph, lane_buckets)
                || !claim_lane_entry(i, crossed, lane_claims)
            {
                *slices.crossing_wait.get_mut(i) += delta;
                if *slices.crossing_wait.get(i) >= CROSSING_GIVE_UP_S {
                    *slices.crossing_wait.get_mut(i) = 0.0;
                    if give_up_crossing(
                        i,
                        lane_id,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        slices,
                    ) {
                        // On its way on: it keeps moving along its lane from here.
                        return false;
                    }
                }
                *s_lane_d.get_mut(i) = detach_d.min(transit_network.lane_system.lanes[lane_id].length);
                *s_speed.get_mut(i) = 0.0;
                return true;
            }
        }
        let detach_allowed = if car {
            claim_lane_entry(i, planned_detach_lane_id, lane_claims)
        } else {
            true
        };
        if detach_allowed {
            let t_bldg_idx = *s_tgt_b.get(i);
            if t_bldg_idx < allocator.entrances.len()
                && let Some(ingress_origin) = local_access_point(
                    *s_tmode.get(i),
                    &allocator.entrances[t_bldg_idx],
                    planned_detach_lane_id,
                    detach_d,
                    transit_network,
                )
            {
                let ingress_origin = if trip.phase == crate::simulation::parking::ParkingTripPhase::WalkToCar {
                    slices.parking.and_then(|p| p.place_curb(trip.source)).unwrap_or(ingress_origin)
                } else { ingress_origin };
                if trip.phase == crate::simulation::parking::ParkingTripPhase::Drive {
                    slices.parking_trip.get_mut(i).phase = crate::simulation::parking::ParkingTripPhase::PullIn;
                }
                *s_pos_x.get_mut(i) = ingress_origin.x;
                *s_pos_y.get_mut(i) = ingress_origin.y;
                s_path.get_mut(i).clear();
                *s_path_idx.get_mut(i) = 0;
                *s_cur_n.get_mut(i) = u32::MAX;
                *s_cur_e.get_mut(i) = usize::MAX;
                *s_lane_id.get_mut(i) = usize::MAX;
                *s_lane_d.get_mut(i) = 0.0;
                *s_speed.get_mut(i) = 0.0;
                *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                // Crossing, it holds the crossed lane at the crossing's distance (`PARK-32`).
                *slices.crossing_wait.get_mut(i) = 0.0;
                *slices.crossing_lane.get_mut(i) = crossing.map_or(u32::MAX, |(lane, _)| lane as u32);
                if let Some((_, crossed_d)) = crossing {
                    *s_lane_d.get_mut(i) = crossed_d;
                }
                if crate::debug::Channel::Traffic.enabled() {
                    let entrance = &allocator.entrances[t_bldg_idx];
                    traffic_log!(
                        "[ACCESS_INGRESS_DETACH] agent={} target_bldg={} lane={}({}) lane_d={:.2} ingress_origin=({:.2},{:.2}) detach_node={} path_count={} flags=0x{:02x}",
                        i,
                        t_bldg_idx,
                        planned_detach_lane_id,
                        local_access_side_label(*s_tmode.get(i), entrance, planned_detach_lane_id,),
                        detach_d,
                        ingress_origin.x,
                        ingress_origin.y,
                        *s_plan_detach_n.get(i),
                        s_path.get(i).len(),
                        *s_access_flags.get(i),
                    );
                }
                return true;
            }
            return false;
        }

        if crate::debug::Channel::Traffic.enabled() {
            let target_entrance = allocator.entrances.get(*s_tgt_b.get(i));
            let side = target_entrance
                .map(|entrance| {
                    local_access_side_label(*s_tmode.get(i), entrance, planned_detach_lane_id)
                })
                .unwrap_or("unknown-target");
            traffic_log!(
                "[ACCESS_INGRESS_WAIT] agent={} target_bldg={} lane={}({}) lane_d={:.2} reason=detach-slot-busy",
                i,
                *s_tgt_b.get(i),
                planned_detach_lane_id,
                side,
                detach_d,
            );
        }
        // Wait at the detach point; a lane-end point within the tolerance waits at the end.
        *s_lane_d.get_mut(i) = detach_d.min(transit_network.lane_system.lanes[lane_id].length);
        *s_speed.get_mut(i) = 0.0;
        true
    }
}

/// The access agent `i` detaches toward: its parking trip's place, a drop-off's kerb stop, or the
/// door of its target (a truck's kerb stop and gate lie on the door's side). Only its side and
/// road are read. O(1) plus a place lookup on a parking leg.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
unsafe fn detach_access(
    i: usize,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Option<BuildingEntrance> {
    unsafe {
        let trip = *slices.parking_trip.get(i);
        let place = slices
            .parking
            .filter(|_| trip.phase != ParkingTripPhase::None)
            .and_then(|p| p.trip_entrance(trip, false, allocator, transit_network, graph));
        place.or_else(|| allocator.entrances.get(*slices.tgt_b.get(i)).cloned())
    }
}

/// Gives up a crossing to a far-side access (`PARK-32`): plans again from the end of lane
/// `lane_id` to the same access, now reached only from its own side, so the car drives on and
/// comes back on the near lane. Whether the plan was made; without one the car waits on.
/// O(one plan).
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
unsafe fn give_up_crossing(
    i: usize,
    lane_id: usize,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let Some(node) = lane_terminal_node(lane_id, transit_network, graph) else {
            return false;
        };
        let trip = *slices.parking_trip.get(i);
        let Some(mut access) = slices
            .parking
            .and_then(|p| p.replan_entrance(trip, allocator, transit_network, graph))
            .or_else(|| allocator.entrances.get(*slices.tgt_b.get(i)).cloned())
        else {
            return false;
        };
        access.vehicle_frontage_access = VehicleFrontageAccess::SameSideOnly;
        BuildingAllocator::refresh_entrance_lanes(&mut access, graph, &transit_network.lane_system);
        let first_lane = (!lane_hosts_turn_preparation(lane_id, transit_network)).then_some(lane_id);
        let plan = plan_network_replan(
            node,
            transit_network.lane_system.lanes[lane_id].edge_id,
            None,
            *slices.tgt_b.get(i),
            MODE_CAR,
            *slices.access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "crossing-give-up")),
            first_lane,
            Some(access),
        );
        match plan {
            Ok(plan) => {
                apply_network_replan(i, plan, slices);
                true
            }
            Err(_) => false,
        }
    }
}
