//! Operational-hour and daily household economy orchestration.

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

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
    ) -> FiscalRevenue {
        self.materialize_arrived_household_carriers(agents, allocator);
        self.debug_validate_agent_household_refs(agents);
        self.rebuild_household_and_worker_counts(agents, allocator);
        self.run_building_economy(allocator);
        let business_purchase_tax = logistics.hourly_tick(
            allocator,
            agents,
            transit_network,
            graph,
            minute_of_day,
            treasury_balance,
        );
        let household_vat = self.run_household_operational_hour(
            agents,
            allocator,
            transit_network,
            graph,
            absolute_hour,
        );
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            service_funding_by_building,
        );
        self.sync_agent_money_from_households(agents);
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
    ) -> FiscalRevenue {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.materialize_arrived_household_carriers(agents, allocator);
        self.debug_validate_agent_household_refs(agents);
        self.rebuild_household_and_worker_counts(agents, allocator);
        self.begin_daily_ledger_settlement();
        // Advance per-agent job-lock countdown once per day.
        agents.job_lock_days.par_iter_mut().for_each(|lock_days| {
            *lock_days = lock_days.saturating_sub(1);
        });
        // Step 1: bankruptcy check — mark buildings that were in distress yesterday and are
        // still negative. Must run before wages so workers are ejected on the same day.
        self.run_bankruptcy_check(allocator);
        // Step 2: pay wages (budget does not go negative from this step).
        let income_tax = self.pay_daily_wages_with_service_funding(
            agents,
            allocator,
            tuning.fiscal.income_tax_rate,
            treasury_balance,
            service_funding_by_building,
        );
        // Step 3: pay unemployment benefit to eligible households from the city treasury.
        self.pay_unemployment_benefits(agents, allocator, treasury_balance);
        // Steps 4 + 5: charge utility, then liquidate if still negative.
        self.settle_daily_utilities(allocator, logistics, treasury_balance);
        // Step 6: collect tax on positive daily business operating-budget growth.
        let business_profit_tax =
            self.settle_business_profit_tax(allocator, tuning.fiscal.business_profit_tax_rate);
        self.resolve_household_housing(agents, allocator);
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            service_funding_by_building,
        );
        self.sync_agent_money_from_households(agents);
        self.finish_daily_ledger_settlement();
        FiscalRevenue {
            income_tax,
            business_profit_tax,
            ..FiscalRevenue::default()
        }
    }
}
