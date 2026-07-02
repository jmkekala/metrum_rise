//! Demand system state and pass orchestration.

use super::actions::DemandBuildingActionPlan;
use super::config::{DemandConfig, load_builtin_demand_config};
use super::credits::{
    advance_household_action_credit, advance_persistent_exit_credit, clamp01,
    normalized_positive_pressure,
};
use super::diagnostics::{
    BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics,
};
use super::snapshot::{
    DailyDemandSnapshot, ResidentialOccupantScratch, ResidentialOccupantSnapshot,
};
use super::types::DEMAND_HOURLY_CADENCE_FRACTION;
use super::{UseTuningBool, UseTuningF32};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoningSystem;
use std::sync::Arc;

/// Demand-owned daily growth state derived from the settled economy snapshot.
pub struct DemandSystem {
    pub(super) config: Arc<DemandConfig>,
    pub(super) runtime_catalog: Arc<RuntimeEconomyCatalog>,
    pub(super) runtime_tuning: Arc<RuntimeEconomyTuning>,
    pub(crate) residential: f32,
    pub(crate) commercial: f32,
    pub(crate) industrial: f32,
    pub(crate) households_to_admit_today: u32,
    pub(crate) households_to_remove_today: u32,
    pub(crate) admission_action_credit: f32,
    pub(crate) removal_action_credit: f32,
    pub(crate) persistent_exit_action_credit: f32,
    pub(crate) spawn_action_credit: UseTuningF32,
    pub(crate) upgrade_action_credit: UseTuningF32,
    pub(crate) downgrade_action_credit: UseTuningF32,
    pub(crate) despawn_action_credit: UseTuningF32,
    pub(crate) spawn_hysteresis_active: UseTuningBool,
    pub(crate) upgrade_hysteresis_active: UseTuningBool,
    pub(crate) downgrade_hysteresis_active: UseTuningBool,
    pub(crate) despawn_hysteresis_active: UseTuningBool,
    pub(crate) recent_household_failure_pressure: f32,
    pub(crate) building_actions: DemandBuildingActionPlan,
    pub(super) last_admission_diagnostics: HouseholdAdmissionDiagnostics,
    pub(super) last_removal_diagnostics: HouseholdRemovalDiagnostics,
    pub(super) last_building_action_diagnostics: BuildingActionDiagnosticsByUse,
    pub(super) residential_occupant_scratch: ResidentialOccupantScratch,
}

impl DemandSystem {
    /// Creates a new demand system using the shipped demand tuning file.
    pub fn new() -> Self {
        let config = load_builtin_demand_config()
            .unwrap_or_else(|err| panic!("could not load built-in demand tuning: {err}"));
        let runtime_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let runtime_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        Self {
            config,
            runtime_catalog,
            runtime_tuning,
            residential: 0.0,
            commercial: 0.0,
            industrial: 0.0,
            households_to_admit_today: 0,
            households_to_remove_today: 0,
            admission_action_credit: 0.0,
            removal_action_credit: 0.0,
            persistent_exit_action_credit: 0.0,
            spawn_action_credit: UseTuningF32::default(),
            upgrade_action_credit: UseTuningF32::default(),
            downgrade_action_credit: UseTuningF32::default(),
            despawn_action_credit: UseTuningF32::default(),
            spawn_hysteresis_active: UseTuningBool::default(),
            upgrade_hysteresis_active: UseTuningBool::default(),
            downgrade_hysteresis_active: UseTuningBool::default(),
            despawn_hysteresis_active: UseTuningBool::default(),
            recent_household_failure_pressure: 0.0,
            building_actions: DemandBuildingActionPlan::default(),
            last_admission_diagnostics: HouseholdAdmissionDiagnostics::default(),
            last_removal_diagnostics: HouseholdRemovalDiagnostics::default(),
            last_building_action_diagnostics: BuildingActionDiagnosticsByUse::default(),
            residential_occupant_scratch: ResidentialOccupantScratch::default(),
        }
    }

    /// Returns the cached compiled economy catalog used by demand planning and execution.
    pub(crate) fn runtime_catalog(&self) -> &RuntimeEconomyCatalog {
        self.runtime_catalog.as_ref()
    }

    /// Returns the cached economy runtime tuning used by demand planning and execution.
    pub(crate) fn runtime_tuning(&self) -> &RuntimeEconomyTuning {
        self.runtime_tuning.as_ref()
    }

    /// Refreshes RCI telemetry and advances hourly household and building demand outputs.
    #[cfg(test)]
    pub(crate) fn run_hourly_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
    ) {
        self.run_hourly_pass_with_service_funding(
            allocator,
            households,
            graph,
            zoning,
            treasury_balance,
            &[],
        );
    }

    /// Refreshes RCI telemetry and advances hourly demand using the supplied service funding state.
    pub(crate) fn run_hourly_pass_with_service_funding(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
        service_funding_by_building: &[f32],
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            &self.config,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            treasury_balance,
            service_funding_by_building,
        );
        let residential_occupants = ResidentialOccupantSnapshot::from_runtime_with_catalog(
            allocator,
            households,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            &mut self.residential_occupant_scratch,
        );

        let pressures = self.update_pressure_channels_from_snapshot(&snapshot);
        let admission_threshold = self.config.household_action.admission_threshold;
        let admission_credit_before = self.admission_action_credit;
        let normalized_admission_pressure =
            normalized_positive_pressure(pressures.admission_pressure, admission_threshold);
        self.households_to_admit_today = advance_household_action_credit(
            &mut self.admission_action_credit,
            pressures.admission_pressure,
            admission_threshold,
            self.config.action_budget.max_households_per_day,
            snapshot.vacant_household_slots,
            DEMAND_HOURLY_CADENCE_FRACTION,
        );
        let mut admission_diagnostics = pressures.admission_diagnostics;
        admission_diagnostics.threshold = admission_threshold;
        admission_diagnostics.normalized_action_pressure = normalized_admission_pressure;
        admission_diagnostics.credit_before = admission_credit_before;
        admission_diagnostics.credit_after = self.admission_action_credit;
        admission_diagnostics.max_actionable_households = snapshot.vacant_household_slots;
        admission_diagnostics.planned_households = self.households_to_admit_today;
        admission_diagnostics.launched_households = 0;
        self.last_admission_diagnostics = admission_diagnostics;
        self.plan_private_building_actions(
            allocator,
            households,
            graph,
            zoning,
            &snapshot,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            &residential_occupants,
            DEMAND_HOURLY_CADENCE_FRACTION,
            "hourly_pass",
        );
    }

    pub(crate) fn with_persisted_state(
        residential: f32,
        commercial: f32,
        industrial: f32,
        households_to_admit_today: u32,
        households_to_remove_today: u32,
        admission_action_credit: f32,
        removal_action_credit: f32,
        persistent_exit_action_credit: f32,
        spawn_action_credit: [f32; 3],
        upgrade_action_credit: [f32; 3],
        downgrade_action_credit: [f32; 3],
        despawn_action_credit: [f32; 3],
        spawn_hysteresis_active: [bool; 3],
        upgrade_hysteresis_active: [bool; 3],
        downgrade_hysteresis_active: [bool; 3],
        despawn_hysteresis_active: [bool; 3],
        recent_household_failure_pressure: f32,
    ) -> Self {
        let mut system = Self::new();
        system.residential = residential;
        system.commercial = commercial;
        system.industrial = industrial;
        system.households_to_admit_today = households_to_admit_today;
        system.households_to_remove_today = households_to_remove_today;
        system.admission_action_credit = admission_action_credit;
        system.removal_action_credit = removal_action_credit;
        system.persistent_exit_action_credit = persistent_exit_action_credit.max(0.0);
        system.spawn_action_credit = UseTuningF32 {
            residential: spawn_action_credit[0],
            commercial: spawn_action_credit[1],
            industrial: spawn_action_credit[2],
        };
        system.upgrade_action_credit = UseTuningF32 {
            residential: upgrade_action_credit[0],
            commercial: upgrade_action_credit[1],
            industrial: upgrade_action_credit[2],
        };
        system.downgrade_action_credit = UseTuningF32 {
            residential: downgrade_action_credit[0],
            commercial: downgrade_action_credit[1],
            industrial: downgrade_action_credit[2],
        };
        system.despawn_action_credit = UseTuningF32 {
            residential: despawn_action_credit[0],
            commercial: despawn_action_credit[1],
            industrial: despawn_action_credit[2],
        };
        system.spawn_hysteresis_active = UseTuningBool {
            residential: spawn_hysteresis_active[0],
            commercial: spawn_hysteresis_active[1],
            industrial: spawn_hysteresis_active[2],
        };
        system.upgrade_hysteresis_active = UseTuningBool {
            residential: upgrade_hysteresis_active[0],
            commercial: upgrade_hysteresis_active[1],
            industrial: upgrade_hysteresis_active[2],
        };
        system.downgrade_hysteresis_active = UseTuningBool {
            residential: downgrade_hysteresis_active[0],
            commercial: downgrade_hysteresis_active[1],
            industrial: downgrade_hysteresis_active[2],
        };
        system.despawn_hysteresis_active = UseTuningBool {
            residential: despawn_hysteresis_active[0],
            commercial: despawn_hysteresis_active[1],
            industrial: despawn_hysteresis_active[2],
        };
        system.recent_household_failure_pressure = clamp01(recent_household_failure_pressure);
        system
    }

    /// Records how many planned household arrivals actually launched as carriers.
    pub(crate) fn record_household_admission_execution(&mut self, launched_households: u32) {
        self.last_admission_diagnostics.launched_households = launched_households;
    }

    /// Records how many planned household removals actually left the city.
    pub(crate) fn record_household_removal_execution(&mut self, removed_households: u32) {
        self.last_removal_diagnostics.removed_households = removed_households;
        let removed_household_ratio = if self.last_removal_diagnostics.total_household_count == 0 {
            0.0
        } else {
            clamp01(
                removed_households as f32
                    / self.last_removal_diagnostics.total_household_count as f32,
            )
        };
        self.last_removal_diagnostics.removed_household_ratio = removed_household_ratio;
        self.recent_household_failure_pressure = self
            .recent_household_failure_pressure
            .max(removed_household_ratio);
        self.last_removal_diagnostics.recent_failure_after = self.recent_household_failure_pressure;
    }

    /// Rebuilds the daily settled household-removal output from the post-settlement snapshot.
    #[cfg(test)]
    pub(crate) fn run_daily_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        _zoning: &ZoningSystem,
        treasury_balance: f64,
    ) {
        self.run_daily_pass_with_service_funding(
            allocator,
            households,
            graph,
            _zoning,
            treasury_balance,
            &[],
        );
    }

    /// Rebuilds daily household-removal output using the supplied service funding state.
    pub(crate) fn run_daily_pass_with_service_funding(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        _zoning: &ZoningSystem,
        treasury_balance: f64,
        service_funding_by_building: &[f32],
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let catalog = Arc::clone(&self.runtime_catalog);
        let economy_tuning = Arc::clone(&self.runtime_tuning);
        let snapshot = DailyDemandSnapshot::from_runtime_with_catalog(
            allocator,
            households,
            graph,
            &self.config,
            catalog.as_ref(),
            economy_tuning.as_ref(),
            treasury_balance,
            service_funding_by_building,
        );
        let pressures = self.update_pressure_channels_from_snapshot(&snapshot);
        self.households_to_admit_today = 0;
        let recent_failure_before = self.recent_household_failure_pressure;
        let decayed_recent_failure =
            recent_failure_before * self.config.household_action.recent_failure_daily_decay;
        self.recent_household_failure_pressure =
            clamp01(decayed_recent_failure.max(pressures.removal_diagnostics.failure_pressure));

        // Removal pressure: households emigrate when they have no home. Shared between
        // the household-action removal output and the residential demand channel.
        // Future: will be extended with an evacuation system when implemented.
        let removal_threshold = self.config.household_action.removal_threshold;
        let removal_credit_before = self.removal_action_credit;
        let persistent_exit_credit_before = self.persistent_exit_action_credit;
        let normalized_removal_pressure =
            normalized_positive_pressure(pressures.removal_pressure, removal_threshold);
        let crisis_removals = advance_household_action_credit(
            &mut self.removal_action_credit,
            pressures.removal_pressure,
            removal_threshold,
            self.config.action_budget.max_households_per_day,
            snapshot.total_household_count,
            1.0,
        );
        let persistent_exit_capacity = self
            .config
            .action_budget
            .max_households_per_day
            .saturating_sub(crisis_removals)
            .min(
                snapshot
                    .total_household_count
                    .saturating_sub(crisis_removals),
            );
        let persistent_exit_removals = advance_persistent_exit_credit(
            &mut self.persistent_exit_action_credit,
            snapshot.persistent_exit_eligible_household_count,
            self.config.household_action.persistent_exit_daily_fraction,
            persistent_exit_capacity,
        );
        self.households_to_remove_today = crisis_removals
            .saturating_add(persistent_exit_removals)
            .min(snapshot.total_household_count);
        let mut removal_diagnostics = pressures.removal_diagnostics;
        removal_diagnostics.threshold = removal_threshold;
        removal_diagnostics.normalized_action_pressure = normalized_removal_pressure;
        removal_diagnostics.credit_before = removal_credit_before;
        removal_diagnostics.credit_after = self.removal_action_credit;
        removal_diagnostics.persistent_exit_credit_before = persistent_exit_credit_before;
        removal_diagnostics.persistent_exit_credit_after = self.persistent_exit_action_credit;
        removal_diagnostics.persistent_exit_planned_households = persistent_exit_removals;
        removal_diagnostics.max_actionable_households = snapshot.total_household_count;
        removal_diagnostics.planned_households = self.households_to_remove_today;
        removal_diagnostics.removed_households = 0;
        removal_diagnostics.removed_household_ratio = 0.0;
        removal_diagnostics.recent_failure_before = recent_failure_before;
        removal_diagnostics.recent_failure_after = self.recent_household_failure_pressure;
        self.last_removal_diagnostics = removal_diagnostics;
    }
}
