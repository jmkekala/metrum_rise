// SPDX-License-Identifier: GPL-3.0-or-later

//! Crossing the road to or from a far-side access (`PARK-32`).
//!
//! A car or truck whose access lies across its road from its lane (`BothSides` access, the far
//! lane planned) crosses at most one lane (`MAX_CROSSED_LANES`). Entering, it waits in its own
//! lane at the detach point for a gap in the crossed lane, and after [`CROSSING_GIVE_UP_S`] drives
//! on to come back on the near side. Leaving, it waits at the access mouth, off the road, for a
//! gap in the crossed lane and in its target lane. While it crosses it holds an entry in the
//! crossed lane's bucket (`Agent::crossing_lane`), so lane traffic stops behind it, and every other
//! crossing or entry there sees it.

use super::super::traffic::crossing_gap_clear;
use crate::config::{AGENT_DRIVEWAY_SPEED_MS, LANE_WIDTH};
use crate::simulation::buildings::allocator::{BuildingAllocator, LaneCrossing, lane_crossing};
use crate::simulation::economy::agents::LaneEntry;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

/// How long a car or truck waits in its lane for a gap to cross to a far-side access before it
/// drives on and comes back on the near side (`PARK-32`): the most it holds its lane.
pub(crate) const CROSSING_GIVE_UP_S: f32 = 30.0;

/// The crossing wait (`Agent::crossing_wait_s`) of a car that gave up a crossing on its present
/// leg (`PARK-32`): it comes back on the near side and does not wait to cross again until it
/// leaves the road. Below every real wait.
pub(crate) const CROSSING_GAVE_UP: f32 = -1.0;

/// The lane a vehicle at distance `lane_d` of lane `lane_id` crosses to or from kerb `side` of
/// that lane's road under the crossing rule ([`lane_crossing`]), and the crossing's distance on
/// it: the lane point projected on the crossed lane. `None` when the lane is the kerb's own
/// (nothing crossed), and for a door's crossing outside the rule (more lanes, or near a lane
/// end), which is driven as before `PARK-32`, untested. O(lanes on the edge × their points),
/// allocation-free.
pub(in crate::simulation::economy::agents::tick) fn crossing_at(
    lane_id: usize,
    lane_d: f32,
    side: i8,
    network: &TransitNetwork,
) -> Option<(usize, f32)> {
    let lanes = &network.lane_system;
    let point = BuildingAllocator::sample_pos_on_lane(lanes.lanes.get(lane_id)?, lane_d);
    let LaneCrossing::Tested(crossed) = lane_crossing(lanes, lane_id, side, point) else {
        return None;
    };
    let d = BuildingAllocator::project_point_to_polyline_s(&lanes.lanes[crossed].geometry, point);
    Some((crossed, d))
}

/// Whether a vehicle of half length `half` may start a crossing that passes lane `lane` at
/// distance `d` ([`crossing_gap_clear`]): the margin upstream is what a car at the lane's speed
/// limit covers in the crossing's time, two lane widths and the body's length at driveway speed
/// (a car 2.3 s, a truck 3.0 s). A lane the buckets do not hold is busy. O(log bucket).
pub(in crate::simulation::economy::agents::tick) fn crossing_clear(
    lane: usize,
    d: f32,
    half: f32,
    network: &TransitNetwork,
    graph: &RegionGraph,
    lane_buckets: &[Vec<LaneEntry>],
) -> bool {
    let Some(edge) = network
        .lane_system
        .lanes
        .get(lane)
        .and_then(|lane| graph.get_edge(lane.edge_id))
    else {
        return false;
    };
    let crossing_s = (2.0 * LANE_WIDTH + 2.0 * half) / AGENT_DRIVEWAY_SPEED_MS;
    lane_buckets
        .get(lane)
        .is_some_and(|bucket| crossing_gap_clear(bucket, d, half, edge.speed_limit * crossing_s))
}

#[cfg(test)]
mod tests {
    use super::crossing_gap_clear;
    use crate::config::IDM_S_MIN;

    // A car's half length.
    const HALF: f32 = 2.25;

    #[test]
    fn crossing_gap_needs_the_margin_upstream_and_only_the_slot_downstream() {
        let crossing = 100.0;
        let approach = 30.0;
        // Exactly the margin behind: clear; any nearer: not.
        let edge = crossing - (HALF + HALF + IDM_S_MIN + approach) - 0.001;
        assert!(crossing_gap_clear(&[(edge, 1, HALF)], crossing, HALF, approach));
        assert!(!crossing_gap_clear(&[(edge + 0.1, 1, HALF)], crossing, HALF, approach));
        // A body ahead clears with the slot alone, however near beyond it.
        let ahead = crossing + HALF + HALF + IDM_S_MIN + 0.001;
        assert!(crossing_gap_clear(&[(ahead, 1, HALF)], crossing, HALF, approach));
        assert!(!crossing_gap_clear(&[(ahead - 0.1, 1, HALF)], crossing, HALF, approach));
        assert!(crossing_gap_clear(&[], crossing, HALF, approach));
    }
}
