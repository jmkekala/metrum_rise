//! Local access-egress movement from a building door to the planned network lane.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK};
use super::super::super::access::{
    advance_along_local_access_path, local_access_path, local_access_side_label,
    local_access_target_segment, planned_attach_is_legal,
};
use super::super::super::lane_nav::lane_origin_node;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::lane_attach_slot_clear;
use super::super::{BUILDING_REPLAN_DELAY_S, transit_mode_label};
use super::reset_invalid_access_plan;
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, AGENT_WALK_SPEED_MS};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;
use godot::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

/// Handles local access movement from a building door to the planned network attachment lane.
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
    lane_buckets: &Vec<Vec<(f32, usize)>>,
    lane_attach_claimed: &Vec<AtomicBool>,
    slices: &MovementSlices,
) {
    unsafe {
        let s_cur_b = &slices.cur_b;
        let s_plan_attach_n = &slices.planned_attach_n;
        let s_plan_attach_lane = &slices.planned_attach_lane;
        let s_plan_attach_lane_d = &slices.planned_attach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_cur_n = &slices.cur_n;
        let s_cur_e = &slices.cur_e;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_pos_x = &slices.pos_x;
        let s_pos_y = &slices.pos_y;

        let b_id = *s_cur_b.get(i);
        if b_id == usize::MAX || b_id >= allocator.buildings.len() {
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, usize::MAX, pos, 0.0, slices);
            return;
        }
        let b = &allocator.buildings[b_id];
        if b.edge_idx >= graph.edge_count() || graph.edge(b.edge_idx).deleted {
            let pos = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            reset_invalid_access_plan(i, b_id, pos, sim_time + BUILDING_REPLAN_DELAY_S, slices);
            return;
        }
        let plan_valid =
            (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0 && b_id < allocator.entrances.len();
        if !plan_valid {
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
            return;
        }
        let entrance = &allocator.entrances[b_id];
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
        let exact_path = if legal_attach {
            local_access_path(
                *s_tmode.get(i),
                entrance,
                attach_lane_id,
                attach_lane_d,
                transit_network,
                graph,
                false,
            )
        } else {
            None
        };

        if let (true, Some(path), Some(origin_node)) = (
            legal_attach,
            exact_path,
            lane_origin_node(attach_lane_id, transit_network, graph),
        ) {
            let current = Vector2::new(*s_pos_x.get(i), *s_pos_y.get(i));
            let step = if *s_tmode.get(i) == MODE_CAR {
                AGENT_DRIVEWAY_SPEED_MS
            } else {
                AGENT_WALK_SPEED_MS
            } * delta;
            let (next_pos, reached_handoff) = advance_along_local_access_path(current, &path, step);
            *s_pos_x.get_mut(i) = next_pos.x;
            *s_pos_y.get_mut(i) = next_pos.y;
            if crate::debug::is_traffic_enabled() {
                let seg_before = local_access_target_segment(current, &path);
                let seg_after = local_access_target_segment(next_pos, &path);
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
            if reached_handoff {
                let attach_allowed = if *s_tmode.get(i) == MODE_CAR {
                    lane_buckets
                        .get(attach_lane_id)
                        .map(|bucket| lane_attach_slot_clear(bucket, attach_lane_d))
                        .unwrap_or(false)
                        && lane_attach_claimed
                            .get(attach_lane_id)
                            .map(|claimed| !claimed.swap(true, Ordering::AcqRel))
                            .unwrap_or(false)
                } else {
                    true
                };
                if !attach_allowed {
                    if crate::debug::is_traffic_enabled() {
                        traffic_log!(
                            "[ACCESS_EGRESS_WAIT] agent={} bldg={} lane={}({}) lane_d={:.2} pos=({:.2},{:.2}) reason=attach-slot-busy",
                            i,
                            b_id,
                            attach_lane_id,
                            local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                            attach_lane_d,
                            path.points[path.count - 1].x,
                            path.points[path.count - 1].y,
                        );
                    }
                    *s_pos_x.get_mut(i) = path.points[path.count - 1].x;
                    *s_pos_y.get_mut(i) = path.points[path.count - 1].y;
                    *s_speed.get_mut(i) = 0.0;
                    return;
                }
                let parent_edge = transit_network.lane_system.lanes[attach_lane_id].edge_id;
                *s_pos_x.get_mut(i) = path.points[path.count - 1].x;
                *s_pos_y.get_mut(i) = path.points[path.count - 1].y;
                *s_cur_b.get_mut(i) = usize::MAX;
                *s_cur_n.get_mut(i) = origin_node;
                *s_cur_e.get_mut(i) = parent_edge;
                *s_lane_id.get_mut(i) = attach_lane_id;
                *s_lane_d.get_mut(i) = attach_lane_d;
                *s_speed.get_mut(i) = if *s_tmode.get(i) == MODE_CAR {
                    graph
                        .edge(parent_edge)
                        .speed_limit
                        .min(AGENT_DRIVEWAY_SPEED_MS)
                } else {
                    0.0
                };
                if crate::debug::is_traffic_enabled() {
                    traffic_log!(
                        "[ACCESS_EGRESS_ATTACH] agent={} bldg={} lane={}({}) lane_d={:.2} origin_node={} edge={} pos=({:.2},{:.2})",
                        i,
                        b_id,
                        attach_lane_id,
                        local_access_side_label(*s_tmode.get(i), entrance, attach_lane_id),
                        attach_lane_d,
                        origin_node,
                        parent_edge,
                        path.points[path.count - 1].x,
                        path.points[path.count - 1].y,
                    );
                }
                *s_transit.get_mut(i) = TRANSIT_NETWORK;
            }
        } else {
            if crate::debug::is_traffic_enabled() {
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
            let origin_door = entrance.door_pos;
            reset_invalid_access_plan(
                i,
                b_id,
                origin_door,
                sim_time + BUILDING_REPLAN_DELAY_S,
                slices,
            );
        }
    }
}
