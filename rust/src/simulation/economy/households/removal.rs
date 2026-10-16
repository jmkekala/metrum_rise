// SPDX-License-Identifier: GPL-2.0-only

//! Demand-owned household removal and household swap-remove repair.

use std::collections::HashMap;

use super::metrics::{
    household_is_housed, household_reserve_days, household_supply_resource_runtime_id,
};
use super::replenishment::{REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE};
use super::{Household, HouseholdSystem};
use crate::debug_log;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use rayon::prelude::*;

/// Household-specific conditions for leaving after an unsuccessful rehousing period.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HouseholdExitPolicy {
    /// Maximum pantry coverage for an early departure due to destitution.
    pub(crate) destitute_stock_days: f32,
    /// Minimum time without a home before a destitute household leaves.
    pub(crate) destitute_unhoused_days: u32,
    /// Maximum time without a home, regardless of savings.
    pub(crate) max_unhoused_days: u32,
}

impl HouseholdExitPolicy {
    /// Rechecks live housing and resources, so recovery cancels a planned departure.
    pub(crate) fn is_eligible(
        self,
        household: &Household,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
    ) -> bool {
        household.member_count > 0
            && !household_is_housed(household, allocator)
            && (household.unhoused_days_elapsed >= self.max_unhoused_days
                || (household.unhoused_days_elapsed >= self.destitute_unhoused_days
                    && household.stock_days <= self.destitute_stock_days
                    && household_reserve_days(catalog, tuning, household) < 1.0))
    }
}

impl HouseholdSystem {
    pub(crate) fn execute_demand_household_removal(
        &mut self,
        households_to_remove_today: u32,
        exit_policy: HouseholdExitPolicy,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> u32 {
        if households_to_remove_today == 0 || self.households.is_empty() {
            return 0;
        }

        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        // Independent eligibility checks use the same policy as the demand snapshot. Only
        // eligible candidates are ranked; housed and recently displaced households never fill
        // an unused departure slot. Reuse the candidate buffer across daily passes.
        let mut candidates = std::mem::take(&mut self.removal_candidates_scratch);
        self.households
            .par_iter()
            .enumerate()
            .map(|(id, household)| {
                if exit_policy.is_eligible(household, allocator, &catalog, &tuning) {
                    (
                        id,
                        household_reserve_days(&catalog, &tuning, household),
                        household.stock_days,
                    )
                } else {
                    (usize::MAX, 0.0, 0.0)
                }
            })
            .collect_into_vec(&mut candidates);
        candidates.retain(|candidate| candidate.0 != usize::MAX);
        candidates.sort_unstable_by(|a, b| {
            a.1.total_cmp(&b.1)
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        });
        let removal_count = candidates.len().min(households_to_remove_today as usize);
        if removal_count == 0 {
            self.removal_candidates_scratch = candidates;
            return 0;
        }
        candidates.truncate(removal_count);
        // Descending IDs keep swap-remove from invalidating another selected household.
        candidates.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));

        debug_log!(
            "economy",
            "demand-owned household removal executing: households_to_remove_today={} selected={}",
            households_to_remove_today,
            candidates.len()
        );

        let removed_count = candidates.len() as u32;
        let mut selected_flags = std::mem::take(&mut self.removal_selected_flags_scratch);
        selected_flags.clear();
        selected_flags.resize(self.households.len(), false);
        for &(household_id, _, _) in &candidates {
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
        // The collection pass already visits agent indices in ascending order.
        agent_indices.reverse();
        for agent_idx in agent_indices.iter().copied() {
            agents.kill_agent(agent_idx, allocator);
        }

        // Track current slot -> original household while descending removals can move the
        // same survivor more than once. Repair surviving agents once after the whole batch.
        let mut moved_households = HashMap::with_capacity(candidates.len());
        for &(household_id, _, _) in &candidates {
            if let Some(last_household_id) = self.remove_household_record_at_index(
                household_id,
                allocator,
                household_supply_resource,
            ) {
                let original_id = moved_households
                    .remove(&last_household_id)
                    .unwrap_or(last_household_id);
                moved_households.insert(household_id, original_id);
            }
        }
        let mapping = moved_households
            .into_iter()
            .map(|(current, original)| (original, current))
            .collect();
        agents.remap_household_indices(&mapping);
        self.removal_candidates_scratch = candidates;
        self.removal_selected_flags_scratch = selected_flags;
        self.removal_agent_indices_scratch = agent_indices;
        removed_count
    }

    fn remove_household_record_at_index(
        &mut self,
        household_id: usize,
        allocator: &mut BuildingAllocator,
        household_supply_resource: u16,
    ) -> Option<usize> {
        if household_id >= self.households.len() {
            return None;
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
        #[cfg(test)]
        {
            let departing = &self.households[household_id];
            let escrow = if departing.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE {
                departing.reserved_total_cost
            } else {
                0.0
            };
            let leaving = f64::from(departing.budget) + f64::from(escrow);
            crate::debug::money::record(
                crate::debug::money::MoneyFlow::DepartedHouseholdCash,
                -leaving,
            );
        }
        self.handles
            .swap_remove(household_id, self.households.len());
        self.households.swap_remove(household_id);
        if household_id < self.daily_ledgers.len() {
            self.daily_ledgers.swap_remove(household_id);
        }
        (household_id < self.households.len()).then_some(last_household_id)
    }
}
