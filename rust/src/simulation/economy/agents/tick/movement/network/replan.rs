// SPDX-License-Identifier: GPL-2.0-only

//! Pre-movement network replan gates for immigration and exact access trips.

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_LANE_ATTACH_PENDING, ACCESS_PLAN_VALID,
    ACCESS_WALK_STRANDED, MODE_CAR, MODE_WALK, TRANSIT_IMMIGRATING, TRANSIT_NETWORK,
};
use super::super::super::access::planned_detach_is_legal;
use super::super::super::lane_nav::{
    lane_hosts_turn_preparation, lane_terminal_node, road_lane_ahead,
};
use super::super::super::planning::{
    BuiltNetworkReplan, BuiltTripPlan, plan_border_arrival, plan_border_network_replan,
    plan_immigration_trip, plan_network_replan,
};
use super::super::super::slices::MovementSlices;
use super::super::super::trip_failures::FailureCause;
use super::super::NETWORK_REPLAN_DELAY_S;
use super::super::replan_watchdog::{
    delay_or_recover_after_network_replan_failure, reset_network_replan_watchdog,
};
use super::walk_home::{give_up_trip, retry_home, retry_home_without_lane};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use std::sync::atomic::AtomicU32;

/// Runs the pre-movement replan gates and returns whether the agent may keep moving this tick.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
pub(super) unsafe fn prepare_network_replan(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        bootstrap_immigration_trip(
            i,
            sim_time,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            slices,
        ) && ensure_exact_access_plan(
            i,
            sim_time,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            slices,
        ) && repair_stale_detach_plan(
            i,
            sim_time,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            slices,
        )
    }
}

// Compiled in its caller's codegen unit, as the movement pass's steps (see
// `handle_network_movement`).
#[inline]
unsafe fn bootstrap_immigration_trip(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;

        if *s_transit.get(i) != TRANSIT_IMMIGRATING
            || (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
        {
            return true;
        }

        if sim_time < *s_next_replan_time.get(i) {
            *s_speed.get_mut(i) = 0.0;
            return false;
        }

        let border_node = *s_cur_n.get(i);
        let home_bldg = *s_tgt_b.get(i);
        let plan = if *slices.pending_household_size.get(i) > 0
            || (slices.parking.is_some() && *slices.freight_shipment_id.get(i) == u64::MAX)
        {
            allocator.entrances.get(home_bldg).and_then(|entrance|
                plan_border_arrival(border_node, home_bldg,
                    crate::simulation::economy::agents::MODE_WALK, entrance,
                    transit_network, graph, pathfind_count))
        } else {
            plan_immigration_trip(border_node, home_bldg, allocator, transit_network, graph, pathfind_count)
        };
        if let Some(plan) = plan {
            apply_immigration_trip(i, border_node, plan, slices);
            true
        } else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                "immigration-bootstrap",
                FailureCause::NoImmigrationRoute {
                    border: border_node,
                    building: home_bldg,
                },
                slices,
            );
            false
        }
    }
}

// Compiled in its caller's codegen unit, as the movement pass's steps (see
// `handle_network_movement`).
#[inline]
unsafe fn ensure_exact_access_plan(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;

        let target_building = *s_tgt_b.get(i);
        if *s_transit.get(i) == TRANSIT_IMMIGRATING
            || (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
            || target_building == usize::MAX
        {
            return true;
        }

        if sim_time < *s_next_replan_time.get(i) {
            *s_speed.get_mut(i) = 0.0;
            return false;
        }

        let Some((start_node, incoming_edge)) = replan_start(i, transit_network, graph, slices)
        else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                "missing-replan-start",
                FailureCause::NoPosition,
                slices,
            );
            return false;
        };

        match plan_network_replan(
            start_node,
            incoming_edge,
            (*s_tmode.get(i) == MODE_WALK).then_some(*slices.lane_id.get(i)),
            target_building,
            *s_tmode.get(i),
            0,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "exact-access-replan")),
            None,
            slices.parking.and_then(|p| p.replan_entrance(*slices.parking_trip.get(i), allocator, transit_network, graph)),
        ) {
            Ok(replan) => {
                apply_network_replan(i, replan, slices);
                true
            }
            Err(failure) if walker_on_road_lane(i, start_node, transit_network, graph, slices) => {
                give_up_trip(
                    i,
                    start_node,
                    *slices.lane_id.get(i),
                    FailureCause::NoRoute(failure),
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
                true
            }
            Err(failure) => {
                delay_or_recover_after_network_replan_failure(
                    i,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    "exact-access-replan",
                    FailureCause::NoRoute(failure),
                    slices,
                );
                false
            }
        }
    }
}

/// Replans car `i` once after it followed a missed turn (`follow_missed_turn`), which leaves it
/// with a valid plan and no node path on the connector or road lane that turn entered. It plans
/// from that road's end, so it chooses the road's next lane early, and keeps driving: a
/// destination car through `plan_network_replan` with its access flags kept, a freight car
/// through `plan_border_network_replan`. Called by the lane-end handler after an entry, so only a
/// car that entered with an empty path pays the checks: a zero-hop trip (its detach node at that
/// road's end, or its detach lane on that road) is left alone. A failed replan sets the replan
/// backoff and changes nothing else; the car drives on, and the empty-path replan at the next lane
/// end stays the last resort. Cost: O(1) checks, one route query on a missed turn. Out of line:
/// it runs only at lane ends.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline(never)]
pub(super) unsafe fn replan_after_missed_turn(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let flags = *slices.access_flags.get(i);
        if *slices.tmode.get(i) != MODE_CAR
            || flags & ACCESS_PLAN_VALID == 0
            || !slices.path.get(i).is_empty()
            || sim_time < *slices.next_replan_time.get(i)
        {
            return;
        }
        let Some(lane_id) = road_lane_ahead(*slices.lane_id.get(i), transit_network) else {
            return;
        };
        let lane = &transit_network.lane_system.lanes[lane_id];
        let Some(node) = lane_terminal_node(lane_id, transit_network, graph) else {
            return;
        };
        // On a road too short to change lanes the route must leave the node on a road this
        // lane reaches; a longer road lets the preparation choose the lane (`TRAFFIC-23`, S3).
        let first_lane =
            (!lane_hosts_turn_preparation(lane_id, transit_network)).then_some(lane_id);
        let target_building = *slices.tgt_b.get(i);
        let replanned = if target_building == usize::MAX {
            let border = *slices.freight_target_border_node.get(i);
            if flags & ACCESS_FREIGHT_BORDER_DESTINATION == 0 || node == border {
                return;
            }
            plan_border_network_replan(
                node,
                lane.edge_id,
                border,
                graph,
                transit_network,
                pathfind_count,
                first_lane,
            )
            .map(|plan| apply_network_replan(i, plan, slices))
            .is_some()
        } else {
            if node == *slices.planned_detach_n.get(i)
                || transit_network
                    .lane_system
                    .lanes
                    .get(*slices.planned_detach_lane.get(i) as usize)
                    .is_some_and(|detach| detach.edge_id == lane.edge_id)
            {
                return;
            }
            plan_network_replan(
                node,
                lane.edge_id,
                None,
                target_building,
                MODE_CAR,
                flags,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                Some((i, "missed-turn-replan")),
                first_lane,
                slices.parking.and_then(|p| p.replan_entrance(*slices.parking_trip.get(i), allocator, transit_network, graph)),
            )
            .map(|plan| apply_network_replan(i, plan, slices))
            .is_ok()
        };
        if !replanned {
            *slices.next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
            traffic_log!(
                "[MISSED_TURN_REPLAN_FAILED] agent={} node={} lane={} target_bldg={} retry_after_s={:.1}",
                i,
                node,
                lane_id,
                target_building,
                NETWORK_REPLAN_DELAY_S,
            );
        }
    }
}

// Compiled in its caller's codegen unit, as the movement pass's steps (see
// `handle_network_movement`).
#[inline]
unsafe fn repair_stale_detach_plan(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_tgt_b = &slices.tgt_b;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;

        if (*s_access_flags.get(i) & ACCESS_PLAN_VALID) == 0 {
            return true;
        }
        if (*s_access_flags.get(i) & ACCESS_FREIGHT_BORDER_DESTINATION) != 0 {
            return true;
        }
        // A stranded walker has no exit lane: it plans home on its backoff from its road lane,
        // wherever it waits on it (`ROAD-74`), or, with no lane, from where it stands
        // (`TRAFFIC-19`).
        if (*s_access_flags.get(i) & ACCESS_WALK_STRANDED) != 0 {
            let lane = *slices.lane_id.get(i);
            if lane == usize::MAX && *s_tmode.get(i) == MODE_WALK {
                retry_home_without_lane(
                    i,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            } else if let Some(node) = lane_terminal_node(lane, transit_network, graph)
                && walker_on_road_lane(i, node, transit_network, graph, slices)
            {
                retry_home(
                    i,
                    node,
                    lane,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
            return true;
        }

        let target_building = *s_tgt_b.get(i);
        let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        let trip = *slices.parking_trip.get(i);
        let parking_entrance = slices.parking.and_then(|p| p.trip_entrance(trip, false, allocator, transit_network, graph));
        let target_entrance = parking_entrance.as_ref().or_else(|| allocator.entrances.get(target_building));
        let detach_still_legal = target_entrance.is_some()
            && planned_detach_lane_id != usize::MAX
            && planned_detach_is_legal(
                *s_tmode.get(i),
                target_entrance.expect("validated destination"),
                planned_detach_lane_id,
                *s_plan_detach_lane_d.get(i),
                *s_plan_detach_n.get(i),
                transit_network,
                graph,
            );
        if detach_still_legal {
            return true;
        }

        if sim_time < *s_next_replan_time.get(i) {
            *s_speed.get_mut(i) = 0.0;
            return false;
        }

        let lane_d = *s_plan_detach_lane_d.get(i);
        let stale_exit = |replan| FailureCause::StaleExit {
            lane: planned_detach_lane_id,
            lane_d,
            replan,
        };
        let Some((start_node, incoming_edge)) = replan_start(i, transit_network, graph, slices)
        else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                "missing-stale-detach-replan-start",
                stale_exit(None),
                slices,
            );
            return false;
        };

        match plan_network_replan(
            start_node,
            incoming_edge,
            (*s_tmode.get(i) == MODE_WALK).then_some(*slices.lane_id.get(i)),
            target_building,
            *s_tmode.get(i),
            *s_access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "stale-detach-replan")),
            None,
            slices.parking.and_then(|p| p.replan_entrance(*slices.parking_trip.get(i), allocator, transit_network, graph)),
        ) {
            Ok(replan) => {
                apply_network_replan(i, replan, slices);
                true
            }
            Err(failure) if walker_on_road_lane(i, start_node, transit_network, graph, slices) => {
                give_up_trip(
                    i,
                    start_node,
                    *slices.lane_id.get(i),
                    FailureCause::NoRoute(failure),
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
                true
            }
            Err(failure) => {
                delay_or_recover_after_network_replan_failure(
                    i,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    "stale-detach-replan",
                    stale_exit(Some(failure)),
                    slices,
                );
                false
            }
        }
    }
}

// Whether walker `i` stands on a road foot lane ending at `start_node`: a failed replan from
// there gives its trip up for a walk home (`ROAD-74`) instead of the watchdog's recovery.
unsafe fn walker_on_road_lane(
    i: usize,
    start_node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let lane = *slices.lane_id.get(i);
        *slices.tmode.get(i) == MODE_WALK
            && transit_network
                .lane_system
                .lanes
                .get(lane)
                .is_some_and(|value| value.edge_id != usize::MAX)
            && lane_terminal_node(lane, transit_network, graph) == Some(start_node)
    }
}

unsafe fn replan_start(
    i: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Option<(u32, usize)> {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_cur_e = &slices.cur_e;
        let s_lane_id = &slices.lane_id;

        let current_lane_id = *s_lane_id.get(i);
        let current_node = *s_cur_n.get(i);
        let current_edge = *s_cur_e.get(i);
        // A car on a connector has one way out, so its route starts past the road that connector
        // leads onto. From the junction itself a route can leave by any road (`TRAFFIC-30`).
        let road_lane = if *slices.tmode.get(i) == MODE_CAR {
            road_lane_ahead(current_lane_id, transit_network)
        } else {
            Some(current_lane_id)
        };
        if let Some(lane) = road_lane
            && let Some(start_node) = lane_terminal_node(lane, transit_network, graph)
        {
            return Some((start_node, transit_network.lane_system.lanes[lane].edge_id));
        }
        (current_node != u32::MAX).then_some((current_node, current_edge))
    }
}

unsafe fn apply_immigration_trip(
    i: usize,
    border_node: u32,
    plan: BuiltTripPlan,
    slices: &MovementSlices,
) {
    unsafe {
        let s_cur_n = &slices.cur_n;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_attach_n = &slices.planned_attach_n;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_attach_lane = &slices.planned_attach_lane;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_attach_lane_d = &slices.planned_attach_lane_d;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_cur_e = &slices.cur_e;

        *s_tmode.get_mut(i) = plan.mode;
        *s_tgt_b.get_mut(i) = plan.target_building;
        *s_plan_attach_n.get_mut(i) = plan.planned_attach_node;
        *s_plan_detach_n.get_mut(i) = plan.planned_detach_node;
        *s_plan_attach_lane.get_mut(i) = u32::MAX;
        *s_plan_detach_lane.get_mut(i) = plan.planned_detach_lane_id as u32;
        *s_plan_attach_lane_d.get_mut(i) = 0.0;
        *s_plan_detach_lane_d.get_mut(i) = plan.planned_detach_lane_d;
        *s_access_flags.get_mut(i) = plan.access_flags;
        *s_next_replan_time.get_mut(i) = 0.0;
        reset_network_replan_watchdog(i, slices);
        *s_cur_n.get_mut(i) = border_node;
        *s_cur_e.get_mut(i) = usize::MAX;
        *s_lane_id.get_mut(i) = usize::MAX;
        *s_lane_d.get_mut(i) = 0.0;
        *s_speed.get_mut(i) = 0.0;
        slices.parking_trip.get_mut(i).route_replaced(plan.current_path.len());
        *s_path.get_mut(i) = plan.current_path;
        slices.walk_lanes.get_mut(i).clear();
        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
        *s_transit.get_mut(i) = TRANSIT_NETWORK;
    }
}

/// Installs a replacement route and destination handoff, then clears retry/watchdog state.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline]
pub(in crate::simulation::economy::agents::tick::movement) unsafe fn apply_network_replan(
    i: usize,
    replan: BuiltNetworkReplan,
    slices: &MovementSlices,
) {
    unsafe {
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;

        slices.parking_trip.get_mut(i).route_replaced(replan.current_path.len());
        *s_path.get_mut(i) = replan.current_path;
        slices.walk_lanes.get_mut(i).clear();
        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
        *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
        *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
        *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
        // A car waiting to attach keeps waiting at its attach point under the new route.
        let hold = *s_access_flags.get(i) & ACCESS_LANE_ATTACH_PENDING;
        *s_access_flags.get_mut(i) = replan.access_flags | hold;
        *s_next_replan_time.get_mut(i) = 0.0;
        reset_network_replan_watchdog(i, slices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::data::AgentSystem;
    use crate::simulation::network::graph::RegionGraph;
    use crate::simulation::network::lanes::Lane;
    use crate::simulation::network::types::NodeType;
    use godot::prelude::Vector3;

    #[test]
    fn replan_start_falls_back_to_current_node_for_connector_lane() {
        let mut graph = RegionGraph::new();
        let node = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let mut transit_network = TransitNetwork::new();
        transit_network.lane_system.lanes.push(Lane {
            edge_id: usize::MAX,
            node_id: node as usize,
            ..Lane::default()
        });

        let mut agents = AgentSystem::new();
        let agent_idx = agents.spawn_border_arrival_agent(usize::MAX, node, 0.0, 0.0);
        agents.agents.current_node[agent_idx] = node;
        agents.agents.current_edge[agent_idx] = usize::MAX;
        agents.agents.current_lane_id[agent_idx] = 0;

        let slices = MovementSlices::new(&mut agents.agents);
        let start = unsafe { replan_start(agent_idx, &transit_network, &graph, &slices) };

        assert_eq!(start, Some((node, usize::MAX)));
    }

    // `TRAFFIC-30`: a car whose plan is cleared on a connector replans past the road the
    // connector leads onto. From the junction its route could leave by another road, and the car
    // would drive this one to a far end that route never names.
    #[test]
    fn replan_start_takes_a_car_on_a_connector_to_the_end_of_its_road() {
        use crate::simulation::network::roadbed::fixtures::{connect, point};
        let mut graph = RegionGraph::new();
        let junction = graph.add_node(point(0.0, 0.0, false), NodeType::Junction);
        let far = graph.add_node(point(100.0, 0.0, false), NodeType::Junction);
        let edge = connect(&mut graph, junction, far, 7.0, false);
        let mut transit_network = TransitNetwork::new();
        let lanes = &mut transit_network.lane_system.lanes;
        lanes.push(Lane {
            edge_id: edge,
            is_fwd: true,
            ..Lane::default()
        });
        lanes.push(Lane {
            edge_id: usize::MAX,
            node_id: junction as usize,
            next_lanes: vec![0].into(),
            ..Lane::default()
        });

        let mut agents = AgentSystem::new();
        let agent_idx = agents.spawn_border_arrival_agent(usize::MAX, junction, 0.0, 0.0);
        agents.agents.transit_mode[agent_idx] = MODE_CAR;
        agents.agents.current_node[agent_idx] = junction;
        agents.agents.current_edge[agent_idx] = usize::MAX;
        agents.agents.current_lane_id[agent_idx] = 1;

        let slices = MovementSlices::new(&mut agents.agents);
        let start = unsafe { replan_start(agent_idx, &transit_network, &graph, &slices) };

        assert_eq!(start, Some((far, edge)));
    }
}
