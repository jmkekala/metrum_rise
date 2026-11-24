// SPDX-License-Identifier: GPL-3.0-or-later

//! Local access-egress movement from a building door to the planned network lane.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR};
use super::super::super::access::{
    FREIGHT_KERB_INSET_M, LocalAccessPath, ManeuverPath, crossing_at, crossing_clear,
    kerb_stop_entrance, left_along, local_access_path, local_access_should_log_step,
    local_access_side_label, local_access_target_segment, planned_attach_is_legal,
    point_back_along, projected_lane_distance_for_entrance,
};
use super::super::super::parking::exit_hold;
use super::super::super::access::CROSSING_GIVE_UP_S;
use super::super::super::claims::LaneClaimContext;
use super::super::super::lane_nav::{
    lane_hosts_turn_preparation, lane_origin_node, lane_terminal_node,
};
use super::super::super::planning::plan_network_replan;
use super::super::network::apply_network_replan;
use super::super::super::slices::{MovementSlices, enter_lane, hold_for_lane_attach};
use super::super::super::traffic::{
    claim_lane_entry, join_hold, lane_attach_slot_clear, lane_half_length,
};
use super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::super::{BUILDING_REPLAN_DELAY_S, transit_mode_label};
use super::{
    point, pulls_out_of_a_dock, reset_invalid_access_plan, stand_in_yard, step_along_access_path,
};
use crate::config::AGENT_DRIVEWAY_SPEED_MS;
use crate::simulation::buildings::allocator::{BuildingAllocator, BuildingEntrance};
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::parking::ParkingTripPhase;
use crate::traffic_log;
use godot::prelude::*;
use std::sync::atomic::AtomicU32;

/// Handles local access movement from a building door to the planned network attachment lane.
///
/// In the movement pass's parallel part (`LaneClaimContext::defers_egress`) a car's step runs
/// only when it stays off every lane: it neither reaches its lane point, nor starts a crossing,
/// nor gives one up, and its crossing test fails on the buckets alone (`PERF-33`). Any other step
/// returns `true` with the agent's row as it found it, and the serial part runs it whole, in
/// agent order. Such a car is in no bucket (a crossing car is `Dynamic`), so no other step reads
/// what it writes, and it reads no other agent's row: its outcome does not depend on the part
/// that runs it.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(in crate::simulation::economy::agents::tick::movement) unsafe fn handle_access_egress(
    i: usize,
    delta: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    lane_buckets: &[Vec<LaneEntry>],
    lane_claims: &LaneClaimContext<'_>,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let defer = lane_claims.defers_egress(i);
        let s_cur_b = &slices.cur_b;
        let s_plan_attach_n = &slices.planned_attach_n;
        let s_plan_attach_lane = &slices.planned_attach_lane;
        let s_plan_attach_lane_d = &slices.planned_attach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;

        let b_id = *s_cur_b.get(i);
        let lost_building = b_id == usize::MAX
            || b_id >= allocator.buildings.len()
            || allocator.buildings[b_id].edge_idx >= graph.edge_count()
            || graph.edge(allocator.buildings[b_id].edge_idx).deleted;
        let plan_valid = !lost_building
            && (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
            && b_id < allocator.entrances.len();
        // A lost building or plan is rare: the serial part handles it.
        if defer && !plan_valid {
            return true;
        }
        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, usize::MAX, pos, 0.0, slices);
            return false;
        }
        let b = &allocator.buildings[b_id];
        if b.edge_idx >= graph.edge_count() || graph.edge(b.edge_idx).deleted {
            // A truck backing out of a dock stays in the yard, whose road went (`PARK-20`).
            if pulls_out_of_a_dock(i, slices) {
                stand_in_yard(i, slices);
                return false;
            }
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, b_id, pos, sim_time + BUILDING_REPLAN_DELAY_S, slices);
            return false;
        }
        if !plan_valid {
            // A road edit dropped a pull-out's plan (`clear_route_plan_after_road_edit`): the
            // truck stops on its route out of the dock and plans again (`PARK-20`), where a
            // car would go back inside its building.
            if pulls_out_of_a_dock(i, slices) {
                stand_in_yard(i, slices);
                return false;
            }
            let origin_door = allocator
                .entrances
                .get(b_id)
                .map(|entrance| entrance.door_pos)
                .unwrap_or(Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i)));
            reset_invalid_access_plan(
                i,
                b_id,
                origin_door,
                sim_time + BUILDING_REPLAN_DELAY_S,
                slices,
            );
            return false;
        }
        let trip = *slices.parking_trip.get(i);
        // Off a parking leg there is no endpoint to look up: most agents here, every tick.
        let parking_entrance = slices
            .parking
            .filter(|_| trip.phase != ParkingTripPhase::None)
            .and_then(|p| p.trip_entrance(trip, true, allocator, transit_network, graph));
        // A truck leaves a building by its kerb stop, never its door (`PARK-36`): it stands
        // there at its dispatch or after its delivery, and pulls out.
        let freight = *slices.freight_shipment_id.get(i) != u64::MAX;
        let kerb_entrance = (freight && trip.phase == ParkingTripPhase::None)
            .then(|| kerb_stop_entrance(&allocator.entrances[b_id], graph, FREIGHT_KERB_INSET_M))
            .flatten();
        let entrance = parking_entrance
            .as_ref()
            .or(kerb_entrance.as_ref())
            .unwrap_or(&allocator.entrances[b_id]);
        let attach_lane_id = *s_plan_attach_lane.get(i) as usize;
        let attach_lane_d = *s_plan_attach_lane_d.get(i);
        let legal_attach = planned_attach_is_legal(
            *s_tmode.get(i),
            entrance,
            attach_lane_id,
            attach_lane_d,
            *s_plan_attach_n.get(i),
            transit_network,
            graph,
        );
        // The walked polyline, borrowed from whichever path was built: a bay's or a dock's
        // manoeuvre (`PARK-21`: a dock's way, on the stack) or the door's access path.
        let maneuver;
        let access;
        // A drop-off leaving its kerb stop (`PARK-34`) takes the door's access path back out.
        let kerb_stop = trip.drop_off && trip.source.is_none();
        let exact_path = if trip.phase == crate::simulation::parking::ParkingTripPhase::PullOut && !kerb_stop {
            maneuver = slices.parking.and_then(|p| p.maneuver_path(trip.source, attach_lane_id, attach_lane_d, transit_network, false, trip.freight_gate));
            maneuver.as_ref().map(ManeuverPath::as_slice)
        } else if legal_attach {
            access = local_access_path(
                *s_tmode.get(i),
                entrance,
                attach_lane_id,
                attach_lane_d,
                transit_network,
                graph,
                false,
            );
            access.as_ref().map(LocalAccessPath::as_slice)
        } else {
            None
        };

        if let (true, Some(path), Some(origin_node)) = (
            legal_attach,
            exact_path,
            lane_origin_node(attach_lane_id, transit_network, graph),
        ) {
            // A far-side exit not yet under way (`PARK-32`): it crosses the road from the access
            // mouth, the polyline's last point but one, only into a gap.
            let may_cross = *s_tmode.get(i) == MODE_CAR
                && *slices.crossing_lane.get(i) == u32::MAX
                && path.len() >= 2;
            // What the step may overwrite, restored when it is left to the serial part.
            let speed_before = *s_speed.get(i);
            // A walker from a car may start at its door, off the polyline (`PARK-35`); a truck
            // leaving its place in the line at a kerb stop, or the gate for that line, beside it
            // (`PARK-36`).
            let (current, mut next_pos, mut reached_handoff) = step_along_access_path(
                i,
                path,
                delta,
                trip.phase == ParkingTripPhase::WalkToDoor
                    || (freight
                        && matches!(trip.phase, ParkingTripPhase::None | ParkingTripPhase::ToKerb)),
                slices,
            );
            // The leaving line (`PARK-32`, `parking/exit_line.rs`): it gets no nearer its
            // polyline's end than its hold, a body and 1 m behind the vehicle ahead through the
            // same mouth, or its whole way while it waits for its turn to leave its place.
            if trip.phase == ParkingTripPhase::PullOut
                && !kerb_stop
                && let Some(hold) = exit_hold(slices.exit_holds, i)
                && hold > 0.0
                && left_along(path, next_pos) < hold
            {
                next_pos = if left_along(path, current) <= hold {
                    current
                } else {
                    point_back_along(path, hold)
                };
                reached_handoff = false;
                *s_pos_x.get_mut(i) = next_pos.x;
                *s_pos_y.get_mut(i) = next_pos.y;
                *s_speed.get_mut(i) = 0.0;
            }
            // Past the mouth this step: onto the road's last leg, or at its lane point.
            let on_road = may_cross
                && (reached_handoff
                    || (local_access_target_segment(next_pos, path) == Some(path.len() - 1)
                        && next_pos.distance_to(path[path.len() - 2]) > 0.05));
            let crossing = on_road
                .then(|| crossing_at(attach_lane_id, attach_lane_d, entrance.side, transit_network))
                .flatten();
            if defer
                && lane_step(
                    crossing,
                    attach_lane_id,
                    attach_lane_d,
                    reached_handoff,
                    lane_half_length(MODE_CAR, *slices.vehicle_type.get(i)),
                    *slices.crossing_wait.get(i),
                    delta,
                    transit_network,
                    graph,
                    lane_buckets,
                )
            {
                *s_pos_x.get_mut(i) = current.x;
                *s_pos_y.get_mut(i) = current.y;
                *s_speed.get_mut(i) = speed_before;
                return true;
            }
            if let Some((crossed, crossed_d)) = crossing {
                let half = lane_half_length(MODE_CAR, *slices.vehicle_type.get(i));
                // A crossing test runs in the serial part, after the parallel part wrote every
                // lane car's speed: the margin takes the speed of the body behind the crossing,
                // so a standing or crawling queue that leaves the slot free lets it in, where a
                // body at the lane's speed limit would not. In the parallel part the step is here
                // only when the test fails on the buckets alone (`lane_step`), and reads no speed.
                let speed = |agent: usize| Some(*s_speed.get(agent));
                let clear = |lane, d| {
                    crossing_clear(lane, d, half, transit_network, graph, lane_buckets, speed)
                };
                // Holding its place in its target lane (`TRAFFIC-45`), its stop line standing
                // there since the last tick, it needs the slot free with the bodies behind held.
                let waited = *slices.crossing_wait.get(i);
                let placed = waited >= CROSSING_GIVE_UP_S && *slices.lane_d.get(i) >= 0.0;
                let hold = |lag: f32| {
                    lane_buckets.get(attach_lane_id).map(|bucket| {
                        join_hold(bucket, attach_lane_d, half, i, |agent| *s_speed.get(agent), lag)
                    })
                };
                let target_open = || {
                    if placed {
                        hold(0.0).is_some_and(|(_, ready)| ready)
                    } else {
                        clear(attach_lane_id, attach_lane_d)
                    }
                };
                if !defer
                    && clear(crossed, crossed_d)
                    && target_open()
                    && claim_lane_entry(i, crossed, lane_claims)
                    && claim_lane_entry(i, attach_lane_id, lane_claims)
                {
                    // Crossing: it holds the crossed lane at the crossing's distance, and a
                    // holder keeps its stop line in its target lane (its wait stays).
                    *slices.crossing_lane.get_mut(i) = crossed as u32;
                    if !placed {
                        *slices.crossing_wait.get_mut(i) = 0.0;
                    }
                    *slices.lane_d.get_mut(i) = crossed_d;
                } else {
                    // Waiting at the mouth, off the road, for a gap, counted. A resident's car
                    // gives up after `CROSSING_GIVE_UP_S`: it leaves on the near lane, the one it
                    // would cross, and plans on from that lane's end; tried once, as the wait
                    // reaches it (no plan: it waits on). Any other then holds its place in its
                    // target lane, its stop line in `lane_distance` (`TRAFFIC-45`), while the
                    // lane it crosses is clear: a far lane held for a busy near one would only
                    // stand; meanwhile `lane_distance` is below zero, no hold.
                    *slices.crossing_wait.get_mut(i) += delta;
                    if waited < CROSSING_GIVE_UP_S
                        && *slices.crossing_wait.get(i) >= CROSSING_GIVE_UP_S
                        && leave_on_the_near_lane(
                            i, crossed, entrance, allocator, transit_network, graph,
                            pathfind_count, slices,
                        )
                    {
                        *slices.crossing_wait.get_mut(i) = 0.0;
                    } else if *slices.crossing_wait.get(i) >= CROSSING_GIVE_UP_S {
                        let lag = if placed { 0.0 } else { delta };
                        *slices.lane_d.get_mut(i) = match hold(lag) {
                            Some((stop, _)) if clear(crossed, crossed_d) => stop,
                            _ => -1.0,
                        };
                    }
                    let mouth = path[path.len() - 2];
                    *s_pos_x.get_mut(i) = mouth.x;
                    *s_pos_y.get_mut(i) = mouth.y;
                    *s_speed.get_mut(i) = 0.0;
                    return false;
                }
            }
            if crate::debug::Channel::Traffic.enabled()
                && local_access_should_log_step(current, next_pos, path, reached_handoff)
            {
                let seg_before = local_access_target_segment(current, path);
                let seg_after = local_access_target_segment(next_pos, path);
                traffic_log!(
                    "[ACCESS_EGRESS_STEP] agent={} bldg={} mode={} lane={}({}) lane_d={:.2} seg_before={:?} seg_after={:?} reached_handoff={} current=({:.2},{:.2}) next=({:.2},{:.2}) p0=({:.2},{:.2}) p1=({:.2},{:.2}) p2=({:.2},{:.2}) p3=({:.2},{:.2}) count={}",
                    i,
                    b_id,
                    transit_mode_label(*s_tmode.get(i)),
                    attach_lane_id,
                    local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                    attach_lane_d,
                    seg_before,
                    seg_after,
                    reached_handoff,
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
            if reached_handoff {
                // A car or truck that waited `CROSSING_GIVE_UP_S` to join holds its place in its
                // lane (`TRAFFIC-45`): it enters once its slot is free ahead with the bodies
                // behind held, and meanwhile its stop line stays in `lane_distance` (at the line
                // itself while it holds a crossed lane there).
                let half = lane_half_length(*s_tmode.get(i), *slices.vehicle_type.get(i));
                let holding = *slices.crossing_wait.get(i) >= CROSSING_GIVE_UP_S;
                let hold = |lag: f32| {
                    lane_buckets.get(attach_lane_id).map(|bucket| {
                        join_hold(bucket, attach_lane_d, half, i, |agent| *s_speed.get(agent), lag)
                    })
                };
                let attach_allowed = if *s_tmode.get(i) == MODE_CAR {
                    let open = if holding {
                        hold(0.0).is_some_and(|(_, ready)| ready)
                    } else {
                        lane_buckets
                            .get(attach_lane_id)
                            .is_some_and(|bucket| lane_attach_slot_clear(bucket, attach_lane_d, half))
                    };
                    open && claim_lane_entry(i, attach_lane_id, lane_claims)
                } else {
                    true
                };
                // Counts the wait to join, and takes or moves up the hold once it is due.
                let wait_to_join = || {
                    let lag = if holding { 0.0 } else { delta };
                    *slices.crossing_wait.get_mut(i) += delta;
                    if *slices.crossing_wait.get(i) >= CROSSING_GIVE_UP_S
                        && *slices.crossing_lane.get(i) == u32::MAX
                        && let Some((stop, _)) = hold(lag)
                    {
                        *slices.lane_d.get_mut(i) = stop;
                    }
                };
                if trip.phase == crate::simulation::parking::ParkingTripPhase::PullOut {
                    if trip.freight_gate {
                        // A truck out of its dock at the gate's lane point: its yard trip ends
                        // here, which frees the dock (`PARK-20`), and the plan's own end, a gate
                        // or a door or a border, is all it keeps.
                        *slices.parking_trip.get_mut(i) = trip.after_freight_pull_out();
                    } else if trip.drop_off {
                        // A drop-off car out of its stop is a car bound for its border, as an
                        // export carrier is (`PARK-34`): only its flag is left.
                        *slices.parking_trip.get_mut(i) = crate::simulation::parking::ParkingTrip {
                            drop_off: true,
                            ..Default::default()
                        };
                    } else {
                        slices.parking_trip.get_mut(i).phase = crate::simulation::parking::ParkingTripPhase::Drive;
                    }
                    if !attach_allowed {
                        *s_pos_x.get_mut(i) = path[path.len() - 1].x;
                        *s_pos_y.get_mut(i) = path[path.len() - 1].y;
                        *s_cur_b.get_mut(i) = usize::MAX;
                        hold_for_lane_attach!(slices, i, transit_network, attach_lane_id, attach_lane_d, origin_node);
                        // Held off the lane, it counts on (`try_pending_lane_attach`); a holder
                        // keeps its stop line.
                        wait_to_join();
                        return false;
                    }
                }
                if !attach_allowed {
                    if crate::debug::Channel::Traffic.enabled() {
                        traffic_log!(
                            "[ACCESS_EGRESS_WAIT] agent={} bldg={} lane={}({}) lane_d={:.2} pos=({:.2},{:.2}) reason=attach-slot-busy",
                            i,
                            b_id,
                            attach_lane_id,
                            local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                            attach_lane_d,
                            path[path.len() - 1].x,
                            path[path.len() - 1].y,
                        );
                    }
                    *s_pos_x.get_mut(i) = path[path.len() - 1].x;
                    *s_pos_y.get_mut(i) = path[path.len() - 1].y;
                    *s_speed.get_mut(i) = 0.0;
                    wait_to_join();
                    return false;
                }
                let parent_edge = transit_network.lane_system.lanes[attach_lane_id].edge_id;
                *s_pos_x.get_mut(i) = path[path.len() - 1].x;
                *s_pos_y.get_mut(i) = path[path.len() - 1].y;
                *s_cur_b.get_mut(i) = usize::MAX;
                enter_lane!(
                    slices,
                    i,
                    transit_network,
                    attach_lane_id,
                    attach_lane_d,
                    origin_node
                );
                *slices.crossing_wait.get_mut(i) = 0.0;
                *s_speed.get_mut(i) = if *s_tmode.get(i) == MODE_CAR {
                    graph
                        .edge(parent_edge)
                        .speed_limit
                        .min(AGENT_DRIVEWAY_SPEED_MS)
                } else {
                    0.0
                };
                if crate::debug::Channel::Traffic.enabled() {
                    traffic_log!(
                        "[ACCESS_EGRESS_ATTACH] agent={} bldg={} lane={}({}) lane_d={:.2} origin_node={} edge={} pos=({:.2},{:.2})",
                        i,
                        b_id,
                        attach_lane_id,
                        local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                        attach_lane_d,
                        origin_node,
                        parent_edge,
                        path[path.len() - 1].x,
                        path[path.len() - 1].y,
                    );
                }
            }
        } else {
            if defer {
                return true;
            }
            if crate::debug::Channel::Traffic.enabled() {
                traffic_log!(
                    "[ACCESS_EGRESS_ABORT] agent={} bldg={} mode={} lane={} lane_d={:.2} legal_attach={} exact_path={} reason=invalid-egress-plan flags=0x{:02x}",
                    i,
                    b_id,
                    transit_mode_label(*s_tmode.get(i)),
                    attach_lane_id,
                    attach_lane_d,
                    legal_attach,
                    exact_path.is_some(),
                    *s_access_flags.get(i),
                );
            }
            let cause = FailureCause::StaleEntry {
                lane: attach_lane_id,
                lane_d: attach_lane_d,
            };
            if pulls_out_of_a_dock(i, slices) {
                // Its way onto the road is stale: it stops in the yard, and the parking phase's
                // plan from the dock completes this record (`PARK-20`, `TRAFFIC-32`).
                record_failure(i, cause, FailureOutcome::ParkingRecovery, slices);
                stand_in_yard(i, slices);
                return false;
            }
            record_failure(
                i,
                cause,
                FailureOutcome::RetryDeparture {
                    after_s: BUILDING_REPLAN_DELAY_S,
                },
                slices,
            );
            let origin_door = entrance.door_pos;
            reset_invalid_access_plan(
                i,
                b_id,
                origin_door,
                sim_time + BUILDING_REPLAN_DELAY_S,
                slices,
            );
        }
        false
    }
}

/// Whether a car's egress step, ending at the mouth or past it (`crossing`, the lane it would
/// cross there) or at its lane point (`reached_handoff`), may test or claim a lane, give up a
/// crossing, or take or move a join hold (`TRAFFIC-45`), this tick: the steps the serial part
/// runs (`PERF-33`). `half` is the car's half length, `waited` its crossing wait before the
/// step: every step that ends with the wait at `CROSSING_GIVE_UP_S` or more is serial. A
/// crossing whose test fails on the buckets alone fails whatever the speeds the serial test
/// reads: the margin behind is a body's speed, never below zero, times a positive time, so the
/// margin at zero speed is the least it can be ([`crossing_clear`]). O(log bucket) for a car at
/// a crossing, O(1) otherwise.
#[allow(clippy::too_many_arguments)]
fn lane_step(
    crossing: Option<(usize, f32)>,
    attach_lane: usize,
    attach_d: f32,
    reached_handoff: bool,
    half: f32,
    waited: f32,
    delta: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
    lane_buckets: &[Vec<LaneEntry>],
) -> bool {
    let Some((crossed, crossed_d)) = crossing else {
        return reached_handoff;
    };
    let open = |lane, d| crossing_clear(lane, d, half, network, graph, lane_buckets, |_| Some(0.0));
    waited + delta >= CROSSING_GIVE_UP_S
        || (open(crossed, crossed_d) && open(attach_lane, attach_d))
}

/// A resident's car that waited [`CROSSING_GIVE_UP_S`] at its access mouth to cross the road
/// (`PARK-32`) leaves on the near lane instead, lane `near`, the one it would have crossed, at
/// its access's lane point there, with a plan from that lane's end to its trip's destination. Its leg then
/// has no crossing: it attaches by the slot test as a same-side exit does. A truck or a drop-off
/// keeps waiting (its plan ends at a gate, a door or a border). Whether the plan was made;
/// without one it waits on: the caller tries once a leg. O(one plan).
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
unsafe fn leave_on_the_near_lane(
    i: usize,
    near: usize,
    exit: &BuildingEntrance,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let trip = *slices.parking_trip.get(i);
        if trip.phase != ParkingTripPhase::PullOut || trip.freight_gate || trip.drop_off {
            return false;
        }
        let (Some(node), Some(near_d)) = (
            lane_terminal_node(near, transit_network, graph),
            projected_lane_distance_for_entrance(exit, near, transit_network, graph),
        ) else {
            return false;
        };
        let Some(mut access) = slices
            .parking
            .and_then(|p| p.replan_entrance(trip, allocator, transit_network, graph))
        else {
            return false;
        };
        BuildingAllocator::refresh_entrance_lanes(&mut access, graph, &transit_network.lane_system);
        let first_lane = (!lane_hosts_turn_preparation(near, transit_network)).then_some(near);
        let Ok(plan) = plan_network_replan(
            node,
            transit_network.lane_system.lanes[near].edge_id,
            None,
            *slices.tgt_b.get(i),
            MODE_CAR,
            *slices.access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "leaving-give-up")),
            first_lane,
            Some(access),
        ) else {
            return false;
        };
        apply_network_replan(i, plan, slices);
        *slices.planned_attach_lane.get_mut(i) = near as u32;
        *slices.planned_attach_lane_d.get_mut(i) = near_d;
        // An attach plan names the lane's end node (`planned_attach_is_legal`).
        *slices.planned_attach_n.get_mut(i) = node;
        true
    }
}
