// SPDX-License-Identifier: GPL-2.0-only

//! Lane occupancy, gap, and entry-claim helpers.

use super::super::super::{TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use super::super::claims::LaneClaimContext;
use crate::config::{CAR_LENGTH, IDM_S_MIN, IDM_T_HEAD};
use crate::simulation::economy::agents::determinism::stable_index;
use crate::simulation::network::lanes::Lane;
use crate::simulation::network::roundabouts::RING_SPEED_MS;

// How far from its end a ring lane's head must be for an entry to go: the lane-change gap's
// separation at the ring's speed. The head's own speed cannot be read here: the speed pass writes
// the speed column while it runs this test, so only the buckets are a stable input (D16).
const RING_ENTRY_HEAD_GAP_M: f32 = CAR_LENGTH + IDM_S_MIN + RING_SPEED_MS * IDM_T_HEAD;
// A ring lane's head this close to the end stands there: movement places a waiting head exactly at
// the end, and a head free to go would have left in the step that reached it.
const RING_HEAD_STANDING_M: f32 = 0.05;

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
pub(in crate::simulation::economy::agents) fn lane_attach_slot_clear(
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

/// The lane an entry onto a roundabout ring gives way to now, `None` when `connector` is free to
/// go or is no ring entry (`ROAD-62`): a car on the circulating connector, a ring lane head
/// arriving within the lane-change gap of the node, or a blocked entry slot on the ring lane
/// itself ("room ahead", the slot test a connector's exit makes, not the box rule). A head already
/// standing at the end holds nothing back: if it waits for the ring, the circulating connector or
/// the ring lane's slot is what it waits for, and those are tested; if it waits for its exit, it
/// is not coming. So an entry only ever stands for the ring lane it enters or for a moving car,
/// and giving way adds no wait a plain junction lacks. A pure function of last tick's buckets, like
/// the claims'. `O(log bucket)`, no allocation.
pub(in crate::simulation::economy::agents::tick) fn ring_entry_blocker(
    connector: usize,
    lanes: &[Lane],
    lane_buckets: &[Vec<(f32, usize)>],
) -> Option<usize> {
    let lane = lanes.get(connector)?;
    let entry = lane.ring_entry?;
    let (circulating, ring_in) = (entry.circulating as usize, entry.ring_in as usize);
    // As `lane_entry_slot_clear`: a lane the snapshot does not hold yet counts as blocked.
    if lane_buckets
        .get(circulating)
        .is_none_or(|bucket| !bucket.is_empty())
    {
        return Some(circulating);
    }
    if let Some(&(head_d, _)) = lane_buckets.get(ring_in).and_then(|bucket| bucket.last()) {
        let to_end_m = lanes[ring_in].length - head_d;
        if to_end_m <= RING_ENTRY_HEAD_GAP_M && to_end_m > RING_HEAD_STANDING_M {
            return Some(ring_in);
        }
    }
    let ring_out = *lane.next_lanes.first()?;
    (!lane_entry_slot_clear(ring_out, lane_buckets)).then_some(ring_out)
}

/// Whether a car may take `connector` as far as give-way on a roundabout ring goes: the one test
/// the speed pass's lane-end hold, the entry claim and the gridlock check share.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn ring_entry_clear(
    connector: usize,
    lanes: &[Lane],
    lane_buckets: &[Vec<(f32, usize)>],
) -> bool {
    ring_entry_blocker(connector, lanes, lane_buckets).is_none()
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

/// Filters connector candidates to clear entries and claims one without allocating.
pub(in crate::simulation::economy::agents::tick) fn claim_connector_entry(
    agent_idx: usize,
    candidate_connectors: &mut Vec<usize>,
    any_routing_valid: bool,
    choice_seed: u64,
    lanes: &[Lane],
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

    candidate_connectors.retain(|&lane_id| {
        lane_entry_slot_clear(lane_id, lane_buckets)
            && ring_entry_clear(lane_id, lanes, lane_buckets)
    });
    if candidate_connectors.is_empty() {
        return ConnectorEntry::Occupied;
    }

    let start = stable_index(choice_seed, candidate_connectors.len());
    for offset in 0..candidate_connectors.len() {
        let candidate = candidate_connectors[(start + offset) % candidate_connectors.len()];
        if claim_lane_entry(agent_idx, candidate, lane_claims) {
            return ConnectorEntry::Enter(candidate);
        }
    }

    ConnectorEntry::ClaimedThisTick
}
