// SPDX-License-Identifier: GPL-2.0-only

//! Operational-hour and daily household economy orchestration.

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, FiscalRevenue};
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
        let counts_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
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
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            service_funding_by_building,
            logistics,
            fiscal_policy,
            *treasury_balance,
            minute_of_day,
        );
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.sync_agent_money_from_households(agents);
        let money_sync_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "operational_hour_detail absolute_hour={} minute={} pending_households={} pending_adults={} agents={} buildings={} households={} pending_ms={:.3} validate_ms={:.3} counts_ms={:.3} building_economy_ms={:.3} logistics_ms={:.3} household_hour_ms={:.3} workplace_ms={:.3} money_sync_ms={:.3} total_ms={:.3}",
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
                building_economy_ms,
                logistics_ms,
                household_hour_ms,
                workplace_ms,
                money_sync_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }

    /// Runs one daily settlement pass after the final operational-hour step of the day.
    ///
    /// Implements the four-step bankruptcy spec from `economy.md § Building Bankruptcy`:
    /// Step 1 — bankruptcy check, Step 2 — wages, Step 3 — utility cost, Step 4 — distress.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn daily_settlement_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
        service_funding_by_building: &[f32],
        fiscal_policy: &CityFiscalPolicy,
        on_wages_paid: impl FnOnce(&AgentSystem, &BuildingAllocator),
    ) -> FiscalRevenue {
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
        let counts_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.begin_daily_ledger_settlement();
        crate::debug::work::add(
            crate::debug::work::Work::BuildingSettlements,
            allocator.buildings.len() as u64,
        );
        crate::debug::work::add(
            crate::debug::work::Work::HouseholdSettlements,
            self.households.len() as u64,
        );
        // Job locks are deadlines in settlement days; nothing scans the agents (`SIM-02-D`).
        agents.settlement_days += 1;
        let ledger_and_locks_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 1: bankruptcy check — mark buildings that were in distress yesterday and are
        // still negative. Must run before wages so workers are ejected on the same day.
        self.run_bankruptcy_check(allocator);
        let bankruptcy_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 2: pay wages (budget does not go negative from this step).
        let income_tax = self.pay_funded_daily_wages(
            agents,
            allocator,
            fiscal_policy.income_tax_rate,
            treasury_balance,
            logistics,
        );
        let wages_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        // Observe paid assignments before the later job search can replace them.
        // The live caller supplies an empty callback, removed by monomorphization.
        on_wages_paid(agents, allocator);
        phase_start = Instant::now();
        // Step 3: pay city transfer benefits from the city treasury.
        self.pay_household_transfers(agents, allocator, treasury_balance, fiscal_policy);
        let benefits_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 4: collect modeled daily property tax from household and building budgets.
        let property_tax_revenue = self.settle_daily_property_tax(allocator, fiscal_policy);
        let property_tax_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Steps 5 + 6: charge utility, then liquidate if still negative.
        self.settle_daily_utilities(allocator, logistics, treasury_balance);
        let utilities_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 7: collect tax on positive daily business operating-budget growth.
        let business_profit_tax =
            self.settle_business_profit_tax(allocator, fiscal_policy.business_profit_tax_rate);
        let profit_tax_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.resolve_household_housing(agents, allocator);
        let housing_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Job searches repeat for a worker only after a multi-day lock, so a commute estimated
        // yesterday is rarely asked again. Clearing the cache daily bounds it to one day's
        // searches; it grew with every pair ever scanned (7.4M entries at 30k by day 15).
        // Complete quotes keep that pricing cadence. Network-only legs are now bounded by
        // lane storage and stay exact across midnight; their own network revision invalidates them.
        // Retain its allocation for the next day's requests, while expiring every price.
        self.workplace_route_cache.clear();
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            service_funding_by_building,
            logistics,
            fiscal_policy,
            *treasury_balance,
            0,
        );
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.sync_agent_money_from_households(agents);
        self.finish_daily_ledger_settlement();
        let sync_and_finish_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "daily_settlement_detail pending_households={} pending_adults={} agents={} buildings={} households={} pending_ms={:.3} validate_ms={:.3} counts_ms={:.3} ledger_and_locks_ms={:.3} bankruptcy_ms={:.3} wages_ms={:.3} benefits_ms={:.3} property_tax_ms={:.3} utilities_ms={:.3} profit_tax_ms={:.3} housing_ms={:.3} workplace_ms={:.3} sync_and_finish_ms={:.3} total_ms={:.3}",
                self.pending_arrivals.households,
                self.pending_arrivals.adults,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                pending_ms,
                validate_ms,
                counts_ms,
                ledger_and_locks_ms,
                bankruptcy_ms,
                wages_ms,
                benefits_ms,
                property_tax_ms,
                utilities_ms,
                profit_tax_ms,
                housing_ms,
                workplace_ms,
                sync_and_finish_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
        FiscalRevenue {
            income_tax,
            business_profit_tax,
            residential_property_tax: property_tax_revenue.residential_property_tax,
            commercial_property_tax: property_tax_revenue.commercial_property_tax,
            industrial_property_tax: property_tax_revenue.industrial_property_tax,
            ..FiscalRevenue::default()
        }
    }
}
