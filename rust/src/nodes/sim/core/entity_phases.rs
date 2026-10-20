// SPDX-License-Identifier: GPL-2.0-only

//! Per-entity hourly economy (`SIM-02-F`, `docs/simulation.md#phases`).
//!
//! Every building, household and agent runs its operational hour at its own hourly phase
//! instead of all at once on the hour: building production with farm and mine output,
//! household consumption, utilities and shopping-leg timeouts, and the happiness cost of an
//! empty household. Each slot visits the entities of one hourly phase in handle order, about
//! N / 960 of them.
//!
//! Since `SIM-02-I` the city-wide hourly passes run there too: a building sets its activity
//! floor from the hour's census before production, then draws its service sales and meters its
//! electricity; a household buys services and meters its electricity. They read the city totals
//! of the last finished hour, and the hour's last slot folds this hour's (`city hour`). Since
//! `SIM-02-J` a building last checks its freight and queues an inbound or export request for
//! the `freight requests` queue; a household queues its rehousing, restock and visible visit,
//! and an agent free to look for work its job search, for their queues.

use super::state::SimCore;
use crate::simulation::core::phases::PHASES_PER_HOUR;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::households::{
    ElectricityProfiles, produce_building_hour, refresh_activity_floor,
};

// Hourly phase of `slot`.
fn hour_phase(slot: u64) -> u32 {
    (slot % u64::from(PHASES_PER_HOUR)) as u32
}

impl SimCore {
    /// The hour of the buildings whose hourly phase is `slot`'s: activity floor, production,
    /// service sales, electricity and the freight check.
    pub(super) fn run_building_hours(&mut self, slot: u64) {
        let handles = &mut self.allocator.building_handles;
        handles.register_appended(self.allocator.buildings.len());
        let mut phase = std::mem::take(&mut self.building_phase_scratch);
        handles.phases().hourly(hour_phase(slot), &mut phase);
        if !phase.is_empty() {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
                panic!("could not load built-in economy runtime tuning: {err}")
            });
            let electricity = ElectricityProfiles::new(&catalog);
            for &handle in &phase {
                let Some(building_idx) = self.allocator.building_handles.resolve(handle) else {
                    continue;
                };
                let building = &mut self.allocator.buildings[building_idx];
                refresh_activity_floor(&catalog, building);
                produce_building_hour(&catalog, building);
                self.agriculture
                    .produce_for_building(building_idx, &mut self.allocator, &catalog);
                self.resource_extraction.produce_for_building(
                    building_idx,
                    &mut self.allocator,
                    &catalog,
                );
                self.households.settle_store_service_sales(
                    &catalog,
                    &mut self.allocator.buildings[building_idx],
                );
                self.households.run_building_electricity(
                    &electricity,
                    &mut self.allocator,
                    building_idx,
                );
                self.logistics.request_freight_at_phase(
                    slot,
                    building_idx,
                    &self.allocator,
                    &catalog,
                    &tuning,
                );
            }
        }
        self.building_phase_scratch = phase;
    }

    /// The hour of the households whose hourly phase is `slot`'s, then their requests.
    pub(super) fn run_household_hours(&mut self, slot: u64) {
        let handles = &mut self.households.handles;
        handles.register_appended(self.households.households.len());
        let mut phase = std::mem::take(&mut self.household_phase_scratch);
        handles.phases().hourly(hour_phase(slot), &mut phase);
        self.households
            .run_household_phase(&phase, &mut self.agents, &mut self.allocator);
        self.households
            .queue_household_requests(&phase, slot, &self.agents, &self.allocator);
        self.household_phase_scratch = phase;
    }

    /// The hour of the agents whose hourly phase is `slot`'s, then their job searches.
    pub(super) fn run_agent_hours(&mut self, slot: u64) {
        self.agents.handles.register_appended(self.agents.len());
        let mut phase = std::mem::take(&mut self.agent_phase_scratch);
        self.agents
            .handles
            .phases()
            .hourly(hour_phase(slot), &mut phase);
        self.households
            .starve_phase_agents(&phase, &mut self.agents);
        self.households
            .queue_job_requests(&phase, slot, &mut self.agents, &mut self.allocator);
        self.agent_phase_scratch = phase;
    }

    /// Folds the hour's city totals at its last slot, after every phase of the hour has run:
    /// service-sales prices and pool and electricity dispatch and coverage for the next hour.
    /// O(1) in the city's size.
    pub(super) fn advance_city_hour(&mut self, slot: u64) {
        if slot % SLOTS_PER_HOUR == SLOTS_PER_HOUR - 1 {
            self.close_city_hour();
        }
    }

    fn close_city_hour(&mut self) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.households.close_city_hour(&catalog);
        self.close_demand_queue_hour();
    }
}
