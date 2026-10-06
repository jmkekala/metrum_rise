// SPDX-License-Identifier: GPL-2.0-only

//! Demand diagnostics snapshots and debug logging.

use super::system::DemandSystem;
use super::types::DemandUse;
use crate::debug_log;

/// Inputs and decisions from the latest household admission pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct HouseholdAdmissionDiagnostics {
    pub(crate) total_household_count: u32,
    pub(crate) vacant_household_slots: u32,
    pub(crate) connected_border_count: u32,
    pub(crate) housing_availability: f32,
    pub(crate) incoming_household_need: f32,
    pub(crate) open_job_household_pull: f32,
    pub(crate) marginal_commercial_job_household_pull: f32,
    pub(crate) regional_growth_household_pull: f32,
    pub(crate) household_affordability: f32,
    pub(crate) move_in_acceptance: f32,
    pub(crate) residential_construction_viability: f32,
    pub(crate) move_in_search_runway_days: f32,
    pub(crate) candidate_household_size: f32,
    pub(crate) candidate_child_count: u16,
    pub(crate) candidate_adult_count: u16,
    pub(crate) candidate_elder_count: u16,
    pub(crate) candidate_effective_workers: f32,
    pub(crate) open_job_slots: u32,
    pub(crate) marginal_commercial_job_slots: u32,
    pub(crate) marginal_commercial_job_equivalent_slots: f32,
    pub(crate) move_in_job_slots: u32,
    pub(crate) move_in_job_equivalent_slots: f32,
    pub(crate) physical_worker_capacity: u32,
    pub(crate) funded_worker_capacity: u32,
    pub(crate) open_jobs_unfunded: u32,
    pub(crate) existing_unemployed_member_count: u32,
    pub(crate) expected_employed_members: f32,
    pub(crate) expected_unemployed_members: f32,
    pub(crate) expected_entry_wage_per_day: f32,
    pub(crate) expected_wage_income_per_day: f32,
    pub(crate) transfer_reliability: f32,
    pub(crate) existing_transfer_claim_per_day: f32,
    pub(crate) candidate_unemployment_claim_per_day: f32,
    pub(crate) candidate_pension_claim_per_day: f32,
    pub(crate) candidate_child_support_claim_per_day: f32,
    pub(crate) candidate_transfer_claim_per_day: f32,
    pub(crate) total_transfer_claim_per_day: f32,
    pub(crate) expected_transfer_income_per_day: f32,
    pub(crate) starter_savings: f32,
    pub(crate) daily_essential_cost: f32,
    pub(crate) daily_deficit: f32,
    pub(crate) unhoused_household_ratio: f32,
    pub(crate) unhoused_factor: f32,
    pub(crate) zero_budget_household_ratio: f32,
    pub(crate) zero_budget_factor: f32,
    pub(crate) failure_factor: f32,
    pub(crate) recent_failure_pressure: f32,
    pub(crate) recent_failure_factor: f32,
    pub(crate) base_pressure: f32,
    pub(crate) pressure: f32,
    pub(crate) threshold: f32,
    pub(crate) normalized_action_pressure: f32,
    pub(crate) credit_before: f32,
    pub(crate) credit_after: f32,
    pub(crate) max_actionable_households: u32,
    pub(crate) planned_households: u32,
    pub(crate) launched_households: u32,
}

/// Inputs and execution counts from the latest daily removal pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct HouseholdRemovalDiagnostics {
    pub(crate) total_household_count: u32,
    pub(crate) housed_household_count: u32,
    pub(crate) unhoused_household_count: u32,
    pub(crate) zero_budget_household_count: u32,
    pub(crate) persistent_exit_eligible_household_count: u32,
    pub(crate) unhoused_household_ratio: f32,
    pub(crate) zero_budget_household_ratio: f32,
    pub(crate) failure_pressure: f32,
    pub(crate) removed_household_ratio: f32,
    pub(crate) recent_failure_before: f32,
    pub(crate) recent_failure_after: f32,
    pub(crate) pressure: f32,
    pub(crate) threshold: f32,
    pub(crate) normalized_action_pressure: f32,
    pub(crate) credit_before: f32,
    pub(crate) credit_after: f32,
    pub(crate) persistent_exit_credit_before: f32,
    pub(crate) persistent_exit_credit_after: f32,
    pub(crate) persistent_exit_planned_households: u32,
    pub(crate) max_actionable_households: u32,
    pub(crate) planned_households: u32,
    pub(crate) removed_households: u32,
}

/// Latest private construction and closure decisions by zone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BuildingActionDiagnosticsByUse {
    pub(crate) residential: BuildingActionDiagnostics,
    pub(crate) commercial: BuildingActionDiagnostics,
    pub(crate) industrial: BuildingActionDiagnostics,
}

impl BuildingActionDiagnosticsByUse {
    pub(super) fn use_mut(&mut self, use_kind: DemandUse) -> &mut BuildingActionDiagnostics {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }

    pub(super) fn iter(self) -> [(DemandUse, BuildingActionDiagnostics); 3] {
        [
            (DemandUse::Residential, self.residential),
            (DemandUse::Commercial, self.commercial),
            (DemandUse::Industrial, self.industrial),
        ]
    }
}

/// Candidate, pressure, credit and absorption decisions for one zone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BuildingActionDiagnostics {
    pub(crate) pressure: f32,
    pub(crate) spawn_candidates: usize,
    pub(crate) spawn_profile_missing: usize,
    pub(crate) spawn_normalized_pressure: f32,
    pub(crate) spawn_need_buildings: f32,
    pub(crate) spawn_credit_before: f32,
    pub(crate) spawn_credit_after: f32,
    pub(crate) spawn_planned: usize,
    pub(crate) spawn_selected: usize,
    pub(crate) spawn_rejected_absorption: usize,
    /// Service candidates whose remaining market cannot fund the first worker and overhead.
    pub(crate) spawn_rejected_viability: usize,
    pub(crate) spawn_skipped_budget: usize,
    pub(crate) upgrade_candidates: usize,
    pub(crate) upgrade_normalized_pressure: f32,
    pub(crate) upgrade_budget_units: f32,
    pub(crate) upgrade_credit_before: f32,
    pub(crate) upgrade_credit_after: f32,
    pub(crate) upgrade_planned: usize,
    pub(crate) upgrade_selected: usize,
    pub(crate) downgrade_candidates: usize,
    pub(crate) downgrade_normalized_pressure: f32,
    pub(crate) downgrade_budget_units: f32,
    pub(crate) downgrade_credit_before: f32,
    pub(crate) downgrade_credit_after: f32,
    pub(crate) downgrade_planned: usize,
    pub(crate) downgrade_selected: usize,
    pub(crate) despawn_candidates: usize,
    pub(crate) despawn_normalized_pressure: f32,
    pub(crate) despawn_budget_units: f32,
    pub(crate) despawn_credit_before: f32,
    pub(crate) despawn_credit_after: f32,
    pub(crate) despawn_planned: usize,
    pub(crate) despawn_selected: usize,
}

impl DemandSystem {
    /// Borrows existing decision records for read-only headless scenario reporting.
    pub(crate) fn scenario_diagnostics(
        &self,
    ) -> (
        &HouseholdAdmissionDiagnostics,
        &HouseholdRemovalDiagnostics,
        &BuildingActionDiagnosticsByUse,
    ) {
        (
            &self.last_admission_diagnostics,
            &self.last_removal_diagnostics,
            &self.last_building_action_diagnostics,
        )
    }

    /// Returns the compact admission diagnostics needed to explain load-time recomputation.
    pub(crate) fn last_admission_debug_summary(
        &self,
    ) -> (u32, u32, u32, f32, f32, f32, f32, f32, f32, f32) {
        let diagnostics = self.last_admission_diagnostics;
        (
            diagnostics.vacant_household_slots,
            diagnostics.open_job_slots,
            diagnostics.move_in_job_slots,
            diagnostics.move_in_job_equivalent_slots,
            diagnostics.regional_growth_household_pull,
            diagnostics.open_job_household_pull,
            diagnostics.marginal_commercial_job_household_pull,
            diagnostics.incoming_household_need,
            diagnostics.move_in_acceptance,
            diagnostics.failure_factor,
        )
    }

    /// Emits the most recent household-admission pressure and credit breakdown.
    pub(crate) fn log_hourly_household_action_diagnostics(
        &self,
        day_index: u32,
        minute_of_day: u16,
    ) {
        let diagnostics = self.last_admission_diagnostics;
        debug_log!(
            "economy",
            "household admission diagnostics: day={} minute={} pressure={:.3} base={:.3} \
             vacancy={:.2} vacant_slots={} households={} border_nodes={} \
             incoming_need={:.2} job_pull={:.2} marginal_com_pull={:.2} regional_pull={:.2} \
             afford={:.2} accept={:.2} runway={:.2} \
             build_viability={:.2} \
             candidate_size={:.1} candidate=(children:{} adults:{} elders:{}) workers={:.1} open_jobs={} marginal_com_jobs={} marginal_com_job_equiv={:.2} move_in_jobs={} move_in_job_equiv={:.2} physical_worker_capacity={} \
             funded_worker_capacity={} open_jobs_unfunded={} existing_unemployed={} \
             expected_employed={:.1} expected_unemployed={:.1} entry_wage={:.1} wage_income={:.1} \
             transfer_rel={:.2} existing_transfer_claim={:.1} candidate_unemployment_claim={:.1} \
             candidate_pension_claim={:.1} candidate_child_support_claim={:.1} \
             candidate_transfer_claim={:.1} total_transfer_claim={:.1} transfer_income={:.1} starter={:.1} daily_cost={:.1} \
             daily_deficit={:.1} unhoused_ratio={:.2} unhoused_factor={:.2} \
             zero_budget_ratio={:.2} zero_budget_factor={:.2} failure_factor={:.2} \
             recent_failure={:.2} recent_failure_factor={:.2} \
             threshold={:.2} norm={:.3} credit={:.3}->{:.3} cap={} plan={} launched={}",
            day_index,
            minute_of_day,
            diagnostics.pressure,
            diagnostics.base_pressure,
            diagnostics.housing_availability,
            diagnostics.vacant_household_slots,
            diagnostics.total_household_count,
            diagnostics.connected_border_count,
            diagnostics.incoming_household_need,
            diagnostics.open_job_household_pull,
            diagnostics.marginal_commercial_job_household_pull,
            diagnostics.regional_growth_household_pull,
            diagnostics.household_affordability,
            diagnostics.move_in_acceptance,
            diagnostics.move_in_search_runway_days,
            diagnostics.residential_construction_viability,
            diagnostics.candidate_household_size,
            diagnostics.candidate_child_count,
            diagnostics.candidate_adult_count,
            diagnostics.candidate_elder_count,
            diagnostics.candidate_effective_workers,
            diagnostics.open_job_slots,
            diagnostics.marginal_commercial_job_slots,
            diagnostics.marginal_commercial_job_equivalent_slots,
            diagnostics.move_in_job_slots,
            diagnostics.move_in_job_equivalent_slots,
            diagnostics.physical_worker_capacity,
            diagnostics.funded_worker_capacity,
            diagnostics.open_jobs_unfunded,
            diagnostics.existing_unemployed_member_count,
            diagnostics.expected_employed_members,
            diagnostics.expected_unemployed_members,
            diagnostics.expected_entry_wage_per_day,
            diagnostics.expected_wage_income_per_day,
            diagnostics.transfer_reliability,
            diagnostics.existing_transfer_claim_per_day,
            diagnostics.candidate_unemployment_claim_per_day,
            diagnostics.candidate_pension_claim_per_day,
            diagnostics.candidate_child_support_claim_per_day,
            diagnostics.candidate_transfer_claim_per_day,
            diagnostics.total_transfer_claim_per_day,
            diagnostics.expected_transfer_income_per_day,
            diagnostics.starter_savings,
            diagnostics.daily_essential_cost,
            diagnostics.daily_deficit,
            diagnostics.unhoused_household_ratio,
            diagnostics.unhoused_factor,
            diagnostics.zero_budget_household_ratio,
            diagnostics.zero_budget_factor,
            diagnostics.failure_factor,
            diagnostics.recent_failure_pressure,
            diagnostics.recent_failure_factor,
            diagnostics.threshold,
            diagnostics.normalized_action_pressure,
            diagnostics.credit_before,
            diagnostics.credit_after,
            diagnostics.max_actionable_households,
            diagnostics.planned_households,
            diagnostics.launched_households,
        );
    }

    /// Emits the most recent private-building pressure, candidate, credit, and gate breakdown.
    pub(crate) fn log_hourly_building_action_diagnostics(
        &self,
        day_index: u32,
        minute_of_day: u16,
    ) {
        for (use_kind, diagnostics) in self.last_building_action_diagnostics.iter() {
            let use_label = match use_kind {
                DemandUse::Residential => "Residential",
                DemandUse::Commercial => "Commercial",
                DemandUse::Industrial => "Industrial",
            };
            debug_log!(
                "economy",
                "building action diagnostics: day={} minute={} use={} pressure={:.3} \
                 spawn_candidates={} spawn_profile_missing={} spawn_norm={:.3} \
                 spawn_need={:.3} spawn_credit={:.3}->{:.3} spawn_plan={} \
                 spawn_selected={} spawn_reject_absorption={} spawn_reject_viability={} \
                 spawn_skip_budget={} \
                 upgrade_candidates={} upgrade_norm={:.3} upgrade_budget={:.3} \
                 upgrade_credit={:.3}->{:.3} upgrade_plan={} upgrade_selected={} \
                 downgrade_candidates={} downgrade_norm={:.3} downgrade_budget={:.3} \
                 downgrade_credit={:.3}->{:.3} downgrade_plan={} downgrade_selected={} \
                 despawn_candidates={} despawn_norm={:.3} despawn_budget={:.3} \
                 despawn_credit={:.3}->{:.3} despawn_plan={} despawn_selected={}",
                day_index,
                minute_of_day,
                use_label,
                diagnostics.pressure,
                diagnostics.spawn_candidates,
                diagnostics.spawn_profile_missing,
                diagnostics.spawn_normalized_pressure,
                diagnostics.spawn_need_buildings,
                diagnostics.spawn_credit_before,
                diagnostics.spawn_credit_after,
                diagnostics.spawn_planned,
                diagnostics.spawn_selected,
                diagnostics.spawn_rejected_absorption,
                diagnostics.spawn_rejected_viability,
                diagnostics.spawn_skipped_budget,
                diagnostics.upgrade_candidates,
                diagnostics.upgrade_normalized_pressure,
                diagnostics.upgrade_budget_units,
                diagnostics.upgrade_credit_before,
                diagnostics.upgrade_credit_after,
                diagnostics.upgrade_planned,
                diagnostics.upgrade_selected,
                diagnostics.downgrade_candidates,
                diagnostics.downgrade_normalized_pressure,
                diagnostics.downgrade_budget_units,
                diagnostics.downgrade_credit_before,
                diagnostics.downgrade_credit_after,
                diagnostics.downgrade_planned,
                diagnostics.downgrade_selected,
                diagnostics.despawn_candidates,
                diagnostics.despawn_normalized_pressure,
                diagnostics.despawn_budget_units,
                diagnostics.despawn_credit_before,
                diagnostics.despawn_credit_after,
                diagnostics.despawn_planned,
                diagnostics.despawn_selected,
            );
        }
    }

    /// Emits the most recent household-removal pressure and credit breakdown.
    pub(crate) fn log_daily_household_action_diagnostics(&self, day_index: u32) {
        let diagnostics = self.last_removal_diagnostics;
        debug_log!(
            "economy",
            "household removal diagnostics: day={} pressure={:.3} failure_pressure={:.3} \
             households={} housed={} unhoused={} zero_budget={} unhoused_ratio={:.2} \
             zero_budget_ratio={:.2} persistent_exit_eligible={} threshold={:.2} \
             norm={:.3} credit={:.3}->{:.3} persistent_credit={:.3}->{:.3} \
             persistent_plan={} \
             cap={} plan={} removed={} removed_ratio={:.3} recent_failure={:.3}->{:.3}",
            day_index,
            diagnostics.pressure,
            diagnostics.failure_pressure,
            diagnostics.total_household_count,
            diagnostics.housed_household_count,
            diagnostics.unhoused_household_count,
            diagnostics.zero_budget_household_count,
            diagnostics.unhoused_household_ratio,
            diagnostics.zero_budget_household_ratio,
            diagnostics.persistent_exit_eligible_household_count,
            diagnostics.threshold,
            diagnostics.normalized_action_pressure,
            diagnostics.credit_before,
            diagnostics.credit_after,
            diagnostics.persistent_exit_credit_before,
            diagnostics.persistent_exit_credit_after,
            diagnostics.persistent_exit_planned_households,
            diagnostics.max_actionable_households,
            diagnostics.planned_households,
            diagnostics.removed_households,
            diagnostics.removed_household_ratio,
            diagnostics.recent_failure_before,
            diagnostics.recent_failure_after,
        );
    }
}
