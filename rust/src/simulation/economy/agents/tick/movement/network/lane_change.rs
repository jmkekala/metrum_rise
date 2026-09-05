// SPDX-License-Identifier: GPL-2.0-only

//! Active network lane-change and conservative overtaking execution.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK};
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::{
    LANE_CHANGE_FINISH_EPS_M, LANE_CHANGE_MIN_LENGTH_M, OVERTAKE_COOLDOWN_S,
    OVERTAKE_DETACH_BUFFER_M, OVERTAKE_EDGE_BUFFER_M, OVERTAKE_MIN_GAP_GAIN_M,
    OVERTAKE_RETURN_TARGET_GAP_M, OVERTAKE_STUCK_TIME_S, OVERTAKE_TARGET_AHEAD_GAP_M,
    cruise_lane_return_target, idm_gap_bucket, lane_change_gap_clear, lane_change_length_for_speed,
    overtaking_lane_target, planned_detach_distance_on_current_edge, planned_lane_change_target,
};
use crate::simulation::network::TransitNetwork;
use crate::traffic_log;

/// Applies active lane-change lifecycle, planned lane changes, and conservative overtaking.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn prepare_lane_change_and_overtake(
    i: usize,
    lane_id: usize,
    lane_buckets: &[Vec<(f32, usize)>],
    transit_network: &TransitNetwork,
    slices: &MovementSlices,
) -> usize {
    unsafe {
        let s_tmode = &slices.tmode;
        let s_speed = &slices.speed;
        let s_transit = &slices.transit;
        let s_plan_detach_lane = &slices.planned_detach_lane;
        let s_plan_detach_lane_d = &slices.planned_detach_lane_d;
        let s_access_flags = &slices.access_flags;
        let s_lane_id = &slices.lane_id;
        let s_lane_d = &slices.lane_d;
        let s_lane_change_from_lane = &slices.lane_change_from_lane;
        let s_lane_change_start_d = &slices.lane_change_start_d;
        let s_lane_change_length = &slices.lane_change_length;
        let s_overtake_blocked_time = &slices.overtake_blocked_time;
        let s_overtake_cooldown = &slices.overtake_cooldown;
        let s_cur_e = &slices.cur_e;

        let mut current_lane_id = lane_id;
        if *s_lane_change_from_lane.get(i) != u32::MAX {
            let finish_d = *s_lane_change_start_d.get(i) + *s_lane_change_length.get(i);
            if *s_transit.get(i) != TRANSIT_NETWORK
                || *s_lane_d.get(i) + LANE_CHANGE_FINISH_EPS_M >= finish_d
            {
                *s_lane_change_from_lane.get_mut(i) = u32::MAX;
                *s_lane_change_start_d.get_mut(i) = 0.0;
                *s_lane_change_length.get_mut(i) = 0.0;
            }
        }

        if *s_tmode.get(i) == MODE_CAR
            && *s_transit.get(i) == TRANSIT_NETWORK
            && *s_lane_change_from_lane.get(i) == u32::MAX
            && (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
        {
            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
            if let Some(target_lane_id) = planned_lane_change_target(
                current_lane_id,
                planned_detach_lane_id,
                *s_lane_d.get(i),
                *s_plan_detach_lane_d.get(i),
                transit_network,
            ) {
                let source_lane = &transit_network.lane_system.lanes[current_lane_id];
                let target_lane = &transit_network.lane_system.lanes[target_lane_id];
                let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                let available_m = (source_lane.length - *s_lane_d.get(i))
                    .min(*s_plan_detach_lane_d.get(i) - *s_lane_d.get(i));
                let maneuver_length = lane_change_length_for_speed(*s_speed.get(i))
                    .min((available_m - LANE_CHANGE_FINISH_EPS_M).max(LANE_CHANGE_MIN_LENGTH_M));
                let gap_clear = lane_buckets
                    .get(target_lane_id)
                    .map(|bucket| lane_change_gap_clear(bucket, lane_d, *s_speed.get(i)))
                    .unwrap_or(false);
                if available_m > LANE_CHANGE_MIN_LENGTH_M && gap_clear {
                    *s_lane_change_from_lane.get_mut(i) = current_lane_id as u32;
                    *s_lane_change_start_d.get_mut(i) = *s_lane_d.get(i);
                    *s_lane_change_length.get_mut(i) = maneuver_length;
                    *s_lane_id.get_mut(i) = target_lane_id;
                    *s_lane_d.get_mut(i) = lane_d;
                    *s_cur_e.get_mut(i) = target_lane.edge_id;
                    current_lane_id = target_lane_id;
                    traffic_log!(
                        "[LANE_CHANGE_START] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} detach_lane={} detach_d={:.2}",
                        i,
                        target_lane.edge_id,
                        *s_lane_change_from_lane.get(i),
                        target_lane_id,
                        *s_lane_change_start_d.get(i),
                        *s_lane_change_length.get(i),
                        *s_speed.get(i),
                        planned_detach_lane_id,
                        *s_plan_detach_lane_d.get(i),
                    );
                } else if !gap_clear {
                    traffic_log!(
                        "[LANE_CHANGE_WAIT] agent={} lane={} target_lane={} lane_d={:.2} speed={:.2} reason=target-gap",
                        i,
                        current_lane_id,
                        target_lane_id,
                        *s_lane_d.get(i),
                        *s_speed.get(i),
                    );
                }
            }
        }

        if *s_tmode.get(i) == MODE_CAR
            && *s_transit.get(i) == TRANSIT_NETWORK
            && *s_lane_change_from_lane.get(i) == u32::MAX
            && *s_overtake_cooldown.get(i) <= 0.0
        {
            let planned_detach_lane_id = *s_plan_detach_lane.get(i) as usize;
            let planned_target_pending = (*s_access_flags.get(i) & ACCESS_PLAN_VALID) != 0
                && planned_lane_change_target(
                    current_lane_id,
                    planned_detach_lane_id,
                    *s_lane_d.get(i),
                    *s_plan_detach_lane_d.get(i),
                    transit_network,
                )
                .is_some();
            if !planned_target_pending {
                let source_lane = &transit_network.lane_system.lanes[current_lane_id];
                let maneuver_length = lane_change_length_for_speed(*s_speed.get(i));
                let dist_to_edge_end = source_lane.length - *s_lane_d.get(i);
                let dist_to_detach = planned_detach_distance_on_current_edge(
                    current_lane_id,
                    planned_detach_lane_id,
                    *s_lane_d.get(i),
                    *s_plan_detach_lane_d.get(i),
                    transit_network,
                );
                let enough_road_left = dist_to_edge_end > maneuver_length + OVERTAKE_EDGE_BUFFER_M;
                let far_from_detach = dist_to_detach > maneuver_length + OVERTAKE_DETACH_BUFFER_M;

                if source_lane.edge_id != usize::MAX && enough_road_left && far_from_detach {
                    let current_gap = lane_buckets
                        .get(current_lane_id)
                        .map(|bucket| idm_gap_bucket(bucket, *s_lane_d.get(i)))
                        .unwrap_or(f32::MAX);
                    let overtake_target =
                        if *s_overtake_blocked_time.get(i) >= OVERTAKE_STUCK_TIME_S {
                            overtaking_lane_target(current_lane_id, transit_network)
                                .map(|target| (target, true))
                        } else if *s_overtake_blocked_time.get(i) <= 0.0
                            && current_gap > OVERTAKE_TARGET_AHEAD_GAP_M
                        {
                            cruise_lane_return_target(current_lane_id, transit_network)
                                .map(|target| (target, false))
                        } else {
                            None
                        };

                    if let Some((target_lane_id, is_overtake)) = overtake_target {
                        let target_lane = &transit_network.lane_system.lanes[target_lane_id];
                        let lane_d = (*s_lane_d.get(i)).min(target_lane.length);
                        let target_gap = lane_buckets
                            .get(target_lane_id)
                            .map(|bucket| idm_gap_bucket(bucket, lane_d))
                            .unwrap_or(f32::MAX);
                        let target_clear = lane_buckets
                            .get(target_lane_id)
                            .map(|bucket| lane_change_gap_clear(bucket, lane_d, *s_speed.get(i)))
                            .unwrap_or(false);
                        let useful_overtake = target_gap > current_gap + OVERTAKE_MIN_GAP_GAIN_M
                            && target_gap > OVERTAKE_TARGET_AHEAD_GAP_M;
                        let safe_return = target_gap > OVERTAKE_RETURN_TARGET_GAP_M;

                        if target_clear
                            && ((is_overtake && useful_overtake) || (!is_overtake && safe_return))
                        {
                            *s_lane_change_from_lane.get_mut(i) = current_lane_id as u32;
                            *s_lane_change_start_d.get_mut(i) = *s_lane_d.get(i);
                            *s_lane_change_length.get_mut(i) = maneuver_length;
                            *s_lane_id.get_mut(i) = target_lane_id;
                            *s_lane_d.get_mut(i) = lane_d;
                            *s_cur_e.get_mut(i) = target_lane.edge_id;
                            *s_overtake_blocked_time.get_mut(i) = 0.0;
                            *s_overtake_cooldown.get_mut(i) = OVERTAKE_COOLDOWN_S;
                            current_lane_id = target_lane_id;
                            if is_overtake {
                                traffic_log!(
                                    "[OVERTAKE_START] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} current_gap={:.2} target_gap={:.2}",
                                    i,
                                    target_lane.edge_id,
                                    *s_lane_change_from_lane.get(i),
                                    target_lane_id,
                                    *s_lane_change_start_d.get(i),
                                    *s_lane_change_length.get(i),
                                    *s_speed.get(i),
                                    current_gap,
                                    target_gap,
                                );
                            } else {
                                traffic_log!(
                                    "[OVERTAKE_RETURN] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} target_gap={:.2}",
                                    i,
                                    target_lane.edge_id,
                                    *s_lane_change_from_lane.get(i),
                                    target_lane_id,
                                    *s_lane_change_start_d.get(i),
                                    *s_lane_change_length.get(i),
                                    *s_speed.get(i),
                                    target_gap,
                                );
                            }
                        }
                    }
                }
            }
        }

        current_lane_id
    }
}
