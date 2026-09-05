// SPDX-License-Identifier: GPL-2.0-only

//! Lane-change and conservative overtaking helper rules.

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
