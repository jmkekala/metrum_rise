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
//!
//! No walker takes the watchdog's teleport (`TRAFFIC-19`). A walker standing with no lane first
//! takes the sidewalk it stands on ([`stand_on_sidewalk`]) and goes on from there by the same
//! rules; one at a node with no sidewalk plans from the node and, with no route, is stranded
//! there. One off every sidewalk (its road removed under it) walks across the ground to the
//! nearest sidewalk point within `OFF_NETWORK_WALK_MAX_M` (200 m, [`walk_to_sidewalk`]) and plans
//! from there by the same rules; with none in reach it is stranded where it stands. Every failure
//! the watchdog would recover goes through [`walker_lost`] instead.
//!
//! A walker whose trip is taken from it on the way (its job ends, its destination is removed)
//! reaches the end of its lane with no plan and no destination. With a home it walks home from
//! there ([`head_home_without_trip`], `TRAFFIC-24`); with none it waits there for its household's
//! next home. A driver in that case turns at the end of its lane ([`car_trip_taken`],
//! `TRAFFIC-29`).

use super::super::super::super::{
    ACCESS_PLAN_VALID, ACCESS_WALK_STRANDED, ACCESS_WALK_TO_SIDEWALK, ACTIVITY_HOME, MODE_WALK,
    TRANSIT_NETWORK,
};
use super::super::super::access::{LocalAccessPath, advance_along_local_access_path};
use super::super::super::lane_nav::{held_attach_origin, lane_terminal_node};
use super::super::super::planning::{BuiltNetworkReplan, ReplanFailure, plan_network_replan};
use super::super::super::slices::{MovementSlices, enter_lane};
use super::super::super::trip_failures::{FailureCause, FailureOutcome, record_failure};
use super::super::replan_watchdog::has_recoverable_network_trip;
use super::apply_network_replan;
use crate::config::AGENT_WALK_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::remap::{LANE_REATTACH_MAX_DIST_M, nearest_sidewalk};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use crate::simulation::network::lanes::geometry::agent_lane_position;
use crate::simulation::parking::ParkingTripPhase;
use godot::prelude::Vector2;
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

/// Walker `i`'s route from the end of road lane `lane` at `node` (from `node` itself when `lane`
/// is `usize::MAX`) to its destination is gone, for `cause`. Gives the trip up for a walk home
/// planned from there, or strands the walker. Returns whether it has a route again. One record
/// either way.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn give_up_trip(
    i: usize,
    node: u32,
    lane: usize,
    cause: FailureCause,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        let home = *slices.home.get(i);
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

/// Walker `i` stands at the end of road lane `lane` at `node` with no plan, no destination and no
/// trip to recover (`TRAFFIC-24`): its trip was taken from it on the way (its job ended mid-walk,
/// its destination was removed), and nothing else gives a walker on the road a new one. With a
/// home it plans home from that lane, the walk home of a give-up ([`give_up_trip`]) with no
/// record, since no trip failed: it walks home, or with no route home is stranded there (one
/// `NoRoute` record, outcome `Stranded`) on the stranded backoff. With no home it keeps waiting
/// for its household's next one (`relocate_household_member_home` gives it that destination).
/// O(1) for a walker with no home; one route query for one with a home, once, since it leaves
/// with a plan or stranded.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn head_home_without_trip(
    i: usize,
    node: u32,
    lane: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let home = *slices.home.get(i);
        if home >= allocator.buildings.len() {
            return;
        }
        crate::debug::work::add(crate::debug::work::Work::WalkersHomeAfterCancel, 1);
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
        head_home(i, home, sim_time, slices);
        match plan {
            Ok(replan) => apply_network_replan(i, replan, slices),
            Err(failure) => {
                record_failure(
                    i,
                    FailureCause::NoRoute(failure),
                    FailureOutcome::Stranded {
                        after_s: retry_after_s(0),
                    },
                    slices,
                );
                strand(i, sim_time, slices);
            }
        }
    }
}

/// Car `i` stands at the end of its road lane, `length` metres long, with no plan and its trip
/// taken from it on the way (`TRAFFIC-29`). On a parking leg its bay and route belong to the
/// ordered phase, so it keeps the lane and asks the leg's recovery, which plans from here in the
/// commit of this same tick. Off one it turns for home: the replan gate plans from the node it
/// reached, as for any car with a destination and no plan. With no home it stands off the lane
/// for its household's next one. Returns whether the car keeps the lane. O(1), then one route
/// query in the recovery or the gate.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
pub(super) unsafe fn car_trip_taken(
    i: usize,
    length: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        if slices.parking_trip.get(i).phase != ParkingTripPhase::None {
            let trip = slices.parking_trip.get_mut(i);
            trip.ready = true;
            trip.replan_needed = true;
            *slices.lane_d.get_mut(i) = length;
            // The cursor a load accepts for a car with no route.
            *slices.path_idx.get_mut(i) = 0;
            *slices.speed.get_mut(i) = 0.0;
            return true;
        }
        let home = *slices.home.get(i);
        if home < allocator.buildings.len() {
            crate::debug::work::add(crate::debug::work::Work::DriversHomeAfterCancel, 1);
            head_home(i, home, sim_time, slices);
        }
        false
    }
}

/// Stranded walker `i` on road lane `lane`, which ends at `node` (at `node` with no lane when
/// `lane` is `usize::MAX`), plans home again when its backoff has run out: walks home on success,
/// else waits for its next step. Returns whether it has a route again. No record.
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
                next_backoff_step(i, sim_time, slices);
                false
            }
        }
    }
}

/// Stranded walker `i` with no lane plans home again when its backoff has run out, from where
/// it stands ([`place_walker`]): the sidewalk it stands on, or its node when it stands there with
/// no sidewalk. Off every sidewalk (its road removed under it) it sets off for the nearest one in
/// reach, or with none waits for its next step. Returns whether it has a route again. No record.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn retry_home_without_lane(
    i: usize,
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
        let (node, lane) = match place_walker(i, transit_network, graph, slices) {
            Place::Sidewalk(node, lane) => (node, lane),
            Place::Node(node) => (node, usize::MAX),
            Place::Walking(_) => return false,
            Place::Off => {
                next_backoff_step(i, sim_time, slices);
                return false;
            }
        };
        retry_home(
            i,
            node,
            lane,
            sim_time,
            allocator,
            transit_network,
            graph,
            pathfind_count,
            slices,
        )
    }
}

/// Walker `i` stands with no lane (`TRAFFIC-19`): before any replan gate it takes the sidewalk
/// it stands on ([`place_walker`]), so it plans, walks home or is stranded from that sidewalk's
/// end by the rules of any walker on a road lane. At a node with no sidewalk it keeps standing,
/// to plan from the node. Off every sidewalk, its road removed under it, it walks to the nearest
/// sidewalk in reach (one record), else is stranded where it stands (one record), and is never
/// planned from a node it is not at. A stranded walker and one with no trip are left alone. O(1)
/// for every other agent; O(arms × lanes per arm) for a walker without a lane, plus one bounded
/// R-tree search when it is off every sidewalk.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
pub(super) unsafe fn stand_on_sidewalk(
    i: usize,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) {
    unsafe {
        if *slices.tmode.get(i) != MODE_WALK
            || *slices.lane_id.get(i) != usize::MAX
            || *slices.transit.get(i) != TRANSIT_NETWORK
            || is_stranded(i, slices)
            || (*slices.access_flags.get(i) & ACCESS_PLAN_VALID == 0
                && !has_recoverable_network_trip(i, slices))
        {
            return;
        }
        match place_walker(i, transit_network, graph, slices) {
            Place::Walking(metres) => record_failure(
                i,
                FailureCause::NoPosition,
                FailureOutcome::WalksToSidewalk { metres },
                slices,
            ),
            Place::Off => strand_in_place(i, FailureCause::NoPosition, sim_time, allocator, slices),
            Place::Sidewalk(..) | Place::Node(_) => {}
        }
    }
}

/// Walker `i`'s trip failed for `cause` at a site where the watchdog recovers a vehicle
/// (`TRAFFIC-19`). It plans its destination again from where it stands: the end of its road lane,
/// or, with no lane, the sidewalk it stands on or its node ([`place_walker`]). A route: one
/// `cause` record, outcome `Replans`. None: it gives the trip up ([`give_up_trip`]). A walker off
/// every sidewalk walks to the nearest one in reach (one record) and plans there. A walker on a
/// crossing, or off every sidewalk with none in reach, is stranded at once (one record): the
/// first plans home from the end of the sidewalk it reaches, the second where it stands. No
/// teleport. One route query, or two with a give-up.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
pub(in super::super) unsafe fn walker_lost(
    i: usize,
    cause: FailureCause,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) {
    unsafe {
        let lane = *slices.lane_id.get(i);
        let start = match transit_network.lane_system.lanes.get(lane) {
            Some(value) if value.edge_id == usize::MAX => None,
            Some(_) => lane_terminal_node(lane, transit_network, graph).map(|node| (node, lane)),
            None => match place_walker(i, transit_network, graph, slices) {
                Place::Sidewalk(node, lane) => Some((node, lane)),
                Place::Node(node) => Some((node, usize::MAX)),
                Place::Walking(metres) => {
                    record_failure(i, cause, FailureOutcome::WalksToSidewalk { metres }, slices);
                    return;
                }
                Place::Off => None,
            },
        };
        let Some((node, lane)) = start else {
            strand_in_place(i, cause, sim_time, allocator, slices);
            return;
        };
        match plan_from_lane(
            i,
            node,
            lane,
            *slices.tgt_b.get(i),
            allocator,
            transit_network,
            graph,
            pathfind_count,
        ) {
            Ok(replan) => {
                record_failure(i, cause, FailureOutcome::Replans, slices);
                apply_network_replan(i, replan, slices);
            }
            Err(_) => {
                give_up_trip(
                    i,
                    node,
                    lane,
                    cause,
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
        }
    }
}

/// Where a walker with no lane stands ([`place_walker`]).
enum Place {
    /// On the sidewalk into this node, now its lane (the second field).
    Sidewalk(u32, usize),
    /// At this node, with no sidewalk into it.
    Node(u32),
    /// Off every sidewalk, now walking this many metres to the nearest one ([`walk_to_sidewalk`]).
    Walking(f32),
    /// Off every sidewalk, with none within `OFF_NETWORK_WALK_MAX_M` (200 m).
    Off,
}

/// Places walker `i`, which has no lane, where it stands at its node: on the foot lane into the
/// node whose end is nearest its position, ties by lane id, entered at its end, when that end is
/// within [`LANE_REATTACH_MAX_DIST_M`] (a road edit's own reattach reach); its route is dropped
/// (its exit kept), to be planned again from that lane. Its position is not changed: it is drawn
/// at the lane's end from the next pose. With no such sidewalk it is at the node when it stands
/// within that reach of it. Otherwise it is off every sidewalk and sets off for the nearest one
/// ([`start_sidewalk_walk`]). O(arms × lanes per arm), plus the bounded search when off.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
unsafe fn place_walker(
    i: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Place {
    unsafe {
        let node = *slices.cur_n.get(i);
        let node = if (node as usize) < graph.node_count() {
            graph.get_valid_node(node)
        } else {
            u32::MAX
        };
        if (node as usize) < graph.node_adjacency_count()
            && let Some(place) = place_at_node(i, node, transit_network, graph, slices)
        {
            return place;
        }
        start_sidewalk_walk(i, transit_network, graph, slices)
    }
}

// `place_walker` at its node `node`: the sidewalk into it, or the node itself, or `None` when the
// walker stands beyond the reattach reach of both.
unsafe fn place_at_node(
    i: usize,
    node: u32,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Option<Place> {
    unsafe {
        let (x, z) = (*slices.pos_x.get(i), *slices.pos_y.get(i));
        let reach2 = LANE_REATTACH_MAX_DIST_M * LANE_REATTACH_MAX_DIST_M;
        let lanes = &transit_network.lane_system.lanes;
        let mut best: Option<(f32, usize)> = None;
        for &edge in graph.node_adjacency(node) {
            for &lane in transit_network
                .lane_system
                .edge_lanes
                .get(&edge)
                .into_iter()
                .flatten()
            {
                let value = &lanes[lane];
                let Some(end) = value.geometry.last() else {
                    continue;
                };
                if value.lane_type != LaneType::Foot
                    || lane_terminal_node(lane, transit_network, graph) != Some(node)
                {
                    continue;
                }
                let d2 = (end.x - x).powi(2) + (end.z - z).powi(2);
                if d2 <= reach2
                    && best.is_none_or(|(best_d2, best_lane)| {
                        d2.total_cmp(&best_d2).then(lane.cmp(&best_lane)).is_lt()
                    })
                {
                    best = Some((d2, lane));
                }
            }
        }
        let Some((_d2, lane)) = best else {
            let at = graph.node(node).pos;
            return ((at.x - x).powi(2) + (at.z - z).powi(2) <= reach2)
                .then_some(Place::Node(node));
        };
        // The squared distance is read only by the `TRAFFIC-19` probe.
        #[cfg(test)]
        super::super::super::trip_failures::probe_tally::sample(|| {
            format!(
                "attach walker agent {i} node {node} lane {lane} snap {:.2} m",
                _d2.sqrt()
            )
        });
        let length = lanes[lane].length;
        enter_lane!(slices, i, transit_network, lane, length, node);
        slices.path.get_mut(i).clear();
        slices.walk_lanes.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
        Some(Place::Sidewalk(node, lane))
    }
}

// Walker `i`, off every sidewalk, sets off for the sidewalk point nearest it within
// `OFF_NETWORK_WALK_MAX_M` (`remap::nearest_sidewalk`, the road-edge R-tree): its walk is
// `ACCESS_WALK_TO_SIDEWALK` to that lane and distance on the lane's edge, with its route, exit,
// backoff and stranding cleared; its position is not changed. `Place::Off` with none in reach.
unsafe fn start_sidewalk_walk(
    i: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    slices: &MovementSlices,
) -> Place {
    unsafe {
        let pos = Vector2::new(*slices.pos_x.get(i), *slices.pos_y.get(i));
        let Some(sidewalk) = nearest_sidewalk(pos, &transit_network.lane_system, graph) else {
            return Place::Off;
        };
        let metres = sidewalk.dist_sq.sqrt();
        #[cfg(test)]
        super::super::super::trip_failures::probe_tally::add_event(
            "sidewalk walk",
            "walker",
            "starts",
        );
        *slices.planned_attach_lane.get_mut(i) = sidewalk.lane_id as u32;
        *slices.planned_attach_lane_d.get_mut(i) = sidewalk.lane_d;
        *slices.cur_e.get_mut(i) = sidewalk.edge_id;
        slices.path.get_mut(i).clear();
        slices.walk_lanes.get_mut(i).clear();
        *slices.path_idx.get_mut(i) = 0;
        *slices.planned_detach_n.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane.get_mut(i) = u32::MAX;
        *slices.planned_detach_lane_d.get_mut(i) = 0.0;
        *slices.access_flags.get_mut(i) = ACCESS_WALK_TO_SIDEWALK;
        *slices.network_replan_failures.get_mut(i) = 0;
        *slices.next_replan_time.get_mut(i) = 0.0;
        *slices.speed.get_mut(i) = AGENT_WALK_SPEED_MS;
        Place::Walking(metres)
    }
}

/// Whether agent `i` is a walker off the road walking to a sidewalk ([`walk_to_sidewalk`]). O(1).
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[inline(always)]
pub(super) unsafe fn walking_to_sidewalk(i: usize, slices: &MovementSlices) -> bool {
    unsafe {
        *slices.lane_id.get(i) == usize::MAX
            && *slices.access_flags.get(i) & ACCESS_WALK_TO_SIDEWALK != 0
    }
}

/// Walker `i` walking to a sidewalk ([`ACCESS_WALK_TO_SIDEWALK`], `TRAFFIC-19`) takes this tick's
/// step: straight across the ground towards its sidewalk point at the access walk's pace
/// (`AGENT_WALK_SPEED_MS`, the building access walk's path stepping), never further. Reaching
/// it, it steps onto the lane there and plans its trip from it: walks on, gives the trip up for
/// home, or is stranded there ([`give_up_trip`]); with no trip it waits on the lane. A walk whose
/// lane is gone (a road edit) is dropped and the walker looks again from where it stands. Returns
/// whether the walk handled this tick. O(1) for every other agent; one route query on arrival.
///
/// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
#[allow(clippy::too_many_arguments)]
// Compiled in its caller's codegen unit: its inlining does not follow the crate's partition
// (see `handle_network_movement`).
#[inline]
pub(super) unsafe fn walk_to_sidewalk(
    i: usize,
    delta: f32,
    sim_time: f32,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
    slices: &MovementSlices,
) -> bool {
    unsafe {
        if !walking_to_sidewalk(i, slices) {
            return false;
        }
        let lane_id = *slices.planned_attach_lane.get(i) as usize;
        let lane_d = *slices.planned_attach_lane_d.get(i);
        let lane = transit_network
            .lane_system
            .lanes
            .get(lane_id)
            .filter(|lane| lane.lane_type == LaneType::Foot);
        let origin = lane.and_then(|_| {
            held_attach_origin(
                lane_id,
                lane_d,
                *slices.cur_e.get(i),
                transit_network,
                graph,
            )
        });
        let target = lane.and_then(|lane| agent_lane_position(lane, lane_d, Some(i)));
        let (Some(origin), Some(target)) = (origin, target) else {
            *slices.access_flags.get_mut(i) &= !ACCESS_WALK_TO_SIDEWALK;
            *slices.planned_attach_lane.get_mut(i) = u32::MAX;
            *slices.planned_attach_lane_d.get_mut(i) = 0.0;
            return false;
        };
        let pos = Vector2::new(*slices.pos_x.get(i), *slices.pos_y.get(i));
        let mut path = LocalAccessPath {
            points: [Vector2::ZERO; 5],
            count: 2,
        };
        path.points[0] = pos;
        path.points[1] = Vector2::new(target.x, target.z);
        let (next, reached) =
            advance_along_local_access_path(pos, &path, AGENT_WALK_SPEED_MS * delta);
        *slices.pos_x.get_mut(i) = next.x;
        *slices.pos_y.get_mut(i) = next.y;
        *slices.speed.get_mut(i) = AGENT_WALK_SPEED_MS;
        if !reached {
            return true;
        }
        #[cfg(test)]
        super::super::super::trip_failures::probe_tally::add_event(
            "sidewalk walk",
            "walker",
            "arrives",
        );
        enter_lane!(slices, i, transit_network, lane_id, lane_d, origin);
        *slices.access_flags.get_mut(i) &= !ACCESS_WALK_TO_SIDEWALK;
        *slices.planned_attach_lane.get_mut(i) = u32::MAX;
        *slices.planned_attach_lane_d.get_mut(i) = 0.0;
        *slices.speed.get_mut(i) = 0.0;
        let target = *slices.tgt_b.get(i);
        let Some(node) = lane_terminal_node(lane_id, transit_network, graph) else {
            return true;
        };
        if target == usize::MAX {
            return true;
        }
        match plan_from_lane(
            i,
            node,
            lane_id,
            target,
            allocator,
            transit_network,
            graph,
            pathfind_count,
        ) {
            Ok(replan) => apply_network_replan(i, replan, slices),
            Err(failure) => {
                give_up_trip(
                    i,
                    node,
                    lane_id,
                    FailureCause::NoRoute(failure),
                    sim_time,
                    allocator,
                    transit_network,
                    graph,
                    pathfind_count,
                    slices,
                );
            }
        }
        true
    }
}

// Stranded where it stands, with no lane or on a crossing: one `cause` record, its trip given up
// for home when it has one, then the backoff.
unsafe fn strand_in_place(
    i: usize,
    cause: FailureCause,
    sim_time: f32,
    allocator: &BuildingAllocator,
    slices: &MovementSlices,
) {
    unsafe {
        let home = *slices.home.get(i);
        record_failure(
            i,
            cause,
            FailureOutcome::Stranded {
                after_s: retry_after_s(0),
            },
            slices,
        );
        if home < allocator.buildings.len() && *slices.tgt_b.get(i) != home {
            head_home(i, home, sim_time, slices);
        }
        strand(i, sim_time, slices);
    }
}

// The next backoff step of a stranded walker whose plan home failed.
unsafe fn next_backoff_step(i: usize, sim_time: f32, slices: &MovementSlices) {
    unsafe {
        let step = slices.network_replan_failures.get(i).saturating_add(1);
        *slices.network_replan_failures.get_mut(i) = step;
        *slices.next_replan_time.get_mut(i) = sim_time + retry_after_s(step);
    }
}

// A walk to `target` from the end of road lane `lane` at `node`, walkable from that lane; from
// the node itself when `lane` is `usize::MAX`.
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
        (lane != usize::MAX).then_some(lane),
        target,
        MODE_WALK,
        0,
        allocator,
        transit_network,
        graph,
        pathfind_count,
        Some((i, "walk-home")),
        None,
        None,
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

// Stranded: no path and no exit lane, so the walker walks to the end of its lane (or stands at
// its node) and waits there; the first home plan after 5 s.
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
