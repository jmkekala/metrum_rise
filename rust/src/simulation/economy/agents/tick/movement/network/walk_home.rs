// SPDX-License-Identifier: GPL-2.0-only

//! A walker whose route is gone (`ROAD-74`, the user's option (c)).
//!
//! A walker on a road lane that needs a new route plans it from that lane through the normal
//! planner, so the route is walkable from its own sidewalk. When its destination is unreachable
//! it gives the trip up and plans a walk home from the same lane (one trip-failure record). When
//! home is unreachable too, or it was already walking home, it is stranded
//! ([`ACCESS_WALK_STRANDED`]): it walks to the end of its lane, waits there and plans home again
//! after 5 s, 30 s, then every 2 min (its backoff step in `network_replan_failures`, the time in
//! `next_replan_time`, both saved), walking home as soon as a plan succeeds. One record when it
//! becomes stranded, none per retry. It never leaves its lane to do so, no crossing is made for
//! it and the watchdog's move home does not apply. Cost: one route query per stranded walker per
//! retry, so at most one per walker every 2 min once settled, and O(1) per stranded walker and
//! tick in between.

use super::super::super::super::{
    ACCESS_PLAN_VALID, ACCESS_WALK_STRANDED, ACTIVITY_HOME, MODE_WALK,
};
use super::super::super::planning::{BuiltNetworkReplan, ReplanFailure, plan_network_replan};
use super::super::super::slices::MovementSlices;
use super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::apply_network_replan;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::AtomicU32;

/// Waits before a stranded walker's home plans, by backoff step; the last one repeats.
pub(super) const STRANDED_RETRY_S: [f32; 3] = [5.0, 30.0, 120.0];

fn retry_after_s(step: u8) -> f32 {
    STRANDED_RETRY_S[usize::from(step).min(STRANDED_RETRY_S.len() - 1)]
}

/// Whether walker `i` is stranded.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline(always)]
pub(super) unsafe fn is_stranded(i: usize, slices: &MovementSlices) -> bool {
    unsafe { *slices.access_flags.get(i) & ACCESS_WALK_STRANDED != 0 }
}

/// Walker `i`'s route from the end of road lane `lane` at `node` to its destination is gone:
/// `failure` is the planner's verdict on it. Gives the trip up for a walk home planned from the
/// lane, or strands the walker. Returns whether it has a route again. One record either way.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn give_up_trip(
    i: usize,
    node: u32,
    lane: usize,
    failure: ReplanFailure,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let home = *slices.home.get(i);
        let cause = FailureCause::NoRoute(failure);
        let stranded = FailureOutcome::Stranded {
            after_s: retry_after_s(0),
        };
        if home < allocator.buildings.len() && *slices.tgt_b.get(i) != home {
            let plan = plan_from_lane(
                i,
                node,
                lane,
                home,
                allocator,
                transit_network,
                graph,
                pathfind_count,
            );
            let walks = plan.is_ok();
            let outcome = if walks {
                FailureOutcome::WalksHome { building: home }
            } else {
                stranded
            };
            record_failure(i, cause, outcome, slices);
            head_home(i, home, sim_time, slices);
            if let Ok(replan) = plan {
                apply_network_replan(i, replan, slices);
                return true;
            }
        } else {
            record_failure(i, cause, stranded, slices);
        }
        strand(i, sim_time, slices);
        false
    }
}

/// Stranded walker `i` on road lane `lane`, which ends at `node`, plans home again when its
/// backoff has run out: walks home on success, else waits for its next step. Returns whether it
/// has a route again. No record.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn retry_home(
    i: usize,
    node: u32,
    lane: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        if sim_time < *slices.next_replan_time.get(i) {
            return false;
        }
        // Home after a give-up; a walker with no home keeps trying its destination.
        let target = *slices.tgt_b.get(i);
        match plan_from_lane(
            i,
            node,
            lane,
            target,
            allocator,
            transit_network,
            graph,
            pathfind_count,
        ) {
            Ok(replan) => {
                apply_network_replan(i, replan, slices);
                true
            }
            Err(_) => {
                let step = slices.network_replan_failures.get(i).saturating_add(1);
                *slices.network_replan_failures.get_mut(i) = step;
                *slices.next_replan_time.get_mut(i) = sim_time + retry_after_s(step);
                false
            }
        }
    }
}

// A walk to `target` from the end of road lane `lane` at `node`, walkable from that lane.
#[allow(clippy::too_many_arguments)]
fn plan_from_lane(
    i: usize,
    node: u32,
    lane: usize,
    target: usize,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> Result<BuiltNetworkReplan, ReplanFailure> {
    plan_network_replan(
        node,
        transit_network
            .lane_system
            .lanes
            .get(lane)
            .map_or(usize::MAX, |value| value.edge_id),
        Some(lane),
        target,
        MODE_WALK,
        0,
        allocator,
        transit_network,
        graph,
        pathfind_count,
        Some((i, "walk-home")),
    )
}

// The trip becomes a walk home, as the watchdog's move home sets it but where the walker stands.
unsafe fn head_home(i: usize, home: usize, sim_time: f32, slices: &MovementSlices) {
    unsafe {
        *slices.tgt_b.get_mut(i) = home;
        *slices.planned_tgt_b.get_mut(i) = usize::MAX;
        *slices.activity.get_mut(i) = ACTIVITY_HOME;
        *slices.planned_activity.get_mut(i) = ACTIVITY_HOME;
        *slices.jstart.get_mut(i) = sim_time;
    }
}

// Stranded: no path and no exit lane, so the walker walks to the end of its lane and waits
// there; the first home plan after 5 s.
unsafe fn strand(i: usize, sim_time: f32, slices: &MovementSlices) {
    unsafe {
        slices.path.get_mut(i).clear();
        slices.walk_lanes.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
        *slices.planned_detach_n.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane_d.get_mut(i) = 0.0;
        *slices.access_flags.get_mut(i) = ACCESS_PLAN_VALID | ACCESS_WALK_STRANDED;
        *slices.network_replan_failures.get_mut(i) = 0;
        *slices.next_replan_time.get_mut(i) = sim_time + retry_after_s(0);
    }
}
