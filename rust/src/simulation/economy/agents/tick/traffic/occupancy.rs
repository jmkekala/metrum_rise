// SPDX-License-Identifier: GPL-3.0-or-later

//! Lane occupancy, gap, and entry-claim helpers.

use super::super::super::{
    ACCESS_PLAN_VALID, ACCESS_WALK_STRANDED, MODE_WALK, TRANSIT_ACCESS_EGRESS,
    TRANSIT_ACCESS_INGRESS, TRANSIT_INTERSECTION, TRANSIT_NETWORK, VEHICLE_FREIGHT_DELIVERY,
};
use super::super::claims::LaneClaimContext;
use crate::config::{
    CAR_BODY_LENGTH_M, IDM_S_MIN, IDM_T_HEAD, TRUCK_BODY_LENGTH_M, WALKER_BODY_LENGTH_M,
};
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::economy::agents::determinism::stable_index;
use crate::simulation::network::lanes::LaneSystem;
use crate::simulation::network::roundabouts::RING_SPEED_MS;

// How far from its end, beyond both bodies' half lengths, a ring lane's head must be for an entry
// to go: the lane-change gap's separation at the ring's speed. The head's own speed cannot be read
// here: the speed pass writes the speed column while it runs this test, so only the buckets are a
// stable input (D16).
const RING_ENTRY_HEAD_GAP_M: f32 = IDM_S_MIN + RING_SPEED_MS * IDM_T_HEAD;
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

/// Half the length an agent's body takes in a lane bucket (`TRAFFIC-38`), from its transit mode
/// and vehicle type, both saved columns: a walker [`WALKER_BODY_LENGTH_M`], a freight carrier
/// ([`VEHICLE_FREIGHT_DELIVERY`]) the `truck` class, every other vehicle the `car` class. A lane
/// distance is the body's centre, so two bodies on one lane are clear of each other when their
/// distances differ by at least the sum of their half lengths. O(1), two compares.
#[inline(always)]
pub(crate) fn lane_half_length(mode: u8, vehicle_type: u8) -> f32 {
    if mode == MODE_WALK {
        WALKER_BODY_LENGTH_M * 0.5
    } else if vehicle_type == VEHICLE_FREIGHT_DELIVERY {
        TRUCK_BODY_LENGTH_M * 0.5
    } else {
        CAR_BODY_LENGTH_M * 0.5
    }
}

/// Returns the bumper-to-bumper gap from a body of half length `my_half` at `my_dist` to the
/// nearest body ahead in a sorted lane bucket: the centre distance less both half lengths.
pub(in crate::simulation::economy::agents::tick) fn idm_gap_bucket(
    bucket: &[LaneEntry],
    my_dist: f32,
    my_half: f32,
) -> f32 {
    let ahead = bucket.partition_point(|e| e.0 <= my_dist + 0.05);
    if ahead < bucket.len() {
        (bucket[ahead].0 - my_dist - bucket[ahead].2 - my_half).max(0.1)
    } else {
        f32::MAX
    }
}

/// Returns whether a body of half length `my_half` can occupy `attach_d` with the standstill gap
/// ([`IDM_S_MIN`]) to the bodies behind and ahead, each pair spaced by both half lengths.
pub(in crate::simulation::economy::agents) fn lane_attach_slot_clear(
    bucket: &[LaneEntry],
    attach_d: f32,
    my_half: f32,
) -> bool {
    let insert = bucket.partition_point(|entry| entry.0 < attach_d);
    if insert > 0 {
        let behind = bucket[insert - 1];
        if attach_d - behind.0 < behind.2 + my_half + IDM_S_MIN {
            return false;
        }
    }
    if insert < bucket.len() {
        let ahead = bucket[insert];
        if ahead.0 - attach_d < ahead.2 + my_half + IDM_S_MIN {
            return false;
        }
    }
    true
}

/// Whether a vehicle of half length `my_half` may start across a lane at `crossing_d`
/// (`PARK-32`): the slot is free ([`lane_attach_slot_clear`]), and no body behind it, upstream in
/// the lane's own direction, lies within `approach_m` more, the distance a car at the lane's
/// speed covers while the vehicle crosses. A body ahead needs only the slot: a queue standing
/// past the crossing does not block it. The bodies' speeds are not read (the speed pass writes
/// them meanwhile), so a body standing upstream within the margin blocks too. O(log bucket).
pub(in crate::simulation::economy::agents::tick) fn crossing_gap_clear(
    bucket: &[LaneEntry],
    crossing_d: f32,
    my_half: f32,
    approach_m: f32,
) -> bool {
    if !lane_attach_slot_clear(bucket, crossing_d, my_half) {
        return false;
    }
    let insert = bucket.partition_point(|entry| entry.0 < crossing_d);
    insert == 0 || {
        let behind = bucket[insert - 1];
        crossing_d - behind.0 >= behind.2 + my_half + IDM_S_MIN + approach_m
    }
}

/// The lane whose bucket holds an access agent crossing it (`PARK-32`): its `crossing_lane`
/// while it is on an access leg, none otherwise (a stale value on a lane agent or indoors is
/// never read). The one membership test of a crossing, for the fill, its patches and its
/// rebuild check. O(1).
#[inline(always)]
pub(in crate::simulation::economy::agents) fn crossing_bucket_lane(
    transit: u8,
    crossing_lane: u32,
) -> Option<usize> {
    (crossing_lane != u32::MAX
        && (transit == TRANSIT_ACCESS_INGRESS || transit == TRANSIT_ACCESS_EGRESS))
        .then_some(crossing_lane as usize)
}

/// Returns whether a transit state should appear in live lane occupancy buckets.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn live_lane_bucket_transit(transit: u8) -> bool {
    transit == TRANSIT_NETWORK || transit == TRANSIT_INTERSECTION
}

/// Whether a live lane agent with transit mode `mode` and access flags `flags` holds its place
/// in its lane's bucket (`TRAFFIC-37`): every car, and a walker walking a planned route. A
/// walker that stands with no route, stranded ([`ACCESS_WALK_STRANDED`]) or with no plan (its
/// trip taken, waiting for its household's next home, `TRAFFIC-24`, or for its replan gate),
/// keeps its lane, distance and node, so it plans from its own sidewalk (`TRAFFIC-19`), but is in
/// no bucket: the walkers behind it pass, as traffic passes a car waiting off its lane
/// (`TRAFFIC-29`, `TRAFFIC-31`). It rejoins with the plan it walks on. The one membership test of
/// the bucket fill, its patches and its rebuild check; O(1), two column reads per lane agent.
#[inline(always)]
pub(in crate::simulation::economy::agents) fn lane_bucket_member(mode: u8, flags: u8) -> bool {
    mode != MODE_WALK || flags & (ACCESS_PLAN_VALID | ACCESS_WALK_STRANDED) == ACCESS_PLAN_VALID
}

/// Returns whether a body of half length `my_half` can enter a connector or lane at distance zero.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn lane_entry_slot_clear(
    lane_id: usize,
    my_half: f32,
    lane_buckets: &[Vec<LaneEntry>],
) -> bool {
    lane_buckets
        .get(lane_id)
        .map(|bucket| lane_attach_slot_clear(bucket, 0.0, my_half))
        .unwrap_or(false)
}

/// The lane an entry onto a roundabout ring gives way to now, `None` when `connector` is free to
/// go or is no ring entry (`ROAD-62`), for an entering body of half length `my_half`: a car on the
/// circulating connector, a ring lane head arriving within the lane-change gap of the node, or a
/// blocked entry slot on the ring lane itself ("room ahead", the slot test a connector's exit makes, not the box rule). A head already
/// standing at the end holds nothing back: if it waits for the ring, the circulating connector or
/// the ring lane's slot is what it waits for, and those are tested; if it waits for its exit, it
/// is not coming. So an entry only ever stands for the ring lane it enters or for a moving car,
/// and giving way adds no wait a plain junction lacks. A pure function of last tick's buckets, like
/// the claims'. `O(log bucket)`, no allocation. Out of line: only a ring entry comes here, and
/// the slot test inlined once more changes how the hot paths around it are compiled.
#[inline(never)]
pub(in crate::simulation::economy::agents::tick) fn ring_entry_blocker(
    connector: usize,
    my_half: f32,
    lanes: &LaneSystem,
    lane_buckets: &[Vec<LaneEntry>],
) -> Option<usize> {
    let entry = lanes.ring_entry(connector)?;
    let (circulating, ring_in) = (entry.circulating as usize, entry.ring_in as usize);
    // As `lane_entry_slot_clear`: a lane the snapshot does not hold yet counts as blocked.
    if lane_buckets
        .get(circulating)
        .is_none_or(|bucket| !bucket.is_empty())
    {
        return Some(circulating);
    }
    if let Some(&(head_d, _, head_half)) =
        lane_buckets.get(ring_in).and_then(|bucket| bucket.last())
    {
        let to_end_m = lanes.lanes[ring_in].length - head_d;
        if to_end_m <= head_half + my_half + RING_ENTRY_HEAD_GAP_M
            && to_end_m > RING_HEAD_STANDING_M
        {
            return Some(ring_in);
        }
    }
    let ring_out = *lanes.lanes[connector].next_lanes.first()?;
    (!lane_entry_slot_clear(ring_out, my_half, lane_buckets)).then_some(ring_out)
}

/// Whether a car of half length `my_half` may take `connector` as far as give-way on a roundabout
/// ring goes: the one test the speed pass's lane-end hold, the entry claim and the gridlock check
/// share.
#[inline(always)]
pub(in crate::simulation::economy::agents::tick) fn ring_entry_clear(
    connector: usize,
    my_half: f32,
    lanes: &LaneSystem,
    lane_buckets: &[Vec<LaneEntry>],
) -> bool {
    lanes
        .lanes
        .get(connector)
        .is_none_or(|lane| lane.ring_entry.is_none())
        || ring_entry_blocker(connector, my_half, lanes, lane_buckets).is_none()
}

/// Returns whether a target lane has a speed-scaled safe gap at the current distance for a body
/// of half length `my_half`: to each neighbour both half lengths, the standstill gap and the
/// headway at `speed`.
pub(in crate::simulation::economy::agents::tick) fn lane_change_gap_clear(
    bucket: &[LaneEntry],
    target_d: f32,
    speed: f32,
    my_half: f32,
) -> bool {
    let min_gap = my_half + IDM_S_MIN + speed.max(0.0) * IDM_T_HEAD;
    let insert = bucket.partition_point(|entry| entry.0 < target_d);
    if insert > 0 {
        let behind = bucket[insert - 1];
        if target_d - behind.0 < behind.2 + min_gap {
            return false;
        }
    }
    if insert < bucket.len() {
        let ahead = bucket[insert];
        if ahead.0 - target_d < ahead.2 + min_gap {
            return false;
        }
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

/// Filters connector candidates to entries clear for a body of half length `my_half` and claims
/// one without allocating.
#[allow(clippy::too_many_arguments)]
pub(in crate::simulation::economy::agents::tick) fn claim_connector_entry(
    agent_idx: usize,
    my_half: f32,
    candidate_connectors: &mut Vec<usize>,
    any_routing_valid: bool,
    choice_seed: u64,
    lanes: &LaneSystem,
    lane_buckets: &[Vec<LaneEntry>],
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
        lane_entry_slot_clear(lane_id, my_half, lane_buckets)
            && ring_entry_clear(lane_id, my_half, lanes, lane_buckets)
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
