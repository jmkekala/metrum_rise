//! Agent-to-household membership rebuilds and money synchronization.

use super::data::{Household, HouseholdSystem};
use super::metrics::{household_demand_profile, stock_days};
use super::replenishment::{
    REPLENISHMENT_STABLE, clear_replenishment_request, stable_replenishment_offset_hours,
};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

impl HouseholdSystem {
    pub(super) fn ensure_agent_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            if agents.pending_household_size[i] > 0 {
                continue;
            }
            let home = agents.home_building[i];
            let hid = agents.household_id[i];
            if home == usize::MAX {
                if hid < self.households.len()
                    && self.households[hid].home_building_id == usize::MAX
                {
                    continue;
                }
                agents.assign_household_id(i, usize::MAX);
                continue;
            }
            let needs_new = hid == usize::MAX
                || hid >= self.households.len()
                || self.households[hid].home_building_id != home;
            if needs_new {
                let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                    panic!("could not load built-in economy catalog during re-housing: {err}")
                });
                let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
                    panic!("could not load built-in economy tuning during re-housing: {err}")
                });
                let profile = household_demand_profile(&catalog);
                let consumption_rate = profile.consumption_rate_per_resident;
                let target_days = profile.stock_target_days;

                let budget = agents.money[i].max(tuning.households.household_starting_budget_floor);
                self.households.push(Household {
                    home_building_id: home,
                    budget,
                    stock: target_days * consumption_rate,
                    member_count: 0,
                    consumption_rate,
                    stock_days: target_days,
                    replenishment_state: REPLENISHMENT_STABLE,
                    cooldown_hours: 0,
                    reserved_store_building_id: usize::MAX,
                    reserved_amount: 0.0,
                    reserved_total_cost: 0.0,
                    pickup_eta_hours: 0,
                    stay_failure_days: 0,
                    unhoused_days_elapsed: 0,
                    replenishment_offset_hours: stable_replenishment_offset_hours(
                        home,
                        self.households.len() as u32,
                    ),
                    unemployment_days_elapsed: 0,
                });
                agents.assign_household_id(i, self.households.len() - 1);
            }
        }
    }

    pub(super) fn rebuild_household_membership(&mut self, agents: &AgentSystem) {
        for household in &mut self.households {
            household.member_count = 0;
        }
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid != usize::MAX && hid < self.households.len() {
                let household = &mut self.households[hid];
                household.member_count = household.member_count.saturating_add(1);
                household.home_building_id = agents.home_building[i];
            }
        }
        for household in &mut self.households {
            household.stock_days = stock_days(
                household.stock,
                household.member_count,
                household.consumption_rate,
            );
            if household.member_count == 0 {
                clear_replenishment_request(household);
            }
        }
    }

    pub(super) fn sync_agent_money_from_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }
            let household = &self.households[hid];
            let per_member = if household.member_count > 0 {
                household.budget / household.member_count as f32
            } else {
                0.0
            };
            agents.money[i] = per_member.max(0.0);
        }
    }
}
