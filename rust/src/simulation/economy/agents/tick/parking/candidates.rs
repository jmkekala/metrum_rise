// SPDX-License-Identifier: GPL-2.0-only

//! Which agents the ordered parking phase visits, found at events rather than by a scan.
//!
//! The phase in `parking.rs` acts on an agent when, with no retry pending, one of these holds:
//! the agent is `ready` (`replan_needed` is only ever set with it); it is on its way out of a
//! building (`phase` `None`, departure due); it walks to a door and is inside; or it drives with
//! the bay it left still held. Each of these has its source here, and test builds check after
//! every tick that no agent in such a state was missed (`audit_parking_candidates`).
//!
//! - **Movement.** An agent's step ends in [`MovementSlices::note_parking_step`], which reads
//!   the row the step has just used, so a site that sets `ready` needs no push of its own. An
//!   agent frozen by `ready` is noted instead of stepped. Notes go into the running shard's
//!   [`TripQueryScratch`] list, and the shards are merged and sorted before the phase walks them.
//! - **Timing wheel.** A retry that comes due is a wake for the next commit.
//! - **Custody.** `ParkingSystem` reports lost reservations; those agents stay on `watch` until
//!   they recover or leave the car. Cars it reports as changed get `has_car` rewritten for the
//!   agents holding them, at the point the old pass rewrote it: just before the tick.
//! - **Rows written outside these events** (load, undo): `rebuild` runs one O(agents) pass.
//!
//! Per tick O(C log C + W) for C candidates and W watched agents, and no pass over the
//! population; the lists are reused, growing only past their largest tick so far.

use super::super::super::{AGE_CHILD, TRANSIT_IN_BUILDING};
use super::super::slices::MovementSlices;
use super::super::TripQueryScratch;
use crate::simulation::core::handles::Handle;
use crate::simulation::economy::agents::{AgentHandle, AgentSystem};
use crate::simulation::network::TransitNetwork;
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase, Vehicle};
use rayon::prelude::*;

/// The parking phase's reused lists. Derived: a load rebuilds them, nothing saves them.
pub(crate) struct ParkingWork {
    // This tick's merged candidates, in ascending agent order once sorted.
    merged: Vec<usize>,
    // Agents the next commit visits though movement will not note them.
    wakes: Vec<AgentHandle>,
    // Agents driving towards a reservation they may have lost.
    watch: Vec<AgentHandle>,
    // Agent rows changed without events; the next tick rebuilds what they imply.
    rebuild: bool,
    // Rows the last commit walked, for tests of the bound.
    #[cfg(test)]
    pub(in crate::simulation::economy::agents) last_candidates: usize,
}

impl Default for ParkingWork {
    fn default() -> Self {
        Self {
            merged: Vec::new(),
            wakes: Vec::new(),
            watch: Vec::new(),
            rebuild: true,
            #[cfg(test)]
            last_candidates: 0,
        }
    }
}

fn on_a_car_leg(trip: &ParkingTrip) -> bool {
    matches!(
        trip.phase,
        ParkingTripPhase::PullOut | ParkingTripPhase::Drive | ParkingTripPhase::PullIn
    )
}

impl MovementSlices<'_> {
    /// Notes an agent the pass does not step: `ready` freezes it until the phase commits it, and
    /// a commit that could not finish is tried again next tick. O(1).
    ///
    /// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
    #[inline(always)]
    pub(in super::super) unsafe fn note_frozen_parking_agent(&self, i: usize) {
        unsafe {
            let trip = self.parking_trip.get(i);
            if self.parking.is_some() && trip.ready && !trip.retry_pending {
                TripQueryScratch::note_parking_candidate(i);
            }
        }
    }

    /// Notes agent `i` when its step left it due in the parking phase. `pulling_out` is whether
    /// it began the step in `PullOut`: the first tick of `Drive` releases the bay. O(1), on the
    /// row the step has just used.
    ///
    /// Safety: `i` must be unique to the current worker for every raw slice in `slices`.
    #[inline(always)]
    pub(in super::super) unsafe fn note_parking_step(&self, i: usize, pulling_out: bool, sim_time: f32) {
        unsafe {
            if self.parking.is_none() {
                return;
            }
            let trip = self.parking_trip.get(i);
            if trip.retry_pending {
                return;
            }
            let due = trip.ready
                || (pulling_out && trip.phase == ParkingTripPhase::Drive)
                || (*self.transit.get(i) == TRANSIT_IN_BUILDING
                    && match trip.phase {
                        ParkingTripPhase::WalkToDoor => true,
                        ParkingTripPhase::None => {
                            *self.planned_tgt_b.get(i) != usize::MAX
                                && sim_time >= *self.next_replan_time.get(i)
                                && *self.freight_shipment_id.get(i) == u64::MAX
                                && *self.pending_household_size.get(i) == 0
                        }
                        _ => false,
                    });
            if due {
                TripQueryScratch::note_parking_candidate(i);
            }
        }
    }
}

impl AgentSystem {
    /// Visits `agent` at the next commit, for a retry the timing wheel just released. O(1).
    pub(crate) fn wake_parking_agent(&mut self, agent: AgentHandle) {
        self.parking_work.wakes.push(agent);
    }

    /// Asks the next tick to rebuild the candidate state after agent rows were replaced
    /// directly (load, undo). The tick's pass is O(agents), once.
    pub(crate) fn rebuild_parking_work(&mut self) {
        self.parking_work.rebuild = true;
    }

    /// Rewrites `has_car` for the agents holding `car`, a car whose custody changed: the members
    /// of `household`, or with none an arrival carrier bringing the car in. O(members), or
    /// O(arrival carriers) for a car no household owns yet.
    pub(crate) fn refresh_car_holders(
        &mut self,
        car: Handle<Vehicle>,
        household: Option<usize>,
        parking: &ParkingSystem,
    ) {
        if let Some(household) = household {
            let mut member = self.first_household_member(household);
            while member != crate::simulation::economy::agents::NO_MEMBER {
                self.refresh_has_car(member, parking);
                member = self.next_household_member(member);
            }
            return;
        }
        for carrier in 0..self.arrival_carriers.len() {
            if let Some(i) = self.resolve_agent(self.arrival_carriers[carrier])
                && self.agents.parking_trip[i].vehicle == car
            {
                self.refresh_has_car(i, parking);
            }
        }
    }

    fn refresh_has_car(&mut self, i: usize, parking: &ParkingSystem) {
        #[cfg(test)]
        super::super::contract::add(super::super::contract::Phase::ParkingEligibilityAgents, 1);
        self.agents.has_car[i] = self.agents.age_group[i] != AGE_CHILD
            && parking.available_to(self.agents.parking_trip[i].vehicle, self.handles.handle(i));
    }

    /// Before movement: flags the drivers whose reservation is gone. O(W), W watched agents.
    pub(in super::super) fn prepare_parking_movement(
        &mut self,
        parking: &mut ParkingSystem,
        network: &TransitNetwork,
    ) {
        if self.parking_work.rebuild {
            self.rebuild_parking_lists(parking);
        }
        self.parking_work.watch.append(&mut parking.lost_reservations);
        #[cfg(test)]
        super::super::contract::add(
            super::super::contract::Phase::ParkingEligibilityAgents,
            self.parking_work.watch.len(),
        );
        let (handles, agents) = (&self.handles, &mut self.agents);
        self.parking_work.watch.retain(|&agent| {
            let Some(i) = handles.resolve(agent) else {
                return false;
            };
            let trip = &mut agents.parking_trip[i];
            if !on_a_car_leg(trip) || parking.trip_reserved(trip.vehicle, trip.destination) {
                return false;
            }
            // A junction connector is no place to replan from: the flags wait for its end.
            let on_connector = trip.phase == ParkingTripPhase::Drive
                && network
                    .lane_system
                    .lanes
                    .get(agents.current_lane_id[i])
                    .is_some_and(|lane| lane.edge_id == usize::MAX);
            if !on_connector {
                trip.ready = true;
                trip.replan_needed = true;
            }
            true
        });
        #[cfg(all(test, debug_assertions))]
        self.audit_parking_flags(parking, network);
    }

    // Everything the events would have told the next tick, from the rows. O(agents), once per
    // load or undo. The list pass is serial because its output is ordered lists it pushes to.
    fn rebuild_parking_lists(&mut self, parking: &ParkingSystem) {
        self.parking_work.rebuild = false;
        let handles = &self.handles;
        let ages = &self.agents.age_group;
        self.agents
            .has_car
            .par_iter_mut()
            .zip(self.agents.parking_trip.par_iter())
            .enumerate()
            .for_each(|(i, (flag, trip))| {
                *flag = ages[i] != AGE_CHILD && parking.available_to(trip.vehicle, handles.handle(i));
            });
        for i in 0..self.agents.len() {
            let trip = self.agents.parking_trip[i];
            if !on_a_car_leg(&trip) {
                continue;
            }
            let agent = self.handles.handle(i);
            self.parking_work.watch.push(agent);
            if trip.phase == ParkingTripPhase::Drive && parking.vehicle_place(trip.vehicle).is_some() {
                self.parking_work.wakes.push(agent);
            }
        }
    }

    /// This tick's candidates in ascending agent order, whatever the worker count: the shards'
    /// notes and the wakes, merged and sorted. O(C log C).
    pub(super) fn take_parking_candidates(&mut self) -> Vec<usize> {
        let mut merged = std::mem::take(&mut self.parking_work.merged);
        for shard in &mut self.movement_query_shards {
            shard.drain_parking_candidates(&mut merged);
        }
        self.movement_serial_query.drain_parking_candidates(&mut merged);
        let handles = &self.handles;
        merged.extend(
            self.parking_work
                .wakes
                .drain(..)
                .filter_map(|agent| handles.resolve(agent)),
        );
        merged.sort_unstable();
        merged.dedup();
        #[cfg(test)]
        {
            self.parking_work.last_candidates = merged.len();
            super::super::contract::add(
                super::super::contract::Phase::ParkingTransitionAgents,
                merged.len(),
            );
        }
        merged
    }

    /// Hands the walked list back for reuse. A car the commit could not release from its bay,
    /// because a replan went first, tries again next tick. O(C).
    pub(super) fn finish_parking_candidates(
        &mut self,
        mut candidates: Vec<usize>,
        parking: &ParkingSystem,
    ) {
        for &i in &candidates {
            let trip = self.agents.parking_trip[i];
            if trip.phase == ParkingTripPhase::Drive
                && !trip.retry_pending
                && parking.vehicle_place(trip.vehicle).is_some()
            {
                self.parking_work.wakes.push(self.handles.handle(i));
            }
        }
        candidates.clear();
        self.parking_work.merged = candidates;
    }

    // The reference the lists replace, full scans: an agent the old passes would have acted on
    // and the events missed is a bug. Debug test builds only: `PERF-06` times release test code.
    #[cfg(all(test, debug_assertions))]
    pub(super) fn audit_parking_candidates(&self, candidates: &[usize], parking: &ParkingSystem) {
        for i in 0..self.agents.len() {
            let trip = self.agents.parking_trip[i];
            let due = !trip.retry_pending
                && (trip.ready
                    || trip.replan_needed
                    || match trip.phase {
                        ParkingTripPhase::None => {
                            self.agents.freight_shipment_id[i] == u64::MAX
                                && self.agents.pending_household_size[i] == 0
                                && self.agents.transit[i] == TRANSIT_IN_BUILDING
                                && self.agents.planned_target_building[i] != usize::MAX
                                && self.sim_time >= self.agents.next_replan_time[i]
                        }
                        ParkingTripPhase::Drive => parking.vehicle_place(trip.vehicle).is_some(),
                        ParkingTripPhase::WalkToDoor => {
                            self.agents.transit[i] == TRANSIT_IN_BUILDING
                        }
                        _ => false,
                    });
            assert!(
                !due || candidates.binary_search(&i).is_ok(),
                "parking candidate missed: agent {i} {trip:?}"
            );
        }
    }

    #[cfg(all(test, debug_assertions))]
    fn audit_parking_flags(&self, parking: &ParkingSystem, network: &TransitNetwork) {
        for i in 0..self.agents.len() {
            let trip = self.agents.parking_trip[i];
            let lost = on_a_car_leg(&trip)
                && !parking.trip_reserved(trip.vehicle, trip.destination)
                && !(trip.phase == ParkingTripPhase::Drive
                    && network
                        .lane_system
                        .lanes
                        .get(self.agents.current_lane_id[i])
                        .is_some_and(|lane| lane.edge_id == usize::MAX));
            assert!(
                !lost || (trip.ready && trip.replan_needed),
                "lost reservation missed: agent {i} {trip:?}"
            );
        }
    }

    /// Every agent's `has_car` against its definition, after the custody flush. A pending
    /// rebuild rewrites it in the tick, as the pass this replaced did after a load.
    #[cfg(all(test, debug_assertions))]
    pub(crate) fn audit_has_car(&self, parking: &ParkingSystem) {
        if self.parking_work.rebuild {
            return;
        }
        for i in 0..self.agents.len() {
            let want = self.agents.age_group[i] != AGE_CHILD
                && parking.available_to(self.agents.parking_trip[i].vehicle, self.handles.handle(i));
            assert_eq!(
                self.agents.has_car[i], want,
                "has_car stale: agent {i} {:?}",
                self.agents.parking_trip[i]
            );
        }
    }
}
