// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: occupancy.rs
//  script_path: rust/src/simulation/economy/agents/tick/traffic/occupancy.rs
//  module_name: occupancy
//  version: 0.1.0
//  description: Lane occupancy, gap, and entry-claim helpers.
//  kind: module
//  spec: none
//  internal_dependencies: []
//  external_dependencies: []
//  features: []
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-27
// ========================================================================

//! Lane occupancy, gap, and entry-claim helpers.

use super::super::super::{TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use super::super::claims::LaneClaimContext;
use crate::config::{CAR_LENGTH, IDM_S_MIN, IDM_T_HEAD};

/// Speed at or below which a car counts as stopped rather than crawling.
///
/// Not zero: a car easing forward in a slow queue reads as a small positive
/// speed and is still clearing the mouth, so treating only exact zero as
/// stopped would let a crawling queue block a junction it is in fact emptying.
/// Below this it is standing still and the space behind it is not opening up.
pub(in crate::simulation::economy::agents::tick) const STOPPED_SPEED_MS: f32 = 0.5;

// ========================================================================
// ENTRY OUTCOME
// ========================================================================

/// Outcome of trying to reserve a connector lane entry for this tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::simulation::economy::agents::tick) enum ConnectorEntry {
    /// A connector lane was clear and successfully claimed.
    Enter(usize),
    /// At least one connector exists, but every entry slot is occupied.
    Occupied,
    /// A connector was clear, but another agent claimed every clear candidate first.
    ClaimedThisTick,
    /// No connector lane exists for the requested topology.
    MissingConnection,
}

// ========================================================================
// GAPS AND SLOTS
// ========================================================================

/// Returns the bumper-to-bumper gap to the nearest vehicle ahead in a sorted lane bucket.
pub(in crate::simulation::economy::agents::tick) fn idm_gap_bucket(
    bucket: &[(f32, usize)],
    my_dist: f32,
) -> f32 {
    let ahead = bucket.partition_point(|e| e.0 <= my_dist + 0.05);
    if ahead < bucket.len() {
        (bucket[ahead].0 - my_dist - CAR_LENGTH).max(0.1)
    } else {
        f32::MAX
    }
}

/// Returns whether a car can occupy `attach_d` without violating static separation.
pub(in crate::simulation::economy::agents::tick) fn lane_attach_slot_clear(
    bucket: &[(f32, usize)],
    attach_d: f32,
) -> bool {
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
pub(in crate::simulation::economy::agents::tick) fn live_lane_bucket_transit(transit: u8) -> bool {
    transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION
}

// ========================================================================
// CONFLICT GATE
// ========================================================================

/// Returns whether every movement that crosses `lane_id` is currently clear.
///
/// A connector is separated only along its own length, so a left-turn connector
/// and the oncoming through connector never test each other: both cars see their
/// own lane empty and both proceed. That is a left turn driven into traffic, and
/// at volume it is two cars occupying one point.
///
/// A movement is admitted only when nothing is inside a crossing movement. This
/// is the permissive-turn rule: the turning car gives way to whatever is already
/// in the box. A car that has cleared its conflict point no longer blocks,
/// because the bucket only holds cars still on the connector.
pub(in crate::simulation::economy::agents::tick) fn conflicting_movements_clear(
    node_id: usize,
    lane_id: usize,
    lane_system: &crate::simulation::network::lanes::LaneSystem,
    lane_buckets: &[Vec<(f32, usize)>],
    // Speed of an agent by index, for the exit test below. A closure rather
    // than a slice because the two callers reach the speed column by different
    // routes, and neither should have to reshape itself around this.
    speed_of: impl Fn(usize) -> f32,
) -> bool {
    for &other in lane_system.conflicting_lanes(node_id, lane_id) {
        if lane_buckets.get(other).is_some_and(|b| !b.is_empty()) {
            super::report::record_hold(node_id, super::report::HoldCause::Conflict);
            return false;
        }
    }

    // Movements that begin where this one begins share the ground a waiting car
    // stands on. Checking only this connector's own bucket lets three cars
    // taking three different turns out of one lane all sit on the same point,
    // which is cars stacked inside each other at the junction mouth.
    //
    // Only the mouth is contested, so a sibling blocks solely while a car sits
    // within one car length of its start. A car held further along a sibling
    // connector, by a red signal or by the queue ahead of it, must not lock the
    // mouth against every other turn out of the same lane: that is a deadlock,
    // and it looks like cars parked at the junction forever.
    for &other in lane_system.co_entrant_lanes(node_id, lane_id) {
        if lane_buckets
            .get(other)
            .is_some_and(|b| b.first().is_some_and(|&(d, _)| d < CAR_LENGTH))
        {
            super::report::record_hold(node_id, super::report::HoldCause::ConnectorBusy);
            return false;
        }
    }

    // Give way to the movements that outrank this one.
    //
    // The checks above are symmetric: each bars a movement while another car
    // holds contested ground, so whichever car arrives first goes. That is
    // wrong wherever one movement has the better claim. A right turn on green
    // and the through traffic crossing it both read an empty box on the same
    // tick and both entered, which is the collision.
    //
    // A movement gives way while any higher-ranked movement it crosses is
    // occupied, and unlike the mutual bar this reads the whole connector rather
    // than its mouth: a through car anywhere on its path is committed and
    // closing, and a turn must not cut in front of it. Through traffic yields
    // to nothing, so its own list is empty and it pays nothing for this.
    for &other in lane_system.yielding_lanes(node_id, lane_id) {
        if lane_buckets.get(other).is_some_and(|b| !b.is_empty()) {
            super::report::record_hold(node_id, super::report::HoldCause::Yielded);
            return false;
        }
    }

    // Do not enter a junction this car cannot clear. A connector leads onto a
    // road lane, and a car that enters while the exit is backed up ends up
    // stopped inside the box, where it blocks every crossing movement. That is
    // gridlock, and it is what strands an emergency vehicle behind traffic that
    // would move over if it had anywhere to go.
    //
    // The discriminator is whether the exit queue is MOVING, not how large the
    // gap is. Demanding the full following distance was tried and deadlocked
    // every junction: ordinary moving traffic keeps the mouth occupied, so no
    // queue ever advanced and the front of every line sat still. A moving queue
    // is one this car can follow into; a stopped one is a wall.
    //
    // So a car may enter behind a car that is rolling, however close, and may
    // not enter behind one that has stopped within a length of the mouth. That
    // is the rule a driver uses at a yellow box.
    if let Some(exit_lane) = lane_system
        .lanes
        .get(lane_id)
        .and_then(|l| l.next_lanes.first().copied())
    {
        let exit_blocked = lane_buckets
            .get(exit_lane)
            .and_then(|b| b.first().copied())
            .is_some_and(|(d, agent)| d < CAR_LENGTH && speed_of(agent) <= STOPPED_SPEED_MS);
        if exit_blocked {
            super::report::record_hold(node_id, super::report::HoldCause::ExitJammed);
            return false;
        }
    }
    true
}

/// Returns whether a connector or lane can be entered at distance zero.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn lane_entry_slot_clear(
    lane_id: usize,
    lane_buckets: &[Vec<(f32, usize)>],
) -> bool {
    lane_buckets
        .get(lane_id)
        .map(|bucket| lane_attach_slot_clear(bucket, 0.0))
        .unwrap_or(false)
}

/// Returns whether a target lane has a speed-scaled safe gap at the current distance.
pub(in crate::simulation::economy::agents::tick) fn lane_change_gap_clear(
    bucket: &[(f32, usize)],
    target_d: f32,
    speed: f32,
) -> bool {
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

// ========================================================================
// CLAIMING
// ========================================================================

/// Atomically claims a lane entry slot for this tick.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn claim_lane_entry(
    agent_idx: usize,
    lane_id: usize,
    lane_claims: &LaneClaimContext<'_>,
) -> bool {
    lane_claims.claim_lane(agent_idx, lane_id)
}

/// Returns a stable pseudo-random index for deterministic candidate ordering.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn deterministic_choice_index(
    seed: u64,
    len: usize,
) -> usize {
    if len <= 1 {
        return 0;
    }
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    ((x ^ (x >> 31)) as usize) % len
}

/// Filters connector candidates to clear entries and claims one without allocating.
pub(in crate::simulation::economy::agents::tick) fn claim_connector_entry(
    agent_idx: usize,
    candidate_connectors: &mut Vec<usize>,
    any_routing_valid: bool,
    choice_seed: u64,
    lane_buckets: &[Vec<(f32, usize)>],
    lane_claims: &LaneClaimContext<'_>,
    lane_system: Option<&crate::simulation::network::lanes::LaneSystem>,
    speed_of: impl Fn(usize) -> f32,
) -> ConnectorEntry {
    if candidate_connectors.is_empty() {
        return if any_routing_valid {
            ConnectorEntry::Occupied
        } else {
            ConnectorEntry::MissingConnection
        };
    }

    // let candidates_at_start = candidate_connectors.len();
    candidate_connectors.retain(|&lane_id| lane_entry_slot_clear(lane_id, lane_buckets));
    if candidate_connectors.is_empty() {
        // Distinguishes an entry-slot rejection from a conflict-rule rejection.
        // Both return Occupied, so `[JUNCTION_WAIT] reason=connector-occupied`
        // cannot tell them apart. Uncomment together with `candidates_at_start`
        // above when a junction is starving and the cause is not obvious.
        //
        // crate::traffic_log!(
        //     "[SLOT_BLOCK] agent={} candidates={} all_rejected_by=entry-slot",
        //     agent_idx,
        //     candidates_at_start,
        // );
        return ConnectorEntry::Occupied;
    }

    // A movement whose path is crossed by an occupied movement gives way. Drop
    // those candidates rather than failing outright: another permitted turn out
    // of this approach may be clear, and the car should take it.
    if let Some(system) = lane_system {
        let before = candidate_connectors.len();
        candidate_connectors.retain(|&lane_id| {
            let node_id = system
                .lanes
                .get(lane_id)
                .map(|l| l.node_id)
                .unwrap_or(usize::MAX);
            node_id == usize::MAX
                || conflicting_movements_clear(node_id, lane_id, system, lane_buckets, &speed_of)
        });
        if candidate_connectors.is_empty() {
            crate::traffic_log!(
                "[CONFLICT_BLOCK] agent={} candidates_before={} all_rejected_by=conflict-rule",
                agent_idx,
                before,
            );
            return ConnectorEntry::Occupied;
        }
    }

    let start = deterministic_choice_index(choice_seed, candidate_connectors.len());
    for offset in 0..candidate_connectors.len() {
        let candidate = candidate_connectors[(start + offset) % candidate_connectors.len()];
        if claim_lane_entry(agent_idx, candidate, lane_claims) {
            return ConnectorEntry::Enter(candidate);
        }
    }

    ConnectorEntry::ClaimedThisTick
}
