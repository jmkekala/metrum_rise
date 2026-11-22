// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight carriers in yards (`PARK-20`): the parking phase's ordered commits for a carrier
//! whose plan ends at a freight gate.
//!
//! A carrier bound for a building with a published dock detaches at the gate's lane point and
//! sets `ready` there (`access_state/ingress.rs`). Its visit here, in ascending agent order,
//! either grants it the building's first free dock in anchor order (`PullIn`, the dock's proven
//! route at driveway speed) or, with every dock held, sends it on to the building's kerb stop
//! (`ToKerb`, `PARK-36`), where it waits (`AtKerb`) until a dock is granted to it in arrival
//! order (`grant_docks_at_kerbs`) and it drives back to the gate. The end of the route sets
//! `ready` again, and the next visit stands it in the bay (`AtDock`), where its shipment's next
//! check delivers (`logistics/progress.rs`).
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
use crate::simulation::economy::agents::tick::kerb_queue_entrance;
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
            // Waiting at the kerb stop: this commit's queue pass has visited it already
            // (`grant_docks_at_kerbs`, `PARK-36`).
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
    // in anchor order with no holder; with every dock held, the end of the line at the kerb
    // stop, where it waits its turn (`PARK-36`). Never the door. O(docks of the building + W) for
    // W trucks waiting this tick, plus one plan for a truck sent on.
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
        let dock = granted.or_else(|| self.first_free_dock(building, parking));
        let sim_time = self.sim_time;
        let a = &mut self.agents;
        let trip = &mut a.parking_trip[i];
        trip.ready = false;
        match dock {
            Some(dock) => {
                parking.set_dock_holder(dock, me);
                trip.destination = dock;
                trip.phase = ParkingTripPhase::PullIn;
            }
            None => {
                // Its place in the queue is the time it reached the gate.
                trip.destination = Handle::NONE;
                trip.phase = ParkingTripPhase::ToKerb;
                a.journey_start_time[i] = sim_time;
                // The end of the line: behind the trucks standing there now.
                let slot = self
                    .parking_work
                    .kerb_queue
                    .iter()
                    .filter(|&&(b, _, j)| b == building && self.agents.parking_trip[j].phase == ParkingTripPhase::AtKerb)
                    .count();
                let sent = allocator
                    .building_handles
                    .resolve(Handle::from_bits(building))
                    .is_some_and(|index| {
                        self.drive_from_gate_to_kerb(i, index, slot, allocator, network, graph)
                    });
                if !sent {
                    // Its plan dropped, the ingress step takes it back onto the lane and on to
                    // the kerb stop itself (`ParkingSystem::replan_entrance`).
                    self.agents.access_flags[i] &= !crate::simulation::economy::agents::ACCESS_PLAN_VALID;
                }
            }
        }
    }

    /// The dock queue at the kerb stops (`PARK-36`), the first pass of the parking phase: every
    /// truck waiting at a kerb stop for a dock of its building is among this tick's `candidates`
    /// (it stands `ready`), and here, before any gate commit, each building's free docks go to its
    /// waiting trucks in the order they reached the gate (`journey_start_time`, then agent
    /// order), each then driving from the kerb to the gate holding its dock. A truck that reaches
    /// the gate meanwhile finds the docks granted, so it queues behind them. One that no road
    /// takes from the kerb to the gate lets the next have the dock and tries again on the retry
    /// wheel in 30 s. The trucks left waiting stand in a line along the kerb in that order, each
    /// moving up at driveway speed to its place (`kerb_queue_entrance`) as the line ahead of it
    /// leaves. O(C + W log W + W × (docks of one building + edge points) + G × Q) for C
    /// candidates, W waiting trucks and G grants; a city without docks pays one emptiness test.
    /// The list is reused.
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
        for &(building, _, i) in &waiting {
            let Some(index) = allocator.building_handles.resolve(Handle::from_bits(building))
            else {
                continue;
            };
            let Some(dock) = self.first_free_dock(building, parking) else {
                continue;
            };
            parking.set_dock_holder(dock, self.handles.handle(i));
            if !self.drive_from_kerb_to_gate(i, index, dock, allocator, network, graph) {
                parking.set_dock_holder(dock, AgentHandle::NONE);
                self.queue_parking_retry(i, 30);
            }
        }
        // The line moves up: each building's trucks still waiting, in their order, to their places.
        let step = crate::config::AGENT_DRIVEWAY_SPEED_MS * delta;
        let mut slot = 0;
        for (k, &(building, _, i)) in waiting.iter().enumerate() {
            if k > 0 && waiting[k - 1].0 != building {
                slot = 0;
            }
            if self.agents.parking_trip[i].phase != ParkingTripPhase::AtKerb {
                continue;
            }
            let place = allocator
                .building_handles
                .resolve(Handle::from_bits(building))
                .and_then(|index| allocator.entrances.get(index))
                .and_then(|door| kerb_queue_entrance(door, graph, slot));
            slot += 1;
            let Some(place) = place else {
                continue;
            };
            let at = Vector2::new(self.agents.pos_x[i], self.agents.pos_y[i]);
            let to = place.door_pos;
            let next = if at.distance_to(to) <= step { to } else { at + (to - at).normalized() * step };
            self.agents.pos_x[i] = next.x;
            self.agents.pos_y[i] = next.y;
        }
        self.parking_work.kerb_queue = waiting;
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

// Whether `trip` holds `dock`.
fn holds_dock(trip: &ParkingTrip, dock: Handle<Place>) -> bool {
    !dock.is_none() && trip.held_dock() == dock
}
