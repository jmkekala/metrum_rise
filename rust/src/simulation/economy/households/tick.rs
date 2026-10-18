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
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::{debug, debug_log};
use std::time::Instant;

impl HouseholdSystem {
    /// Runs one operational-hour household pass for housing, logistics, restock and visits, and
    /// labor, on the census published before it (`SIM-02-I`). Trips started here leave at paced
    /// slots and settle when they arrive (`SIM-02-E`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn operational_hour_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        minute_of_day: u16,
        treasury_balance: &mut f64,
        fiscal_policy: &CityFiscalPolicy,
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
        agents.assert_household_members_match_rebuild();
        let validate_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Member counts and wages come from each household's last hourly phase (`SIM-02-I`).
        self.rehouse_unhoused_before_growth(agents, allocator);
        // Reviews queued by the daily phases since the last hour.
        self.review_queued_housing(agents, allocator);
        let housing_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Restock and visit planning and hiring read the census published at this slot,
        // before the hour (`SIM-02-I`, `households/census.rs`).
        logistics.hourly_tick(
            allocator,
            agents,
            transit_network,
            graph,
            minute_of_day,
            treasury_balance,
        );
        let logistics_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.run_household_operational_hour(
            agents,
            allocator,
            transit_network,
            graph,
            absolute_hour,
            fiscal_policy.household_vat_rate,
        );
        let household_hour_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.assign_agent_workplaces_from_census(
            agents,
            allocator,
            transit_network,
            graph,
            minute_of_day,
        );
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "operational_hour_detail absolute_hour={} minute={} pending_households={} pending_adults={} agents={} buildings={} households={} pending_ms={:.3} validate_ms={:.3} housing_ms={:.3} logistics_ms={:.3} household_hour_ms={:.3} workplace_ms={:.3} total_ms={:.3}",
                absolute_hour,
                minute_of_day,
                self.pending_arrivals.households,
                self.pending_arrivals.adults,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                pending_ms,
                validate_ms,
                housing_ms,
                logistics_ms,
                household_hour_ms,
                workplace_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }

    /// Ends the hour's city totals at its last slot, after every phase of the hour
    /// (`SIM-02-I`): the service-sales prices and pool, then electricity dispatch and coverage,
    /// for the next hour. O(profiles² + resources).
    pub(crate) fn close_city_hour(&mut self, catalog: &RuntimeEconomyCatalog) {
        self.service_sales.close_hour(catalog);
        self.close_electricity_hour();
    }
}
