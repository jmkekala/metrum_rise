// SPDX-License-Identifier: GPL-2.0-only

//! Lane-change and conservative overtaking helper rules.

use super::occupancy::{idm_gap_bucket, lane_change_gap_clear};
use crate::config::{CAR_LENGTH, IDM_S_MIN, IDM_T_HEAD};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::lanes::LaneType;

const LANE_CHANGE_DURATION_S: f32 = 3.5;
/// Minimum longitudinal distance for a lane-change S-curve.
pub(in crate::simulation::economy::agents::tick) const LANE_CHANGE_MIN_LENGTH_M: f32 = 18.0;
const LANE_CHANGE_MAX_LENGTH_M: f32 = 70.0;
/// Distance before the nominal finish where the lane-change is considered complete.
pub(in crate::simulation::economy::agents::tick) const LANE_CHANGE_FINISH_EPS_M: f32 = 0.25;
/// Time spent blocked before a car may attempt conservative overtaking.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_STUCK_TIME_S: f32 = 2.0;
/// Cooldown between discretionary overtaking or return maneuvers.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_COOLDOWN_S: f32 = 8.0;
/// Minimum speed-limit advantage required before overtake pressure builds.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_MIN_SPEED_GAIN_MS: f32 = 2.0;
/// Minimum extra clear distance needed in the target lane to overtake.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_MIN_GAP_GAIN_M: f32 = 12.0;
/// Required clear distance ahead in a target lane for overtaking.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_TARGET_AHEAD_GAP_M: f32 = 30.0;
/// Required clear distance ahead before returning to the cruise lane.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_RETURN_TARGET_GAP_M: f32 = 20.0;
/// Minimum distance from the edge end before starting an overtake.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_EDGE_BUFFER_M: f32 = 12.0;
/// Minimum distance from a planned detach point before starting an overtake.
pub(in crate::simulation::economy::agents::tick) const OVERTAKE_DETACH_BUFFER_M: f32 = 25.0;

/// Per-agent inputs shared by deterministic claim classification and movement.
pub(in crate::simulation::economy::agents::tick) struct LaneChangeParams {
    /// Lane occupied before this maneuver.
    pub lane_id: usize,
    /// Current longitudinal position in metres.
    pub lane_d: f32,
    /// Speed after this tick's IDM update.
    pub speed: f32,
    /// Planned frontage lane, including the invalid-ID sentinel when absent.
    pub detach_lane_id: usize,
    /// Planned frontage distance in metres.
    pub detach_lane_d: f32,
    /// Whether the exact access plan currently governs lane selection.
    pub has_access_plan: bool,
    /// Accumulated overtaking pressure in seconds.
    pub blocked_time: f32,
    /// Remaining discretionary maneuver cooldown in seconds.
    pub cooldown: f32,
}

/// Reason for a selected lateral maneuver.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::simulation::economy::agents::tick) enum LaneChangeKind {
    /// Required by the current frontage route.
    Planned,
    /// Pass a slower queue using an inner lane.
    Overtake,
    /// Return to the outer cruising lane.
    Return,
}

/// A gap-checked maneuver awaiting the shared lane reservation.
pub(in crate::simulation::economy::agents::tick) struct LaneChange {
    /// Adjacent lane that must be reserved before applying the maneuver.
    pub target_lane_id: usize,
    /// Longitudinal span of the rendered transition in metres.
    pub length: f32,
    /// Governs discretionary cooldown and diagnostics.
    pub kind: LaneChangeKind,
}

/// Tests the same finish boundary during claim classification and movement.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn lane_change_finished(
    lane_d: f32,
    start_d: f32,
    length: f32,
) -> bool {
    lane_d + LANE_CHANGE_FINISH_EPS_M >= start_d + length
}

/// Selects a legal, gap-checked lane change using only the immutable lane snapshot.
/// Work is bounded by the current edge's lane count and O(log K) occupancy queries.
pub(in crate::simulation::economy::agents::tick) fn choose_lane_change(
    params: LaneChangeParams,
    transit_network: &TransitNetwork,
    lane_buckets: &[Vec<(f32, usize)>],
) -> Option<LaneChange> {
    let source = transit_network.lane_system.lanes.get(params.lane_id)?;
    if params.has_access_plan {
        if let Some(target_lane_id) = planned_lane_change_target(
            params.lane_id,
            params.detach_lane_id,
            params.lane_d,
            params.detach_lane_d,
            transit_network,
        ) {
            let target = &transit_network.lane_system.lanes[target_lane_id];
            let lane_d = params.lane_d.min(target.length);
            let available =
                (source.length - params.lane_d).min(params.detach_lane_d - params.lane_d);
            let length = lane_change_length_for_speed(params.speed)
                .min((available - LANE_CHANGE_FINISH_EPS_M).max(LANE_CHANGE_MIN_LENGTH_M));
            let gap_clear = lane_buckets
                .get(target_lane_id)
                .is_some_and(|bucket| lane_change_gap_clear(bucket, lane_d, params.speed));
            // A pending frontage maneuver takes priority even while its gap is blocked.
            return (available > LANE_CHANGE_MIN_LENGTH_M && gap_clear).then_some(LaneChange {
                target_lane_id,
                length,
                kind: LaneChangeKind::Planned,
            });
        }
    }
    if !(params.cooldown <= 0.0) || source.edge_id == usize::MAX {
        return None;
    }
    let length = lane_change_length_for_speed(params.speed);
    let detach_distance = planned_detach_distance_on_current_edge(
        params.lane_id,
        params.detach_lane_id,
        params.lane_d,
        params.detach_lane_d,
        transit_network,
    );
    if !(source.length - params.lane_d > length + OVERTAKE_EDGE_BUFFER_M
        && detach_distance > length + OVERTAKE_DETACH_BUFFER_M)
    {
        return None;
    }
    let (target_lane_id, kind) = if params.blocked_time >= OVERTAKE_STUCK_TIME_S {
        (
            overtaking_lane_target(params.lane_id, transit_network)?,
            LaneChangeKind::Overtake,
        )
    } else if params.blocked_time <= 0.0 {
        (
            cruise_lane_return_target(params.lane_id, transit_network)?,
            LaneChangeKind::Return,
        )
    } else {
        return None;
    };
    // Most cruising cars already occupy the outer lane; skip gap queries without a sibling.
    let current_gap = lane_buckets
        .get(params.lane_id)
        .map(|bucket| idm_gap_bucket(bucket, params.lane_d))
        .unwrap_or(f32::MAX);
    if kind == LaneChangeKind::Return && !(current_gap > OVERTAKE_TARGET_AHEAD_GAP_M) {
        return None;
    }
    let target = &transit_network.lane_system.lanes[target_lane_id];
    let lane_d = params.lane_d.min(target.length);
    let target_bucket = lane_buckets.get(target_lane_id)?;
    let target_gap = idm_gap_bucket(target_bucket, lane_d);
    let useful = if kind == LaneChangeKind::Overtake {
        target_gap > current_gap + OVERTAKE_MIN_GAP_GAIN_M
            && target_gap > OVERTAKE_TARGET_AHEAD_GAP_M
    } else {
        target_gap > OVERTAKE_RETURN_TARGET_GAP_M
    };
    (useful && lane_change_gap_clear(target_bucket, lane_d, params.speed)).then_some(LaneChange {
        target_lane_id,
        length,
        kind,
    })
}

/// Returns the speed-scaled longitudinal lane-change length.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn lane_change_length_for_speed(
    speed: f32,
) -> f32 {
    (speed.max(0.0) * LANE_CHANGE_DURATION_S)
        .clamp(LANE_CHANGE_MIN_LENGTH_M, LANE_CHANGE_MAX_LENGTH_M)
}

/// Returns the gap below which a car starts building overtaking pressure.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn overtake_follow_gap(speed: f32) -> f32 {
    (CAR_LENGTH + IDM_S_MIN + speed.max(0.0) * IDM_T_HEAD + 8.0)
        .max(OVERTAKE_TARGET_AHEAD_GAP_M * 0.5)
}

/// Returns the next adjacent same-edge lane toward the final planned lane.
pub(in crate::simulation::economy::agents::tick) fn lane_change_target_toward(
    from_lane_id: usize,
    final_target_lane_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    if from_lane_id == final_target_lane_id {
        return None;
    }
    let from_lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    let target_lane = transit_network
        .lane_system
        .lanes
        .get(final_target_lane_id)?;
    if from_lane.edge_id == usize::MAX
        || from_lane.edge_id != target_lane.edge_id
        || from_lane.is_fwd != target_lane.is_fwd
        || from_lane.lane_type != LaneType::Vehicle
        || target_lane.lane_type != LaneType::Vehicle
    {
        return None;
    }

    let lane_idx_delta = target_lane.lane_idx - from_lane.lane_idx;
    if lane_idx_delta.abs() <= 1 {
        return Some(final_target_lane_id);
    }
    let next_lane_idx = from_lane.lane_idx + lane_idx_delta.signum();
    transit_network
        .lane_system
        .edge_lanes
        .get(&from_lane.edge_id)?
        .iter()
        .find(|&&lane_id| {
            transit_network
                .lane_system
                .lanes
                .get(lane_id)
                .is_some_and(|lane| {
                    lane.is_fwd == from_lane.is_fwd
                        && lane.lane_type == LaneType::Vehicle
                        && lane.lane_idx == next_lane_idx
                })
        })
        .copied()
}

fn sibling_vehicle_lane_with_idx(
    from_lane_id: usize,
    target_lane_idx: i8,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let from_lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    if from_lane.edge_id == usize::MAX || from_lane.lane_type != LaneType::Vehicle {
        return None;
    }
    transit_network
        .lane_system
        .edge_lanes
        .get(&from_lane.edge_id)?
        .iter()
        .find(|&&lane_id| {
            transit_network
                .lane_system
                .lanes
                .get(lane_id)
                .is_some_and(|lane| {
                    lane.is_fwd == from_lane.is_fwd
                        && lane.lane_type == LaneType::Vehicle
                        && lane.lane_idx == target_lane_idx
                })
        })
        .copied()
}

/// Returns a conservative same-edge overtake target lane.
pub(in crate::simulation::economy::agents::tick) fn overtaking_lane_target(
    from_lane_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    let target_idx = if lane.lane_idx > 0 {
        lane.lane_idx - 1
    } else if lane.lane_idx < -1 {
        lane.lane_idx + 1
    } else {
        return None;
    };
    sibling_vehicle_lane_with_idx(from_lane_id, target_idx, transit_network)
}

/// Returns the preferred outer cruise lane for a discretionary return maneuver.
pub(in crate::simulation::economy::agents::tick) fn cruise_lane_return_target(
    from_lane_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let lane = transit_network.lane_system.lanes.get(from_lane_id)?;
    let target_idx = if lane.lane_idx >= 0 {
        lane.lane_idx + 1
    } else {
        lane.lane_idx - 1
    };
    sibling_vehicle_lane_with_idx(from_lane_id, target_idx, transit_network)
}

/// Returns remaining distance to the planned detach point when it is on the current edge.
pub(in crate::simulation::economy::agents::tick) fn planned_detach_distance_on_current_edge(
    current_lane_id: usize,
    planned_detach_lane_id: usize,
    lane_d: f32,
    planned_detach_lane_d: f32,
    transit_network: &TransitNetwork,
) -> f32 {
    let Some(current_lane) = transit_network.lane_system.lanes.get(current_lane_id) else {
        return f32::MAX;
    };
    let Some(detach_lane) = transit_network
        .lane_system
        .lanes
        .get(planned_detach_lane_id)
    else {
        return f32::MAX;
    };
    if current_lane.edge_id == usize::MAX
        || current_lane.edge_id != detach_lane.edge_id
        || current_lane.is_fwd != detach_lane.is_fwd
        || lane_d >= planned_detach_lane_d
    {
        return f32::MAX;
    }
    planned_detach_lane_d - lane_d
}

/// Returns the next lane-change target required to reach the planned detach lane.
pub(in crate::simulation::economy::agents::tick) fn planned_lane_change_target(
    current_lane_id: usize,
    planned_detach_lane_id: usize,
    lane_d: f32,
    planned_detach_lane_d: f32,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    if planned_detach_lane_id == usize::MAX || lane_d >= planned_detach_lane_d {
        return None;
    }
    lane_change_target_toward(current_lane_id, planned_detach_lane_id, transit_network)
}
