// SPDX-License-Identifier: GPL-2.0-only

//! Household membership counts and shopping car access, recounted from each household's
//! member list at its hourly phase (`SIM-02-I`).

use super::data::HouseholdSystem;
use super::employment::active_work_profile;
use super::metrics::stock_days;
use super::replenishment::clear_replenishment_request;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, AgentSystem, age_group_can_shop, age_group_can_work,
};
use crate::simulation::economy::definitions::RuntimeEconomyCatalog;

impl HouseholdSystem {
    pub(super) fn debug_validate_agent_household_refs(&self, _agents: &AgentSystem) {
        #[cfg(debug_assertions)]
        {
            use rayon::prelude::*;
            let agents = _agents;
            let household_count = self.households.len();
            let households = &self.households;
            let pending_household_size = &agents.agents.pending_household_size;
            let home_building = &agents.agents.home_building;
            agents
                .agents
                .household_id
                .par_iter()
                .enumerate()
                .for_each(|(i, &household_id)| {
                    if pending_household_size[i] > 0 || household_id == usize::MAX {
                        return;
                    }
                    assert!(
                        household_id < household_count,
                        "agent {i} references missing household {household_id}"
                    );
                    let home = home_building[i];
                    assert_eq!(
                        households[household_id].home_building_id, home,
                        "agent {i} household/home mismatch"
                    );
                });
        }
    }

    /// Recounts household `household_id` from its member list at its hourly phase
    /// (`SIM-02-I`): member, child, adult and elder counts, car access for shopping, the stock
    /// horizon and the cleared request of an empty household, and the employed adults and
    /// expected gross daily wages of its assignments. Members are visited in ascending agent
    /// index, so the wage sum does not depend on how the member list was built. O(members).
    pub(crate) fn recount_household(
        &mut self,
        household_id: usize,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let (mut members, mut children, mut adults, mut elders) = (0_u32, 0_u32, 0_u32, 0_u32);
        let mut shopping_has_car = false;
        let mut employed = 0_u16;
        let mut wages = 0.0_f64;
        for member in agents.household_members(household_id) {
            let age_group = agents.age_group[member];
            members += 1;
            shopping_has_car |= agents.has_car[member] && age_group_can_shop(age_group);
            match age_group {
                AGE_CHILD => children += 1,
                AGE_ADULT => adults += 1,
                AGE_ELDER => elders += 1,
                _ => {}
            }
            if !age_group_can_work(age_group) {
                continue;
            }
            let work = agents.work_building[member];
            // Transfers count an assignment until employment actually ends,
            // even if the employer has stopped operating or paying wages.
            if work != usize::MAX {
                employed = employed.saturating_add(1);
            }
            if let Some(building) = allocator.buildings.get(work)
                && let Some(profile) = active_work_profile(catalog, building)
                && agents.consecutive_unpaid_days[member] == 0
            {
                wages += f64::from(profile.average_daily_wage());
            }
        }
        let count = |value: u32| value.min(u32::from(u16::MAX)) as u16;
        let household = &mut self.households[household_id];
        household.member_count = count(members);
        household.child_count = count(children);
        household.adult_count = count(adults);
        household.elder_count = count(elders);
        household.shopping_has_car = shopping_has_car;
        household.stock_days = stock_days(
            household.stock,
            household.member_count,
            household.consumption_rate,
        );
        if household.member_count == 0 {
            clear_replenishment_request(household);
        }
        household.employed_adults = employed;
        household.gross_daily_wages = wages as f32;
    }

    /// Every household's phase recount at once, in household order.
    #[cfg(test)]
    pub(crate) fn recount_every_household(
        &mut self,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        for household_id in 0..self.households.len() {
            self.recount_household(household_id, agents, allocator, &catalog);
        }
    }
}

/// Asserts that every building's `worker_count` equals its assigned working-age agents. The
/// hiring, release and removal mutators maintain the counts; nothing recounts them
/// (`SIM-02-I`).
#[cfg(test)]
pub(crate) fn assert_worker_counts_match_assignments(
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
) {
    let mut counts = vec![0_u32; allocator.buildings.len()];
    for (&work, &age_group) in agents.work_building.iter().zip(&agents.age_group) {
        if age_group_can_work(age_group) && work < counts.len() {
            counts[work] += 1;
        }
    }
    for (index, (building, count)) in allocator.buildings.iter().zip(counts).enumerate() {
        assert_eq!(
            building.worker_count, count,
            "building {index} worker count"
        );
    }
}
