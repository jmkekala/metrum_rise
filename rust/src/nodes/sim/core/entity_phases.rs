// SPDX-License-Identifier: GPL-2.0-only

//! Per-entity hourly economy (`SIM-02-F`, `docs/simulation.md#phases`).
//!
//! Every building, household and agent runs its operational hour at its own hourly phase
//! instead of all at once on the hour: building production with farm and mine output,
//! household consumption, utilities and shopping-leg timeouts, and the happiness cost of an
//! empty household. Each slot visits the entities of one hourly phase in handle order, about
//! N / 960 of them.

use super::state::SimCore;
use crate::simulation::core::phases::PHASES_PER_HOUR;
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::households::produce_building_hour;

// Hourly phase of `slot`.
fn hour_phase(slot: u64) -> u32 {
    (slot % u64::from(PHASES_PER_HOUR)) as u32
}

impl SimCore {
    /// Production of the buildings whose hourly phase is `slot`'s.
    pub(super) fn run_building_hours(&mut self, slot: u64) {
        let handles = &mut self.allocator.building_handles;
        handles.register_appended(self.allocator.buildings.len());
        let mut phase = std::mem::take(&mut self.building_phase_scratch);
        handles.phases().hourly(hour_phase(slot), &mut phase);
        if !phase.is_empty() {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            for &handle in &phase {
                let Some(building_idx) = self.allocator.building_handles.resolve(handle) else {
                    continue;
                };
                produce_building_hour(&catalog, &mut self.allocator.buildings[building_idx]);
                self.agriculture
                    .produce_for_building(building_idx, &mut self.allocator, &catalog);
                self.resource_extraction.produce_for_building(
                    building_idx,
                    &mut self.allocator,
                    &catalog,
                );
            }
        }
        self.building_phase_scratch = phase;
    }

    /// The hour of the households whose hourly phase is `slot`'s.
    pub(super) fn run_household_hours(&mut self, slot: u64) {
        let handles = &mut self.households.handles;
        handles.register_appended(self.households.households.len());
        let mut phase = std::mem::take(&mut self.household_phase_scratch);
        handles.phases().hourly(hour_phase(slot), &mut phase);
        self.households
            .run_household_phase(&phase, &mut self.agents, &mut self.allocator);
        self.household_phase_scratch = phase;
    }

    /// The hour of the agents whose hourly phase is `slot`'s.
    pub(super) fn run_agent_hours(&mut self, slot: u64) {
        self.agents.handles.register_appended(self.agents.len());
        let mut phase = std::mem::take(&mut self.agent_phase_scratch);
        self.agents
            .handles
            .phases()
            .hourly(hour_phase(slot), &mut phase);
        self.households
            .starve_phase_agents(&phase, &mut self.agents);
        self.agent_phase_scratch = phase;
    }

    /// Every entity's hour at once, phase by phase, for callers that step a whole operational
    /// hour without the slot clock (tests and tools).
    pub(super) fn run_every_hour_phase(&mut self) {
        for phase in 0..u64::from(PHASES_PER_HOUR) {
            self.run_building_hours(phase);
            self.run_household_hours(phase);
            self.run_agent_hours(phase);
        }
    }
}
