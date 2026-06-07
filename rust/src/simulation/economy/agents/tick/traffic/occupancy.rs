//! Lane occupancy, gap, and entry-claim helpers.

use super::super::super::{TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use super::super::claims::LaneClaimContext;
use crate::config::{CAR_LENGTH, IDM_S_MIN, IDM_T_HEAD};

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

    let start = deterministic_choice_index(choice_seed, candidate_connectors.len());
    for offset in 0..candidate_connectors.len() {
        let candidate = candidate_connectors[(start + offset) % candidate_connectors.len()];
        if claim_lane_entry(agent_idx, candidate, lane_claims) {
            return ConnectorEntry::Enter(candidate);
        }
    }

    ConnectorEntry::ClaimedThisTick
}
