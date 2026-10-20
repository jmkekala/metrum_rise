// SPDX-License-Identifier: GPL-2.0-only

//! Operational-hour household economy orchestration. Daily settlement runs per entity at
//! each one's daily phase (`households/settlement.rs`). Each household recounts its members at
//! its hourly phase (`households/membership.rs`); the activity floor, service sales and
//! electricity also run at each entity's hourly phase, folded at the hour's last slot
//! (`SIM-02-I`).

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::RuntimeEconomyCatalog;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::{debug, debug_log};
use std::time::Instant;

impl HouseholdSystem {
    /// Runs what is left of the operational-hour household pass: pending arrivals, member
    /// checks, the logistics clock and the commute-quote expiry. Freight, housing, restock,
    /// visits and job search are served by their request queues (`SIM-02-J`).
    pub(crate) fn operational_hour_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
        logistics: &mut ShipmentSystem,
        absolute_hour: u32,
        minute_of_day: u16,
    ) {
        let timing_category = if debug::Channel::EconomyPhases.enabled() {
            debug::Channel::EconomyPhases
        } else {
            debug::Channel::Economy
        };
        let timing_enabled = timing_category.enabled();
        let total_start = Instant::now();
        let mut phase_start = total_start;
        // Carriers become households when they arrive (`SIM-02-E`); count those still coming.
        self.recount_pending_arrivals(agents);
        let pending_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.debug_validate_agent_household_refs(agents);
        // Agents a loader or tool appended directly join their lists before housing walks them.
        agents.register_appended_household_members();
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            agents.assert_household_members_match_rebuild();
        }
        let validate_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Freight, housing, restock, visits and job search are served by their request queues
        // (`SIM-02-J`); the hour only starts the logistics clock.
        logistics.hourly_tick();
        let logistics_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Commute quotes expire a day of hours after they were added (`SIM-02-G`).
        self.expire_workplace_route_cache();
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "operational_hour_detail absolute_hour={} minute={} pending_households={} pending_adults={} agents={} buildings={} households={} pending_ms={:.3} validate_ms={:.3} logistics_ms={:.3} workplace_ms={:.3} total_ms={:.3}",
                absolute_hour,
                minute_of_day,
                self.pending_arrivals.households,
                self.pending_arrivals.adults,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                pending_ms,
                validate_ms,
                logistics_ms,
                workplace_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }

    /// Ends the hour's city totals at its last slot, after every phase of the hour
    /// (`SIM-02-I`): the service-sales prices and pool, then electricity dispatch and coverage,
    /// for the next hour, and logs the hour's housing requests. O(profiles² + resources).
    pub(crate) fn close_city_hour(&mut self, catalog: &RuntimeEconomyCatalog) {
        self.service_sales.close_hour(catalog);
        self.close_electricity_hour();
        self.close_housing_hour();
        self.close_shopping_hour();
        self.close_job_hour();
        self.close_departure_and_admission_hour();
    }
}
