//! Pre-movement network replan gates for immigration and exact access trips.

use super::super::super::super::{
    ACCESS_FREIGHT_BORDER_DESTINATION, ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_IMMIGRATING,
    TRANSIT_NETWORK,
};
use super::super::super::access::planned_detach_is_legal;
use super::super::super::lane_nav::lane_terminal_node;
use super::super::super::planning::{
    BuiltNetworkReplan, BuiltTripPlan, plan_immigration_trip, plan_network_replan,
};
use super::super::super::slices::MovementSlices;
use super::super::replan_watchdog::{
    delay_or_recover_after_network_replan_failure, reset_network_replan_watchdog,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::AtomicU32;

/// Runs the pre-movement replan gates and returns whether the agent may keep moving this tick.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
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
        if let Some(plan) = plan_immigration_trip(
            border_node,
            home_bldg,
            allocator,
            transit_network,
            graph,
            pathfind_count,
        ) {
            apply_immigration_trip(i, border_node, plan, slices);
            true
        } else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                graph,
                "immigration-bootstrap",
                slices,
            );
            false
        }
    }
}

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
                graph,
                "missing-replan-start",
                slices,
            );
            return false;
        };

        if let Some(replan) = plan_network_replan(
            start_node,
            incoming_edge,
            target_building,
            *s_tmode.get(i),
            0,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "exact-access-replan")),
        ) {
            apply_network_replan(i, replan, slices);
            true
        } else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                graph,
                "exact-access-replan",
                slices,
            );
            false
        }
    }
}

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

        let target_building = *s_tgt_b.get(i);
        let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        let detach_still_legal = target_building < allocator.entrances.len()
            && planned_detach_lane_id != usize::MAX
            && planned_detach_is_legal(
                *s_tmode.get(i),
                &allocator.entrances[target_building],
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

        let Some((start_node, incoming_edge)) = replan_start(i, transit_network, graph, slices)
        else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                graph,
                "missing-stale-detach-replan-start",
                slices,
            );
            return false;
        };

        if let Some(replan) = plan_network_replan(
            start_node,
            incoming_edge,
            target_building,
            *s_tmode.get(i),
            *s_access_flags.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
            Some((i, "stale-detach-replan")),
        ) {
            apply_network_replan(i, replan, slices);
            true
        } else {
            delay_or_recover_after_network_replan_failure(
                i,
                sim_time,
                allocator,
                graph,
                "stale-detach-replan",
                slices,
            );
            false
        }
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
        let lane_valid = current_lane_id != usize::MAX
            && current_lane_id < transit_network.lane_system.lanes.len();
        if lane_valid {
            let lane_edge = transit_network.lane_system.lanes[current_lane_id].edge_id;
            if lane_edge != usize::MAX
                && let Some(start_node) =
                    lane_terminal_node(current_lane_id, transit_network, graph)
            {
                return Some((start_node, lane_edge));
            }
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

        *s_tmode.get_mut(i) = MODE_CAR;
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
        *s_path.get_mut(i) = plan.current_path;
        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
        *s_transit.get_mut(i) = TRANSIT_NETWORK;
    }
}

unsafe fn apply_network_replan(i: usize, replan: BuiltNetworkReplan, slices: &MovementSlices) {
    unsafe {
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;

        *s_path.get_mut(i) = replan.current_path;
        *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
        *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
        *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
        *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
        *s_access_flags.get_mut(i) = replan.access_flags;
        *s_next_replan_time.get_mut(i) = 0.0;
        reset_network_replan_watchdog(i, slices);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::data::AgentSystem;
    use crate::simulation::economy::agents::tick::slices::{MovementSlices, RawSlice};
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
        let agent_idx =
            agents.spawn_border_arrival_agent(usize::MAX, node, 0.0, 0.0, node, 0.0, 0.0);
        agents.agents.current_node[agent_idx] = node;
        agents.agents.current_edge[agent_idx] = usize::MAX;
        agents.agents.current_lane_id[agent_idx] = 0;

        let slices = movement_slices(&mut agents);
        let start = unsafe { replan_start(agent_idx, &transit_network, &graph, &slices) };

        assert_eq!(start, Some((node, usize::MAX)));
    }

    fn movement_slices(agents: &mut AgentSystem) -> MovementSlices {
        MovementSlices {
            home: RawSlice::new(&mut agents.agents.home_building),
            work: RawSlice::new(&mut agents.agents.work_building),
            age_group: RawSlice::new(&mut agents.agents.age_group),
            pos_x: RawSlice::new(&mut agents.agents.pos_x),
            pos_y: RawSlice::new(&mut agents.agents.pos_y),
            activity: RawSlice::new(&mut agents.agents.activity),
            transit: RawSlice::new(&mut agents.agents.transit),
            happiness: RawSlice::new(&mut agents.agents.happiness),
            jstart: RawSlice::new(&mut agents.agents.journey_start_time),
            schedule_seed: RawSlice::new(&mut agents.agents.schedule_seed),
            cached_commute_minutes: RawSlice::new(&mut agents.agents.cached_commute_minutes),
            next_commute_refresh_time: RawSlice::new(&mut agents.agents.next_commute_refresh_time),
            next_departure_day: RawSlice::new(&mut agents.agents.next_departure_day),
            next_departure_minute: RawSlice::new(&mut agents.agents.next_departure_minute),
            next_departure_origin: RawSlice::new(&mut agents.agents.next_departure_origin_building),
            next_departure_target: RawSlice::new(&mut agents.agents.next_departure_target_building),
            next_departure_activity: RawSlice::new(&mut agents.agents.next_departure_activity),
            cached_schedule_work_building: RawSlice::new(
                &mut agents.agents.cached_schedule_work_building,
            ),
            cached_work_profile_index: RawSlice::new(&mut agents.agents.cached_work_profile_index),
            pending_household_size: RawSlice::new(&mut agents.agents.pending_household_size),
            freight_shipment_id: RawSlice::new(&mut agents.agents.freight_shipment_id),
            cur_b: RawSlice::new(&mut agents.agents.current_building),
            tgt_b: RawSlice::new(&mut agents.agents.target_building),
            planned_tgt_b: RawSlice::new(&mut agents.agents.planned_target_building),
            freight_target_border_node: RawSlice::new(
                &mut agents.agents.freight_target_border_node,
            ),
            cur_n: RawSlice::new(&mut agents.agents.current_node),
            planned_attach_n: RawSlice::new(&mut agents.agents.planned_attach_node),
            planned_detach_n: RawSlice::new(&mut agents.agents.planned_detach_node),
            planned_attach_lane: RawSlice::new(&mut agents.agents.planned_attach_lane_id),
            planned_detach_lane: RawSlice::new(&mut agents.agents.planned_detach_lane_id),
            planned_attach_lane_d: RawSlice::new(&mut agents.agents.planned_attach_lane_d),
            planned_detach_lane_d: RawSlice::new(&mut agents.agents.planned_detach_lane_d),
            access_flags: RawSlice::new(&mut agents.agents.access_flags),
            next_replan_time: RawSlice::new(&mut agents.agents.next_replan_time),
            network_replan_failures: RawSlice::new(&mut agents.agents.network_replan_failures),
            cur_e: RawSlice::new(&mut agents.agents.current_edge),
            lane_id: RawSlice::new(&mut agents.agents.current_lane_id),
            lane_d: RawSlice::new(&mut agents.agents.lane_distance),
            lane_change_from_lane: RawSlice::new(&mut agents.agents.lane_change_from_lane_id),
            lane_change_start_d: RawSlice::new(&mut agents.agents.lane_change_start_d),
            lane_change_length: RawSlice::new(&mut agents.agents.lane_change_length_m),
            overtake_blocked_time: RawSlice::new(&mut agents.agents.overtake_blocked_time_s),
            overtake_cooldown: RawSlice::new(&mut agents.agents.overtake_cooldown_s),
            junction_release_time: RawSlice::new(&mut agents.agents.junction_release_time_s),
            next_reroute_time: RawSlice::new(&mut agents.agents.next_reroute_time_s),
            tmode: RawSlice::new(&mut agents.agents.transit_mode),
            planned_activity: RawSlice::new(&mut agents.agents.planned_activity),
            path: RawSlice::new(&mut agents.agents.current_path),
            path_idx: RawSlice::new(&mut agents.agents.current_path_index),
            has_car: RawSlice::new(&mut agents.agents.has_car),
            speed: RawSlice::new(&mut agents.agents.speed),
            walk_phase: RawSlice::new(&mut agents.agents.walk_phase),
        }
    }
}
