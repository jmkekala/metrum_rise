// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: lane_change.rs
//  script_path: rust/src/simulation/economy/agents/tick/traffic/lane_change.rs
//  module_name: lane_change
//  version: 0.1.0
//  description: Which lane a car should be in: the next step toward a
//           planned detach lane, the turn pocket for its upcoming
//           movement, and the conservative overtake and return rules.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: [lane-change-targets, turn-pockets, conservative-overtaking]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-28
// ========================================================================

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

/// Returns the lane a car should be in to make `movement` at the far node.
///
/// A turn pocket is a lane that permits one movement, opening part way along
/// the edge. Without this a turning car sits in the through lane to the stop
/// line, and everything behind it waits on a movement it does not share, which
/// is the whole reason a real street widens as it approaches a junction.
///
/// Returns `None` when the current lane already permits the movement and no
/// pocket is better, which is the ordinary case and costs one field read. A
/// pocket that has not opened yet at `lane_d` is not offered, because a car
/// cannot move into a lane that does not exist there.
pub(in crate::simulation::economy::agents::tick) fn turn_lane_target(
    current_lane_id: usize,
    movement: u8,
    lane_d: f32,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let lanes = &transit_network.lane_system.lanes;
    let current = lanes.get(current_lane_id)?;
    if current.edge_id == usize::MAX || current.lane_type != LaneType::Vehicle {
        return None;
    }

    // Already in a lane that permits it and is not a general-purpose lane a
    // pocket would serve better: nothing to do. An unrestricted lane still
    // looks for a pocket, because that is exactly the case this exists for.
    if !current.turns.is_unrestricted() && current.turns.allows(movement) {
        return None;
    }

    let t = if current.length > 1e-3 {
        (lane_d / current.length).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // The closest lane that names this movement and exists here. Closest by
    // lane index, so a car crosses as few lanes as it has to; a two-lane move
    // still resolves one lane at a time through `lane_change_target_toward`.
    let mut best: Option<(i8, usize)> = None;
    for &lid in transit_network
        .lane_system
        .edge_lanes
        .get(&current.edge_id)?
        .iter()
    {
        let Some(lane) = lanes.get(lid) else { continue };
        if lane.is_fwd != current.is_fwd
            || lane.lane_type != LaneType::Vehicle
            || lane.turns.is_unrestricted()
            || !lane.turns.allows(movement)
        {
            continue;
        }
        // The pocket has to exist where the car is, or it cannot enter it.
        if t < lane.extent.0 || t > lane.extent.1 {
            continue;
        }
        let dist = (lane.lane_idx - current.lane_idx).abs();
        if best.is_none_or(|(d, _)| dist < d) {
            best = Some((dist, lid));
        }
    }

    let (_, target) = best?;
    if target == current_lane_id {
        return None;
    }
    lane_change_target_toward(current_lane_id, target, transit_network)
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

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::TurnSet;
    use crate::simulation::network::lanes::Lane;

    /// One forward road lane on edge 0, with a turn set and an extent.
    fn road_lane(lane_idx: i8, turns: u8, extent: (f32, f32)) -> Lane {
        Lane {
            edge_id: 0,
            is_fwd: true,
            lane_idx,
            length: 100.0,
            lane_type: LaneType::Vehicle,
            turns: TurnSet(turns),
            extent,
            ..Default::default()
        }
    }

    /// A network of `lanes`, all on edge 0.
    fn network(lanes: Vec<Lane>) -> TransitNetwork {
        let mut n = TransitNetwork::new();
        let ids: Vec<usize> = (0..lanes.len()).collect();
        n.lane_system.lanes = lanes;
        n.lane_system.edge_lanes.insert(0, ids);
        n
    }

    #[test]
    fn a_through_lane_with_no_pocket_stays_put() {
        // Nothing names the movement, so there is nowhere better to be. The
        // ordinary case, and it must not churn lanes looking for one.
        let net = network(vec![road_lane(0, 0, (0.0, 1.0))]);
        assert_eq!(turn_lane_target(0, TurnSet::RIGHT, 50.0, &net), None);
    }

    #[test]
    fn a_car_turning_right_moves_into_the_right_pocket() {
        let net = network(vec![
            road_lane(0, 0, (0.0, 1.0)),
            road_lane(1, TurnSet::RIGHT, (0.75, 1.0)),
        ]);
        // Past the pocket's opening, so it exists here.
        assert_eq!(turn_lane_target(0, TurnSet::RIGHT, 80.0, &net), Some(1));
    }

    #[test]
    fn a_pocket_that_has_not_opened_yet_is_not_offered() {
        // The pocket runs the last quarter of the edge. A car at 10 m cannot
        // move into a lane that does not exist there, and offering it would
        // steer the car into the verge.
        let net = network(vec![
            road_lane(0, 0, (0.0, 1.0)),
            road_lane(1, TurnSet::RIGHT, (0.75, 1.0)),
        ]);
        assert_eq!(turn_lane_target(0, TurnSet::RIGHT, 10.0, &net), None);
    }

    #[test]
    fn a_car_already_in_a_lane_permitting_the_movement_stays() {
        let net = network(vec![
            road_lane(0, 0, (0.0, 1.0)),
            road_lane(1, TurnSet::RIGHT, (0.0, 1.0)),
        ]);
        assert_eq!(turn_lane_target(1, TurnSet::RIGHT, 80.0, &net), None);
    }

    #[test]
    fn the_nearest_pocket_naming_the_movement_wins() {
        // Two right pockets, one adjacent and one two lanes out. The car
        // crosses as few lanes as it has to.
        let net = network(vec![
            road_lane(0, 0, (0.0, 1.0)),
            road_lane(1, TurnSet::RIGHT, (0.0, 1.0)),
            road_lane(2, TurnSet::RIGHT, (0.0, 1.0)),
        ]);
        assert_eq!(turn_lane_target(0, TurnSet::RIGHT, 80.0, &net), Some(1));
    }

    #[test]
    fn a_left_pocket_is_not_offered_to_a_right_turn() {
        let net = network(vec![
            road_lane(0, 0, (0.0, 1.0)),
            road_lane(1, TurnSet::LEFT, (0.0, 1.0)),
        ]);
        assert_eq!(turn_lane_target(0, TurnSet::RIGHT, 80.0, &net), None);
    }
}
