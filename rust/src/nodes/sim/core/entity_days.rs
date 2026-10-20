// SPDX-License-Identifier: GPL-2.0-only

//! Per-entity daily settlement (`SIM-02-G`, `docs/simulation.md#phases`).
//!
//! Every building, household and agent settles its own 24-hour window at its daily phase
//! instead of all at midnight: a building's zoning check (an expired one queues its removal,
//! `SIM-02-J`), bankruptcy, taxes, bills and distress; a household's transfers, taxes and bills, its housing review and exit check; a
//! worker's wage and its daily happiness. Each slot visits the entities of one daily phase in
//! handle order, about N / 23,040 of them. The calendar close in `state.rs` only rolls the city's
//! day totals.

use super::frame::EconomyObserver;
use super::state::SimCore;
use crate::debug::work::{Work, add as add_work};
use crate::simulation::buildings::allocator::{BuildingRemovalKind, BuildingRemovalRequest};
use crate::simulation::core::phases::PHASES_PER_DAY;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

// Daily phase of `slot`.
fn day_phase(slot: u64) -> u32 {
    (slot % u64::from(PHASES_PER_DAY)) as u32
}

impl SimCore {
    /// The day of the buildings whose daily phase is `slot`'s.
    pub(super) fn run_building_days(&mut self, slot: u64, observer: &mut impl EconomyObserver) {
        let handles = &mut self.allocator.building_handles;
        handles.register_appended(self.allocator.buildings.len());
        let mut phase = std::mem::take(&mut self.building_phase_scratch);
        handles.phases().daily(day_phase(slot), &mut phase);
        if !phase.is_empty() {
            let day = self.households.day_settlement();
            for &handle in &phase {
                let Some(building_idx) = self.allocator.building_handles.resolve(handle) else {
                    continue;
                };
                // A building whose attachment went or whose rezoning grace ran out waits for
                // the `building removals` queue (`SIM-02-J`); its day ends with that decision,
                // as it ended with the removal before.
                if self.allocator.compatibility_expired(
                    building_idx,
                    1,
                    &self.zoning,
                    &self.region_graph,
                ) {
                    self.allocator
                        .removal_requests
                        .push_back(BuildingRemovalRequest {
                            slot,
                            building: handle,
                            kind: BuildingRemovalKind::Incompatible,
                        });
                    continue;
                }
                observer.building_day_opened(self, building_idx);
                let revenue = self.households.settle_building_day(
                    building_idx,
                    &mut self.allocator,
                    &day,
                    &self.fiscal_policy,
                    &mut self.treasury.balance,
                );
                self.collect_fiscal_revenue(revenue);
                observer.building_day_closed(self, building_idx);
                self.allocator.roll_daily_accumulators(building_idx);
                add_work(Work::BuildingSettlements, 1);
            }
        }
        self.building_phase_scratch = phase;
    }

    // Derived state after a slot removed buildings, as the daily maintenance repaired it.
    pub(super) fn finish_building_removals(&mut self, observer: &mut impl EconomyObserver) {
        self.allocator.finish_phase_removals(
            &self.zoning,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        self.publish_pending_building_site_changes();
        observer.buildings_changed(self);
        self.mark_changed_zone_flow_fields();
    }

    /// The day of the households whose daily phase is `slot`'s.
    pub(super) fn run_household_days(&mut self, slot: u64, observer: &mut impl EconomyObserver) {
        let handles = &mut self.households.handles;
        handles.register_appended(self.households.households.len());
        let mut phase = std::mem::take(&mut self.household_phase_scratch);
        handles.phases().daily(day_phase(slot), &mut phase);
        if !phase.is_empty() {
            let day = self.households.day_settlement();
            let exit_policy = self.demand.household_exit_policy();
            for &handle in &phase {
                let Some(household_id) = self.households.handles.resolve(handle) else {
                    continue;
                };
                let revenue = self.households.settle_household_day(
                    household_id,
                    &self.allocator,
                    &day,
                    &self.fiscal_policy,
                    &mut self.treasury.balance,
                );
                self.collect_fiscal_revenue(revenue);
                observer.household_day_closed(self, household_id);
                self.households.open_household_window(household_id);
                self.households.request_housing_review(household_id, slot);
                if self
                    .households
                    .exit_eligible(household_id, exit_policy, &self.allocator)
                    && self.demand.claim_household_exit()
                {
                    self.households.queue_exit(household_id);
                }
                add_work(Work::HouseholdSettlements, 1);
            }
        }
        self.household_phase_scratch = phase;
    }

    /// The day of the agents whose daily phase is `slot`'s: wages and daily happiness.
    pub(super) fn run_agent_days(&mut self, slot: u64, observer: &mut impl EconomyObserver) {
        self.agents.handles.register_appended(self.agents.len());
        let mut phase = std::mem::take(&mut self.agent_phase_scratch);
        self.agents
            .handles
            .phases()
            .daily(day_phase(slot), &mut phase);
        if !phase.is_empty() {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            let liquidation = load_runtime_economy_tuning()
                .unwrap_or_else(|err| {
                    panic!("could not load built-in economy runtime tuning: {err}")
                })
                .owa_distress_liquidation_multiplier;
            for &handle in &phase {
                let Some(agent_idx) = self.agents.handles.resolve(handle) else {
                    continue;
                };
                if let Some(payment) = self.households.pay_agent_daily_wage(
                    agent_idx,
                    &mut self.agents,
                    &mut self.allocator,
                    &catalog,
                    liquidation,
                    self.fiscal_policy.income_tax_rate,
                    &mut self.treasury.balance,
                ) {
                    self.treasury
                        .collect_income_tax(f64::from(payment.income_tax));
                    observer.wage_paid(self, payment);
                }
                self.agents
                    .daily_update_agent(agent_idx, &self.pollution, &self.config);
            }
        }
        self.agent_phase_scratch = phase;
    }
}
