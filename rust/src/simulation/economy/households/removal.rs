//! Demand-owned household removal and household swap-remove repair.

use super::HouseholdSystem;
use super::metrics::{
    household_is_housed, household_reserve_days, household_supply_resource_runtime_id,
};
use super::replenishment::{REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

impl HouseholdSystem {
    pub(crate) fn execute_demand_household_removal(
        &mut self,
        households_to_remove_today: u32,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> u32 {
        if households_to_remove_today == 0 || self.households.is_empty() {
            return 0;
        }

        let mut unhoused_candidates = Vec::new();
        let mut housed_candidates = Vec::new();
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        for (household_id, household) in self.households.iter().enumerate() {
            if household.member_count == 0 {
                continue;
            }
            let reserve_days = household_reserve_days(&catalog, &tuning, household);
            let candidate = (household_id, reserve_days, household.stock_days);
            if household_is_housed(household, allocator) {
                housed_candidates.push(candidate);
            } else {
                unhoused_candidates.push(candidate);
            }
        }

        let candidate_order = |a: &(usize, f32, f32), b: &(usize, f32, f32)| {
            a.1.total_cmp(&b.1)
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        };
        unhoused_candidates.sort_by(candidate_order);
        housed_candidates.sort_by(candidate_order);

        let mut ordered_households = Vec::with_capacity(
            unhoused_candidates
                .len()
                .saturating_add(housed_candidates.len()),
        );
        ordered_households.extend(unhoused_candidates.into_iter().map(|candidate| candidate.0));
        ordered_households.extend(housed_candidates.into_iter().map(|candidate| candidate.0));

        let removal_count = ordered_households
            .len()
            .min(households_to_remove_today as usize);
        if removal_count == 0 {
            return 0;
        }

        let mut selected_households: Vec<_> =
            ordered_households.into_iter().take(removal_count).collect();
        selected_households.sort_unstable_by(|a, b| b.cmp(a));

        debug_log!(
            "economy",
            "demand-owned household removal executing: households_to_remove_today={} selected={}",
            households_to_remove_today,
            selected_households.len()
        );

        let removed_count = selected_households.len() as u32;
        let mut selected_flags = std::mem::take(&mut self.removal_selected_flags_scratch);
        selected_flags.clear();
        selected_flags.resize(self.households.len(), false);
        for &household_id in &selected_households {
            selected_flags[household_id] = true;
        }

        let mut agent_indices = std::mem::take(&mut self.removal_agent_indices_scratch);
        agent_indices.clear();
        for agent_idx in 0..agents.len() {
            let household_id = agents.household_id[agent_idx];
            if household_id < selected_flags.len() && selected_flags[household_id] {
                agent_indices.push(agent_idx);
            }
        }
        agent_indices.sort_unstable_by(|a, b| b.cmp(a));
        for agent_idx in agent_indices.iter().copied() {
            agents.kill_agent(agent_idx, allocator);
        }

        for household_id in selected_households {
            self.remove_household_record_at_index(
                household_id,
                agents,
                allocator,
                household_supply_resource,
            );
        }
        self.removal_selected_flags_scratch = selected_flags;
        self.removal_agent_indices_scratch = agent_indices;
        removed_count
    }

    fn remove_household_record_at_index(
        &mut self,
        household_id: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        household_supply_resource: u16,
    ) {
        if household_id >= self.households.len() {
            return;
        }

        let household = &self.households[household_id];
        if matches!(
            household.replenishment_state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        ) {
            let store_idx = household.reserved_store_building_id;
            if store_idx < allocator.buildings.len()
                && household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
            {
                allocator.buildings[store_idx]
                    .add_inventory_units(household_supply_resource, household.reserved_amount);
            }
        }

        debug_log!(
            "economy",
            "removing household_id={} members={} home_building={}",
            household_id,
            self.households[household_id].member_count,
            self.households[household_id].home_building_id
        );

        // Release the household's residential slot if they had a home.
        let home_idx = self.households[household_id].home_building_id;
        if home_idx < allocator.buildings.len() {
            allocator.release_vacancy(home_idx);
        }

        let last_household_id = self.households.len() - 1;
        self.households.swap_remove(household_id);
        if household_id < self.daily_ledgers.len() {
            self.daily_ledgers.swap_remove(household_id);
        }
        if household_id < self.households.len() {
            let mut mapping = std::collections::HashMap::with_capacity(1);
            mapping.insert(last_household_id, household_id);
            agents.remap_household_indices(&mapping);
        }
    }
}
