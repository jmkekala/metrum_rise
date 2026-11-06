// SPDX-License-Identifier: GPL-2.0-only

//! The household request queues (`SIM-02-J`, `docs/simulation.md#request-queues`).
//!
//! Households queue requests at their phases and these systems serve the oldest a few per
//! slot, after the cell-lot and freight queues: `housing requests` rehouses households without
//! a usable home and serves the housing reviews of their daily phases, oldest first; `restock
//! requests` reserves purchases and `visit requests` plans visible service visits against the
//! rolling `household destinations` index; `job requests` runs agents' job searches against
//! the hour's job board. A reserved shopper leaves near its household's slot of the hour ahead
//! (`spread_slot`), in the least busy minute nearby (`MinuteLeveler`); a visitor, paced by the
//! visit credit, leaves at the next slot.

use super::state::SimCore;
use super::trips::TripEvent;
use crate::simulation::core::scheduler::spread_slot;

impl SimCore {
    /// One slot of the `housing requests` queue: up to `quota` rehousings and housing reviews.
    /// O(1) with nothing queued; see `HouseholdSystem::serve_housing_requests`.
    pub(super) fn serve_housing_request_queue(&mut self, quota: usize) {
        self.households
            .serve_housing_requests(quota, &mut self.agents, &mut self.allocator);
    }

    /// One slot of the `restock requests` queue: plans up to `quota` restocks, then schedules
    /// each reserved shopper's departure. O(1) with nothing queued; see
    /// `HouseholdSystem::serve_restock_requests`.
    pub(super) fn serve_restock_request_queue(&mut self, slot: u64, quota: usize) {
        self.households.serve_restock_requests(
            slot,
            quota,
            &mut self.agents,
            &mut self.allocator,
            &self.transit_network,
            &self.region_graph,
            self.fiscal_policy.household_vat_rate,
            Some(&self.parking),
        );
        let mut shoppers = std::mem::take(&mut self.households.held_shoppers);
        for household in shoppers.drain(..) {
            let departure = self
                .households
                .shopper_departures
                .place(slot, spread_slot(slot, household.to_bits()));
            self.schedule_trip(departure, TripEvent::ShopperDeparture { household });
        }
        self.households.held_shoppers = shoppers;
    }

    /// One slot of the `visit requests` queue: its visit credit, and a visit when a whole one
    /// has built up; the visitor leaves at the next slot. O(1) without a visit.
    pub(super) fn serve_visit_request_queue(&mut self, slot: u64) {
        self.households.serve_visit_requests(
            slot,
            &mut self.agents,
            &mut self.allocator,
            &self.transit_network,
            &self.region_graph,
            Some(&self.parking),
        );
        let mut visitors = std::mem::take(&mut self.households.held_visitors);
        for visitor in visitors.drain(..) {
            self.schedule_trip(slot + 1, TripEvent::VisitDeparture { visitor });
        }
        self.households.held_visitors = visitors;
    }

    /// One slot of the `job requests` queue: up to `quota` job searches. O(1) with nothing
    /// queued; see `HouseholdSystem::serve_job_requests`.
    pub(super) fn serve_job_request_queue(&mut self, slot: u64, quota: usize) {
        let (_, minute_of_day) = super::schedule::slot_calendar(slot);
        self.households.serve_job_requests(
            slot,
            quota,
            &mut self.agents,
            &mut self.allocator,
            &self.transit_network,
            &self.region_graph,
            minute_of_day,
            Some(&self.parking),
        );
    }

    /// Slot `slot`'s chunk of the rolling job board.
    pub(super) fn advance_job_board(&mut self, slot: u64) {
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.households.advance_job_board(slot, &self.allocator);
    }

    /// Slot `slot`'s chunk of the rolling household destination index.
    pub(super) fn advance_household_destinations(&mut self, slot: u64) {
        self.allocator
            .building_handles
            .register_appended(self.allocator.buildings.len());
        self.households.advance_destination_sweep(
            slot,
            &self.allocator,
            &self.transit_network,
            &self.region_graph,
            Some(&self.parking),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::sim::core::schedule::SimEvent;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::handles::Handle;
    use crate::simulation::core::scheduler::{SLOTS_PER_HOUR, SLOTS_PER_MINUTE};

    #[test]
    fn reserved_shoppers_leave_within_the_hour_ahead_levelled_per_minute() {
        let mut core = SimCore::new(WorldConfig::new(512.0, 512.0, 40.0, 10.0));
        let start = core.events.next_slot();
        // Shoppers reserved over a few slots, as the queue would hold them.
        for (slot, households) in [(start, 0..40), (start + 3, 40..80), (start + 7, 80..120)] {
            core.households
                .held_shoppers
                .extend(households.map(|index| Handle::from_parts(index, 0)));
            core.serve_restock_request_queue(slot, 16);
            assert!(core.households.held_shoppers.is_empty());
        }
        let mut per_minute = [0_u32; 128];
        for timed in core.events.pending() {
            let SimEvent::Trip(TripEvent::ShopperDeparture { .. }) = timed.event else {
                panic!("only shopper departures were scheduled");
            };
            assert!(
                timed.slot > start
                    && timed.slot <= start + 7 + SLOTS_PER_HOUR + 15 * SLOTS_PER_MINUTE
            );
            per_minute[((timed.slot - start) / SLOTS_PER_MINUTE) as usize] += 1;
        }
        assert_eq!(per_minute.iter().sum::<u32>(), 120);
        assert!(
            per_minute.iter().all(|&count| count <= 120 / 60 + 2),
            "{per_minute:?}"
        );
    }
}
