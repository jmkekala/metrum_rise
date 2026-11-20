// SPDX-License-Identifier: GPL-3.0-or-later

//! Freight carriers in yards (`PARK-20`): the parking phase's ordered commits for a carrier
//! whose plan ends at a freight gate.
//!
//! A carrier bound for a building with a published dock detaches at the gate's lane point and
//! sets `ready` there (`access_state/ingress.rs`). Its visit here, in ascending agent order,
//! either grants it the building's first free dock in anchor order (`PullIn`, the dock's proven
//! route at driveway speed) or refuses it (`GateToDoor`, the plain access path from the gate's
//! lane point to the door): yard capacity never delays a delivery. The end of the route sets
//! `ready` again, and the next visit stands it in the bay (`AtDock`), where its shipment's next
//! check delivers (`logistics/progress.rs`).
//!
//! A dock's `freight_holder` counts only while the holder's trip still names the dock in
//! `PullIn` or `AtDock`. A new plan, an arrival, the end of the building and every removal end
//! that trip, so none of them needs the parking system to free the dock: the next visit that
//! looks at the dock finds it free. The holder and the trips are saved, so the choice is the same
//! after a load and at any worker count.

use super::super::super::{AgentHandle, AgentSystem};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::parking::{ParkingSystem, ParkingTrip, ParkingTripPhase, Place};

impl AgentSystem {
    /// One freight carrier's visit, `i` with a yard trip. O(docks of its building) for the gate
    /// commit, O(1) otherwise; no allocation.
    pub(super) fn commit_freight_yard_visit(
        &mut self,
        i: usize,
        parking: &mut ParkingSystem,
        allocator: &BuildingAllocator,
    ) {
        let trip = self.agents.parking_trip[i];
        if !trip.ready {
            return;
        }
        match trip.phase {
            ParkingTripPhase::None
                if !trip.replan_needed && allocator.freight_gates.contains_key(&trip.target) =>
            {
                self.commit_freight_gate(i, trip.target, parking);
            }
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
            // The gate went before the carrier reached it, or a failure asked for recovery:
            // it goes on as a carrier without a yard, to the door from where it stands.
            _ => self.leave_freight_yard(i, parking),
        }
    }

    // The gate commit: the building's first dock in anchor order with no holder, or the door.
    // O(docks of the building).
    fn commit_freight_gate(&mut self, i: usize, building: u64, parking: &mut ParkingSystem) {
        let docks = &*parking;
        let dock = docks
            .building_docks(building)
            .find(|&dock| self.dock_is_free(dock, docks));
        let trip = &mut self.agents.parking_trip[i];
        trip.ready = false;
        match dock {
            Some(dock) => {
                parking.set_dock_holder(dock, self.handles.handle(i));
                trip.destination = dock;
                trip.phase = ParkingTripPhase::PullIn;
            }
            None => trip.phase = ParkingTripPhase::GateToDoor,
        }
    }

    // Whether `dock` has no holder: none recorded, or one whose trip no longer names it. O(1).
    fn dock_is_free(&self, dock: Handle<Place>, parking: &ParkingSystem) -> bool {
        !self.holds_freight_dock(parking.dock_holder(dock), dock)
    }

    /// Whether carrier `holder` still holds `dock`: it is alive and pulling into the dock or
    /// standing in it. A dock's recorded holder counts only while this holds (`PARK-20`). O(1).
    pub(crate) fn holds_freight_dock(&self, holder: AgentHandle, dock: Handle<Place>) -> bool {
        self.handles
            .resolve(holder)
            .is_some_and(|i| holds_dock(&self.agents.parking_trip[i], dock))
    }

    // Ends carrier `i`'s yard trip where it stands, freeing a dock it holds. O(1).
    fn leave_freight_yard(&mut self, i: usize, parking: &mut ParkingSystem) {
        let trip = self.agents.parking_trip[i];
        if holds_dock(&trip, trip.destination)
            && parking.dock_holder(trip.destination) == self.handles.handle(i)
        {
            parking.set_dock_holder(trip.destination, AgentHandle::NONE);
        }
        self.agents.parking_trip[i] = ParkingTrip::default();
    }
}

// Whether `trip` holds `dock`: a carrier pulling into it or standing in it.
fn holds_dock(trip: &ParkingTrip, dock: Handle<Place>) -> bool {
    trip.freight_gate
        && trip.destination == dock
        && matches!(trip.phase, ParkingTripPhase::PullIn | ParkingTripPhase::AtDock)
}
