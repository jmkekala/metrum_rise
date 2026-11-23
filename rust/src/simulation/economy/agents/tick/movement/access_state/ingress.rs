// SPDX-License-Identifier: GPL-3.0-or-later

//! Local access-ingress movement from the planned network lane to a building door.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_WALK, TRANSIT_NETWORK};
use super::super::super::access::{
    FREIGHT_KERB_INSET_M, LocalAccessPath, ManeuverPath, kerb_stop_beside, kerb_stop_entrance,
    local_access_path, local_access_point,
    local_access_should_log_step, local_access_side_label, local_access_target_segment,
    planned_detach_is_legal,
};
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::{MovementSlices, enter_lane, hold_for_lane_attach};
use super::super::super::trip_failures::FailureCause;
use super::super::network::apply_network_replan;
use super::super::replan_watchdog::delay_or_recover_after_network_replan_failure;
use super::super::{NETWORK_REPLAN_DELAY_S, transit_mode_label};
use super::{arrive_in_building, point, reset_invalid_access_plan, step_along_access_path};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::parking::{ParkingTrip, ParkingTripPhase};
use crate::traffic_log;
use godot::prelude::*;
use std::sync::atomic::AtomicU32;

/// Handles local access movement from a planned network detach lane to the target building door.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(in crate::simulation::economy::agents::tick::movement) unsafe fn handle_access_ingress(
    i: usize,
    delta: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;
        let s_activity = &slices.activity;
        let s_tgt_b = &slices.tgt_b;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;

        let trip = *slices.parking_trip.get(i);
        // `freight_gate` and the phase are the tests a carrier without a yard trip, and every
        // resident, pays here (`PARK-20`): bytes of the row this step reads anyway.
        if trip.phase == ParkingTripPhase::AtKerb
            || (trip.freight_gate && trip.phase == ParkingTripPhase::AtDock)
        {
            // Standing in the bay or at the kerb stop (`PARK-36`) until its shipment's check
            // delivers or a dock is granted; nothing moves it.
            *s_speed.get_mut(i) = 0.0;
            return;
        }
        let b_id = *s_tgt_b.get(i);
        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
            // A carrier's yard trip ends with its building: the hold it had goes with the trip.
            if trip.freight_gate {
                *slices.parking_trip.get_mut(i) = Default::default();
            }
            // The building went under a parking leg: its recovery owns the car and the bay.
            if slices.parking_trip.get(i).phase != crate::simulation::parking::ParkingTripPhase::None {
                let trip = slices.parking_trip.get_mut(i);
                trip.ready = true;
                trip.replan_needed = true;
                *slices.speed.get_mut(i) = 0.0;
                return;
            }
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, usize::MAX, pos, 0.0, slices);
            return;
        }
        let plan_valid =
            (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0 && b_id < allocator.entrances.len();
        // A freight carrier never enters a building (`PARK-36`): one whose plan a road edit
        // dropped, or one the gate commit sent on to the kerb stop (`ToKerb`, its plan dropped
        // there), goes back onto the lane at its lane point and plans again.
        let freight = *slices.freight_shipment_id.get(i) != u64::MAX;
        if !plan_valid && freight && b_id < allocator.entrances.len() {
            return_to_lane(
                i,
                b_id,
                *s_plan_detach_lane.get(i) as usize,
                *s_plan_detach_lane_d.get(i),
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
            return;
        }
        if !plan_valid {
            // A road edit drops the access plan of an agent already on its access path
            // (`PARK-11`). A walk to the car or a pull-in ends where it was going, as one that
            // reaches its end below does, and the parking phase plans the next leg from the bay;
            // only a leg that ends at a door arrives there.
            if matches!(
                slices.parking_trip.get(i).phase,
                crate::simulation::parking::ParkingTripPhase::WalkToCar
                    | crate::simulation::parking::ParkingTripPhase::PullIn
            ) {
                slices.parking_trip.get_mut(i).ready = true;
                *s_speed.get_mut(i) = 0.0;
                return;
            }
            let ingress_target = allocator
                .entrances
                .get(b_id)
                .map(|entrance| entrance.door_pos)
                .unwrap_or(Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i)));
            arrive_in_building(i, b_id, ingress_target, sim_time, slices);
            return;
        }
        if trip.freight_gate && trip.phase == ParkingTripPhase::None && slices.parking.is_some() {
            // At the gate's lane point, holding the dock granted to it at the end of the line
            // before the gate (`PARK-36`): the parking phase's ordered commit sends it in this
            // tick (`PARK-20`). It stands meanwhile.
            slices.parking_trip.get_mut(i).ready = true;
            *s_speed.get_mut(i) = 0.0;
            return;
        }
        // Off a parking leg there is no endpoint to look up: most agents here, every tick.
        let parking_entrance = slices
            .parking
            .filter(|_| trip.phase != ParkingTripPhase::None)
            .and_then(|p| p.trip_entrance(trip, false, allocator, transit_network, graph));
        let detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        let detach_lane_d = *s_plan_detach_lane_d.get(i);
        // A truck bound for a building's door stops at its kerb instead (`PARK-36`); one a save
        // from before the line moved to the gate sent on to wait for a dock at the door's kerb
        // stop, at its place in the line there, beside where it leaves the lane.
        let at_kerb = freight && !trip.freight_gate && trip.phase == ParkingTripPhase::None;
        let door = &allocator.entrances[b_id];
        let kerb_entrance = if at_kerb {
            kerb_stop_entrance(door, graph, FREIGHT_KERB_INSET_M)
        } else if trip.freight_gate && trip.phase == ParkingTripPhase::ToKerb {
            transit_network.lane_system.lanes.get(detach_lane_id).and_then(|lane| {
                let point = BuildingAllocator::sample_pos_on_lane(lane, detach_lane_d);
                kerb_stop_beside(door, graph, point)
            })
        } else {
            None
        };
        let entrance = kerb_entrance
            .as_ref()
            .or(parking_entrance.as_ref())
            .unwrap_or(door);
        let legal_detach = planned_detach_is_legal(
            *s_tmode.get(i),
            entrance,
            detach_lane_id,
            detach_lane_d,
            *s_plan_detach_n.get(i),
            transit_network,
            graph,
        );
        // The walked polyline, borrowed from whichever path was built: a bay's or a dock's
        // manoeuvre (`PARK-21`: a dock's way, on the stack) or the door's access path.
        let maneuver;
        let access;
        // A drop-off's kerb stop (`PARK-34`) is reached by the door's access path, its door point
        // at the kerb: no place, no manoeuvre.
        let kerb_stop = trip.drop_off && trip.destination.is_none();
        let exact_path = if trip.phase == crate::simulation::parking::ParkingTripPhase::PullIn && !kerb_stop {
            maneuver = slices.parking.and_then(|p| p.maneuver_path(trip.destination, detach_lane_id, detach_lane_d, transit_network, true, trip.freight_gate));
            maneuver.as_ref().map(ManeuverPath::as_slice)
        } else if legal_detach {
            access = local_access_path(
                *s_tmode.get(i),
                entrance,
                detach_lane_id,
                detach_lane_d,
                transit_network,
                graph,
                true,
            );
            access.as_ref().map(LocalAccessPath::as_slice)
        } else {
            None
        };

        if let (true, Some(path)) = (legal_detach, exact_path) {
            // A truck granted a dock while it waits at the kerb before the gate (`PARK-36`)
            // stands beside its pull-in's first point: it heads for the gate and follows the
            // way in from there. Every other pull-in starts on its polyline.
            let (current, next_pos, reached_door) = step_along_access_path(
                i,
                path,
                delta,
                trip.freight_gate && trip.phase == ParkingTripPhase::PullIn,
                slices,
            );
            // Past the near kerb, the hop's second point, it has crossed (`PARK-32`).
            if *slices.crossing_lane.get(i) != u32::MAX
                && (reached_door
                    || local_access_target_segment(next_pos, path).is_none_or(|next| next > 1))
            {
                *slices.crossing_lane.get_mut(i) = u32::MAX;
            }
            if crate::debug::Channel::Traffic.enabled()
                && local_access_should_log_step(current, next_pos, path, reached_door)
            {
                let seg_before = local_access_target_segment(current, path);
                let seg_after = local_access_target_segment(next_pos, path);
                traffic_log!(
                    "[ACCESS_INGRESS_STEP] agent={} bldg={} mode={} lane={}({}) lane_d={:.2} seg_before={:?} seg_after={:?} reached_door={} current=({:.2},{:.2}) next=({:.2},{:.2}) p0=({:.2},{:.2}) p1=({:.2},{:.2}) p2=({:.2},{:.2}) p3=({:.2},{:.2}) count={}",
                    i,
                    b_id,
                    transit_mode_label(*s_tmode.get(i)),
                    detach_lane_id,
                    local_access_side_label(*s_tmode.get(i), entrance, detach_lane_id),
                    detach_lane_d,
                    seg_before,
                    seg_after,
                    reached_door,
                    current.x,
                    current.y,
                    next_pos.x,
                    next_pos.y,
                    point(path, 0).x,
                    point(path, 0).y,
                    point(path, 1).x,
                    point(path, 1).y,
                    point(path, 2).x,
                    point(path, 2).y,
                    point(path, 3).x,
                    point(path, 3).y,
                    path.len(),
                );
            }
            if reached_door {
                let ingress_target = path[path.len() - 1];
                if crate::debug::Channel::Traffic.enabled() {
                    traffic_log!(
                        "[ACCESS_INGRESS_DONE] agent={} bldg={} pos=({:.2},{:.2}) activity_before={} flags=0x{:02x}",
                        i,
                        b_id,
                        ingress_target.x,
                        ingress_target.y,
                        *s_activity.get(i),
                        *s_access_flags.get(i),
                    );
                }
                if matches!(trip.phase, crate::simulation::parking::ParkingTripPhase::WalkToCar | crate::simulation::parking::ParkingTripPhase::PullIn) {
                    slices.parking_trip.get_mut(i).ready = true;
                    *s_speed.get_mut(i) = 0.0;
                } else if at_kerb || trip.phase == ParkingTripPhase::ToKerb {
                    stand_at_kerb(i, b_id, sim_time, allocator, slices);
                } else {
                    arrive_in_building(i, b_id, ingress_target, sim_time, slices);
                }
            }
        } else if trip.freight_gate && trip.phase == ParkingTripPhase::PullIn {
            // A truck pulling into its dock whose way in went stale (`PARK-20`): it ends in its
            // bay, as one whose plan was dropped does above, and is never put back on the road.
            slices.parking_trip.get_mut(i).ready = true;
            *s_speed.get_mut(i) = 0.0;
        } else {
            return_to_lane(
                i,
                b_id,
                detach_lane_id,
                detach_lane_d,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                slices,
            );
        }
    }
}

/// Puts agent `i`, on its access path towards building `b_id`, back on its detach lane at its
/// lane point (a car through an attach gap, `TRAFFIC-11`) and plans it again to its entrance:
/// for an invalid ingress plan, and for a freight carrier that never enters a building
/// (`PARK-36`). Without a lane point it waits at its detach node for a replan.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
unsafe fn return_to_lane(
    i: usize,
    b_id: usize,
    detach_lane_id: usize,
    detach_lane_d: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;
        let s_transit = &slices.transit;
        let s_cur_n = &slices.cur_n;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_cur_e = &slices.cur_e;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_tmode = &slices.tmode;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_speed = &slices.speed;
        let entrance = &allocator.entrances[b_id];
        if let Some(ingress_origin) = local_access_point(
            *s_tmode.get(i),
            entrance,
            detach_lane_id,
            detach_lane_d,
            transit_network,
        ) {
            *s_pos_x.get_mut(i) = ingress_origin.x;
            *s_pos_y.get_mut(i) = ingress_origin.y;
            // A car returns to the lane through an attach gap, as egress does (`TRAFFIC-11`).
            if *s_tmode.get(i) == MODE_WALK {
                enter_lane!(
                    slices,
                    i,
                    transit_network,
                    detach_lane_id,
                    detach_lane_d,
                    *s_plan_detach_n.get(i)
                );
            } else {
                hold_for_lane_attach!(
                    slices,
                    i,
                    transit_network,
                    detach_lane_id,
                    detach_lane_d,
                    *s_plan_detach_n.get(i)
                );
            }
            *s_speed.get_mut(i) = 0.0;
            s_path.get_mut(i).clear();
            if crate::debug::Channel::Traffic.enabled() {
                traffic_log!(
                    "[ACCESS_INGRESS_ABORT] agent={} bldg={} mode={} lane={} lane_d={:.2} ingress_origin=({:.2},{:.2}) reason=invalid-ingress-plan flags=0x{:02x}",
                    i,
                    b_id,
                    transit_mode_label(*s_tmode.get(i)),
                    detach_lane_id,
                    detach_lane_d,
                    ingress_origin.x,
                    ingress_origin.y,
                    *s_access_flags.get(i),
                );
            }
            *s_path_idx.get_mut(i) = 0;
            *s_transit.get_mut(i) = TRANSIT_NETWORK;
            if sim_time >= *s_next_replan_time.get(i) {
                match plan_network_replan(
                    *s_plan_detach_n.get(i),
                    *s_cur_e.get(i),
                    None,
                    b_id,
                    *s_tmode.get(i),
                    *s_access_flags.get(i),
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    Some((i, "invalid-ingress-replan")),
                    None,
                    slices.parking.and_then(|p| p.replan_entrance(*slices.parking_trip.get(i), allocator, transit_network, graph)),
                ) {
                    Ok(replan) => apply_network_replan(i, replan, slices),
                    Err(failure) => delay_or_recover_after_network_replan_failure(
                        i,
                        sim_time,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                        "invalid-ingress-replan",
                        FailureCause::StaleExit {
                            lane: detach_lane_id,
                            lane_d: detach_lane_d,
                            replan: Some(failure),
                        },
                        slices,
                    ),
                }
            }
        } else {
            *s_cur_n.get_mut(i) = *s_plan_detach_n.get(i);
            *s_cur_e.get_mut(i) = usize::MAX;
            *s_lane_id.get_mut(i) = usize::MAX;
            *s_lane_d.get_mut(i) = 0.0;
            *s_speed.get_mut(i) = 0.0;
            s_path.get_mut(i).clear();
            *s_path_idx.get_mut(i) = 0;
            *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
            *s_transit.get_mut(i) = TRANSIT_NETWORK;
        }    }
}

/// Stands freight carrier `i` at the kerb stop of building `b_id`, which its access path has
/// reached (`PARK-36`), out of the driving lane and drawn: one with a yard trip waits there,
/// `ready`, for a dock in the order its gate stamped; any other delivers or loads there at its
/// shipment's next check, its stand timed from now. O(1).
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
unsafe fn stand_at_kerb(
    i: usize,
    b_id: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    slices: &MovementSlices,
) {
    unsafe {
        let trip = slices.parking_trip.get_mut(i);
        if trip.freight_gate {
            trip.phase = ParkingTripPhase::AtKerb;
            trip.ready = true;
        } else {
            *trip = ParkingTrip {
                phase: ParkingTripPhase::AtKerb,
                target: allocator.building_handles.handle(b_id).to_bits(),
                ..ParkingTrip::default()
            };
            *slices.jstart.get_mut(i) = sim_time;
        }
        *slices.speed.get_mut(i) = 0.0;
    }
}
