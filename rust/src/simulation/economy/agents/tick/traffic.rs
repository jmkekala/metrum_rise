//! Traffic movement helpers for car following, junctions, and lane changes.

use super::{TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use crate::config::{
    CAR_JUNCTION_LATERAL_ACCEL_MS2, CAR_JUNCTION_MIN_SPEED_MS, CAR_JUNCTION_SPEED_MS, CAR_LENGTH,
    IDM_A_MAX, IDM_B, IDM_S_MIN, IDM_T_HEAD,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::lanes::{Lane, LaneType};
use godot::prelude::Vector3;
use rand::Rng;
use std::sync::atomic::{AtomicBool, Ordering};

const LANE_CHANGE_DURATION_S: f32 = 3.5;
/// Minimum longitudinal distance for a lane-change S-curve.
pub(super) const LANE_CHANGE_MIN_LENGTH_M: f32 = 18.0;
const LANE_CHANGE_MAX_LENGTH_M: f32 = 70.0;
/// Distance before the nominal finish where the lane-change is considered complete.
pub(super) const LANE_CHANGE_FINISH_EPS_M: f32 = 0.25;
/// Time spent blocked before a car may attempt conservative overtaking.
pub(super) const OVERTAKE_STUCK_TIME_S: f32 = 2.0;
/// Cooldown between discretionary overtaking or return maneuvers.
pub(super) const OVERTAKE_COOLDOWN_S: f32 = 8.0;
/// Minimum speed-limit advantage required before overtake pressure builds.
pub(super) const OVERTAKE_MIN_SPEED_GAIN_MS: f32 = 2.0;
/// Minimum extra clear distance needed in the target lane to overtake.
pub(super) const OVERTAKE_MIN_GAP_GAIN_M: f32 = 12.0;
/// Required clear distance ahead in a target lane for overtaking.
pub(super) const OVERTAKE_TARGET_AHEAD_GAP_M: f32 = 30.0;
/// Required clear distance ahead before returning to the cruise lane.
pub(super) const OVERTAKE_RETURN_TARGET_GAP_M: f32 = 20.0;
/// Minimum distance from the edge end before starting an overtake.
pub(super) const OVERTAKE_EDGE_BUFFER_M: f32 = 12.0;
/// Minimum distance from a planned detach point before starting an overtake.
pub(super) const OVERTAKE_DETACH_BUFFER_M: f32 = 25.0;

/// Outcome of trying to reserve a connector lane entry for this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ConnectorEntry {
    /// A connector lane was clear and successfully claimed.
    Enter(usize),
    /// At least one connector exists, but every entry slot is occupied.
    Occupied,
    /// A connector was clear, but another agent claimed every clear candidate first.
    ClaimedThisTick,
    /// No connector lane exists for the requested topology.
    MissingConnection,
}

/// Returns the bumper-to-bumper gap to the nearest vehicle ahead in a sorted lane bucket.
pub(super) fn idm_gap_bucket(bucket: &[(f32, usize)], my_dist: f32) -> f32 {
    let ahead = bucket.partition_point(|e| e.0 <= my_dist + 0.05);
    if ahead < bucket.len() {
        (bucket[ahead].0 - my_dist - CAR_LENGTH).max(0.1)
    } else {
        f32::MAX
    }
}

/// Returns whether a car can occupy `attach_d` without violating static separation.
pub(super) fn lane_attach_slot_clear(bucket: &[(f32, usize)], attach_d: f32) -> bool {
    let min_sep = CAR_LENGTH + IDM_S_MIN;
    let insert = bucket.partition_point(|entry| entry.0 < attach_d);
    if insert > 0 && attach_d - bucket[insert - 1].0 < min_sep {
        return false;
    }
    if insert < bucket.len() && bucket[insert].0 - attach_d < min_sep {
        return false;
    }
    true
}

/// Returns whether a transit state should appear in live lane occupancy buckets.
#[inline(always)]
pub(super) fn live_lane_bucket_transit(transit: u8) -> bool {
    transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION
}

/// Returns whether a connector or lane can be entered at distance zero.
#[inline(always)]
pub(super) fn lane_entry_slot_clear(lane_id: usize, lane_buckets: &[Vec<(f32, usize)>]) -> bool {
    lane_buckets
        .get(lane_id)
        .map(|bucket| lane_attach_slot_clear(bucket, 0.0))
        .unwrap_or(false)
}

/// Returns whether a target lane has a speed-scaled safe gap at the current distance.
pub(super) fn lane_change_gap_clear(bucket: &[(f32, usize)], target_d: f32, speed: f32) -> bool {
    let min_sep = CAR_LENGTH + IDM_S_MIN + speed.max(0.0) * IDM_T_HEAD;
    let insert = bucket.partition_point(|entry| entry.0 < target_d);
    if insert > 0 && target_d - bucket[insert - 1].0 < min_sep {
        return false;
    }
    if insert < bucket.len() && bucket[insert].0 - target_d < min_sep {
        return false;
    }
    true
}

/// Atomically claims a lane entry slot for this tick.
#[inline(always)]
pub(super) fn claim_lane_entry(lane_id: usize, lane_attach_claimed: &[AtomicBool]) -> bool {
    lane_attach_claimed
        .get(lane_id)
        .map(|claimed| !claimed.swap(true, Ordering::AcqRel))
        .unwrap_or(false)
}

/// Filters connector candidates to clear entries and claims one without allocating.
pub(super) fn claim_connector_entry<R: Rng + ?Sized>(
    candidate_connectors: &mut Vec<usize>,
    any_routing_valid: bool,
    rng: &mut R,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_attach_claimed: &[AtomicBool],
) -> ConnectorEntry {
    if candidate_connectors.is_empty() {
        return if any_routing_valid {
            ConnectorEntry::Occupied
        } else {
            ConnectorEntry::MissingConnection
        };
    }

    candidate_connectors.retain(|&lane_id| lane_entry_slot_clear(lane_id, lane_buckets));
    if candidate_connectors.is_empty() {
        return ConnectorEntry::Occupied;
    }

    let start = rng.gen_range(0..candidate_connectors.len());
    for offset in 0..candidate_connectors.len() {
        let candidate = candidate_connectors[(start + offset) % candidate_connectors.len()];
        if claim_lane_entry(candidate, lane_attach_claimed) {
            return ConnectorEntry::Enter(candidate);
        }
    }

    ConnectorEntry::ClaimedThisTick
}

/// Returns the speed-scaled longitudinal lane-change length.
#[inline(always)]
pub(super) fn lane_change_length_for_speed(speed: f32) -> f32 {
    (speed.max(0.0) * LANE_CHANGE_DURATION_S)
        .clamp(LANE_CHANGE_MIN_LENGTH_M, LANE_CHANGE_MAX_LENGTH_M)
}

/// Returns the gap below which a car starts building overtaking pressure.
#[inline(always)]
pub(super) fn overtake_follow_gap(speed: f32) -> f32 {
    (CAR_LENGTH + IDM_S_MIN + speed.max(0.0) * IDM_T_HEAD + 8.0)
        .max(OVERTAKE_TARGET_AHEAD_GAP_M * 0.5)
}

/// Returns the next speed for one simplified IDM time step.
pub(super) fn idm_new_speed(v: f32, v_max: f32, gap: f32, dt: f32) -> f32 {
    let free = (v / v_max.max(0.1)).powi(4);
    let acc = if gap < f32::MAX / 2.0 {
        let s_star = IDM_S_MIN + v * IDM_T_HEAD;
        IDM_A_MAX * (1.0 - free - (s_star / gap).powi(2))
    } else {
        IDM_A_MAX * (1.0 - free)
    };
    (v + acc * dt).clamp(0.0, v_max)
}

/// Caps a car speed by the global junction design speed.
#[inline(always)]
pub(super) fn junction_car_speed(speed: f32) -> f32 {
    speed.min(CAR_JUNCTION_SPEED_MS)
}

/// Limits a speed change by acceleration or comfortable braking.
#[inline(always)]
pub(super) fn limit_speed_change(current: f32, target: f32, dt: f32) -> f32 {
    if target >= current {
        target.min(current + IDM_A_MAX * dt)
    } else {
        target.max(current - IDM_B * dt)
    }
}

/// Returns the highest speed that can brake to `target_speed` within `distance_m`.
#[inline(always)]
pub(super) fn braking_speed_for_distance(target_speed: f32, distance_m: f32) -> f32 {
    (target_speed * target_speed + 2.0 * IDM_B * distance_m.max(0.0)).sqrt()
}

fn flat_unit(v: Vector3) -> Option<Vector3> {
    let flat = Vector3::new(v.x, 0.0, v.z);
    if flat.length_squared() > 1.0e-8 {
        Some(flat.normalized())
    } else {
        None
    }
}

fn lane_end_tangent(lane: &Lane, at_start: bool) -> Option<Vector3> {
    if lane.geometry.len() < 2 {
        return None;
    }
    if at_start {
        for segment in lane.geometry.windows(2) {
            if let Some(tangent) = flat_unit(segment[1] - segment[0]) {
                return Some(tangent);
            }
        }
    } else {
        for idx in (1..lane.geometry.len()).rev() {
            if let Some(tangent) = flat_unit(lane.geometry[idx] - lane.geometry[idx - 1]) {
                return Some(tangent);
            }
        }
    }
    None
}

/// Returns the curvature-limited speed cap for a junction connector lane.
pub(super) fn connector_turn_speed(connector_lane: &Lane) -> f32 {
    let Some(start_tangent) = lane_end_tangent(connector_lane, true) else {
        return CAR_JUNCTION_SPEED_MS;
    };
    let Some(end_tangent) = lane_end_tangent(connector_lane, false) else {
        return CAR_JUNCTION_SPEED_MS;
    };

    let dot = start_tangent.dot(end_tangent).clamp(-1.0, 1.0);
    let turn_angle_rad = dot.acos();
    if turn_angle_rad < 0.15 {
        return CAR_JUNCTION_SPEED_MS;
    }

    let radius_m = connector_lane.length.max(0.1) / turn_angle_rad;
    (CAR_JUNCTION_LATERAL_ACCEL_MS2 * radius_m)
        .sqrt()
        .clamp(CAR_JUNCTION_MIN_SPEED_MS, CAR_JUNCTION_SPEED_MS)
}

/// Caps a car entering a connector by the connector-specific turn speed.
#[inline(always)]
pub(super) fn junction_entry_speed(speed: f32, connector_lane: &Lane) -> f32 {
    speed.min(connector_turn_speed(connector_lane))
}

/// Returns the next adjacent same-edge lane toward the final planned lane.
pub(super) fn lane_change_target_toward(
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
pub(super) fn overtaking_lane_target(
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
pub(super) fn cruise_lane_return_target(
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
pub(super) fn planned_detach_distance_on_current_edge(
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
pub(super) fn planned_lane_change_target(
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

#[cfg(test)]
mod tests {
    use super::{
        ConnectorEntry, claim_connector_entry, connector_turn_speed, cruise_lane_return_target,
        junction_car_speed, junction_entry_speed, lane_change_gap_clear,
        lane_change_length_for_speed, lane_change_target_toward, limit_speed_change,
        overtaking_lane_target,
    };
    use crate::config::{CAR_JUNCTION_SPEED_MS, IDM_B};
    use crate::simulation::network::TransitNetwork;
    use crate::simulation::network::lanes::{Lane, LaneType};
    use godot::prelude::Vector3;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn test_junction_car_speed_caps_fast_turns() {
        assert_eq!(junction_car_speed(50.0), CAR_JUNCTION_SPEED_MS);
        assert_eq!(junction_car_speed(0.0), 0.0);
        assert_eq!(junction_car_speed(4.0), 4.0);
    }

    #[test]
    fn test_connector_turn_speed_slows_tight_turns() {
        let straight = Lane {
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(20.0, 0.0, 0.0)],
            length: 20.0,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };
        let tight_turn = Lane {
            geometry: vec![
                Vector3::new(0.0, 0.0, 0.0),
                Vector3::new(3.0, 0.0, 0.0),
                Vector3::new(3.0, 0.0, 3.0),
            ],
            length: 6.0,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        };

        assert_eq!(connector_turn_speed(&straight), CAR_JUNCTION_SPEED_MS);
        assert!(connector_turn_speed(&tight_turn) < CAR_JUNCTION_SPEED_MS);
        assert!(junction_entry_speed(50.0, &tight_turn) < CAR_JUNCTION_SPEED_MS);
    }

    #[test]
    fn test_limit_speed_change_uses_comfortable_braking() {
        let next = limit_speed_change(14.0, 0.0, 0.1);
        assert!((next - (14.0 - IDM_B * 0.1)).abs() < 1.0e-5);
    }

    #[test]
    fn test_claim_connector_entry_reports_entry_blockers() {
        let mut rng = rand::thread_rng();

        let lane_buckets = vec![Vec::new()];
        let lane_claims = [AtomicBool::new(false)];
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(&mut candidates, true, &mut rng, &lane_buckets, &lane_claims),
            ConnectorEntry::Enter(0)
        );
        assert!(lane_claims[0].load(Ordering::Acquire));

        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(&mut candidates, true, &mut rng, &lane_buckets, &lane_claims),
            ConnectorEntry::ClaimedThisTick
        );

        let occupied_buckets = vec![vec![(0.0, 1)]];
        let lane_claims = [AtomicBool::new(false)];
        let mut candidates = vec![0];
        assert_eq!(
            claim_connector_entry(
                &mut candidates,
                true,
                &mut rng,
                &occupied_buckets,
                &lane_claims,
            ),
            ConnectorEntry::Occupied
        );

        let mut candidates = Vec::new();
        assert_eq!(
            claim_connector_entry(
                &mut candidates,
                false,
                &mut rng,
                &lane_buckets,
                &lane_claims,
            ),
            ConnectorEntry::MissingConnection
        );
    }

    #[test]
    fn test_lane_change_length_scales_with_speed() {
        let slow = lane_change_length_for_speed(4.0);
        let urban = lane_change_length_for_speed(14.0);
        let fast = lane_change_length_for_speed(40.0);

        assert!(urban > slow);
        assert!(fast >= urban);
    }

    #[test]
    fn test_lane_change_gap_clear_requires_speed_scaled_space() {
        let bucket = vec![(17.0, 0)];

        assert!(lane_change_gap_clear(&bucket, 10.0, 4.0));
        assert!(!lane_change_gap_clear(&bucket, 10.0, 14.0));
    }

    #[test]
    fn test_lane_change_target_steps_one_lane_at_a_time() {
        let mut network = TransitNetwork::new();
        for lane_idx in 0..3 {
            network.lane_system.lanes.push(Lane {
                edge_id: 7,
                is_fwd: true,
                lane_idx,
                lane_type: LaneType::Vehicle,
                ..Default::default()
            });
        }
        network.lane_system.edge_lanes.insert(7, vec![0, 1, 2]);

        assert_eq!(lane_change_target_toward(0, 2, &network), Some(1));
        assert_eq!(lane_change_target_toward(1, 2, &network), Some(2));
    }

    #[test]
    fn test_overtaking_targets_center_and_returns_outward() {
        let mut network = TransitNetwork::new();
        for (is_fwd, lane_idx) in [(true, 0), (true, 1), (false, -1), (false, -2)] {
            network.lane_system.lanes.push(Lane {
                edge_id: 7,
                is_fwd,
                lane_idx,
                lane_type: LaneType::Vehicle,
                ..Default::default()
            });
        }
        network.lane_system.edge_lanes.insert(7, vec![0, 1, 2, 3]);

        assert_eq!(overtaking_lane_target(1, &network), Some(0));
        assert_eq!(overtaking_lane_target(0, &network), None);
        assert_eq!(cruise_lane_return_target(0, &network), Some(1));
        assert_eq!(overtaking_lane_target(3, &network), Some(2));
        assert_eq!(overtaking_lane_target(2, &network), None);
        assert_eq!(cruise_lane_return_target(2, &network), Some(3));
    }
}
