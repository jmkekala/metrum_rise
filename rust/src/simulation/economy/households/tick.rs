//! Operational-hour and daily household economy orchestration.

use super::HouseholdSystem;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl HouseholdSystem {
    /// Runs one operational-hour household pass for membership, production, logistics, and labor.
    pub fn operational_hour_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        minute_of_day: u16,
    ) {
        self.materialize_arrived_household_carriers(agents, allocator);
        self.ensure_agent_households(agents);
        self.rebuild_household_membership(agents);
        self.recount_worker_assignments(agents, allocator);
        self.run_building_economy(allocator);
        logistics.hourly_tick(allocator, transit_network, graph, minute_of_day);
        self.consume_household_stock(agents);
        self.run_household_replenishment(allocator, absolute_hour);
        self.assign_agent_workplaces(agents, allocator, transit_network, graph);
        self.sync_agent_money_from_households(agents);
    }

    /// Runs one daily settlement pass after the final operational-hour step of the day.
    ///
    /// Implements the four-step bankruptcy spec from `economy.md § Building Bankruptcy`:
    /// Step 1 — bankruptcy check, Step 2 — wages, Step 3 — utility cost, Step 4 — distress.
    pub fn daily_settlement_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        treasury_balance: &mut f64,
    ) {
        self.materialize_arrived_household_carriers(agents, allocator);
        self.ensure_agent_households(agents);
        self.rebuild_household_membership(agents);
        self.recount_worker_assignments(agents, allocator);
        // Advance per-agent job-lock countdown once per day.
        for i in 0..agents.len() {
            if agents.job_lock_days[i] > 0 {
                agents.job_lock_days[i] -= 1;
            }
        }
        // Step 1: bankruptcy check — mark buildings that were in distress yesterday and are
        // still negative. Must run before wages so workers are ejected on the same day.
        self.run_bankruptcy_check(allocator);
        // Step 2: pay wages (budget does not go negative from this step).
        self.pay_daily_wages(agents, allocator);
        // Step 3: pay unemployment benefit to eligible households from the city treasury.
        self.pay_unemployment_benefits(agents, allocator, treasury_balance);
        // Steps 4 + 5: charge utility, then liquidate if still negative.
        self.settle_daily_utilities(allocator, logistics);
        self.resolve_household_housing(agents, allocator);
        self.assign_agent_workplaces(agents, allocator, transit_network, graph);
        self.sync_agent_money_from_households(agents);
    }
}
