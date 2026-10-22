// SPDX-License-Identifier: GPL-2.0-only

//! Household admission and arrived carrier materialization.

use super::data::{DailyHouseholdLedger, Household, HouseholdSystem};
use super::metrics::household_demand_profile;
use super::replenishment::{REPLENISHMENT_STABLE, stable_replenishment_offset_hours};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::agents::{
    AgentHandle, AgentSystem, HouseholdAgeComposition, TRANSIT_IN_BUILDING,
};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};

/// A household the hour's actions slot admitted, waiting for the `admissions` queue to create
/// it and its arrival carrier (`SIM-02-J`): its claimed home, size and composition and the
/// border it arrives from. Saved in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct QueuedAdmission {
    /// The home whose vacancy admission claimed.
    pub(crate) home: Handle<Building>,
    /// Members, at least one.
    pub(crate) size: u16,
    /// Approved adults, elders and children.
    pub(crate) composition: HouseholdAgeComposition,
    /// Border node the carrier sets out from; `u32::MAX` when a save found it gone.
    pub(crate) border_node: u32,
}

// Credit one admission costs: the hour's batch is created over one hour of slots.
const ADMISSION_CREDIT: u32 = SLOTS_PER_HOUR as u32;

impl HouseholdSystem {
    /// Queues the hour's admitted households in admission order and starts the credit: one
    /// whole admission at once, then the waiting count per slot, so the batch is created over
    /// the following hour and the last one before the next actions slot. O(batch).
    pub(crate) fn queue_admissions(&mut self, admitted: impl IntoIterator<Item = QueuedAdmission>) {
        self.admission_requests.extend(admitted);
        self.admission_rate = self.admission_requests.len() as u32;
        if self.admission_rate > 0 {
            self.admission_credit = ADMISSION_CREDIT;
        }
    }

    /// Accrues one slot's admission credit; returns whether an admission waits. With nothing
    /// waiting the credit lapses. O(1).
    pub(crate) fn accrue_admission_credit(&mut self) -> bool {
        if self.admission_requests.is_empty() {
            self.admission_credit = 0;
            return false;
        }
        self.admission_credit = self.admission_credit.saturating_add(self.admission_rate);
        true
    }

    /// The oldest waiting admission when a whole credit pays for it. O(1).
    pub(crate) fn pop_due_admission(&mut self) -> Option<QueuedAdmission> {
        if self.admission_credit < ADMISSION_CREDIT {
            return None;
        }
        let admission = self.admission_requests.pop_front()?;
        self.admission_credit -= ADMISSION_CREDIT;
        Some(admission)
    }

    /// Publishes a successfully admitted carrier's claim before the next hourly recount.
    pub(crate) fn record_pending_arrival(&mut self, composition: HouseholdAgeComposition) {
        self.pending_arrivals.households += 1;
        self.pending_arrivals.adults += u32::from(composition.adult_count);
        self.pending_arrivals.elders += u32::from(composition.elder_count);
        self.pending_arrivals.children += u32::from(composition.child_count);
    }

    pub(crate) fn admit_immigrant_household(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        home_building_id: usize,
        member_count: u16,
        age_composition: HouseholdAgeComposition,
    ) -> usize {
        // Restored household records may not have transient daily ledgers yet.
        self.ensure_daily_ledger_len();
        let profile = household_demand_profile(catalog);
        let consumption_rate = profile.consumption_rate_per_resident;
        let target_days = profile.stock_target_days;
        let starting_stock_days = target_days.min(tuning.households.immigrant_starting_stock_days);

        let member_count = member_count.max(1);
        let starting_budget =
            tuning.households.immigrant_starting_budget_per_member * member_count as f32;
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::ImmigrantCash,
            0.0,
            starting_budget,
        );
        self.households.push(Household {
            home_building_id,
            power_arrears: Default::default(),
            budget: starting_budget,
            stock: member_count as f32 * consumption_rate * starting_stock_days,
            member_count,
            child_count: age_composition.child_count,
            adult_count: age_composition.adult_count,
            employed_adults: 0,
            gross_daily_wages: 0.0,
            shopping_has_car: false,
            elder_count: age_composition.elder_count,
            consumption_rate,
            stock_days: starting_stock_days,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_hours: 0,
            replenishment_failure_count: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            shopping_agent: AgentHandle::NONE,
            shopping_timeout_hours_remaining: 0,
            replenishment_search_cursor: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            unemployment_days_elapsed: 0,
            replenishment_offset_hours: stable_replenishment_offset_hours(
                home_building_id,
                self.households.len() as u32,
            ),
        });
        self.handles.register_appended(self.households.len());
        let ledger = DailyHouseholdLedger {
            budget_before: starting_budget,
            budget_after: starting_budget,
            ..Default::default()
        };
        self.daily_ledgers.push(ledger);
        self.households.len() - 1
    }

    /// Recounts the households still on their way in from [`AgentSystem::arrival_carriers`],
    /// dropping carriers that arrived or left, and those the admission queue holds.
    /// O(carriers in flight + queued admissions); replaces the scan of every agent
    /// (`SIM-02-E`).
    pub(crate) fn recount_pending_arrivals(&mut self, agents: &mut AgentSystem) {
        self.pending_arrivals = Default::default();
        // Admitted households the `admissions` queue has not created yet (`SIM-02-J`).
        for index in 0..self.admission_requests.len() {
            let composition = self.admission_requests[index].composition;
            self.record_pending_arrival(composition);
        }
        let mut carriers = std::mem::take(&mut agents.arrival_carriers);
        carriers.retain(|&carrier| {
            let Some(agent_idx) = agents
                .resolve_agent(carrier)
                .filter(|&agent_idx| agents.pending_household_size[agent_idx] > 0)
            else {
                return false;
            };
            self.record_pending_arrival(agents.pending_household_composition(agent_idx));
            true
        });
        agents.arrival_carriers = carriers;
    }

    /// Turns an arrival carrier that reached its home into a resident household; false while
    /// it is still on its way or once it carries no household. O(household size).
    pub(crate) fn materialize_arrival_carrier(
        &mut self,
        agent_idx: usize,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) -> bool {
        let pending_size = agents.pending_household_size[agent_idx];
        let home = agents.home_building[agent_idx];
        if pending_size == 0
            || home >= allocator.buildings.len()
            || agents.transit[agent_idx] != TRANSIT_IN_BUILDING
            || agents.current_building[agent_idx] != home
        {
            return false;
        }
        let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
            panic!("could not load built-in economy catalog during carrier arrival: {err}")
        });
        let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
            panic!("could not load built-in economy tuning during carrier arrival: {err}")
        });
        let composition = agents.pending_household_composition(agent_idx);
        let household_id =
            self.admit_immigrant_household(&catalog, &tuning, home, pending_size, composition);
        let home_door = allocator
            .entrances
            .get(home)
            .map(|entrance| entrance.door_pos);
        let carrier_age_group = composition.member_age_group(0);
        agents.materialize_household_carrier(agent_idx, household_id, carrier_age_group, home_door);
        for member_index in 1..pending_size {
            let (x, y) = home_door
                .map(|door| (door.x, door.y))
                .unwrap_or((agents.pos_x[agent_idx], agents.pos_y[agent_idx]));
            let age_group = composition.member_age_group(member_index);
            let resident_idx = agents.spawn_housed_agent_with_age_group(home, x, y, age_group);
            agents.assign_household_id(resident_idx, household_id);
        }
        // Counted from its members at once, as a reload rebuilds its car access from them.
        self.recount_household(household_id, agents, allocator, &catalog);
        for category in [crate::debug::Channel::Economy, crate::debug::Channel::Spawn] {
            debug_log!(
                category,
                "household arrival carrier materialized household_id={} size={} home_building={} carrier_agent={}",
                household_id,
                pending_size,
                home,
                agent_idx,
            );
        }
        true
    }

    /// Test driver: materializes every carrier at its home in dense order, then recounts.
    /// Returns the households and residents created.
    #[cfg(test)]
    pub(crate) fn materialize_arrived_household_carriers(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) -> (usize, usize) {
        let mut materialized = (0, 0);
        for agent_idx in 0..agents.len() {
            let members = usize::from(agents.pending_household_size[agent_idx]);
            if self.materialize_arrival_carrier(agent_idx, agents, allocator) {
                materialized.0 += 1;
                materialized.1 += members;
            }
        }
        agents.rebuild_arrival_carriers();
        self.recount_pending_arrivals(agents);
        materialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admission_preserves_ledger_indices_after_restoring_bare_records() {
        let catalog = load_runtime_economy_catalog().unwrap();
        let tuning = load_runtime_economy_tuning().unwrap();
        let mut households = HouseholdSystem::new();
        households.admit_immigrant_household(
            &catalog,
            &tuning,
            0,
            1,
            crate::simulation::economy::agents::household_age_composition(0, 0, 1),
        );
        households.admit_immigrant_household(
            &catalog,
            &tuning,
            1,
            4,
            crate::simulation::economy::agents::household_age_composition(1, 1, 4),
        );
        households.households[0].budget = 1_234.0;
        households.households[1].budget = 4_321.0;
        // Bare records from older saves need their ledger slots before another admission.
        let mut restored = HouseholdSystem::new();
        restored.households = households.households;
        assert!(restored.daily_ledgers.is_empty());
        let admitted = restored.admit_immigrant_household(
            &catalog,
            &tuning,
            2,
            2,
            crate::simulation::economy::agents::household_age_composition(2, 2, 2),
        );
        assert_eq!(admitted, 2);
        assert_eq!(restored.daily_ledgers.len(), 3);
        for (household, ledger) in restored.households.iter().zip(&restored.daily_ledgers) {
            assert_eq!(ledger.budget_before, household.budget);
            assert_eq!(ledger.budget_after, household.budget);
        }
    }
}
