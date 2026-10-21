// SPDX-License-Identifier: GPL-2.0-only

//! Demand diagnostics snapshots and debug logging.

use super::snapshot::DailyDemandSnapshot;
use super::spawn_need::residential_vacancy_reserve;
use super::system::DemandSystem;
use super::types::DemandUse;
use crate::debug_log;

/// Inputs and decisions from the latest household admission pass.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct HouseholdAdmissionDiagnostics {
    pub(crate) total_household_count: u32,
    pub(crate) vacant_household_slots: u32,
    pub(crate) connected_border_count: u32,
    pub(crate) incoming_household_need: f32,
    pub(crate) open_job_slots: u32,
    pub(crate) move_in_job_slots: u32,
    pub(crate) physical_worker_capacity: u32,
    pub(crate) funded_worker_capacity: u32,
    pub(crate) open_jobs_unfunded: u32,
    pub(crate) existing_unemployed_member_count: u32,
    pub(crate) planned_households: u32,
    pub(crate) launched_households: u32,
    /// Actual applicants checked against the batch's remaining recurring income.
    pub(crate) examined_candidates: u32,
    /// Applicants rejected without consuming wages, transfers or a home.
    pub(crate) income_rejections: u32,
    /// Nonworker applicants that would consume housing already promised to a workforce.
    pub(crate) worker_housing_rejections: u32,
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
    pub(crate) removed_household_ratio: f32,
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

/// Candidate, pacing, reservation and absorption decisions for one zone.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BuildingActionDiagnostics {
    pub(crate) spawn_candidates: usize,
    /// Whether an hourly planner has produced this record since world creation/load.
    pub(crate) assessed: bool,
    /// Legal asset and free-lot availability before economic decisions.
    pub(crate) sites: super::actions::SpawnSiteAvailability,
    /// Projects already physically under construction at assessment time.
    pub(crate) constructing: u32,
    /// Earlier projects still waiting in the placement queue at assessment time.
    pub(crate) queued: u32,
    /// Missing household places before this pass selects actual residential capacities.
    pub(crate) housing_needed_slots: u32,
    pub(crate) spawn_planned: usize,
    pub(crate) spawn_selected: usize,
    pub(crate) spawn_rejected_absorption: usize,
    /// Service candidates whose remaining market cannot fund the first worker and overhead.
    pub(crate) spawn_rejected_viability: usize,
    /// Candidates whose workforce cannot be reserved by their construction completion.
    pub(crate) spawn_rejected_workforce: usize,
    pub(crate) spawn_not_needed: usize,
    pub(crate) upgrade_candidates: usize,
    pub(crate) upgrade_budget_units: f32,
    pub(crate) upgrade_credit_before: f32,
    pub(crate) upgrade_credit_after: f32,
    pub(crate) upgrade_planned: usize,
    pub(crate) upgrade_selected: usize,
    pub(crate) downgrade_candidates: usize,
    pub(crate) downgrade_budget_units: f32,
    pub(crate) downgrade_credit_before: f32,
    pub(crate) downgrade_credit_after: f32,
    pub(crate) downgrade_planned: usize,
    pub(crate) downgrade_selected: usize,
    pub(crate) despawn_candidates: usize,
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

    /// Returns the current admission opportunities without evaluating a hypothetical family.
    pub(crate) fn last_admission_debug_summary(&self) -> &HouseholdAdmissionDiagnostics {
        &self.last_admission_diagnostics
    }

    /// Housing balance of the latest pass (`DEM-02-C`): net open jobs arrivals must staff,
    /// the vacant places housing construction retains, and households the latest daily pass
    /// removed.
    pub(crate) fn housing_balance(&self) -> (u32, u32, u32) {
        let admission = &self.last_admission_diagnostics;
        (
            admission.move_in_job_slots,
            residential_vacancy_reserve(admission.total_household_count),
            self.last_removal_diagnostics.removed_households,
        )
    }

    /// Emits actual batch capacity and applicant decisions from the most recent hour.
    pub(crate) fn log_hourly_household_action_diagnostics(
        &self,
        day_index: u32,
        minute_of_day: u16,
    ) {
        let d = self.last_admission_diagnostics;
        debug_log!(
            "economy",
            "household admission: day={} minute={} border_nodes={} homes={} open_jobs={} \
             available_jobs={} local_unemployed={} funded_capacity={} physical_capacity={} \
             unfunded_jobs={} plan={} examined={} income_rejected={} worker_housing_rejected={} launched={}",
            day_index,
            minute_of_day,
            d.connected_border_count,
            d.vacant_household_slots,
            d.open_job_slots,
            d.move_in_job_slots,
            d.existing_unemployed_member_count,
            d.funded_worker_capacity,
            d.physical_worker_capacity,
            d.open_jobs_unfunded,
            d.planned_households,
            d.examined_candidates,
            d.income_rejections,
            d.worker_housing_rejections,
            d.launched_households,
        );
    }

    /// Emits the most recent private-building candidate, credit, and gate breakdown.
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
                "building action diagnostics: day={} minute={} use={} \
                 spawn_candidates={} \
                 housing_needed_slots={} spawn_plan={} \
                 spawn_selected={} spawn_reject_absorption={} spawn_reject_viability={} \
                 spawn_reject_workforce={} spawn_not_needed={} \
                 upgrade_candidates={} upgrade_budget={:.3} \
                 upgrade_credit={:.3}->{:.3} upgrade_plan={} upgrade_selected={} \
                 downgrade_candidates={} downgrade_budget={:.3} \
                 downgrade_credit={:.3}->{:.3} downgrade_plan={} downgrade_selected={} \
                 despawn_candidates={} despawn_budget={:.3} \
                 despawn_credit={:.3}->{:.3} despawn_plan={} despawn_selected={}",
                day_index,
                minute_of_day,
                use_label,
                diagnostics.spawn_candidates,
                diagnostics.housing_needed_slots,
                diagnostics.spawn_planned,
                diagnostics.spawn_selected,
                diagnostics.spawn_rejected_absorption,
                diagnostics.spawn_rejected_viability,
                diagnostics.spawn_rejected_workforce,
                diagnostics.spawn_not_needed,
                diagnostics.upgrade_candidates,
                diagnostics.upgrade_budget_units,
                diagnostics.upgrade_credit_before,
                diagnostics.upgrade_credit_after,
                diagnostics.upgrade_planned,
                diagnostics.upgrade_selected,
                diagnostics.downgrade_candidates,
                diagnostics.downgrade_budget_units,
                diagnostics.downgrade_credit_before,
                diagnostics.downgrade_credit_after,
                diagnostics.downgrade_planned,
                diagnostics.downgrade_selected,
                diagnostics.despawn_candidates,
                diagnostics.despawn_budget_units,
                diagnostics.despawn_credit_before,
                diagnostics.despawn_credit_after,
                diagnostics.despawn_planned,
                diagnostics.despawn_selected,
            );
        }
    }

    /// Reports household-specific exit eligibility and the remaining pacing credit.
    pub(crate) fn log_daily_household_action_diagnostics(&self, day_index: u32) {
        let d = self.last_removal_diagnostics;
        debug_log!(
            "economy",
            "household removal: day={} households={} housed={} unhoused={} zero_budget={} eligible={} credit={:.3}->{:.3} plan={} removed={}",
            day_index,
            d.total_household_count,
            d.housed_household_count,
            d.unhoused_household_count,
            d.zero_budget_household_count,
            d.persistent_exit_eligible_household_count,
            d.persistent_exit_credit_before,
            d.persistent_exit_credit_after,
            d.planned_households,
            d.removed_households,
        );
    }
}

impl HouseholdAdmissionDiagnostics {
    /// Copies the concrete opportunity/eligibility inputs without advancing decisions.
    pub(super) fn from_snapshot(snapshot: &DailyDemandSnapshot) -> Self {
        HouseholdAdmissionDiagnostics {
            total_household_count: snapshot.total_household_count,
            vacant_household_slots: snapshot.vacant_household_slots,
            connected_border_count: snapshot.connected_border_count,
            // Residential planning fills in the applicant walk's result.
            incoming_household_need: 0.0,
            open_job_slots: snapshot.open_job_slots,
            move_in_job_slots: snapshot.move_in_job_slots,
            physical_worker_capacity: snapshot.physical_worker_capacity,
            funded_worker_capacity: snapshot.funded_worker_capacity,
            open_jobs_unfunded: snapshot.open_jobs_unfunded,
            existing_unemployed_member_count: snapshot.existing_unemployed_member_count,
            ..HouseholdAdmissionDiagnostics::default()
        }
    }
}

impl HouseholdRemovalDiagnostics {
    /// Copies the concrete opportunity/eligibility inputs without advancing decisions.
    pub(super) fn from_snapshot(snapshot: &DailyDemandSnapshot) -> Self {
        HouseholdRemovalDiagnostics {
            total_household_count: snapshot.total_household_count,
            housed_household_count: snapshot.housed_household_count,
            unhoused_household_count: snapshot.unhoused_household_count,
            zero_budget_household_count: snapshot.zero_budget_household_count,
            unhoused_household_ratio: snapshot.unhoused_household_ratio,
            zero_budget_household_ratio: snapshot.zero_budget_household_ratio,
            persistent_exit_eligible_household_count: snapshot
                .persistent_exit_eligible_household_count,
            ..HouseholdRemovalDiagnostics::default()
        }
    }
}

// A diagnostics field of four bytes, saved little-endian (`f32` by its bits).
trait SavedField: Sized {
    fn from_saved(bytes: [u8; 4]) -> Self;
}

impl SavedField for u32 {
    fn from_saved(bytes: [u8; 4]) -> Self {
        u32::from_le_bytes(bytes)
    }
}

impl SavedField for f32 {
    fn from_saved(bytes: [u8; 4]) -> Self {
        f32::from_le_bytes(bytes)
    }
}

// The saved form of a diagnostics struct: every listed field in order, four bytes each. A field
// left out of the list is not saved and loads as its default.
macro_rules! saved_diagnostics {
    ($name:ident { $($field:ident),* $(,)? }) => {
        impl $name {
            fn to_saved(self) -> Vec<u8> {
                let mut out = Vec::new();
                $(out.extend_from_slice(&self.$field.to_le_bytes());)*
                out
            }

            fn from_saved(bytes: &[u8]) -> Option<Self> {
                let mut rest = bytes;
                let mut value = Self::default();
                $(
                    let (head, tail) = rest.split_first_chunk::<4>()?;
                    value.$field = SavedField::from_saved(*head);
                    rest = tail;
                )*
                rest.is_empty().then_some(value)
            }
        }
    };
}

saved_diagnostics!(HouseholdAdmissionDiagnostics {
    total_household_count,
    vacant_household_slots,
    connected_border_count,
    incoming_household_need,
    open_job_slots,
    move_in_job_slots,
    physical_worker_capacity,
    funded_worker_capacity,
    open_jobs_unfunded,
    existing_unemployed_member_count,
    planned_households,
    launched_households,
    examined_candidates,
    income_rejections,
    worker_housing_rejections,
});

saved_diagnostics!(HouseholdRemovalDiagnostics {
    total_household_count,
    housed_household_count,
    unhoused_household_count,
    zero_budget_household_count,
    persistent_exit_eligible_household_count,
    unhoused_household_ratio,
    zero_budget_household_ratio,
    removed_household_ratio,
    persistent_exit_credit_before,
    persistent_exit_credit_after,
    persistent_exit_planned_households,
    max_actionable_households,
    planned_households,
    removed_households,
});

impl DemandSystem {
    /// The saved forms of the last admission and removal diagnostics (`SIM-02-K`): the actions
    /// slot and the daily removal pass set them from the state of their slot, which a load
    /// cannot recompute later in the hour, so a save keeps them and a loaded game shows what
    /// the saved one showed.
    pub(crate) fn saved_household_diagnostics(&self) -> [Vec<u8>; 2] {
        [
            self.last_admission_diagnostics.to_saved(),
            self.last_removal_diagnostics.to_saved(),
        ]
    }

    /// Restores [`Self::saved_household_diagnostics`]; a malformed buffer is an error.
    pub(crate) fn restore_household_diagnostics(
        &mut self,
        admission: &[u8],
        removal: &[u8],
    ) -> Result<(), String> {
        self.last_admission_diagnostics = HouseholdAdmissionDiagnostics::from_saved(admission)
            .ok_or("malformed saved admission diagnostics")?;
        self.last_removal_diagnostics = HouseholdRemovalDiagnostics::from_saved(removal)
            .ok_or("malformed saved removal diagnostics")?;
        Ok(())
    }
}
