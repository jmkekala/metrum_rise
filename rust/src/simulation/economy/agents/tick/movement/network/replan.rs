//! Pre-movement network replan gates for immigration and exact access trips.

use super::super::super::super::{
    ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_IMMIGRATING, TRANSIT_NETWORK,
};
use super::super::super::access::planned_detach_is_legal;
use super::super::super::lane_nav::lane_terminal_node;
use super::super::super::planning::{
    BuiltNetworkReplan, BuiltTripPlan, plan_immigration_trip, plan_network_replan,
};
use super::super::super::slices::MovementSlices;
use super::super::NETWORK_REPLAN_DELAY_S;
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
            *s_speed.get_mut(i) = 0.0;
            *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
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
            clear_path_and_delay(i, sim_time, slices);
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
        ) {
            apply_network_replan(i, replan, slices);
            true
        } else {
            clear_path_and_delay(i, sim_time, slices);
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
            clear_path_and_delay(i, sim_time, slices);
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
        ) {
            apply_network_replan(i, replan, slices);
            true
        } else {
            clear_path_and_delay(i, sim_time, slices);
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
        let lane_valid = current_lane_id != usize::MAX
            && current_lane_id < transit_network.lane_system.lanes.len();
        let start_node = if lane_valid {
            lane_terminal_node(current_lane_id, transit_network, graph)
        } else if *s_cur_n.get(i) != u32::MAX {
            Some(*s_cur_n.get(i))
        } else {
            None
        }?;
        let incoming_edge = if lane_valid {
            transit_network.lane_system.lanes[current_lane_id].edge_id
        } else {
            *s_cur_e.get(i)
        };
        Some((start_node, incoming_edge))
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
    }
}

unsafe fn clear_path_and_delay(i: usize, sim_time: f32, slices: &MovementSlices) {
    unsafe {
        let s_speed = &slices.speed;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_next_replan_time = &slices.next_replan_time;

        s_path.get_mut(i).clear();
        *s_path_idx.get_mut(i) = 0;
        *s_speed.get_mut(i) = 0.0;
        *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
    }
}
