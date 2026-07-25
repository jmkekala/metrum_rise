//! Local access-ingress movement from the planned network lane to a building door.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK};
use super::super::super::access::{
    advance_along_local_access_path, local_access_path, local_access_point,
    local_access_should_log_step, local_access_side_label, local_access_target_segment,
    planned_detach_is_legal,
};
use super::super::super::planning::plan_network_replan;
use super::super::super::slices::MovementSlices;
use super::super::{NETWORK_REPLAN_DELAY_S, transit_mode_label};
use super::{arrive_in_building, reset_invalid_access_plan};
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, AGENT_WALK_SPEED_MS};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
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
        let s_transit = &slices.transit;
        let s_tgt_b = &slices.tgt_b;
        let s_cur_n = &slices.cur_n;
        let s_plan_detach_n = &slices.planned_detach_n;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_next_replan_time = &slices.next_replan_time;
        let s_cur_e = &slices.cur_e;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_tmode = &slices.tmode;
        let s_path = &slices.path;
        let s_path_idx = &slices.path_idx;
        let s_speed = &slices.speed;

        let b_id = *s_tgt_b.get(i);
        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, usize::MAX, pos, 0.0, slices);
            return;
        }
        let plan_valid =
            (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0 && b_id < allocator.entrances.len();
        if !plan_valid {
            let ingress_target = allocator
                .entrances
                .get(b_id)
                .map(|entrance| entrance.door_pos)
                .unwrap_or(Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i)));
            arrive_in_building(i, b_id, ingress_target, sim_time, slices);
            return;
        }
        let entrance = &allocator.entrances[b_id];
        let detach_lane_id = *s_plan_detach_lane.get(i) as usize;
        let detach_lane_d = *s_plan_detach_lane_d.get(i);
        let legal_detach = planned_detach_is_legal(
            *s_tmode.get(i),
            entrance,
            detach_lane_id,
            detach_lane_d,
            *s_plan_detach_n.get(i),
            transit_network,
            graph,
        );
        let exact_path = if legal_detach {
            local_access_path(
                *s_tmode.get(i),
                entrance,
                detach_lane_id,
                detach_lane_d,
                transit_network,
                graph,
                true,
            )
        } else {
            None
        };

        if let (true, Some(path)) = (legal_detach, exact_path) {
            let current = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            let step = if *s_tmode.get(i) == MODE_CAR {
                AGENT_DRIVEWAY_SPEED_MS
            } else {
                AGENT_WALK_SPEED_MS
            } * delta;
            let (next_pos, reached_door) = advance_along_local_access_path(current, &path, step);
            *s_pos_x.get_mut(i) = next_pos.x;
            *s_pos_y.get_mut(i) = next_pos.y;
            if crate::debug::is_traffic_enabled()
                && local_access_should_log_step(current, next_pos, &path, reached_door)
            {
                let seg_before = local_access_target_segment(current, &path);
                let seg_after = local_access_target_segment(next_pos, &path);
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
                    path.points[0].x,
                    path.points[0].y,
                    path.points[1].x,
                    path.points[1].y,
                    path.points[2].x,
                    path.points[2].y,
                    path.points[3].x,
                    path.points[3].y,
                    path.count,
                );
            }
            if reached_door {
                let ingress_target = path.points[path.count - 1];
                if crate::debug::is_traffic_enabled() {
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
                arrive_in_building(i, b_id, ingress_target, sim_time, slices);
            }
        } else if let Some(ingress_origin) = local_access_point(
            *s_tmode.get(i),
            entrance,
            detach_lane_id,
            detach_lane_d,
            transit_network,
        ) {
            *s_pos_x.get_mut(i) = ingress_origin.x;
            *s_pos_y.get_mut(i) = ingress_origin.y;
            *s_cur_n.get_mut(i) = *s_plan_detach_n.get(i);
            *s_cur_e.get_mut(i) = transit_network
                .lane_system
                .lanes
                .get(detach_lane_id)
                .map(|lane| lane.edge_id)
                .unwrap_or(usize::MAX);
            *s_lane_id.get_mut(i) = detach_lane_id;
            *s_lane_d.get_mut(i) = detach_lane_d;
            *s_speed.get_mut(i) = 0.0;
            s_path.get_mut(i).clear();
            if crate::debug::is_traffic_enabled() {
                traffic_log!(
                    "[ACCESS_INGRESS_ABORT] agent={} bldg={} mode={} lane={} lane_d={:.2} legal_detach={} exact_path={} ingress_origin=({:.2},{:.2}) reason=invalid-ingress-plan flags=0x{:02x}",
                    i,
                    b_id,
                    transit_mode_label(*s_tmode.get(i)),
                    detach_lane_id,
                    detach_lane_d,
                    legal_detach,
                    exact_path.is_some(),
                    ingress_origin.x,
                    ingress_origin.y,
                    *s_access_flags.get(i),
                );
            }
            *s_path_idx.get_mut(i) = 0;
            *s_transit.get_mut(i) = TRANSIT_NETWORK;
            if sim_time >= *s_next_replan_time.get(i) {
                if let Some(replan) = plan_network_replan(
                    *s_plan_detach_n.get(i),
                    *s_cur_e.get(i),
                    b_id,
                    *s_tmode.get(i),
                    *s_access_flags.get(i),
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                ) {
                    *s_path.get_mut(i) = replan.current_path;
                    *s_path_idx.get_mut(i) = if s_path.get(i).len() >= 2 { 1 } else { 0 };
                    *s_plan_detach_n.get_mut(i) = replan.planned_detach_node;
                    *s_plan_detach_lane.get_mut(i) = replan.planned_detach_lane_id as u32;
                    *s_plan_detach_lane_d.get_mut(i) = replan.planned_detach_lane_d;
                    *s_access_flags.get_mut(i) = replan.access_flags;
                    *s_next_replan_time.get_mut(i) = 0.0;
                } else {
                    *s_next_replan_time.get_mut(i) = sim_time + NETWORK_REPLAN_DELAY_S;
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
        }
    }
}
