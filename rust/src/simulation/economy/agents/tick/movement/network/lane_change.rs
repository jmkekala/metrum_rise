// SPDX-License-Identifier: GPL-2.0-only

//! Active network lane-change and conservative overtaking execution.

use super::super::super::super::{ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_NETWORK};
use super::super::super::claims::LaneClaimContext;
use super::super::super::slices::MovementSlices;
use super::super::super::traffic::{
    LaneChangeKind, LaneChangeParams, OVERTAKE_COOLDOWN_S, choose_lane_change, claim_lane_entry,
    lane_change_finished, lane_change_gap_clear, planned_lane_change_target,
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
    lane_claims: &LaneClaimContext<'_>,
    slices: &MovementSlices,
) -> usize {
    unsafe {
        let transit = *slices.transit.get(i);
        let lane_d = *slices.lane_d.get(i);
        if *slices.lane_change_from_lane.get(i) != u32::MAX
            && (transit != TRANSIT_NETWORK
                || lane_change_finished(
                    lane_d,
                    *slices.lane_change_start_d.get(i),
                    *slices.lane_change_length.get(i),
                ))
        {
            *slices.lane_change_from_lane.get_mut(i) = u32::MAX;
            *slices.lane_change_start_d.get_mut(i) = 0.0;
            *slices.lane_change_length.get_mut(i) = 0.0;
        }
        if *slices.tmode.get(i) != MODE_CAR
            || transit != TRANSIT_NETWORK
            || *slices.lane_change_from_lane.get(i) != u32::MAX
            || !lane_claims.agent_may_change_lanes(i)
        {
            return lane_id;
        }
        let speed = *slices.speed.get(i);
        let detach_lane_id = *slices.planned_detach_lane.get(i) as usize;
        let detach_lane_d = *slices.planned_detach_lane_d.get(i);
        let has_access_plan = (*slices.access_flags.get(i) & ACCESS_PLAN_VALID) != 0;
        let choice = choose_lane_change(
            LaneChangeParams {
                lane_id,
                lane_d,
                speed,
                detach_lane_id,
                detach_lane_d,
                has_access_plan,
                blocked_time: *slices.overtake_blocked_time.get(i),
                cooldown: *slices.overtake_cooldown.get(i),
            },
            transit_network,
            lane_buckets,
        );
        let Some(choice) = choice else {
            if crate::debug::is_traffic_enabled() && has_access_plan {
                if let Some(target_lane_id) = planned_lane_change_target(
                    lane_id,
                    detach_lane_id,
                    lane_d,
                    detach_lane_d,
                    transit_network,
                ) {
                    let target_d =
                        lane_d.min(transit_network.lane_system.lanes[target_lane_id].length);
                    if !lane_buckets
                        .get(target_lane_id)
                        .is_some_and(|bucket| lane_change_gap_clear(bucket, target_d, speed))
                    {
                        traffic_log!(
                            "[LANE_CHANGE_WAIT] agent={} lane={} target_lane={} lane_d={:.2} speed={:.2} reason=target-gap",
                            i,
                            lane_id,
                            target_lane_id,
                            lane_d,
                            speed,
                        );
                    }
                }
            }
            return lane_id;
        };
        if !claim_lane_entry(i, choice.target_lane_id, lane_claims) {
            traffic_log!(
                "[LANE_CHANGE_WAIT] agent={} lane={} target_lane={} reason=target-entry-claimed",
                i,
                lane_id,
                choice.target_lane_id,
            );
            return lane_id;
        }
        let target = &transit_network.lane_system.lanes[choice.target_lane_id];
        *slices.lane_change_from_lane.get_mut(i) = lane_id as u32;
        *slices.lane_change_start_d.get_mut(i) = lane_d;
        *slices.lane_change_length.get_mut(i) = choice.length;
        *slices.lane_id.get_mut(i) = choice.target_lane_id;
        *slices.lane_d.get_mut(i) = lane_d.min(target.length);
        *slices.cur_e.get_mut(i) = target.edge_id;
        if choice.kind != LaneChangeKind::Planned {
            *slices.overtake_blocked_time.get_mut(i) = 0.0;
            *slices.overtake_cooldown.get_mut(i) = OVERTAKE_COOLDOWN_S;
        }
        let label = match choice.kind {
            LaneChangeKind::Planned => "LANE_CHANGE_START",
            LaneChangeKind::Overtake => "OVERTAKE_START",
            LaneChangeKind::Return => "OVERTAKE_RETURN",
        };
        traffic_log!(
            "[{}] agent={} edge={} from_lane={} to_lane={} start_d={:.2} length={:.2} speed={:.2} detach_lane={} detach_d={:.2}",
            label,
            i,
            target.edge_id,
            lane_id,
            choice.target_lane_id,
            lane_d,
            choice.length,
            speed,
            detach_lane_id,
            detach_lane_d,
        );
        choice.target_lane_id
    }
}
