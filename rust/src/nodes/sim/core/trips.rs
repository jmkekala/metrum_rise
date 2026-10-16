// SPDX-License-Identifier: GPL-2.0-only

//! Trips as timed events (`SIM-02-E`, `docs/simulation.md#deadlines-and-timed-events`).
//!
//! The hourly passes still decide which freight carriers, shoppers, service visitors and
//! household arrival carriers set out, but none of them leaves at the boundary: each pass holds
//! its trips and lists them, and [`SimCore::schedule_trip_departures`] spaces every kind evenly
//! over the following hour. A trip is then checked until it arrives, so deliveries, pickups and
//! move-ins happen when the agent gets there instead of at the next hour. Rechecks back off with the trip's age, so a trip stuck in traffic is checked every few
//! minutes. Each event costs O(1) plus the arrival it settles.

use super::frame::EconomyObserver;
use super::schedule::SimEvent;
use super::state::SimCore;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::{
    SLOTS_PER_HOUR, SLOTS_PER_MINUTE, TRIP_CHECK_MAX_SLOTS, TRIP_CHECK_SLOTS, paced_slot,
};
use crate::simulation::economy::agents::AgentHandle;
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::Household;
use crate::simulation::economy::logistics::Shipment;

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
    /// Spaces the trips the passes of `slot` held over the following hour, each kind on its own
    /// even spread, in the order the passes listed them.
    pub(super) fn schedule_trip_departures(&mut self, slot: u64) {
        // Each list is drained in place so its capacity is reused the next hour.
        let mut dispatches = std::mem::take(&mut self.logistics.held_dispatches);
        let count = dispatches.len();
        for (index, shipment) in dispatches.drain(..).enumerate() {
            self.schedule_trip(
                paced_slot(slot, index, count),
                TripEvent::FreightDispatch { shipment },
            );
        }
        self.logistics.held_dispatches = dispatches;
        let mut shoppers = std::mem::take(&mut self.households.held_shoppers);
        let count = shoppers.len();
        for (index, household) in shoppers.drain(..).enumerate() {
            self.schedule_trip(
                paced_slot(slot, index, count),
                TripEvent::ShopperDeparture { household },
            );
        }
        self.households.held_shoppers = shoppers;
        let mut visitors = std::mem::take(&mut self.households.held_visitors);
        let count = visitors.len();
        for (index, visitor) in visitors.drain(..).enumerate() {
            self.schedule_trip(
                paced_slot(slot, index, count),
                TripEvent::VisitDeparture { visitor },
            );
        }
        self.households.held_visitors = visitors;
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

    fn schedule_trip(&mut self, slot: u64, trip: TripEvent) {
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
                if self.logistics.dispatch_shipment(
                    shipment,
                    &mut self.allocator,
                    &mut self.agents,
                    &self.transit_network,
                    &self.region_graph,
                    &mut self.treasury.balance,
                ) {
                    self.schedule_trip(next_check, TripEvent::FreightCheck { shipment });
                }
            }
            TripEvent::FreightCheck { shipment } => {
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
                    self.schedule_trip(next, TripEvent::FreightCheck { shipment });
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
        core.logistics.held_dispatches.extend(0..120);
        core.households
            .held_shoppers
            .extend((0..3).map(|index| Handle::from_parts(index, 0)));
        core.schedule_trip_departures(slot);
        assert!(core.logistics.held_dispatches.is_empty());
        assert!(core.households.held_shoppers.is_empty());
        let pending = core.events.pending();
        let mut per_minute = [0_u32; 60];
        let mut dispatches = 0;
        for timed in &pending {
            assert!(timed.slot > slot && timed.slot < slot + SLOTS_PER_HOUR);
            if matches!(
                timed.event,
                SimEvent::Trip(TripEvent::FreightDispatch { .. })
            ) {
                dispatches += 1;
                per_minute[((timed.slot - slot - 1) / SLOTS_PER_MINUTE) as usize] += 1;
            }
        }
        assert_eq!(dispatches, 120);
        assert!(per_minute.iter().all(|&count| count <= 120 / 60 + 2));
        assert_eq!(pending.len(), 123);
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
