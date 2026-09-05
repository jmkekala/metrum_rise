// SPDX-License-Identifier: GPL-2.0-only

//! In-building movement state handling for scheduled trip activation.

use super::super::super::TRANSIT_ACCESS_EGRESS;
use super::super::access::local_access_side_label;
use super::super::planning::plan_building_origin_trip;
use super::super::schedule::{ScheduleCacheMut, maybe_schedule_work_trip};
use super::super::slices::MovementSlices;
use super::{BUILDING_REPLAN_DELAY_S, transit_mode_label};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::age_group_can_work;
use crate::simulation::economy::definitions::{
    OperationalClockRuntimeTuning, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use std::sync::atomic::AtomicU32;

/// Handles agents waiting inside a building and starts a planned access-egress trip when ready.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn handle_in_building(
    i: usize,
    sim_time: f32,
    day_index: u32,
    minute_of_day: u16,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    operational_clock: &OperationalClockRuntimeTuning,
    economy_catalog: &RuntimeEconomyCatalog,
    slices: &MovementSlices,
) {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_activity = &slices.activity;
        let s_work = &slices.work;
        let s_home = &slices.home;
        let s_age_group = &slices.age_group;
        let s_cur_b = &slices.cur_b;
        let s_tgt_b = &slices.tgt_b;
        let s_plan_b = &slices.planned_tgt_b;
        let s_has_car = &slices.has_car;
        let s_jstart = &slices.jstart;
        let s_schedule_seed = &slices.schedule_seed;
        let s_cached_commute_minutes = &slices.cached_commute_minutes;
        let s_next_commute_refresh_time = &slices.next_commute_refresh_time;
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
        let s_network_replan_failures = &slices.network_replan_failures;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;
        let s_cur_e = &slices.cur_e;
        let s_cur_n = &slices.cur_n;
        let s_plan_act = &slices.planned_activity;

        let curr_bldg = *s_cur_b.get(i);
        if *s_plan_b.get(i) == usize::MAX
            && curr_bldg != usize::MAX
            && curr_bldg < allocator.buildings.len()
            && age_group_can_work(*s_age_group.get(i))
        {
            let mut schedule_cache = ScheduleCacheMut {
                cached_commute_minutes: s_cached_commute_minutes.get_mut(i),
                next_commute_refresh_time: s_next_commute_refresh_time.get_mut(i),
                next_departure_day: slices.next_departure_day.get_mut(i),
                next_departure_minute: slices.next_departure_minute.get_mut(i),
                next_departure_origin_building: slices.next_departure_origin.get_mut(i),
                next_departure_target_building: slices.next_departure_target.get_mut(i),
                next_departure_activity: slices.next_departure_activity.get_mut(i),
                cached_schedule_work_building: slices.cached_schedule_work_building.get_mut(i),
                cached_work_profile_index: slices.cached_work_profile_index.get_mut(i),
            };
            if let Some((target_building, activity)) = maybe_schedule_work_trip(
                curr_bldg,
                *s_home.get(i),
                *s_work.get(i),
                *s_has_car.get(i),
                *s_schedule_seed.get(i),
                &mut schedule_cache,
                sim_time,
                day_index,
                minute_of_day,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                operational_clock,
                economy_catalog,
            ) {
                *s_plan_b.get_mut(i) = target_building;
                *s_plan_act.get_mut(i) = activity;
            }
        }

        let next_bldg = *s_plan_b.get(i);
        let next_act = *s_plan_act.get(i);
        if next_bldg == usize::MAX
            || next_bldg >= allocator.buildings.len()
            || curr_bldg == usize::MAX
            || curr_bldg >= allocator.buildings.len()
        {
            // No actionable next trip.
        } else if sim_time < *s_next_replan_time.get(i) {
            // Cooldown gate blocks replanning this tick.
        } else if let Some(plan) = plan_building_origin_trip(
            curr_bldg,
            next_bldg,
            next_act,
            *s_has_car.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
        ) {
            let origin_entrance = &allocator.entrances[curr_bldg];
            *s_tgt_b.get_mut(i) = plan.target_building;
            *s_activity.get_mut(i) = plan.activity;
            *s_jstart.get_mut(i) = sim_time;
            *s_tmode.get_mut(i) = plan.mode;
            *s_plan_attach_n.get_mut(i) = plan.planned_attach_node;
            *s_plan_detach_n.get_mut(i) = plan.planned_detach_node;
            *s_plan_attach_lane.get_mut(i) = plan.planned_attach_lane_id as u32;
            *s_plan_detach_lane.get_mut(i) = plan.planned_detach_lane_id as u32;
            *s_plan_attach_lane_d.get_mut(i) = plan.planned_attach_lane_d;
            *s_plan_detach_lane_d.get_mut(i) = plan.planned_detach_lane_d;
            *s_access_flags.get_mut(i) = plan.access_flags;
            *s_next_replan_time.get_mut(i) = 0.0;
            *s_network_replan_failures.get_mut(i) = 0;
            *s_cur_n.get_mut(i) = u32::MAX;
            *s_cur_e.get_mut(i) = usize::MAX;
            *s_lane_id.get_mut(i) = usize::MAX;
            *s_lane_d.get_mut(i) = 0.0;
            *s_speed.get_mut(i) = 0.0;
            *s_pos_x.get_mut(i) = origin_entrance.door_pos.x;
            *s_pos_y.get_mut(i) = origin_entrance.door_pos.y;
            *s_transit.get_mut(i) = TRANSIT_ACCESS_EGRESS;
            *s_path.get_mut(i) = plan.current_path;
            *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
            *s_plan_b.get_mut(i) = usize::MAX;
            *s_plan_act.get_mut(i) = 0;
            *slices.next_departure_day.get_mut(i) = u32::MAX;
            *slices.next_departure_minute.get_mut(i) = 0;
            *slices.next_departure_origin.get_mut(i) = usize::MAX;
            *slices.next_departure_target.get_mut(i) = usize::MAX;
            *slices.next_departure_activity.get_mut(i) = 0;
            if crate::debug::is_traffic_enabled() {
                let target_entrance = allocator.entrances.get(plan.target_building);
                let attach_side = local_access_side_label(
                    plan.mode,
                    origin_entrance,
                    plan.planned_attach_lane_id,
                );
                let detach_side = target_entrance
                    .map(|entrance| {
                        local_access_side_label(plan.mode, entrance, plan.planned_detach_lane_id)
                    })
                    .unwrap_or("unknown-target");
                traffic_log!(
                    "[ACCESS_PLAN] agent={} mode={} origin_bldg={} target_bldg={} attach_lane={}({}) attach_d={:.2} attach_node={} detach_lane={}({}) detach_d={:.2} detach_node={} flags=0x{:02x} node_path={:?}",
                    i,
                    transit_mode_label(plan.mode),
                    curr_bldg,
                    plan.target_building,
                    plan.planned_attach_lane_id,
                    attach_side,
                    plan.planned_attach_lane_d,
                    plan.planned_attach_node,
                    plan.planned_detach_lane_id,
                    detach_side,
                    plan.planned_detach_lane_d,
                    plan.planned_detach_node,
                    plan.access_flags,
                    s_path.get(i),
                );
            }
        } else {
            *s_access_flags.get_mut(i) = 0;
            *s_plan_attach_n.get_mut(i) = u32::MAX;
            *s_plan_detach_n.get_mut(i) = u32::MAX;
            *s_plan_attach_lane.get_mut(i) = u32::MAX;
            *s_plan_detach_lane.get_mut(i) = u32::MAX;
            *s_plan_attach_lane_d.get_mut(i) = 0.0;
            *s_plan_detach_lane_d.get_mut(i) = 0.0;
            *s_next_replan_time.get_mut(i) = sim_time + BUILDING_REPLAN_DELAY_S;
        }
    }
}
