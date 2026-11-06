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
#[cfg(test)]
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
    /// Whether household `household_id` may leave under `exit_policy` now, checked at its daily
    /// phase (`SIM-02-G`). O(1).
    pub(crate) fn exit_eligible(
        &self,
        household_id: usize,
        exit_policy: HouseholdExitPolicy,
        allocator: &BuildingAllocator,
    ) -> bool {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        exit_policy.is_eligible(&self.households[household_id], allocator, &catalog, &tuning)
    }

    /// One slot of the `departures` queue (`SIM-02-J`): takes up to `quota` of the households
    /// whose daily phase claimed a departure, oldest first, and removes those still eligible;
    /// a household that recovered meanwhile stays. The claims are paced by the daily exit
    /// credit; the caller serves only in the hour's removal window. Returns how many left.
    /// O(1) with nothing queued, else O(quota + members) plus the removal's parallel remaps.
    pub(crate) fn serve_departures(
        &mut self,
        quota: usize,
        exit_policy: HouseholdExitPolicy,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> u32 {
        if self.exits.is_empty() {
            return 0;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let mut ids = Vec::new();
        let mut taken = 0;
        while taken < quota
            && let Some(handle) = self.exits.pop_front()
        {
            taken += 1;
            if let Some(household_id) = self.handles.resolve(handle)
                && exit_policy.is_eligible(
                    &self.households[household_id],
                    allocator,
                    &catalog,
                    &tuning,
                )
            {
                ids.push(household_id);
            }
        }
        self.departure_stats.record_slot(taken, self.exits.len());
        if ids.is_empty() {
            return 0;
        }
        self.remove_households(&mut ids, agents, allocator)
    }

    /// Logs and resets the hour's departure and admission queue service, at the hour's last
    /// slot (`SIM-02-J`).
    pub(crate) fn close_departure_and_admission_hour(&mut self) {
        for (name, stats) in [
            (
                "departure_requests_hour",
                std::mem::take(&mut self.departure_stats),
            ),
            (
                "admission_requests_hour",
                std::mem::take(&mut self.admission_stats),
            ),
        ] {
            if stats.taken > 0 {
                debug_log!(crate::debug::Channel::EconomyPhases, "{} {}", name, stats);
            }
        }
    }

    /// Removes up to `households_to_remove_today` eligible households at once, those with the
    /// fewest reserve days first.
    #[cfg(test)]
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

        let mut ids: Vec<usize> = candidates.iter().map(|candidate| candidate.0).collect();
        self.removal_candidates_scratch = candidates;
        self.remove_households(&mut ids, agents, allocator)
    }

    /// Removes households `ids` with their members and releases their homes; returns how many
    /// left. The ids are sorted descending first, so swap-removal cannot move one still to
    /// remove. O(members log members + removed households) plus the parallel O(agents)
    /// household remap.
    pub(crate) fn remove_households(
        &mut self,
        ids: &mut Vec<usize>,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> u32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.dedup();
        ids.retain(|&id| id < self.households.len());
        let removed_count = ids.len() as u32;

        // The members of every selected household, from their maintained lists, removed in
        // descending agent index so a swap-remove never moves one still to remove.
        // O(members log members) instead of a scan of every agent (`SIM-02-J`).
        agents.register_appended_household_members();
        let mut agent_indices = std::mem::take(&mut self.removal_agent_indices_scratch);
        agent_indices.clear();
        for &household_id in ids.iter() {
            agent_indices.extend(agents.household_members(household_id));
        }
        agent_indices.sort_unstable_by(|a, b| b.cmp(a));
        for agent_idx in agent_indices.iter().copied() {
            agents.kill_agent(agent_idx, allocator);
        }

        // Track current slot -> original household while descending removals can move the
        // same survivor more than once. Repair surviving agents once after the whole batch.
        let mut moved_households = HashMap::with_capacity(ids.len());
        for &household_id in ids.iter() {
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
        self.census_inputs
            .remove_household(household_id, &self.households);
        self.note_household_structure_change();
        crate::debug::work::add(crate::debug::work::Work::HouseholdsRemoved, 1);
        self.parking_changes.push(self.handles.handle(household_id));
        self.handles
            .swap_remove(household_id, self.households.len());
        self.households.swap_remove(household_id);
        if household_id < self.daily_ledgers.len() {
            self.daily_ledgers.swap_remove(household_id);
        }
        (household_id < self.households.len()).then_some(last_household_id)
    }
}
