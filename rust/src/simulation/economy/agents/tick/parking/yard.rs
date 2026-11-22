// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight carriers in yards (`PARK-20`): the parking phase's ordered commits for a carrier
//! whose plan ends at a freight gate.
//!
//! A carrier bound for a building with a published dock detaches at the gate's lane point and
//! sets `ready` there (`access_state/ingress.rs`). Its visit here, in ascending agent order,
//! either grants it the building's first free dock in anchor order (`PullIn`, the dock's proven
//! route at driveway speed) or, with every dock held, stands it on the way in of a drive-through
//! dock with room, behind the trucks waiting there (`AtKerb` naming the dock, `PARK-36`), or sends
//! it on to the building's kerb stop (`ToKerb`), where it waits (`AtKerb`) until a dock is granted
//! to it in arrival order (`grant_docks_at_kerbs`) and it drives back to the gate. One on a way in
//! is granted that dock when it is free and it is first in its line, and pulls on into the bay.
//! The end of the route sets `ready` again, and the next visit stands it in the bay (`AtDock`),
//! where its shipment's next check delivers (`logistics/progress.rs`).
//!
//! The check sends it back by `PullOut` (`lifecycle.rs`): the route reversed, from the stop to
//! the gate's lane point, or for a dock with a way out (`PARK-21`) that way forward, from the
//! stop to the exit gate's lane point, where the egress step ends the yard trip as it attaches
//! or waits to (`access_state/egress.rs`). A dispatch from a building with a free dock starts its carrier
//! the same way, standing in the dock it takes.
//!
//! A dock's `freight_holder` counts only while the holder's trip still names the dock in
//! `PullIn`, `AtDock` or `PullOut` (`ParkingTrip::held_dock`). A new plan, an arrival, the lane
//! point, the end of the building and every removal end that trip, so none of them needs the
//! parking system to free the dock: the next visit that looks at the dock finds it free. The
//! holder and the trips are saved, so the choice is the same after a load and at any worker
//! count.
//!
//! A truck in a yard is never moved by a failure. A pull-in whose way in goes ends in its bay; a
//! pull-out whose plan a road edit drops, or whose way onto the road goes stale, stops on its
//! route and is planned again here from the dock's own access, retrying on the parking wheel
//! while no road leaves the gate, until its shipment's leg times out
//! (`access_state.rs` `stand_in_yard`, `plan_way_out_of_dock` below).

use super::super::super::{AgentHandle, AgentSystem};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use super::super::access::{
    KERB_QUEUE_SPACING_M, advance_along_local_access_path, local_access_target_segment,
};
use crate::simulation::economy::agents::tick::{MANEUVER_POINTS, kerb_queue_entrance};
use crate::simulation::economy::agents::tick::movement::retry_after_s;
use crate::simulation::economy::agents::tick::trip_failures::{FailureCause, FailureOutcome};
use crate::simulation::network::{TransitNetwork, graph::RegionGraph};
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase, Place};
use godot::prelude::Vector2;

impl AgentSystem {
    /// One freight carrier's visit, `i` with a yard trip. O(docks of its building) for the gate
    /// commit, one plan for a pull-out planned again, O(1) otherwise; no allocation beyond the
    /// plan's route.
    pub(super) fn commit_freight_yard_visit(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let trip = self.agents.parking_trip[i];
        // A pull-out waiting on the retry wheel is visited again when the wheel wakes it.
        if !trip.ready || trip.retry_pending {
            return;
        }
        match trip.phase {
            ParkingTripPhase::None
                if !trip.replan_needed && allocator.freight_gates.contains_key(&trip.target) =>
            {
                self.commit_freight_gate(i, trip.target, parking, allocator, network, graph);
            }
            // Waiting at the kerb stop or on a drive-through way in: this commit's queue pass has
            // visited it already (`grant_docks_at_kerbs`, `PARK-36`).
            ParkingTripPhase::AtKerb if allocator.freight_gates.contains_key(&trip.target) => {}
            ParkingTripPhase::PullIn if !trip.replan_needed => {
                // The route's end, or a pull-in a road edit cut short (`PARK-11`): it ends in
                // its bay, at the route's stop.
                let Some(route) = parking.dock_route(trip.destination) else {
                    self.leave_freight_yard(i, parking);
                    return;
                };
                let [x, z] = route[3];
                let a = &mut self.agents;
                a.pos_x[i] = x;
                a.pos_y[i] = z;
                a.speed[i] = 0.0;
                a.parking_trip[i].phase = ParkingTripPhase::AtDock;
                a.parking_trip[i].ready = false;
            }
            ParkingTripPhase::PullOut => {
                self.plan_way_out_of_dock(i, parking, allocator, network, graph);
            }
            // The gate went before the carrier reached it, or a failure asked for recovery:
            // it goes on as a carrier without a yard, to the door from where it stands.
            _ => self.leave_freight_yard(i, parking),
        }
    }

    // The gate commit: the dock granted to it at the kerb stop, else the building's first dock
    // in anchor order with no holder and no truck waiting on its way in; with every dock held, a
    // place on the way in of a drive-through dock with room while nobody of the building waits
    // at the kerb, else the end of the line at the kerb stop, where it waits its turn
    // (`PARK-36`). Never the door. O(docks of the building × (W + way points)) for W trucks
    // waiting this tick, plus one plan for a truck sent on to the kerb.
    fn commit_freight_gate(
        &mut self,
        i: usize,
        building: u64,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let me = self.handles.handle(i);
        let granted = self.agents.parking_trip[i].destination;
        let granted = (!granted.is_none()
            && parking.dock_holder(granted) == me
            && parking.building_docks(building).any(|dock| dock == granted))
        .then_some(granted);
        let waiting = std::mem::take(&mut self.parking_work.kerb_queue);
        let dock = granted.or_else(|| self.first_free_unlined_dock(building, parking, &waiting));
        let way_in = dock
            .is_none()
            .then(|| self.way_in_with_room(i, building, parking, network, &waiting))
            .flatten();
        self.parking_work.kerb_queue = waiting;
        let sim_time = self.sim_time;
        let a = &mut self.agents;
        let trip = &mut a.parking_trip[i];
        trip.ready = false;
        if let Some(dock) = dock {
            parking.set_dock_holder(dock, me);
            trip.destination = dock;
            trip.phase = ParkingTripPhase::PullIn;
            return;
        }
        // Its place in the queue is the time it reached the gate.
        a.journey_start_time[i] = sim_time;
        let stamp = sim_time.to_bits();
        if let Some(dock) = way_in {
            // On the way in, behind the trucks waiting there: it stands, `ready`, where it is,
            // and the queue pass moves it up to its place from the next tick.
            trip.destination = dock;
            trip.phase = ParkingTripPhase::AtKerb;
            trip.ready = true;
            self.parking_work.kerb_queue.push((building, stamp, i));
            return;
        }
        trip.destination = Handle::NONE;
        trip.phase = ParkingTripPhase::ToKerb;
        // The end of the line: behind the trucks standing there now and those sent there
        // earlier this tick.
        let slot = self
            .parking_work
            .kerb_queue
            .iter()
            .filter(|&&(b, _, j)| b == building && self.waits_at_kerb(j))
            .count();
        self.parking_work.kerb_queue.push((building, stamp, i));
        let sent = allocator
            .building_handles
            .resolve(Handle::from_bits(building))
            .is_some_and(|index| self.drive_from_gate_to_kerb(i, index, slot, allocator, network, graph));
        if !sent {
            // Its plan dropped, the ingress step takes it back onto the lane and on to the kerb
            // stop itself (`ParkingSystem::replan_entrance`).
            self.agents.access_flags[i] &= !crate::simulation::economy::agents::ACCESS_PLAN_VALID;
        }
    }

    // Whether truck `j` waits for a dock at its building's kerb stop or is on its way there
    // (`PARK-36`): a waiting yard trip that names no dock. O(1).
    fn waits_at_kerb(&self, j: usize) -> bool {
        let trip = &self.agents.parking_trip[j];
        trip.freight_gate
            && trip.destination.is_none()
            && matches!(trip.phase, ParkingTripPhase::AtKerb | ParkingTripPhase::ToKerb)
    }

    // The truck waiting on the way in of `dock` (`PARK-36`): a yard trip standing `AtKerb` that
    // names the dock. O(1).
    fn waits_on_way_in(&self, j: usize, dock: Handle<Place>) -> bool {
        let trip = &self.agents.parking_trip[j];
        trip.freight_gate && trip.phase == ParkingTripPhase::AtKerb && trip.destination == dock
    }

    // The first dock of building `building` with no holder and no truck of `waiting` on its way
    // in: a freed drive-through dock goes to the first truck of its own line (`PARK-36`). O(docks
    // × W).
    fn first_free_unlined_dock(
        &self,
        building: u64,
        parking: &ParkingSystem,
        waiting: &[(u64, u32, usize)],
    ) -> Option<Handle<Place>> {
        parking.building_docks(building).find(|&dock| {
            !self.holds_freight_dock(parking.dock_holder(dock), dock)
                && !waiting.iter().any(|&(b, _, j)| b == building && self.waits_on_way_in(j, dock))
        })
    }

    // The drive-through dock (`PARK-21`: a way out to its own exit gate, entered nose first) of
    // building `building` whose way in holds one more truck behind those waiting there, for
    // truck `i` at the gate's lane point, first in anchor order; none while any truck of the
    // building waits at the kerb stop or is on its way there, so the kerb line is never passed
    // (`PARK-36`). O(docks × (W + way points)).
    fn way_in_with_room(
        &self,
        i: usize,
        building: u64,
        parking: &ParkingSystem,
        network: &TransitNetwork,
        waiting: &[(u64, u32, usize)],
    ) -> Option<Handle<Place>> {
        if waiting.iter().any(|&(b, _, j)| b == building && self.waits_at_kerb(j)) {
            return None;
        }
        let a = &self.agents;
        let (lane, d) = (a.planned_detach_lane_id[i] as usize, a.planned_detach_lane_d[i]);
        parking.building_docks(building).find(|&dock| {
            if !parking.dock_leaves_forward(dock) || parking.dock_backs_in(dock) {
                return false;
            }
            let ahead = waiting
                .iter()
                .filter(|&&(b, _, j)| b == building && self.waits_on_way_in(j, dock))
                .count();
            let back = (ahead + 1) as f32 * KERB_QUEUE_SPACING_M;
            parking
                .maneuver_path(dock, lane, d, network, true, true)
                .is_some_and(|path| way_in_place(path.as_slice(), back).is_some())
        })
    }

    /// The dock queue at the kerb stops and on the drive-through ways in (`PARK-36`), the first
    /// pass of the parking phase: every truck waiting for a dock of its building is among this
    /// tick's `candidates` (it stands `ready`), and here, before any gate commit, the waiting
    /// trucks are taken in the order they reached the gate (`journey_start_time`, then agent
    /// order). One on the way in of a drive-through dock waits for that dock only: once it is
    /// free the first truck of its line is granted it and pulls on into the bay from where it
    /// stands. One at the kerb takes the building's first free dock with no line on its way in
    /// and drives from the kerb to the gate holding it. A truck that reaches the gate meanwhile
    /// finds the docks granted, so it queues behind them. One that no road takes from the kerb
    /// to the gate lets the next have the dock and tries again on the retry wheel in 30 s. The
    /// trucks left waiting move up at driveway speed to their places in the line at the kerb
    /// (`kerb_queue_entrance`), and at the yard speed along the way in to theirs (`way_in_place`),
    /// as the trucks ahead leave. For C candidates, W waiting trucks and G grants:
    /// O(C + W log W + W × (docks of one building × W + way points) + G × Q); a city without
    /// docks pays one emptiness test. The list is reused.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn grant_docks_at_kerbs(
        &mut self,
        candidates: &[usize],
        delta: f32,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        if allocator.freight_gates.is_empty() {
            self.parking_work.kerb_queue.clear();
            return;
        }
        let mut waiting = std::mem::take(&mut self.parking_work.kerb_queue);
        waiting.clear();
        waiting.extend(candidates.iter().filter_map(|&i| {
            let trip = &self.agents.parking_trip[i];
            (trip.freight_gate
                && trip.phase == ParkingTripPhase::AtKerb
                && trip.ready
                && !trip.retry_pending)
                .then(|| (trip.target, self.agents.journey_start_time[i].to_bits(), i))
        }));
        // Stamps are times since the start, never negative, so their bits order as they do.
        waiting.sort_unstable();
        for k in 0..waiting.len() {
            let (building, _, i) = waiting[k];
            let Some(index) = allocator.building_handles.resolve(Handle::from_bits(building))
            else {
                continue;
            };
            let line = self.agents.parking_trip[i].destination;
            if !line.is_none() {
                if !parking.building_docks(building).any(|dock| dock == line) {
                    // Its dock was retired under it by an edit: it joins the line at the kerb.
                    self.agents.parking_trip[i].destination = Handle::NONE;
                } else if !self.holds_freight_dock(parking.dock_holder(line), line) {
                    // The first of its line (the others find the dock held): on into the bay.
                    parking.set_dock_holder(line, self.handles.handle(i));
                    let trip = &mut self.agents.parking_trip[i];
                    trip.phase = ParkingTripPhase::PullIn;
                    trip.ready = false;
                }
                continue;
            }
            let Some(dock) = self.first_free_unlined_dock(building, parking, &waiting) else {
                continue;
            };
            parking.set_dock_holder(dock, self.handles.handle(i));
            if !self.drive_from_kerb_to_gate(i, index, dock, allocator, network, graph) {
                parking.set_dock_holder(dock, AgentHandle::NONE);
                self.queue_parking_retry(i, 30);
            }
        }
        self.move_lines_up(&waiting, delta, parking, allocator, network, graph);
        self.parking_work.kerb_queue = waiting;
    }

    // The lines move up: each building's trucks still waiting, in their order, to their places,
    // at the kerb at driveway speed and on each drive-through way in at the yard speed. O(W ×
    // (W of one building + edge points + way points)).
    fn move_lines_up(
        &mut self,
        waiting: &[(u64, u32, usize)],
        delta: f32,
        parking: &ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let mut slot = 0;
        for (k, &(building, _, i)) in waiting.iter().enumerate() {
            if k > 0 && waiting[k - 1].0 != building {
                slot = 0;
            }
            if self.agents.parking_trip[i].phase != ParkingTripPhase::AtKerb {
                continue;
            }
            let at = Vector2::new(self.agents.pos_x[i], self.agents.pos_y[i]);
            let line = self.agents.parking_trip[i].destination;
            let next = if line.is_none() {
                let place = allocator
                    .building_handles
                    .resolve(Handle::from_bits(building))
                    .and_then(|index| allocator.entrances.get(index))
                    .and_then(|door| kerb_queue_entrance(door, graph, slot));
                slot += 1;
                let Some(place) = place else {
                    continue;
                };
                let step = crate::config::AGENT_DRIVEWAY_SPEED_MS * delta;
                let to = place.door_pos;
                if at.distance_to(to) <= step { to } else { at + (to - at).normalized() * step }
            } else {
                // Its place: behind the bay and each truck of its line ahead of it, and never
                // nearer than a body and 1 m behind the truck just ahead of it, which may still
                // be on its way up (the one before it in the line, moved already this tick, or
                // for the first the truck pulling into the bay).
                let mut ahead = waiting[..k]
                    .iter()
                    .rev()
                    .take_while(|&&(b, _, _)| b == building)
                    .filter(|&&(_, _, j)| self.waits_on_way_in(j, line));
                let places = ahead.clone().count() + 1;
                let next = ahead.next().map(|&(_, _, j)| j).or_else(|| {
                    self.handles
                        .resolve(parking.dock_holder(line))
                        .filter(|&j| {
                            let trip = &self.agents.parking_trip[j];
                            trip.phase == ParkingTripPhase::PullIn && trip.destination == line
                        })
                });
                let a = &self.agents;
                let lane = a.planned_detach_lane_id[i] as usize;
                let Some(path) =
                    parking.maneuver_path(line, lane, a.planned_detach_lane_d[i], network, true, true)
                else {
                    continue;
                };
                let path = path.as_slice();
                let mut back = places as f32 * KERB_QUEUE_SPACING_M;
                if let Some(j) = next {
                    let at_j = Vector2::new(a.pos_x[j], a.pos_y[j]);
                    back = back.max(left_along(path, at_j) + KERB_QUEUE_SPACING_M);
                }
                // A truck ahead still near the gate: it stands where it is until there is room.
                let Some((points, count)) = way_in_place(path, back) else {
                    continue;
                };
                let step = crate::config::FREIGHT_YARD_SPEED_MS * delta;
                advance_along_local_access_path(at, &points[..count], step).0
            };
            self.agents.pos_x[i] = next.x;
            self.agents.pos_y[i] = next.y;
        }
    }

    /// The first active dock of the building whose handle bits are `building`, in anchor order,
    /// with no holder: none recorded, or one whose trip no longer names it (`PARK-20`). The gate
    /// commit grants it, and a dispatch from the building spawns its carrier in it. O(docks of
    /// the building), no allocation.
    pub(crate) fn first_free_dock(
        &self,
        building: u64,
        parking: &ParkingSystem,
    ) -> Option<Handle<Place>> {
        parking
            .building_docks(building)
            .find(|&dock| !self.holds_freight_dock(parking.dock_holder(dock), dock))
    }

    /// Whether carrier `holder` still holds `dock`: it is alive and pulling into the dock,
    /// standing in it or pulling out of it, short of the lane (`ParkingTrip::held_dock`). A
    /// dock's recorded holder counts only while this holds (`PARK-20`). O(1).
    pub(crate) fn holds_freight_dock(&self, holder: AgentHandle, dock: Handle<Place>) -> bool {
        self.handles
            .resolve(holder)
            .is_some_and(|i| holds_dock(&self.agents.parking_trip[i], dock))
    }

    // A truck that stopped while leaving its dock (`stand_in_yard`: a road edit dropped its
    // plan, its way onto the road went stale, or its road went) plans its way out again from the
    // dock's own departure (`dock_departure`): its exit gate's road connection for a dock with a
    // way out (`PARK-21`), else its gate's, which outlives the building's gate record (an
    // inactive row keeps it). It keeps its place on its way and its dock. With no
    // way out it stays, holding the dock, and tries again on the retry wheel with the stranded
    // walkers' backoff (5 s, 30 s, then every 2 min; the step in `network_replan_failures`),
    // recorded once (`TRAFFIC-32`). The shipment's leg timeout bounds the wait: its check
    // expires a dispatch or finishes a return and removes the truck, which frees the dock. One
    // plan per visit, which runs only on a wake or after the failure, never every tick.
    fn plan_way_out_of_dock(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let dock = self.agents.parking_trip[i].source;
        let building = self.agents.current_building[i];
        let road_stands = allocator.buildings.get(building).is_some_and(|b| {
            graph.get_edge(b.edge_idx).is_some_and(|edge| !edge.deleted)
        });
        if road_stands
            && let Some(exit) = parking.dock_departure(dock, network, graph)
            && self.replan_freight_pull_out(i, &exit, allocator, network, graph)
        {
            self.record_parking_outcome(i, None, FailureOutcome::Replans);
            return;
        }
        let step = self.agents.network_replan_failures[i];
        let after_s = retry_after_s(step);
        self.agents.network_replan_failures[i] = step.saturating_add(1);
        self.record_parking_outcome(
            i,
            (step == 0).then_some(FailureCause::NoRouteFromDock),
            FailureOutcome::StandsInYard { after_s },
        );
        self.queue_parking_retry(i, after_s as u16);
    }

    // Ends carrier `i`'s yard trip where it stands, freeing a dock it holds. O(1).
    fn leave_freight_yard(&mut self, i: usize, parking: &mut ParkingSystem) {
        let dock = self.agents.parking_trip[i].held_dock();
        if !dock.is_none() && parking.dock_holder(dock) == self.handles.handle(i) {
            parking.set_dock_holder(dock, AgentHandle::NONE);
        }
        self.agents.parking_trip[i] = ParkingTrip::default();
    }
}

// The place `back` metres back from the stop along a drive-through dock's way in (`PARK-36`),
// where a waiting truck stands: `path` is the pull-in's polyline (the gate's lane point, then the
// way in from the gate's road connection to the stop). Place `k` of the line is `k` spacings
// (`KERB_QUEUE_SPACING_M`, a body and 1 m) back, so its truck stands 1 m behind the one in the
// bay or at the place ahead. Returns the polyline cut at the place, on the stack, along which a
// waiting truck moves up and on which its pull-in goes on; `None` when the place's truck would
// not stand wholly on the way in (half a body behind the place past the gate's road
// connection): the way in is full. O(way points), no allocation.
fn way_in_place(path: &[Vector2], back: f32) -> Option<([Vector2; MANEUVER_POINTS], usize)> {
    let way: f32 = path.get(1..)?.windows(2).map(|w| w[0].distance_to(w[1])).sum();
    if back + crate::config::TRUCK_BODY_LENGTH_M * 0.5 > way {
        return None;
    }
    // Back from the stop, segment by segment, to the one the place lies on.
    let (mut j, mut left) = (path.len() - 1, back);
    while j > 1 && left > path[j - 1].distance_to(path[j]) {
        left -= path[j - 1].distance_to(path[j]);
        j -= 1;
    }
    let seg = path[j - 1].distance_to(path[j]);
    let t = if seg > 1e-6 { left / seg } else { 0.0 };
    let mut points = [Vector2::ZERO; MANEUVER_POINTS];
    points[..j].copy_from_slice(&path[..j]);
    points[j] = path[j] + (path[j - 1] - path[j]) * t;
    Some((points, j + 1))
}

// How far `pos` is from the end of `path` along it: 0 at the end, infinite off it (a truck not on
// this way in). O(points).
fn left_along(path: &[Vector2], pos: Vector2) -> f32 {
    match local_access_target_segment(pos, path) {
        Some(k) => {
            pos.distance_to(path[k])
                + path[k..].windows(2).map(|w| w[0].distance_to(w[1])).sum::<f32>()
        }
        None if path.last().is_some_and(|end| end.distance_to(pos) <= 0.05) => 0.0,
        None => f32::INFINITY,
    }
}

// Whether `trip` holds `dock`.
fn holds_dock(trip: &ParkingTrip, dock: Handle<Place>) -> bool {
    !dock.is_none() && trip.held_dock() == dock
}
