//! Operational-hour and daily household economy orchestration.

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::fiscal::{CityFiscalPolicy, FiscalRevenue};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::{debug, debug_log};
use rayon::prelude::*;
use std::time::Instant;

impl HouseholdSystem {
    /// Runs one operational-hour household pass for membership, production, logistics, and labor.
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
    ) -> FiscalRevenue {
        let timing_enabled = debug::category_enabled("economy");
        let total_start = Instant::now();
        let mut phase_start = total_start;
        let materialized = self.materialize_arrived_household_carriers(agents, allocator);
        let materialize_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.debug_validate_agent_household_refs(agents);
        let validate_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.rebuild_household_and_worker_counts(agents, allocator);
        let counts_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.run_building_economy(allocator);
        let building_economy_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let business_purchase_tax = logistics.hourly_tick(
            allocator,
            agents,
            transit_network,
            graph,
            minute_of_day,
            treasury_balance,
            fiscal_policy.business_purchase_tax_rate,
        );
        let logistics_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let household_vat = self.run_household_operational_hour(
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
        );
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.sync_agent_money_from_households(agents);
        let money_sync_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                "economy",
                "operational_hour_detail absolute_hour={} minute={} materialized_households={} materialized_residents={} agents={} buildings={} households={} materialize_ms={:.3} validate_ms={:.3} counts_ms={:.3} building_economy_ms={:.3} logistics_ms={:.3} household_hour_ms={:.3} workplace_ms={:.3} money_sync_ms={:.3} total_ms={:.3}",
                absolute_hour,
                minute_of_day,
                materialized.households,
                materialized.residents,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                materialize_ms,
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
        FiscalRevenue {
            household_vat,
            business_purchase_tax,
            ..FiscalRevenue::default()
        }
    }

    /// Runs one daily settlement pass after the final operational-hour step of the day.
    ///
    /// Implements the four-step bankruptcy spec from `economy.md § Building Bankruptcy`:
    /// Step 1 — bankruptcy check, Step 2 — wages, Step 3 — utility cost, Step 4 — distress.
    pub(crate) fn daily_settlement_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
        service_funding_by_building: &[f32],
        fiscal_policy: &CityFiscalPolicy,
    ) -> FiscalRevenue {
        let timing_enabled = debug::category_enabled("economy");
        let total_start = Instant::now();
        let mut phase_start = total_start;
        let materialized = self.materialize_arrived_household_carriers(agents, allocator);
        let materialize_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.debug_validate_agent_household_refs(agents);
        let validate_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.rebuild_household_and_worker_counts(agents, allocator);
        let counts_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.begin_daily_ledger_settlement();
        // Advance per-agent job-lock countdown once per day.
        agents.job_lock_days.par_iter_mut().for_each(|lock_days| {
            *lock_days = lock_days.saturating_sub(1);
        });
        let ledger_and_locks_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 1: bankruptcy check — mark buildings that were in distress yesterday and are
        // still negative. Must run before wages so workers are ejected on the same day.
        self.run_bankruptcy_check(allocator);
        let bankruptcy_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 2: pay wages (budget does not go negative from this step).
        let income_tax = self.pay_daily_wages_with_service_funding(
            agents,
            allocator,
            fiscal_policy.income_tax_rate,
            treasury_balance,
            service_funding_by_building,
        );
        let wages_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 3: pay city transfer benefits from the city treasury.
        self.pay_household_transfers(agents, allocator, treasury_balance, fiscal_policy);
        let benefits_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Steps 4 + 5: charge utility, then liquidate if still negative.
        self.settle_daily_utilities(allocator, logistics, treasury_balance);
        let utilities_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Step 6: collect tax on positive daily business operating-budget growth.
        let business_profit_tax =
            self.settle_business_profit_tax(allocator, fiscal_policy.business_profit_tax_rate);
        let profit_tax_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.resolve_household_housing(agents, allocator);
        let housing_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            service_funding_by_building,
        );
        let workplace_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        self.sync_agent_money_from_households(agents);
        self.finish_daily_ledger_settlement();
        let sync_and_finish_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                "economy",
                "daily_settlement_detail materialized_households={} materialized_residents={} agents={} buildings={} households={} materialize_ms={:.3} validate_ms={:.3} counts_ms={:.3} ledger_and_locks_ms={:.3} bankruptcy_ms={:.3} wages_ms={:.3} benefits_ms={:.3} utilities_ms={:.3} profit_tax_ms={:.3} housing_ms={:.3} workplace_ms={:.3} sync_and_finish_ms={:.3} total_ms={:.3}",
                materialized.households,
                materialized.residents,
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                materialize_ms,
                validate_ms,
                counts_ms,
                ledger_and_locks_ms,
                bankruptcy_ms,
                wages_ms,
                benefits_ms,
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
            ..FiscalRevenue::default()
        }
    }
}
