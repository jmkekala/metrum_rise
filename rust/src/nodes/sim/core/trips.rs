// SPDX-License-Identifier: GPL-2.0-only

//! Trips as timed events (`SIM-02-E`, `docs/simulation.md#deadlines-and-timed-events`).
//!
//! The hour's actions slot decides which households arrive, and the `admissions` queue
//! creates their carriers over the hour (`SIM-02-J`); none leaves at once: each slot's carriers
//! are held and listed, and [`SimCore::schedule_arrival_departures`] spaces them evenly over
//! the hour after their slot. Freight, shoppers and
//! visitors are planned by request queues in any slot (`SIM-02-J`); each carrier the freight
//! queue loads leaves near its shipment id's slot of the hour ahead (`spread_slot`), moved to
//! the least busy nearby minute (`MinuteLeveler`), and shoppers and visitors leave as
//! `household_requests.rs` schedules them. A shipment has one live trip event, whose slot it
//! records; another event of it is a leftover and does nothing. A trip is then checked until it
//! arrives, so deliveries, pickups and move-ins happen when the agent gets there instead of at
//! the next hour. Rechecks back off with the trip's age, so a trip stuck in traffic is checked
//! every few minutes. Each event costs O(1) plus the arrival it settles.

use super::frame::EconomyObserver;
use super::schedule::{SimEvent, event_slot};
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::{
    SLOTS_PER_HOUR, SLOTS_PER_MINUTE, TRIP_CHECK_MAX_SLOTS, TRIP_CHECK_SLOTS, paced_slot,
    spread_slot,
};
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::Household;
use crate::simulation::economy::logistics::Shipment;
use crate::simulation::economy::logistics::ShipmentStatus;

/// One step of a trip, keyed by the entity that owns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TripEvent {
    /// Loaded shipment `shipment` sends its carrier.
    FreightDispatch { shipment: u64 },
    /// Checks whether shipment `shipment`'s carrier arrived.
    FreightCheck { shipment: u64 },
    /// `household`'s held shopper sets out for the reserved store.
    ShopperDeparture { household: Handle<Household> },
    /// Checks whether `household`'s shopper reached the store or got home.
    ShopperCheck { household: Handle<Household> },
    /// A held service visitor sets out.
    VisitDeparture { visitor: AgentHandle },
    /// The visit ends: the visitor heads home once it got there.
    VisitReturn { visitor: AgentHandle },
    /// A household arrival carrier leaves the border.
    ArrivalDeparture { carrier: AgentHandle },
    /// Checks whether an arrival carrier reached its home.
    ArrivalCheck { carrier: AgentHandle },
}

impl TripEvent {
    // Saved kinds 2..=9 follow the construction events in `SimEvent::to_saved`.
    pub(super) fn to_saved(self) -> (u16, u64) {
        match self {
            Self::FreightDispatch { shipment } => (2, shipment),
            Self::FreightCheck { shipment } => (3, shipment),
            Self::ShopperDeparture { household } => (4, household.to_bits()),
            Self::ShopperCheck { household } => (5, household.to_bits()),
            Self::VisitDeparture { visitor } => (6, visitor.to_bits()),
            Self::VisitReturn { visitor } => (7, visitor.to_bits()),
            Self::ArrivalDeparture { carrier } => (8, carrier.to_bits()),
            Self::ArrivalCheck { carrier } => (9, carrier.to_bits()),
        }
    }

    pub(super) fn from_saved(kind: u16, a: u64) -> Option<Self> {
        Some(match kind {
            2 => Self::FreightDispatch { shipment: a },
            3 => Self::FreightCheck { shipment: a },
            4 => Self::ShopperDeparture {
                household: Handle::from_bits(a),
            },
            5 => Self::ShopperCheck {
                household: Handle::from_bits(a),
            },
            6 => Self::VisitDeparture {
                visitor: Handle::from_bits(a),
            },
            7 => Self::VisitReturn {
                visitor: Handle::from_bits(a),
            },
            8 => Self::ArrivalDeparture {
                carrier: Handle::from_bits(a),
            },
            9 => Self::ArrivalCheck {
                carrier: Handle::from_bits(a),
            },
            _ => return None,
        })
    }
}

impl SimCore {
    /// One slot of the `freight requests` queue (`SIM-02-J`): plans up to `quota` queued
    /// requests, then schedules each loaded carrier's departure at its shipment id's slot of the
    /// hour ahead. O(1) with nothing queued; see `ShipmentSystem::serve_freight_requests`.
    pub(super) fn serve_freight_request_queue(&mut self, slot: u64, quota: usize) {
        let (_, minute_of_day) = super::schedule::slot_calendar(slot);
        self.logistics.serve_freight_requests(
            quota,
            &mut self.allocator,
            &self.transit_network,
            &self.region_graph,
            minute_of_day,
            &mut self.treasury.balance,
        );
        self.schedule_freight_dispatches(slot);
    }

    // Each carrier loaded since the last call leaves near its shipment id's offset in the hour
    // after `slot`, in the least busy minute nearby. The list is drained in place so its
    // capacity is reused. O(window) per carrier.
    fn schedule_freight_dispatches(&mut self, slot: u64) {
        let mut dispatches = std::mem::take(&mut self.logistics.held_dispatches);
        for shipment in dispatches.drain(..) {
            let departure = self
                .logistics
                .departure_minutes
                .place(slot, spread_slot(slot, shipment));
            self.schedule_freight_trip(
                departure,
                shipment,
                TripEvent::FreightDispatch { shipment },
            );
        }
        self.logistics.held_dispatches = dispatches;
    }

    // Schedules `trip` of shipment `shipment` at `slot`, or the wheel's next slot if that is
    // later, moved out of a queue-free slot (`event_slot`), and records the slot on the shipment
    // as its one live trip event. O(log S).
    fn schedule_freight_trip(&mut self, slot: u64, shipment: u64, trip: TripEvent) {
        let slot = event_slot(slot.max(self.events.next_slot()));
        self.logistics.set_trip_event_slot(shipment, slot);
        self.schedule_trip(slot, trip);
    }

    /// Resumes the trips of shipments an undo restored, whose events may have fired and found
    /// nothing while they were removed: a loaded shipment without a carrier departs at its
    /// recorded slot, or at the next slot if that has passed, and one with a carrier is checked
    /// at the next slot. A departure still pending at the recorded slot fires twice there, and
    /// the second finds the carrier gone; a check still pending is a leftover once the new one
    /// is recorded. O(restored shipments × log S).
    pub(crate) fn resume_restored_freight(&mut self, shipment_ids: &[u64]) {
        let next_slot = self.time.absolute_slot() + 1;
        for &shipment in shipment_ids {
            let Some(idx) = self.logistics.shipment_index(shipment) else {
                continue;
            };
            let restored = &self.logistics.shipments[idx];
            if !restored.status.is_open() {
                continue;
            }
            if !restored.carrier.is_none() {
                self.schedule_freight_trip(
                    next_slot,
                    shipment,
                    TripEvent::FreightCheck { shipment },
                );
            } else if restored.status == ShipmentStatus::InTransit && restored.cargo_loaded {
                let departure = restored.trip_event_slot.max(next_slot);
                self.schedule_freight_trip(
                    departure,
                    shipment,
                    TripEvent::FreightDispatch { shipment },
                );
            }
        }
    }

    /// Paces the departures of the arrival carriers admission launched, evenly over the hour
    /// after `slot`, in the order admission listed them. The `admissions` queue creates the
    /// hour's admitted households over the hour and calls this in each slot that created one
    /// (`SIM-02-J`), so no carrier is still held when the slot ends. Freight, shoppers and
    /// visitors leave from their queues.
    pub(super) fn schedule_arrival_departures(&mut self, slot: u64) {
        // The list is drained in place so its capacity is reused the next hour.
        let mut arrivals = std::mem::take(&mut self.agents.held_arrivals);
        let count = arrivals.len();
        for (index, carrier) in arrivals.drain(..).enumerate() {
            self.schedule_trip(
                paced_slot(slot, index, count),
                TripEvent::ArrivalDeparture { carrier },
            );
        }
        self.agents.held_arrivals = arrivals;
    }

    /// Schedules `trip` at `slot`, or the wheel's next slot if that is later, moved out of a
    /// queue-free slot (`event_slot`, `SIM-02-K`). O(log S).
    pub(super) fn schedule_trip(&mut self, slot: u64, trip: TripEvent) {
        let slot = event_slot(slot.max(self.events.next_slot()));
        self.events.schedule(slot, SimEvent::Trip(trip));
    }

    // Slot of the next check of a trip whose agent is `agent`: a quarter of the agent's time on
    // the road, from one to eight minutes. An on-time arrival settles within minutes, and a trip
    // stuck in traffic costs a check every few minutes instead of every minute. O(1).
    fn next_trip_check(&self, slot: u64, agent: Option<usize>) -> u64 {
        let Some(agent_idx) = agent else {
            return slot + TRIP_CHECK_SLOTS;
        };
        let on_road_s = (self.agents.sim_time - self.agents.journey_start_time[agent_idx]).max(0.0);
        let on_road_slots = (f64::from(on_road_s) * SLOTS_PER_MINUTE as f64
            / self.time.seconds_per_minute()) as u64;
        slot + (on_road_slots / 4).clamp(TRIP_CHECK_SLOTS, TRIP_CHECK_MAX_SLOTS)
    }

    /// Runs one trip event due at `slot`.
    pub(super) fn run_trip_event(
        &mut self,
        event: TripEvent,
        slot: u64,
        observer: &mut impl EconomyObserver,
    ) {
        let next_check = slot + TRIP_CHECK_SLOTS;
        match event {
            TripEvent::FreightDispatch { shipment } => {
                if !self.logistics.trip_event_due(shipment, slot) {
                    return;
                }
                if self.logistics.dispatch_shipment(
                    shipment,
                    &mut self.allocator,
                    &mut self.agents,
                    &self.transit_network,
                    &self.region_graph,
                    &mut self.treasury.balance,
                ) {
                    self.schedule_freight_trip(
                        next_check,
                        shipment,
                        TripEvent::FreightCheck { shipment },
                    );
                }
            }
            TripEvent::FreightCheck { shipment } => {
                if !self.logistics.trip_event_due(shipment, slot) {
                    return;
                }
                let under_way = {
                    let mut on_delivery = |delivered: &Shipment, allocator: &BuildingAllocator| {
                        observer.freight_delivered(delivered, allocator)
                    };
                    self.logistics.check_shipment(
                        shipment,
                        &mut self.allocator,
                        &mut self.agents,
                        &self.transit_network,
                        &self.region_graph,
                        &mut self.treasury.balance,
                        &mut on_delivery,
                    )
                };
                if under_way {
                    let carrier = self.logistics.shipment_index(shipment).and_then(|idx| {
                        self.agents
                            .resolve_agent(self.logistics.shipments[idx].carrier)
                    });
                    let next = self.next_trip_check(slot, carrier);
                    self.schedule_freight_trip(
                        next,
                        shipment,
                        TripEvent::FreightCheck { shipment },
                    );
                }
            }
            TripEvent::ShopperDeparture { household } => {
                if let Some(hid) = self.households.handles.resolve(household)
                    && self
                        .households
                        .release_shopper(hid, &mut self.agents, &mut self.allocator)
                {
                    self.schedule_trip(next_check, TripEvent::ShopperCheck { household });
                }
            }
            TripEvent::ShopperCheck { household } => {
                let Some(hid) = self.households.handles.resolve(household) else {
                    return;
                };
                let (under_way, household_vat) = self.households.check_shopping_trip(
                    hid,
                    &mut self.agents,
                    &mut self.allocator,
                    &self.transit_network,
                    &self.region_graph,
                    self.fiscal_policy.household_vat_rate,
                );
                if household_vat != 0.0 {
                    self.collect_fiscal_revenue(FiscalRevenue {
                        household_vat,
                        ..FiscalRevenue::default()
                    });
                }
                if under_way {
                    let shopper = self
                        .agents
                        .resolve_agent(self.households.households[hid].shopping_agent);
                    let next = self.next_trip_check(slot, shopper);
                    self.schedule_trip(next, TripEvent::ShopperCheck { household });
                }
            }
            TripEvent::VisitDeparture { visitor } => {
                if let Some(agent_idx) = self.agents.resolve_agent(visitor)
                    && self.agents.release_departure(agent_idx)
                {
                    // A visit lasts until the hour after the visitor set out, as visits that
                    // all left on the hour ended at the next one.
                    self.schedule_trip(slot + SLOTS_PER_HOUR, TripEvent::VisitReturn { visitor });
                }
            }
            TripEvent::VisitReturn { visitor } => {
                if let Some(agent_idx) = self.agents.resolve_agent(visitor)
                    && self.households.end_service_visit(
                        agent_idx,
                        &mut self.agents,
                        &self.allocator,
                    )
                {
                    let next = self.next_trip_check(slot, Some(agent_idx));
                    self.schedule_trip(next, TripEvent::VisitReturn { visitor });
                }
            }
            TripEvent::ArrivalDeparture { carrier } => {
                if let Some(agent_idx) = self.agents.resolve_agent(carrier)
                    && self.agents.release_departure(agent_idx)
                {
                    crate::debug::work::add(crate::debug::work::Work::ArrivalLaunches, 1);
                    self.schedule_trip(next_check, TripEvent::ArrivalCheck { carrier });
                }
            }
            TripEvent::ArrivalCheck { carrier } => {
                let Some(agent_idx) = self.agents.resolve_agent(carrier) else {
                    return;
                };
                if self.agents.pending_household_size[agent_idx] == 0 {
                    return;
                }
                if !self.households.materialize_arrival_carrier(
                    agent_idx,
                    &mut self.agents,
                    &self.allocator,
                ) {
                    let next = self.next_trip_check(slot, Some(agent_idx));
                    self.schedule_trip(next, TripEvent::ArrivalCheck { carrier });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;

    #[test]
    fn trip_events_round_trip_through_saved_rows() {
        let household = Handle::from_parts(7, 3);
        let agent = Handle::from_parts(9, 2);
        for trip in [
            TripEvent::FreightDispatch { shipment: 42 },
            TripEvent::FreightCheck { shipment: 43 },
            TripEvent::ShopperDeparture { household },
            TripEvent::ShopperCheck { household },
            TripEvent::VisitDeparture { visitor: agent },
            TripEvent::VisitReturn { visitor: agent },
            TripEvent::ArrivalDeparture { carrier: agent },
            TripEvent::ArrivalCheck { carrier: agent },
        ] {
            let (kind, a) = SimEvent::Trip(trip).to_saved();
            assert_eq!(SimEvent::from_saved(kind, a), Some(SimEvent::Trip(trip)));
        }
    }

    #[test]
    fn held_trips_leave_evenly_over_the_following_hour() {
        let mut core = SimCore::new(WorldConfig::new(512.0, 512.0, 40.0, 10.0));
        let slot = core.events.next_slot() - 1;
        core.agents
            .held_arrivals
            .extend((0..120).map(|index| Handle::from_parts(index, 0)));
        core.schedule_arrival_departures(slot);
        assert!(core.agents.held_arrivals.is_empty());
        let pending = core.events.pending();
        let mut per_minute = [0_u32; 60];
        for timed in &pending {
            assert!(timed.slot > slot && timed.slot < slot + SLOTS_PER_HOUR);
            assert!(matches!(
                timed.event,
                SimEvent::Trip(TripEvent::ArrivalDeparture { .. })
            ));
            per_minute[((timed.slot - slot - 1) / SLOTS_PER_MINUTE) as usize] += 1;
        }
        assert_eq!(pending.len(), 120);
        assert!(per_minute.iter().all(|&count| count <= 120 / 60 + 2));
    }

    #[test]
    fn only_a_shipments_recorded_trip_event_steps_it() {
        use crate::simulation::economy::logistics::{
            CarrierClass, ShipmentEndpoint, ShipmentStatus,
        };
        let mut core = SimCore::new(WorldConfig::new(512.0, 512.0, 40.0, 10.0));
        let slot = core.events.next_slot();
        // A return leg whose carrier is gone finishes at its next live check.
        core.logistics.shipments.push(Shipment {
            id: 0,
            resource_runtime_id: 1,
            amount: 40.0,
            source: ShipmentEndpoint::OwaBorder(0),
            destination: ShipmentEndpoint::Building(0),
            carrier_class: CarrierClass::Truck,
            status: ShipmentStatus::Returning,
            carrier: Handle::from_parts(3, 1),
            total_cost: 0.0,
            freight_cost: 0.0,
            eta_hours: 1,
            leg_started_hour: 0,
            trip_event_slot: slot + 2,
            cargo_loaded: true,
        });
        let check = TripEvent::FreightCheck { shipment: 0 };
        core.run_trip_event(check, slot + 1, &mut ());
        assert_eq!(
            core.logistics.shipments[0].status,
            ShipmentStatus::Returning
        );
        core.run_trip_event(check, slot + 2, &mut ());
        assert_eq!(
            core.logistics.shipments[0].status,
            ShipmentStatus::Fulfilled
        );
    }

    #[test]
    fn a_clock_set_directly_keeps_trips_in_progress() {
        let mut core = SimCore::new(WorldConfig::new(512.0, 512.0, 40.0, 10.0));
        let slot = core.events.next_slot() - 1;
        core.schedule_trip(slot + 500, TripEvent::FreightCheck { shipment: 5 });
        core.schedule_trip(
            slot + 5 * SLOTS_PER_HOUR,
            TripEvent::FreightCheck { shipment: 6 },
        );
        core.time.minute_of_day += 120;
        core.align_events_to_clock();
        let pending = core.events.pending();
        assert_eq!(
            pending.iter().map(|timed| timed.slot).collect::<Vec<_>>(),
            [core.time.absolute_slot() + 1, slot + 5 * SLOTS_PER_HOUR],
            "an overdue check fires next, a later one keeps its slot"
        );
    }
}
