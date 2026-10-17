// SPDX-License-Identifier: GPL-2.0-only

//! Operational-hour household economy orchestration. Daily settlement runs per entity at
//! each one's daily phase (`households/settlement.rs`).

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::{debug, debug_log};
use std::time::Instant;

impl HouseholdSystem {
    /// Runs one operational-hour household pass for membership, production, logistics, and labor.
    /// Trips started here leave at paced slots and settle when they arrive (`SIM-02-E`).
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
        service_funding_by_building: &[f32],
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
        let validate_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.rebuild_household_and_worker_counts(agents, allocator);
        self.rehouse_unhoused_before_growth(agents, allocator);
        // Reviews queued by the daily phases since the last hour, with the same member links.
        self.review_queued_housing(agents, allocator);
        let counts_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // The census comes first, so the activity floor, store sales, restock and visit
        // planning and hiring all read a census of this hour's state (`SIM-02-F`).
        self.refresh_funding_census(
            allocator,
            logistics,
            transit_network,
            graph,
            service_funding_by_building,
            fiscal_policy,
            *treasury_balance,
            minute_of_day,
        );
        let census_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.run_building_economy(allocator);
        self.run_electricity_hour(allocator);
        let building_economy_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
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
        phase_start = Instant::now();
        self.count_employed_adults(agents);
        let money_sync_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "operational_hour_detail absolute_hour={} minute={} pending_households={} pending_adults={} agents={} buildings={} households={} pending_ms={:.3} validate_ms={:.3} counts_ms={:.3} census_ms={:.3} building_economy_ms={:.3} logistics_ms={:.3} household_hour_ms={:.3} workplace_ms={:.3} money_sync_ms={:.3} total_ms={:.3}",
                absolute_hour,
                minute_of_day,
                self.pending_arrivals.households,
                self.pending_arrivals.adults,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                pending_ms,
                validate_ms,
                counts_ms,
                census_ms,
                building_economy_ms,
                logistics_ms,
                household_hour_ms,
                workplace_ms,
                money_sync_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }
}
