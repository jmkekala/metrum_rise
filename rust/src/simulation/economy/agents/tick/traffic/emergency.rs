// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: emergency.rs
//  script_path: rust/src/simulation/economy/agents/tick/traffic/emergency.rs
//  module_name: emergency
//  version: 0.1.0
//  description: Yielding to a responder running lights: who has to move,
//           which way, and what happens when there is nowhere to go.
//           Signal preemption gives a responder a green; this is what
//           gives it a road, and it is the half that decides whether the
//           responder actually gets through.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: [yield-to-responder, pull-over, resume, nowhere-to-go]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-29
// ========================================================================

//! Clearing a path for a responder running lights.

use crate::config::CAR_LENGTH;
use crate::simulation::network::TransitNetwork;

// ========================================================================
// REACH
// ========================================================================

/// How far ahead of a responder a car starts yielding, in metres.
///
/// Far enough that a driver has time to check a mirror, signal, and move rather
/// than braking to a stop in the running lane, which is the manoeuvre that
/// blocks the responder instead of clearing it.
pub const YIELD_REACH_M: f32 = 60.0;

/// How far behind a responder a car holds before resuming, in metres.
///
/// A car that pulls back out the instant the responder's nose passes is beside
/// it again while it is still going by. Held until the whole vehicle and a
/// margin are past.
pub const RESUME_CLEAR_M: f32 = 12.0;

/// Fraction of the road speed limit a yielding car slows to.
///
/// Slowed rather than stopped. A car stopped dead in a live lane is an obstacle
/// the responder has to route around, which is worse than one moving over at
/// half speed.
pub const YIELD_SPEED_FRACTION: f32 = 0.4;

// ========================================================================
// WHO YIELDS
// ========================================================================

/// What a car should do about a responder near it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YieldAction {
    /// Carry on. No responder is close enough behind on this lane.
    None,
    /// Move aside and slow: a responder is approaching from behind.
    ///
    /// Aside means toward the outside of the carriageway, the direction a
    /// driver pulls to in traffic that drives on the right. The lane change
    /// itself goes through the ordinary machinery, so it still respects gaps
    /// and refuses when there is no room.
    PullOver,
    /// Stay slowed but do not move: a responder is close and there is nowhere
    /// to go.
    ///
    /// The case that strands a responder in real traffic, and the reason
    /// preemption alone does not deliver one through a jam: the cars ahead
    /// would move if they could. Modelled rather than papered over, because a
    /// player watching a responder stuck behind a boxed-in queue is seeing the
    /// city's actual failure and should be able to fix it by fixing the road.
    HoldSlowed,
}

/// Whether `follower_d` is inside the reach of a responder at `responder_d`.
///
/// Both distances are along the same lane. A responder only claims the road
/// ahead of it, so a car already past it is not yielding to anything.
#[inline]
pub fn within_reach(responder_d: f32, follower_d: f32) -> bool {
    follower_d > responder_d && follower_d - responder_d <= YIELD_REACH_M
}

/// Whether a car at `follower_d` has been cleared by a responder at
/// `responder_d`, so it may resume.
///
/// True once the responder is far enough ahead that pulling back out does not
/// put the car alongside it.
#[inline]
pub fn is_clear(responder_d: f32, follower_d: f32) -> bool {
    responder_d > follower_d + RESUME_CLEAR_M
}

/// What a car should do, given the nearest responder on its lane.
///
/// `responder_d` is `None` when no responder is on this lane. `can_move_aside`
/// answers whether an outward lane exists with room in it, which the caller
/// resolves through the ordinary lane-change gap rules rather than this module
/// guessing at it.
pub fn action_for(
    responder_d: Option<f32>,
    follower_d: f32,
    can_move_aside: bool,
) -> YieldAction {
    let Some(rd) = responder_d else {
        return YieldAction::None;
    };
    if !within_reach(rd, follower_d) {
        return YieldAction::None;
    }
    if can_move_aside {
        YieldAction::PullOver
    } else {
        YieldAction::HoldSlowed
    }
}

/// The speed a yielding car should be held to on a road with `limit_ms`.
#[inline]
pub fn yield_speed_ms(limit_ms: f32) -> f32 {
    (limit_ms * YIELD_SPEED_FRACTION).max(0.0)
}

// ========================================================================
// FINDING THE RESPONDER
// ========================================================================

/// Distance along `lane_id` of the nearest responder behind `follower_d`.
///
/// Scans the lane's occupancy bucket, which is already sorted by distance and
/// already built every tick, so this costs a walk rather than a search
/// structure of its own.
///
/// `is_responder` answers whether an agent index is a vehicle running lights.
/// Passed in rather than read here because this module owns the yielding rule
/// and not the definition of a responder, which belongs with the services that
/// dispatch them.
pub fn nearest_responder_behind(
    lane_id: usize,
    follower_d: f32,
    lane_buckets: &[Vec<(f32, usize)>],
    is_responder: impl Fn(usize) -> bool,
) -> Option<f32> {
    let bucket = lane_buckets.get(lane_id)?;
    // Sorted ascending by distance, so the last responder before the follower
    // is the nearest one behind it.
    let mut nearest = None;
    for &(d, agent) in bucket.iter() {
        if d >= follower_d {
            break;
        }
        if is_responder(agent) {
            nearest = Some(d);
        }
    }
    nearest
}

/// The lane a yielding car should move into, if one exists.
///
/// Outward from the centre, which is the side a driver pulls to. Returns `None`
/// when the car is already in the outermost lane, which is the `HoldSlowed`
/// case: there is no shoulder in this model, so the outermost lane is as far
/// aside as a car can get.
pub fn pull_over_target(
    lane_id: usize,
    transit_network: &TransitNetwork,
) -> Option<usize> {
    let lanes = &transit_network.lane_system.lanes;
    let current = lanes.get(lane_id)?;
    if current.edge_id == usize::MAX {
        return None;
    }
    // Forward lanes count outward from the centre from 0, backward from -1, so
    // outward is away from zero in whichever direction this lane runs.
    let target_idx = if current.lane_idx >= 0 {
        current.lane_idx + 1
    } else {
        current.lane_idx - 1
    };
    transit_network
        .lane_system
        .edge_lanes
        .get(&current.edge_id)?
        .iter()
        .find(|&&lid| {
            lanes.get(lid).is_some_and(|l| {
                l.is_fwd == current.is_fwd
                    && l.lane_type == crate::simulation::network::lanes::LaneType::Vehicle
                    && l.lane_idx == target_idx
            })
        })
        .copied()
}

/// Whether a car at `follower_d` has room to sit while yielding.
///
/// A yielding car still needs somewhere to be. When the queue ahead is solid to
/// within a car length, moving aside changes nothing and the honest outcome is
/// that the responder waits, which is what happens on a real road.
#[inline]
pub fn has_room_ahead(lane_id: usize, follower_d: f32, lane_buckets: &[Vec<(f32, usize)>]) -> bool {
    lane_buckets
        .get(lane_id)
        .map(|bucket| {
            bucket
                .iter()
                .find(|&&(d, _)| d > follower_d)
                .is_none_or(|&(d, _)| d - follower_d > CAR_LENGTH)
        })
        .unwrap_or(true)
}

// ========================================================================
// TESTS
// ========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_car_ahead_of_an_approaching_responder_pulls_over() {
        assert_eq!(
            action_for(Some(100.0), 130.0, true),
            YieldAction::PullOver
        );
    }

    #[test]
    fn a_car_with_nowhere_to_go_slows_and_holds() {
        // The case that strands a responder in real traffic. The cars ahead
        // would move if they could, and modelling it honestly is the point:
        // the player sees a road problem they can fix.
        assert_eq!(
            action_for(Some(100.0), 130.0, false),
            YieldAction::HoldSlowed
        );
    }

    #[test]
    fn a_car_already_past_the_responder_does_not_yield() {
        // A responder only claims the road ahead of it.
        assert_eq!(action_for(Some(130.0), 100.0, true), YieldAction::None);
    }

    #[test]
    fn a_car_far_ahead_carries_on() {
        assert_eq!(action_for(Some(0.0), 500.0, true), YieldAction::None);
    }

    #[test]
    fn no_responder_means_no_yielding() {
        assert_eq!(action_for(None, 130.0, true), YieldAction::None);
    }

    #[test]
    fn a_car_resumes_only_once_the_responder_is_fully_past() {
        // Pulling back out as the nose goes by puts the car alongside a vehicle
        // still passing it.
        assert!(!is_clear(105.0, 100.0), "nose barely past");
        assert!(is_clear(100.0 + RESUME_CLEAR_M + 1.0, 100.0));
    }

    #[test]
    fn yielding_slows_rather_than_stops() {
        // A car stopped dead in a live lane is an obstacle to route around,
        // which is worse than one moving aside at reduced speed.
        let slowed = yield_speed_ms(13.89);
        assert!(slowed > 0.0, "not a dead stop");
        assert!(slowed < 13.89);
    }

    #[test]
    fn the_nearest_responder_behind_is_the_one_that_matters() {
        // Two responders on one lane: the closer one governs, because it is the
        // one about to arrive.
        let bucket = vec![(10.0, 0), (50.0, 1), (90.0, 2)];
        let buckets = vec![bucket];
        let found = nearest_responder_behind(0, 100.0, &buckets, |a| a == 0 || a == 1);
        assert_eq!(found, Some(50.0));
    }

    #[test]
    fn a_responder_ahead_is_not_found_behind() {
        let buckets = vec![vec![(150.0, 0)]];
        assert_eq!(
            nearest_responder_behind(0, 100.0, &buckets, |_| true),
            None
        );
    }

    #[test]
    fn a_solid_queue_ahead_leaves_no_room() {
        // Bumper to bumper: moving aside changes nothing.
        let buckets = vec![vec![(100.0, 0), (101.0, 1)]];
        assert!(!has_room_ahead(0, 100.0, &buckets));
    }

    #[test]
    fn an_open_road_ahead_has_room() {
        let buckets = vec![vec![(100.0, 0), (200.0, 1)]];
        assert!(has_room_ahead(0, 100.0, &buckets));
    }
}
