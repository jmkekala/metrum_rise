//! Demand-driven daily growth pass built from authored baseline tuning.

use crate::debug_log;
use crate::simulation::buildings::allocator::{
    BuildingAllocator, resolve_building_economy_profile_binding,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::households::{
    HouseholdSystem, building_operating_buffer_days, building_staffing_ratio,
    building_total_output_inventory, household_reserve_days, industrial_input_coverage_factor,
    industrial_output_headroom_factor, level_tuning_value,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

const GROWTH_PROFILES_FILE: &str = "demand/growth_profiles.toml";
const EPSILON: f32 = 0.0001;
const DEMAND_HOURLY_CADENCE_FRACTION: f32 = 1.0 / 24.0;
const SHIPPED_PROFILE_ORDER: [(&str, DemandChannel); 9] = [
    ("residential_low_default", DemandChannel::ResidentialGrowth),
    (
        "residential_medium_default",
        DemandChannel::ResidentialGrowth,
    ),
    ("residential_high_default", DemandChannel::ResidentialGrowth),
    ("commercial_low_default", DemandChannel::CommercialGrowth),
    ("commercial_medium_default", DemandChannel::CommercialGrowth),
    ("commercial_high_default", DemandChannel::CommercialGrowth),
    ("industrial_low_default", DemandChannel::IndustrialGrowth),
    ("industrial_medium_default", DemandChannel::IndustrialGrowth),
    ("industrial_high_default", DemandChannel::IndustrialGrowth),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DemandUse {
    Residential,
    Commercial,
    Industrial,
}

impl DemandUse {
    fn zone_type(self) -> ZoneType {
        match self {
            Self::Residential => ZoneType::Residential,
            Self::Commercial => ZoneType::Commercial,
            Self::Industrial => ZoneType::Industrial,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum DemandChannel {
    ResidentialGrowth,
    CommercialGrowth,
    IndustrialGrowth,
}

impl DemandChannel {
    fn from_str_name(value: &str) -> Option<Self> {
        match value.trim() {
            "ResidentialGrowth" => Some(Self::ResidentialGrowth),
            "CommercialGrowth" => Some(Self::CommercialGrowth),
            "IndustrialGrowth" => Some(Self::IndustrialGrowth),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct GrowthProfileRuntime {
    demand_channel: DemandChannel,
    spawn_threshold: f32,
    despawn_threshold: f32,
    upgrade_threshold: f32,
    downgrade_threshold: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UseTuningF32 {
    pub(crate) residential: f32,
    pub(crate) commercial: f32,
    pub(crate) industrial: f32,
}

impl UseTuningF32 {
    fn get(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    fn get_mut(&mut self, use_kind: DemandUse) -> &mut f32 {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }

    pub(crate) fn as_array(self) -> [f32; 3] {
        [self.residential, self.commercial, self.industrial]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct DemandBuildingActionKey {
    pub(crate) parcel_id: u64,
    pub(crate) edge_idx: usize,
    pub(crate) side: i8,
    pub(crate) cell_x: usize,
    pub(crate) width_cells: u16,
    pub(crate) depth_cells: u16,
    pub(crate) level: u8,
    pub(crate) asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandLevelChangeAction {
    pub(crate) building: DemandBuildingActionKey,
    pub(crate) target_asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandSpawnAction {
    pub(crate) parcel_id: u64,
    pub(crate) asset_id: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandUseActionPlan {
    pub(crate) despawns: Vec<DemandBuildingActionKey>,
    pub(crate) downgrades: Vec<DemandLevelChangeAction>,
    pub(crate) upgrades: Vec<DemandLevelChangeAction>,
    pub(crate) spawns: Vec<DemandSpawnAction>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandBuildingActionPlan {
    pub(crate) residential: DemandUseActionPlan,
    pub(crate) commercial: DemandUseActionPlan,
    pub(crate) industrial: DemandUseActionPlan,
}

impl DemandBuildingActionPlan {
    fn use_plan_mut(&mut self, use_kind: DemandUse) -> &mut DemandUseActionPlan {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }
}

#[derive(Clone, Debug)]
struct SignalNormalizationConfig {
    household_affordability_target_reserve_days: f32,
    household_stock_stability_target_days: f32,
}

#[derive(Clone, Debug)]
struct HouseholdActionConfig {
    admission_threshold: f32,
    admission_unhoused_ratio_penalty: f32,
    admission_zero_budget_penalty: f32,
    admission_recent_failure_penalty: f32,
    move_in_min_search_runway_days: f32,
    move_in_target_search_runway_days: f32,
    move_in_benefit_treasury_coverage_days: f32,
    recent_failure_daily_decay: f32,
    removal_threshold: f32,
    persistent_exit_destitute_stock_days: f32,
    persistent_exit_destitute_unhoused_days: u32,
    persistent_exit_max_unhoused_days: u32,
    persistent_exit_daily_fraction: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct ActionBudgetConfig {
    max_households_per_day: u32,
    spawn_batch_fraction_by_use: UseTuningF32,
    upgrade_batch_fraction_by_use: UseTuningF32,
    downgrade_batch_fraction_by_use: UseTuningF32,
    despawn_batch_fraction_by_use: UseTuningF32,
}

#[derive(Clone, Copy, Debug)]
struct DemandPressureInputs {
    admission_pressure: f32,
    removal_pressure: f32,
    admission_diagnostics: HouseholdAdmissionDiagnostics,
    removal_diagnostics: HouseholdRemovalDiagnostics,
}

#[derive(Clone, Copy, Debug, Default)]
struct HouseholdAdmissionDiagnostics {
    total_household_count: u32,
    vacant_household_slots: u32,
    connected_border_count: u32,
    housing_availability: f32,
    household_affordability: f32,
    move_in_acceptance: f32,
    move_in_search_runway_days: f32,
    move_in_runway_factor: f32,
    candidate_household_size: f32,
    candidate_effective_workers: f32,
    open_job_slots: u32,
    existing_unemployed_member_count: u32,
    expected_employed_members: f32,
    expected_unemployed_members: f32,
    expected_entry_wage_per_day: f32,
    expected_wage_income_per_day: f32,
    benefit_reliability: f32,
    existing_benefit_claim_per_day: f32,
    candidate_benefit_claim_per_day: f32,
    total_benefit_claim_per_day: f32,
    expected_benefit_income_per_day: f32,
    starter_savings: f32,
    daily_essential_cost: f32,
    daily_deficit: f32,
    unhoused_household_ratio: f32,
    unhoused_factor: f32,
    zero_budget_household_ratio: f32,
    zero_budget_factor: f32,
    failure_factor: f32,
    recent_failure_pressure: f32,
    recent_failure_factor: f32,
    base_pressure: f32,
    pressure: f32,
    threshold: f32,
    normalized_action_pressure: f32,
    credit_before: f32,
    credit_after: f32,
    max_actionable_households: u32,
    planned_households: u32,
    launched_households: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct HouseholdRemovalDiagnostics {
    total_household_count: u32,
    housed_household_count: u32,
    unhoused_household_count: u32,
    zero_budget_household_count: u32,
    persistent_exit_eligible_household_count: u32,
    unhoused_household_ratio: f32,
    zero_budget_household_ratio: f32,
    failure_pressure: f32,
    removed_household_ratio: f32,
    recent_failure_before: f32,
    recent_failure_after: f32,
    pressure: f32,
    threshold: f32,
    normalized_action_pressure: f32,
    credit_before: f32,
    credit_after: f32,
    persistent_exit_credit_before: f32,
    persistent_exit_credit_after: f32,
    persistent_exit_planned_households: u32,
    max_actionable_households: u32,
    planned_households: u32,
    removed_households: u32,
}

#[derive(Clone, Copy, Debug, Default)]
struct MoveInAcceptance {
    candidate_household_size: f32,
    candidate_effective_workers: f32,
    expected_employed_members: f32,
    expected_unemployed_members: f32,
    expected_entry_wage_per_day: f32,
    expected_wage_income_per_day: f32,
    existing_benefit_claim_per_day: f32,
    candidate_benefit_claim_per_day: f32,
    total_benefit_claim_per_day: f32,
    benefit_reliability: f32,
    expected_benefit_income_per_day: f32,
    starter_savings: f32,
    daily_essential_cost: f32,
    daily_deficit: f32,
    search_runway_days: f32,
    runway_factor: f32,
    acceptance: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
struct DemandConfig {
    signal_normalization: SignalNormalizationConfig,
    household_action: HouseholdActionConfig,
    action_budget: ActionBudgetConfig,
    profiles: Vec<GrowthProfileRuntime>,
}

impl DemandConfig {
    fn profile_for_zone_density(
        &self,
        zone_type: ZoneType,
        density: &str,
    ) -> Option<&GrowthProfileRuntime> {
        let idx = match (zone_type, density) {
            (ZoneType::Residential, "low") => 0,
            (ZoneType::Residential, "medium") => 1,
            (ZoneType::Residential, "high") => 2,
            (ZoneType::Commercial, "low") => 3,
            (ZoneType::Commercial, "medium") => 4,
            (ZoneType::Commercial, "high") => 5,
            (ZoneType::Industrial, "low") => 6,
            (ZoneType::Industrial, "medium") => 7,
            (ZoneType::Industrial, "high") => 8,
            _ => return None,
        };
        self.profiles.get(idx)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfilesFile {
    signal_normalization: AuthoredSignalNormalization,
    household_action: AuthoredHouseholdAction,
    action_budget: AuthoredActionBudget,
    profiles: Vec<AuthoredGrowthProfile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredSignalNormalization {
    household_affordability_target_reserve_days: f32,
    household_stock_stability_target_days: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredHouseholdAction {
    admission_threshold: f32,
    admission_unhoused_ratio_penalty: f32,
    admission_zero_budget_penalty: f32,
    admission_recent_failure_penalty: f32,
    move_in_min_search_runway_days: f32,
    move_in_target_search_runway_days: f32,
    move_in_benefit_treasury_coverage_days: f32,
    recent_failure_daily_decay: f32,
    removal_threshold: f32,
    persistent_exit_destitute_stock_days: f32,
    persistent_exit_destitute_unhoused_days: u32,
    persistent_exit_max_unhoused_days: u32,
    persistent_exit_daily_fraction: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredActionBudget {
    max_households_per_day: u32,
    spawn_batch_fraction_by_use: AuthoredUseTuningF32,
    upgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    downgrade_batch_fraction_by_use: AuthoredUseTuningF32,
    despawn_batch_fraction_by_use: AuthoredUseTuningF32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredUseTuningF32 {
    residential: f32,
    commercial: f32,
    industrial: f32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthoredGrowthProfile {
    id: String,
    demand_channel: String,
    spawn_threshold: f32,
    despawn_threshold: f32,
    upgrade_threshold: f32,
    downgrade_threshold: f32,
    hysteresis_margin: f32,
}

static BUILTIN_CONFIG: OnceLock<Result<DemandConfig, String>> = OnceLock::new();

/// Demand-owned daily growth state derived from the settled economy snapshot.
pub struct DemandSystem {
    config: Arc<DemandConfig>,
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
    pub(crate) recent_household_failure_pressure: f32,
    pub(crate) building_actions: DemandBuildingActionPlan,
    last_admission_diagnostics: HouseholdAdmissionDiagnostics,
    last_removal_diagnostics: HouseholdRemovalDiagnostics,
}

impl DemandSystem {
    /// Creates a new demand system using the shipped demand tuning file.
    pub fn new() -> Self {
        let config = load_builtin_demand_config()
            .unwrap_or_else(|err| panic!("could not load built-in demand tuning: {err}"));
        Self {
            config,
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
            recent_household_failure_pressure: 0.0,
            building_actions: DemandBuildingActionPlan::default(),
            last_admission_diagnostics: HouseholdAdmissionDiagnostics::default(),
            last_removal_diagnostics: HouseholdRemovalDiagnostics::default(),
        }
    }

    /// Refreshes RCI telemetry and advances hourly household and building demand outputs.
    pub(crate) fn run_hourly_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        treasury_balance: f64,
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let snapshot = DailyDemandSnapshot::from_runtime(
            allocator,
            households,
            graph,
            &self.config,
            treasury_balance,
        );
        let economy_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let residential_occupants =
            ResidentialOccupantSnapshot::from_runtime(allocator, households);

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
            economy_tuning.as_ref(),
            &residential_occupants,
            DEMAND_HOURLY_CADENCE_FRACTION,
            "hourly_pass",
        );
    }

    fn update_pressure_channels_from_snapshot(
        &mut self,
        snapshot: &DailyDemandSnapshot,
    ) -> DemandPressureInputs {
        let housing_shortage = 1.0 - snapshot.housing_availability;
        let goods_shortage = 1.0 - snapshot.household_stock_stability;
        let commercial_need = goods_shortage.max(snapshot.commercial_capacity_deficit);
        let household_purchase_power = clamp01(
            snapshot.household_affordability
                * self
                    .config
                    .signal_normalization
                    .household_affordability_target_reserve_days,
        );
        let ext_conn = snapshot.external_connection_available;

        // Admission pressure fills existing vacancies, then soft-damps immigration when the
        // candidate household does not have a credible benefit-backed job-search runway or
        // the existing household economy is already failing.
        let admission_base_pressure = ext_conn * snapshot.housing_availability;
        let move_in = compute_move_in_acceptance(&self.config, snapshot);
        let admission_unhoused_factor = 1.0
            - self
                .config
                .household_action
                .admission_unhoused_ratio_penalty
                * snapshot.unhoused_household_ratio;
        let admission_zero_budget_factor = 1.0
            - self.config.household_action.admission_zero_budget_penalty
                * snapshot.zero_budget_household_ratio;
        let admission_recent_failure_factor = 1.0
            - self
                .config
                .household_action
                .admission_recent_failure_penalty
                * self.recent_household_failure_pressure;
        let admission_failure_factor = clamp01(admission_unhoused_factor)
            * clamp01(admission_zero_budget_factor)
            * clamp01(admission_recent_failure_factor);
        let admission_pressure = clamp01(
            admission_base_pressure
                * clamp01(move_in.acceptance)
                * clamp01(admission_failure_factor),
        );
        let removal_pressure = snapshot.unhoused_household_ratio;
        let failure_pressure = snapshot
            .unhoused_household_ratio
            .max(snapshot.zero_budget_household_ratio);
        let admission_diagnostics = HouseholdAdmissionDiagnostics {
            total_household_count: snapshot.total_household_count,
            vacant_household_slots: snapshot.vacant_household_slots,
            connected_border_count: snapshot.connected_border_count,
            housing_availability: snapshot.housing_availability,
            household_affordability: snapshot.household_affordability,
            move_in_acceptance: clamp01(move_in.acceptance),
            move_in_search_runway_days: move_in.search_runway_days,
            move_in_runway_factor: clamp01(move_in.runway_factor),
            candidate_household_size: move_in.candidate_household_size,
            candidate_effective_workers: move_in.candidate_effective_workers,
            open_job_slots: snapshot.open_job_slots,
            existing_unemployed_member_count: snapshot.existing_unemployed_member_count,
            expected_employed_members: move_in.expected_employed_members,
            expected_unemployed_members: move_in.expected_unemployed_members,
            expected_entry_wage_per_day: move_in.expected_entry_wage_per_day,
            expected_wage_income_per_day: move_in.expected_wage_income_per_day,
            benefit_reliability: move_in.benefit_reliability,
            existing_benefit_claim_per_day: move_in.existing_benefit_claim_per_day,
            candidate_benefit_claim_per_day: move_in.candidate_benefit_claim_per_day,
            total_benefit_claim_per_day: move_in.total_benefit_claim_per_day,
            expected_benefit_income_per_day: move_in.expected_benefit_income_per_day,
            starter_savings: move_in.starter_savings,
            daily_essential_cost: move_in.daily_essential_cost,
            daily_deficit: move_in.daily_deficit,
            unhoused_household_ratio: snapshot.unhoused_household_ratio,
            unhoused_factor: clamp01(admission_unhoused_factor),
            zero_budget_household_ratio: snapshot.zero_budget_household_ratio,
            zero_budget_factor: clamp01(admission_zero_budget_factor),
            failure_factor: clamp01(admission_failure_factor),
            recent_failure_pressure: self.recent_household_failure_pressure,
            recent_failure_factor: clamp01(admission_recent_failure_factor),
            base_pressure: admission_base_pressure,
            pressure: admission_pressure,
            ..HouseholdAdmissionDiagnostics::default()
        };
        let removal_diagnostics = HouseholdRemovalDiagnostics {
            total_household_count: snapshot.total_household_count,
            housed_household_count: snapshot.housed_household_count,
            unhoused_household_count: snapshot.unhoused_household_count,
            zero_budget_household_count: snapshot.zero_budget_household_count,
            unhoused_household_ratio: snapshot.unhoused_household_ratio,
            zero_budget_household_ratio: snapshot.zero_budget_household_ratio,
            failure_pressure,
            persistent_exit_eligible_household_count: snapshot
                .persistent_exit_eligible_household_count,
            pressure: removal_pressure,
            ..HouseholdRemovalDiagnostics::default()
        };

        // Residential demand follows net migration balance rather than raw vacancy.
        // inflow_desire measures unmet demand for new capacity, while admission pressure
        // fills existing slots.
        let inflow_desire = clamp01(ext_conn * housing_shortage);
        let net_residential = (inflow_desire - removal_pressure).clamp(-1.0, 1.0);
        self.residential = net_residential * 0.5 + 0.5;

        // Commercial: residents need both existing stocked households and enough shop output
        // capacity to keep those stocks stable. Uses short-run purchase power rather than the
        // long-run reserve target so starter cities can spawn shops before household stockout.
        self.commercial = clamp01(commercial_need * household_purchase_power * ext_conn);
        // Industrial: fraction of commercial input value sourced from OWA rather than local supply.
        self.industrial = clamp01(snapshot.commercial_owa_dependency * ext_conn);

        DemandPressureInputs {
            admission_pressure,
            removal_pressure,
            admission_diagnostics,
            removal_diagnostics,
        }
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
             afford={:.2} accept={:.2} runway={:.2} runway_factor={:.2} \
             candidate_size={:.1} workers={:.1} open_jobs={} existing_unemployed={} \
             expected_employed={:.1} expected_unemployed={:.1} entry_wage={:.1} wage_income={:.1} \
             benefit_rel={:.2} existing_benefit_claim={:.1} candidate_benefit_claim={:.1} \
             total_benefit_claim={:.1} benefit_income={:.1} starter={:.1} daily_cost={:.1} \
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
            diagnostics.household_affordability,
            diagnostics.move_in_acceptance,
            diagnostics.move_in_search_runway_days,
            diagnostics.move_in_runway_factor,
            diagnostics.candidate_household_size,
            diagnostics.candidate_effective_workers,
            diagnostics.open_job_slots,
            diagnostics.existing_unemployed_member_count,
            diagnostics.expected_employed_members,
            diagnostics.expected_unemployed_members,
            diagnostics.expected_entry_wage_per_day,
            diagnostics.expected_wage_income_per_day,
            diagnostics.benefit_reliability,
            diagnostics.existing_benefit_claim_per_day,
            diagnostics.candidate_benefit_claim_per_day,
            diagnostics.total_benefit_claim_per_day,
            diagnostics.expected_benefit_income_per_day,
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

    /// Rebuilds the daily settled household-removal output from the post-settlement snapshot.
    pub(crate) fn run_daily_pass(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        _zoning: &ZoningSystem,
        treasury_balance: f64,
    ) {
        self.building_actions = DemandBuildingActionPlan::default();
        let snapshot = DailyDemandSnapshot::from_runtime(
            allocator,
            households,
            graph,
            &self.config,
            treasury_balance,
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

    fn plan_private_building_actions(
        &mut self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
        snapshot: &DailyDemandSnapshot,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        cadence_fraction: f32,
        log_label: &str,
    ) {
        for use_kind in [
            DemandUse::Residential,
            DemandUse::Commercial,
            DemandUse::Industrial,
        ] {
            let zone_type = use_kind.zone_type();
            let growth_pressure = self.pressure_for_use(use_kind);
            let spawn_candidates =
                allocator.collect_demand_spawn_candidates(zone_type, zoning, graph);
            let existing_candidates = self.collect_existing_building_candidates(
                allocator,
                households,
                economy_tuning,
                &residential_occupants,
                zone_type,
                growth_pressure,
            );

            let normalized_spawn_pressure = spawn_candidates
                .iter()
                .filter_map(|candidate| {
                    self.config
                        .profile_for_zone_density(zone_type, &candidate.density)
                        .map(|profile| {
                            normalized_positive_pressure(growth_pressure, profile.spawn_threshold)
                        })
                })
                .sum::<f32>();
            // All use families use spawn_limit = 1.0. Residential no longer needs the
            // quadratic housing_shortage throttle because housing_shortage is already
            // embedded in inflow_desire inside ResidentialGrowth: when vacancy is high,
            // the channel falls toward 0.5 and spawn pressure drops naturally.
            let spawn_limit = 1.0_f32;

            let spawn_budget_units = normalized_spawn_pressure
                * self
                    .config
                    .action_budget
                    .spawn_batch_fraction_by_use
                    .get(use_kind)
                * spawn_limit;
            let spawns_today = advance_building_action_credit(
                self.spawn_action_credit.get_mut(use_kind),
                spawn_budget_units,
                spawn_candidates.len(),
                cadence_fraction,
            );
            debug_log!(
                "spawn",
                "{} zone={:?}: pressure={:.3} \
                 candidates={} norm_pressure={:.3} spawn_limit={:.3} \
                 budget_units={:.3} spawns_today={}",
                log_label,
                zone_type,
                growth_pressure,
                spawn_candidates.len(),
                normalized_spawn_pressure,
                spawn_limit,
                spawn_budget_units,
                spawns_today,
            );
            let selected_spawns: Vec<_> = if zone_type == ZoneType::Residential {
                spawn_candidates
                    .into_iter()
                    .take(spawns_today)
                    .map(|candidate| candidate.action)
                    .collect()
            } else {
                // Non-residential: apply labour and output-absorption gates per candidate.
                // available_unemployed starts from the housed resident count (the city's
                // potential workforce) and decreases as each passing candidate claims
                // its worker_capacity, preventing spawning more capacity than people exist
                // to fill it.  Using open_reachable_job_slots here was wrong: that value
                // counts unfilled slots in already-existing buildings (demand for workers),
                // not the supply of workers, and is 0 at bootstrap causing a permanent deadlock.
                let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                    panic!("could not load built-in runtime economy catalog: {err}")
                });
                let mut available_unemployed = snapshot.housed_resident_count;
                let mut passed = 0;
                spawn_candidates
                    .into_iter()
                    .filter(|candidate| {
                        if passed >= spawns_today {
                            return false;
                        }
                        if !nonresidential_passes_labour_gate(
                            allocator,
                            &candidate.action.asset_id,
                            available_unemployed,
                        ) {
                            return false;
                        }
                        if !nonresidential_passes_absorption_gate(
                            allocator,
                            &catalog,
                            &candidate.action.asset_id,
                            snapshot.housed_resident_count,
                        ) {
                            return false;
                        }
                        // Consume workers from the running pool.
                        let req = allocator.worker_capacity_for_asset(&candidate.action.asset_id);
                        available_unemployed = available_unemployed.saturating_sub(req);
                        passed += 1;
                        true
                    })
                    .map(|candidate| candidate.action)
                    .collect()
            };

            let normalized_upgrade_pressure = existing_candidates
                .upgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let upgrade_budget_units = normalized_upgrade_pressure
                * self
                    .config
                    .action_budget
                    .upgrade_batch_fraction_by_use
                    .get(use_kind);
            let upgrades_today = advance_building_action_credit(
                self.upgrade_action_credit.get_mut(use_kind),
                upgrade_budget_units,
                existing_candidates.upgrades.len(),
                cadence_fraction,
            );
            let selected_upgrades: Vec<_> = existing_candidates
                .upgrades
                .iter()
                .take(upgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let normalized_downgrade_pressure = existing_candidates
                .downgrades
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let downgrade_budget_units = normalized_downgrade_pressure
                * self
                    .config
                    .action_budget
                    .downgrade_batch_fraction_by_use
                    .get(use_kind);
            let downgrades_today = advance_building_action_credit(
                self.downgrade_action_credit.get_mut(use_kind),
                downgrade_budget_units,
                existing_candidates.downgrades.len(),
                cadence_fraction,
            );
            let selected_downgrades: Vec<_> = existing_candidates
                .downgrades
                .iter()
                .take(downgrades_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let normalized_despawn_pressure = existing_candidates
                .despawns
                .iter()
                .map(|candidate| candidate.normalized_action_pressure)
                .sum::<f32>();
            let despawn_budget_units = normalized_despawn_pressure
                * self
                    .config
                    .action_budget
                    .despawn_batch_fraction_by_use
                    .get(use_kind);
            let despawns_today = advance_building_action_credit(
                self.despawn_action_credit.get_mut(use_kind),
                despawn_budget_units,
                existing_candidates.despawns.len(),
                cadence_fraction,
            );
            let selected_despawns: Vec<_> = existing_candidates
                .despawns
                .iter()
                .take(despawns_today)
                .map(|candidate| candidate.action.clone())
                .collect();

            let plan = self.building_actions.use_plan_mut(use_kind);
            plan.spawns.extend(selected_spawns);
            plan.upgrades.extend(selected_upgrades);
            plan.downgrades.extend(selected_downgrades);
            plan.despawns.extend(selected_despawns);
        }
    }
}

fn compute_move_in_acceptance(
    config: &DemandConfig,
    snapshot: &DailyDemandSnapshot,
) -> MoveInAcceptance {
    if snapshot.vacant_household_slots == 0 || snapshot.candidate_household_size <= EPSILON {
        return MoveInAcceptance::default();
    }

    let candidate_household_size = snapshot.candidate_household_size.max(1.0);
    let candidate_effective_workers = candidate_household_size;
    let expected_employed_members = candidate_effective_workers.min(snapshot.open_job_slots as f32);
    let expected_unemployed_members =
        (candidate_effective_workers - expected_employed_members).max(0.0);
    let expected_entry_wage_per_day = snapshot.average_open_job_wage_per_day.max(0.0);
    let expected_wage_income_per_day = expected_employed_members * expected_entry_wage_per_day;

    let benefit_per_member = snapshot.unemployment_daily_benefit_per_member.max(0.0);
    let existing_benefit_claim_per_day =
        snapshot.existing_unemployed_member_count as f32 * benefit_per_member;
    let candidate_benefit_claim_per_day = expected_unemployed_members * benefit_per_member;
    let total_benefit_claim_per_day =
        existing_benefit_claim_per_day + candidate_benefit_claim_per_day;
    let coverage_days = config
        .household_action
        .move_in_benefit_treasury_coverage_days
        .max(EPSILON);
    let benefit_reliability = if total_benefit_claim_per_day <= EPSILON {
        1.0
    } else {
        clamp01(
            snapshot.city_treasury_balance.max(0.0) / (total_benefit_claim_per_day * coverage_days),
        )
    };
    let expected_benefit_income_per_day = candidate_benefit_claim_per_day * benefit_reliability;
    let expected_daily_income = expected_wage_income_per_day + expected_benefit_income_per_day;

    let starter_savings = snapshot.immigrant_starter_savings_per_household.max(0.0);
    let daily_essential_cost = snapshot.candidate_daily_essential_cost.max(0.0);
    let daily_deficit = (daily_essential_cost - expected_daily_income).max(0.0);
    let target_days = config
        .household_action
        .move_in_target_search_runway_days
        .max(EPSILON);
    let min_days = config
        .household_action
        .move_in_min_search_runway_days
        .max(0.0);
    let search_runway_days = if daily_deficit <= EPSILON {
        target_days
    } else {
        starter_savings / daily_deficit
    };
    let runway_span = (target_days - min_days).max(EPSILON);
    let runway_factor = clamp01((search_runway_days - min_days) / runway_span);

    MoveInAcceptance {
        candidate_household_size,
        candidate_effective_workers,
        expected_employed_members,
        expected_unemployed_members,
        expected_entry_wage_per_day,
        expected_wage_income_per_day,
        existing_benefit_claim_per_day,
        candidate_benefit_claim_per_day,
        total_benefit_claim_per_day,
        benefit_reliability,
        expected_benefit_income_per_day,
        starter_savings,
        daily_essential_cost,
        daily_deficit,
        search_runway_days,
        runway_factor,
        acceptance: runway_factor,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DemandSpawnCandidate {
    pub(crate) action: DemandSpawnAction,
    pub(crate) density: String,
}

#[derive(Clone, Debug)]
struct WeightedLevelChangeCandidate {
    action: DemandLevelChangeAction,
    normalized_action_pressure: f32,
}

#[derive(Clone, Debug)]
struct WeightedDespawnCandidate {
    action: DemandBuildingActionKey,
    normalized_action_pressure: f32,
}

#[derive(Clone, Debug, Default)]
struct ExistingBuildingCandidates {
    despawns: Vec<WeightedDespawnCandidate>,
    downgrades: Vec<WeightedLevelChangeCandidate>,
    upgrades: Vec<WeightedLevelChangeCandidate>,
}

#[derive(Clone, Debug)]
struct ResidentialOccupantSnapshot {
    household_count_by_building: Vec<u32>,
    min_reserve_days_by_building: Vec<f32>,
}

impl ResidentialOccupantSnapshot {
    fn from_runtime(allocator: &BuildingAllocator, households: &HouseholdSystem) -> Self {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let mut household_count_by_building = vec![0_u32; allocator.buildings.len()];
        let mut min_reserve_days_by_building = vec![f32::INFINITY; allocator.buildings.len()];
        for household in &households.households {
            if household.member_count == 0 {
                continue;
            }
            let home_building_id = household.home_building_id;
            if home_building_id >= allocator.buildings.len()
                || allocator.buildings[home_building_id].broken
                || allocator.buildings[home_building_id].economy_broken
            {
                continue;
            }
            household_count_by_building[home_building_id] =
                household_count_by_building[home_building_id].saturating_add(1);
            min_reserve_days_by_building[home_building_id] = min_reserve_days_by_building
                [home_building_id]
                .min(household_reserve_days(&catalog, &tuning, household));
        }

        Self {
            household_count_by_building,
            min_reserve_days_by_building,
        }
    }
}

impl DemandSystem {
    fn pressure_for_use(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    // Net growth/decline pressure for display, in −1.0..+1.0.
    //
    // Uses the low-density profile thresholds as reference:
    // - Positive: raw channel is above the spawn threshold (city wants to grow this use)
    // - Negative: raw channel is below the despawn threshold (city wants to shrink this use)
    // - Zero: channel is in the dead zone between thresholds (no pressure either way)
    fn net_pressure_for(&self, use_kind: DemandUse) -> f32 {
        let channel = self.pressure_for_use(use_kind);
        let zone_type = match use_kind {
            DemandUse::Residential => ZoneType::Residential,
            DemandUse::Commercial => ZoneType::Commercial,
            DemandUse::Industrial => ZoneType::Industrial,
        };
        let Some(profile) = self.config.profile_for_zone_density(zone_type, "low") else {
            return 0.0;
        };
        let spawn_t = profile.spawn_threshold;
        let despawn_t = profile.despawn_threshold;
        if channel > spawn_t {
            (channel - spawn_t) / (1.0 - spawn_t).max(EPSILON)
        } else if channel < despawn_t {
            -(despawn_t - channel) / despawn_t.max(EPSILON)
        } else {
            0.0
        }
    }

    /// Net residential growth pressure for display. Positive = spawn pressure,
    /// negative = despawn pressure, zero = equilibrium dead zone.
    pub(crate) fn net_residential_pressure(&self) -> f32 {
        self.net_pressure_for(DemandUse::Residential)
    }

    /// Net commercial growth pressure for display. Positive = spawn pressure,
    /// negative = despawn pressure, zero = equilibrium dead zone.
    pub(crate) fn net_commercial_pressure(&self) -> f32 {
        self.net_pressure_for(DemandUse::Commercial)
    }

    /// Net industrial growth pressure for display. Positive = spawn pressure,
    /// negative = despawn pressure, zero = equilibrium dead zone.
    pub(crate) fn net_industrial_pressure(&self) -> f32 {
        self.net_pressure_for(DemandUse::Industrial)
    }

    fn collect_existing_building_candidates(
        &self,
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        economy_tuning: &RuntimeEconomyTuning,
        residential_occupants: &ResidentialOccupantSnapshot,
        zone_type: ZoneType,
        growth_pressure: f32,
    ) -> ExistingBuildingCandidates {
        let mut building_indices: Vec<usize> = allocator
            .buildings
            .iter()
            .enumerate()
            .filter_map(|(idx, building)| (building.zone_type == zone_type).then_some(idx))
            .collect();
        building_indices.sort_by(|&a, &b| {
            let left = &allocator.buildings[a];
            let right = &allocator.buildings[b];
            attachment_sort_key(left).cmp(&attachment_sort_key(right))
        });

        let mut candidates = ExistingBuildingCandidates::default();
        for building_idx in building_indices {
            let building = &allocator.buildings[building_idx];
            if building.broken || building.economy_broken || building.pending_redevelopment {
                continue;
            }
            let Some(entry) = allocator.registry.get(&building.asset_id) else {
                continue;
            };
            let Some(asset_building) = entry.manifest.building.as_ref() else {
                continue;
            };
            if !asset_building.is_zoned_private() {
                continue;
            }
            let Some(density) = asset_building.density_key() else {
                continue;
            };
            let Some(profile) = self.config.profile_for_zone_density(zone_type, density) else {
                continue;
            };

            let despawn_pressure =
                normalized_negative_pressure(growth_pressure, profile.despawn_threshold);
            let downgrade_pressure =
                normalized_negative_pressure(growth_pressure, profile.downgrade_threshold);
            let upgrade_pressure =
                normalized_positive_pressure(growth_pressure, profile.upgrade_threshold);

            if building.occupancy == 0 && building.worker_count == 0 && despawn_pressure > 0.0 {
                candidates.despawns.push(WeightedDespawnCandidate {
                    action: demand_building_action_key(building),
                    normalized_action_pressure: despawn_pressure,
                });
                continue;
            }

            if downgrade_pressure > 0.0
                && let Some(target_asset_id) = allocator.registry.prev_level(&building.asset_id)
                && level_change_is_compatible(allocator, building_idx, target_asset_id)
                && building_is_viable_for_downgrade(
                    allocator,
                    households,
                    economy_tuning,
                    residential_occupants,
                    building_idx,
                    target_asset_id,
                )
            {
                candidates.downgrades.push(WeightedLevelChangeCandidate {
                    action: DemandLevelChangeAction {
                        building: demand_building_action_key(building),
                        target_asset_id: target_asset_id.to_owned(),
                    },
                    normalized_action_pressure: downgrade_pressure,
                });
                continue;
            }

            if upgrade_pressure > 0.0
                && let Some(target_asset_id) = allocator.registry.next_level(&building.asset_id)
                && level_change_is_compatible(allocator, building_idx, target_asset_id)
                && building_is_viable_for_upgrade(
                    allocator,
                    households,
                    economy_tuning,
                    residential_occupants,
                    building_idx,
                    target_asset_id,
                )
            {
                candidates.upgrades.push(WeightedLevelChangeCandidate {
                    action: DemandLevelChangeAction {
                        building: demand_building_action_key(building),
                        target_asset_id: target_asset_id.to_owned(),
                    },
                    normalized_action_pressure: upgrade_pressure,
                });
            }
        }

        candidates
    }
}

fn building_is_viable_for_upgrade(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    match building.zone_type {
        ZoneType::Residential => residential_upgrade_viable(
            allocator,
            households,
            economy_tuning,
            residential_occupants,
            building_idx,
            target_asset_id,
        ),
        ZoneType::Commercial => {
            nonresidential_upgrade_viable(allocator, economy_tuning, building_idx, target_asset_id)
        }
        ZoneType::Industrial => {
            industrial_upgrade_viable(allocator, economy_tuning, building_idx, target_asset_id)
        }
        _ => false,
    }
}

fn building_is_viable_for_downgrade(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    _target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    match building.zone_type {
        ZoneType::Residential => residential_downgrade_viable(
            allocator,
            households,
            economy_tuning,
            residential_occupants,
            building_idx,
        ),
        ZoneType::Commercial => {
            nonresidential_downgrade_viable(allocator, economy_tuning, building_idx)
        }
        ZoneType::Industrial => {
            industrial_downgrade_viable(allocator, economy_tuning, building_idx)
        }
        _ => false,
    }
}

fn residential_upgrade_viable(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let Some(target_building) = allocator
        .registry
        .get(target_asset_id)
        .and_then(|entry| entry.manifest.building.as_ref())
    else {
        return false;
    };
    let household_capacity = allocator.household_capacity(building_idx);
    if household_capacity == 0 {
        return false;
    }
    let occupancy_ratio = clamp01(building.occupancy as f32 / household_capacity as f32);
    let min_occupancy_ratio = level_tuning_value(
        &economy_tuning
            .viability
            .residential_min_occupancy_ratio_for_upgrade,
        target_building.level,
    );
    if occupancy_ratio + EPSILON < min_occupancy_ratio {
        return false;
    }
    if building.occupancy > 0
        && residential_occupants.household_count_by_building[building_idx] == 0
    {
        return false;
    }
    let required_reserve_days = level_tuning_value(
        &economy_tuning
            .households
            .residential_move_in_min_reserve_days_by_level,
        target_building.level,
    );
    let min_reserve_days = residential_occupants.min_reserve_days_by_building[building_idx];
    if building.occupancy > 0 && min_reserve_days + EPSILON < required_reserve_days {
        return false;
    }

    let _ = households;
    true
}

fn residential_downgrade_viable(
    allocator: &BuildingAllocator,
    households: &HouseholdSystem,
    economy_tuning: &RuntimeEconomyTuning,
    residential_occupants: &ResidentialOccupantSnapshot,
    building_idx: usize,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let household_capacity = allocator.household_capacity(building_idx);
    if household_capacity == 0 {
        return false;
    }
    let occupancy_ratio = clamp01(building.occupancy as f32 / household_capacity as f32);
    let max_occupancy_ratio = level_tuning_value(
        &economy_tuning
            .viability
            .residential_max_occupancy_ratio_for_downgrade,
        building.level,
    );
    if occupancy_ratio > max_occupancy_ratio + EPSILON {
        return false;
    }
    if building.occupancy > 0
        && residential_occupants.household_count_by_building[building_idx] == 0
    {
        return false;
    }

    let _ = households;
    true
}

fn nonresidential_upgrade_viable(
    allocator: &BuildingAllocator,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    if building.is_deserted {
        return false;
    }
    let Some(target_level) = allocator
        .registry
        .get(target_asset_id)
        .and_then(|entry| entry.manifest.building.as_ref())
        .map(|target| target.level)
    else {
        return false;
    };
    let staffing_ratio = building_staffing_ratio(allocator, building_idx, building);
    if staffing_ratio + EPSILON
        < economy_tuning
            .viability
            .nonresidential_min_staffing_ratio_for_upgrade
    {
        return false;
    }
    let min_buffer_days = level_tuning_value(
        &economy_tuning
            .viability
            .nonresidential_min_buffer_days_by_level,
        target_level,
    );
    if building_operating_buffer_days(&catalog, economy_tuning, building) + EPSILON
        < min_buffer_days
    {
        return false;
    }
    if matches!(building.zone_type, ZoneType::Commercial)
        && building_total_output_inventory(&catalog, building) <= EPSILON
    {
        return false;
    }
    true
}

fn nonresidential_downgrade_viable(
    allocator: &BuildingAllocator,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
) -> bool {
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let staffing_ratio = building_staffing_ratio(allocator, building_idx, building);
    let buffer_days = building_operating_buffer_days(&catalog, economy_tuning, building);
    let max_buffer_days = level_tuning_value(
        &economy_tuning
            .viability
            .nonresidential_max_buffer_days_for_downgrade,
        building.level,
    );
    building.is_deserted
        || staffing_ratio
            <= economy_tuning
                .viability
                .nonresidential_max_staffing_ratio_for_downgrade
                + EPSILON
        || buffer_days <= max_buffer_days + EPSILON
        || matches!(building.zone_type, ZoneType::Commercial)
            && building_total_output_inventory(&catalog, building) <= EPSILON
}

fn industrial_upgrade_viable(
    allocator: &BuildingAllocator,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    nonresidential_upgrade_viable(allocator, economy_tuning, building_idx, target_asset_id)
        && industrial_input_coverage_factor(&catalog, building) + EPSILON
            >= economy_tuning
                .viability
                .industrial_min_input_coverage_for_upgrade
        && industrial_output_headroom_factor(&catalog, building) + EPSILON
            >= economy_tuning
                .viability
                .industrial_min_output_headroom_for_upgrade
}

fn industrial_downgrade_viable(
    allocator: &BuildingAllocator,
    economy_tuning: &RuntimeEconomyTuning,
    building_idx: usize,
) -> bool {
    let catalog = load_runtime_economy_catalog()
        .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    nonresidential_downgrade_viable(allocator, economy_tuning, building_idx)
        || industrial_input_coverage_factor(&catalog, building)
            <= economy_tuning
                .viability
                .industrial_max_input_coverage_for_downgrade
                + EPSILON
        || industrial_output_headroom_factor(&catalog, building)
            <= economy_tuning
                .viability
                .industrial_max_output_headroom_for_downgrade
                + EPSILON
}

fn attachment_sort_key(
    building: &crate::simulation::buildings::allocator::Building,
) -> (u64, usize, u8, usize, u16, u16, u8, &str) {
    (
        building.parcel_id,
        building.edge_idx,
        if building.side > 0 { 0 } else { 1 },
        building.cell_x,
        building.width_cells,
        building.depth_cells,
        building.level,
        building.asset_id.as_str(),
    )
}

fn demand_building_action_key(
    building: &crate::simulation::buildings::allocator::Building,
) -> DemandBuildingActionKey {
    DemandBuildingActionKey {
        parcel_id: building.parcel_id,
        edge_idx: building.edge_idx,
        side: building.side,
        cell_x: building.cell_x,
        width_cells: building.width_cells,
        depth_cells: building.depth_cells,
        level: building.level,
        asset_id: building.asset_id.clone(),
    }
}

fn level_change_is_compatible(
    allocator: &BuildingAllocator,
    building_idx: usize,
    target_asset_id: &str,
) -> bool {
    let Some(building) = allocator.buildings.get(building_idx) else {
        return false;
    };
    let Some(target_entry) = allocator.registry.get(target_asset_id) else {
        return false;
    };
    let Some(target_building) = target_entry.manifest.building.as_ref() else {
        return false;
    };
    if !target_building.is_zoned_private() {
        return false;
    }
    if target_building.lot_width_cells != building.width_cells
        || target_building.lot_depth_cells != building.depth_cells
    {
        return false;
    }
    if matches!(
        target_building.zone_type,
        Some(
            crate::assets::asset::ZoneClass::Commercial
                | crate::assets::asset::ZoneClass::Industrial
        )
    ) {
        let binding =
            resolve_building_economy_profile_binding(&allocator.registry, target_asset_id);
        if binding.economy_broken || binding.runtime_id == 0 {
            return false;
        }
    }
    allocator.registry.household_capacity(target_asset_id) >= building.occupancy
        && allocator.worker_capacity_for_asset(target_asset_id) >= building.worker_count
}

fn load_builtin_demand_config() -> Result<Arc<DemandConfig>, String> {
    match BUILTIN_CONFIG.get_or_init(load_config_from_disk) {
        Ok(config) => Ok(Arc::new(config.clone())),
        Err(err) => Err(err.clone()),
    }
}

fn load_config_from_disk() -> Result<DemandConfig, String> {
    let path = repo_relative_path(GROWTH_PROFILES_FILE);
    let content = std::fs::read_to_string(&path)
        .map_err(|err| format!("could not read '{}': {err}", path.display()))?;
    let authored: AuthoredGrowthProfilesFile = toml::from_str(&content)
        .map_err(|err| format!("could not parse '{}': {err}", path.display()))?;
    compile_config(authored)
}

fn repo_relative_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn compile_config(authored: AuthoredGrowthProfilesFile) -> Result<DemandConfig, String> {
    validate_positive_f32(
        authored
            .signal_normalization
            .household_affordability_target_reserve_days,
        "signal_normalization.household_affordability_target_reserve_days",
    )?;
    validate_positive_f32(
        authored
            .signal_normalization
            .household_stock_stability_target_days,
        "signal_normalization.household_stock_stability_target_days",
    )?;

    validate_range_f32(
        authored.household_action.admission_threshold,
        0.0,
        1.0,
        "household_action.admission_threshold",
    )?;
    validate_range_f32(
        authored.household_action.admission_unhoused_ratio_penalty,
        0.0,
        1.0,
        "household_action.admission_unhoused_ratio_penalty",
    )?;
    validate_range_f32(
        authored.household_action.admission_zero_budget_penalty,
        0.0,
        1.0,
        "household_action.admission_zero_budget_penalty",
    )?;
    validate_range_f32(
        authored.household_action.admission_recent_failure_penalty,
        0.0,
        1.0,
        "household_action.admission_recent_failure_penalty",
    )?;
    validate_positive_f32(
        authored.household_action.move_in_min_search_runway_days,
        "household_action.move_in_min_search_runway_days",
    )?;
    validate_positive_f32(
        authored.household_action.move_in_target_search_runway_days,
        "household_action.move_in_target_search_runway_days",
    )?;
    if authored.household_action.move_in_target_search_runway_days
        <= authored.household_action.move_in_min_search_runway_days
    {
        return Err(
            "household_action.move_in_target_search_runway_days must be greater than move_in_min_search_runway_days"
                .to_owned(),
        );
    }
    validate_positive_f32(
        authored
            .household_action
            .move_in_benefit_treasury_coverage_days,
        "household_action.move_in_benefit_treasury_coverage_days",
    )?;
    validate_range_f32(
        authored.household_action.recent_failure_daily_decay,
        0.0,
        1.0,
        "household_action.recent_failure_daily_decay",
    )?;
    validate_range_f32(
        authored.household_action.removal_threshold,
        0.0,
        1.0,
        "household_action.removal_threshold",
    )?;
    validate_range_f32(
        authored
            .household_action
            .persistent_exit_destitute_stock_days,
        0.0,
        365.0,
        "household_action.persistent_exit_destitute_stock_days",
    )?;
    if authored
        .household_action
        .persistent_exit_destitute_unhoused_days
        == 0
    {
        return Err(
            "household_action.persistent_exit_destitute_unhoused_days must be >= 1".to_owned(),
        );
    }
    if authored.household_action.persistent_exit_max_unhoused_days == 0 {
        return Err("household_action.persistent_exit_max_unhoused_days must be >= 1".to_owned());
    }
    validate_range_f32(
        authored.household_action.persistent_exit_daily_fraction,
        0.0,
        1.0,
        "household_action.persistent_exit_daily_fraction",
    )?;

    let spawn_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.spawn_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.spawn_batch_fraction_by_use",
    )?;
    let upgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.upgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.upgrade_batch_fraction_by_use",
    )?;
    let downgrade_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.downgrade_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.downgrade_batch_fraction_by_use",
    )?;
    let despawn_batch_fraction_by_use = validate_use_tuning(
        authored.action_budget.despawn_batch_fraction_by_use,
        0.0,
        1.0,
        "action_budget.despawn_batch_fraction_by_use",
    )?;

    let mut by_id = std::collections::HashMap::new();
    for profile in authored.profiles {
        if by_id.contains_key(&profile.id) {
            return Err(format!("duplicate GrowthProfile id '{}'", profile.id));
        }
        let Some(demand_channel) = DemandChannel::from_str_name(&profile.demand_channel) else {
            return Err(format!(
                "unknown demand_channel '{}' for GrowthProfile '{}'",
                profile.demand_channel, profile.id
            ));
        };
        validate_range_f32(
            profile.spawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.spawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.despawn_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.despawn_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.upgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.upgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.downgrade_threshold,
            0.0,
            1.0,
            &format!("profiles.{}.downgrade_threshold", profile.id),
        )?;
        validate_range_f32(
            profile.hysteresis_margin,
            0.0,
            1.0,
            &format!("profiles.{}.hysteresis_margin", profile.id),
        )?;
        if profile.upgrade_threshold < profile.downgrade_threshold {
            return Err(format!(
                "profiles.{}.upgrade_threshold must be >= downgrade_threshold",
                profile.id
            ));
        }
        by_id.insert(
            profile.id.clone(),
            GrowthProfileRuntime {
                demand_channel,
                spawn_threshold: profile.spawn_threshold,
                despawn_threshold: profile.despawn_threshold,
                upgrade_threshold: profile.upgrade_threshold,
                downgrade_threshold: profile.downgrade_threshold,
            },
        );
    }

    if by_id.len() != SHIPPED_PROFILE_ORDER.len() {
        return Err(format!(
            "expected {} shipped GrowthProfiles, found {}",
            SHIPPED_PROFILE_ORDER.len(),
            by_id.len()
        ));
    }

    let mut profiles = Vec::with_capacity(SHIPPED_PROFILE_ORDER.len());
    for (id, expected_channel) in SHIPPED_PROFILE_ORDER {
        let Some(profile) = by_id.remove(id) else {
            return Err(format!("missing shipped GrowthProfile '{}'", id));
        };
        if profile.demand_channel != expected_channel {
            return Err(format!(
                "GrowthProfile '{}' must use demand_channel {:?}",
                id, expected_channel
            ));
        }
        profiles.push(profile);
    }
    if let Some(extra_id) = by_id.keys().next() {
        return Err(format!(
            "unexpected extra shipped GrowthProfile '{}'",
            extra_id
        ));
    }

    Ok(DemandConfig {
        signal_normalization: SignalNormalizationConfig {
            household_affordability_target_reserve_days: authored
                .signal_normalization
                .household_affordability_target_reserve_days,
            household_stock_stability_target_days: authored
                .signal_normalization
                .household_stock_stability_target_days,
        },

        household_action: HouseholdActionConfig {
            admission_threshold: authored.household_action.admission_threshold,
            admission_unhoused_ratio_penalty: authored
                .household_action
                .admission_unhoused_ratio_penalty,
            admission_zero_budget_penalty: authored.household_action.admission_zero_budget_penalty,
            admission_recent_failure_penalty: authored
                .household_action
                .admission_recent_failure_penalty,
            move_in_min_search_runway_days: authored
                .household_action
                .move_in_min_search_runway_days,
            move_in_target_search_runway_days: authored
                .household_action
                .move_in_target_search_runway_days,
            move_in_benefit_treasury_coverage_days: authored
                .household_action
                .move_in_benefit_treasury_coverage_days,
            recent_failure_daily_decay: authored.household_action.recent_failure_daily_decay,
            removal_threshold: authored.household_action.removal_threshold,
            persistent_exit_destitute_stock_days: authored
                .household_action
                .persistent_exit_destitute_stock_days,
            persistent_exit_destitute_unhoused_days: authored
                .household_action
                .persistent_exit_destitute_unhoused_days,
            persistent_exit_max_unhoused_days: authored
                .household_action
                .persistent_exit_max_unhoused_days,
            persistent_exit_daily_fraction: authored
                .household_action
                .persistent_exit_daily_fraction,
        },
        action_budget: ActionBudgetConfig {
            max_households_per_day: authored.action_budget.max_households_per_day,
            spawn_batch_fraction_by_use,
            upgrade_batch_fraction_by_use,
            downgrade_batch_fraction_by_use,
            despawn_batch_fraction_by_use,
        },
        profiles,
    })
}

fn validate_use_tuning(
    authored: AuthoredUseTuningF32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<UseTuningF32, String> {
    validate_range_f32(
        authored.residential,
        min_value,
        max_value,
        &format!("{label}.residential"),
    )?;
    validate_range_f32(
        authored.commercial,
        min_value,
        max_value,
        &format!("{label}.commercial"),
    )?;
    validate_range_f32(
        authored.industrial,
        min_value,
        max_value,
        &format!("{label}.industrial"),
    )?;
    Ok(UseTuningF32 {
        residential: authored.residential,
        commercial: authored.commercial,
        industrial: authored.industrial,
    })
}

fn validate_positive_f32(value: f32, label: &str) -> Result<(), String> {
    validate_range_f32(value, EPSILON, f32::INFINITY, label)
}

fn validate_range_f32(
    value: f32,
    min_value: f32,
    max_value: f32,
    label: &str,
) -> Result<(), String> {
    if !value.is_finite() || value < min_value || value > max_value {
        Err(format!(
            "{label} must be finite and in [{}..={}]",
            min_value, max_value
        ))
    } else {
        Ok(())
    }
}

fn clamp01(value: f32) -> f32 {
    value.clamp(0.0, 1.0)
}

fn advance_household_action_credit(
    credit: &mut f32,
    pressure: f32,
    threshold: f32,
    max_households_per_day: u32,
    max_actionable_households: u32,
    cadence_fraction: f32,
) -> u32 {
    let normalized_action_pressure = if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    };
    *credit += normalized_action_pressure * max_households_per_day as f32 * cadence_fraction;
    let households_to_act = (*credit).floor().max(0.0) as u32;
    *credit -= households_to_act as f32;
    households_to_act
        .min(max_households_per_day)
        .min(max_actionable_households)
}

fn advance_persistent_exit_credit(
    credit: &mut f32,
    eligible_households: u32,
    daily_fraction: f32,
    max_actionable_households: u32,
) -> u32 {
    if eligible_households == 0 {
        *credit = 0.0;
        return 0;
    }
    *credit += eligible_households as f32 * daily_fraction.max(0.0);
    let households_to_act = (*credit).floor().max(0.0) as u32;
    let households_to_act = households_to_act
        .min(eligible_households)
        .min(max_actionable_households);
    *credit -= households_to_act as f32;
    households_to_act
}

fn advance_building_action_credit(
    credit: &mut f32,
    budget_units: f32,
    max_actionable_buildings: usize,
    cadence_fraction: f32,
) -> usize {
    *credit += budget_units.max(0.0) * cadence_fraction;
    let buildings_to_act = (*credit).floor().max(0.0) as usize;
    *credit -= buildings_to_act as f32;
    buildings_to_act.min(max_actionable_buildings)
}

fn normalized_positive_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold >= 1.0 {
        0.0
    } else {
        clamp01((pressure - threshold) / (1.0 - threshold))
    }
}

fn normalized_negative_pressure(pressure: f32, threshold: f32) -> f32 {
    if threshold <= 0.0 {
        0.0
    } else {
        clamp01((threshold - pressure) / threshold.max(EPSILON))
    }
}

fn add_resource_amount(
    amounts: &mut Vec<(ResourceRuntimeId, f32)>,
    resource_runtime_id: ResourceRuntimeId,
    amount: f32,
) {
    if amount <= 0.0 {
        return;
    }
    if let Some((_, existing)) = amounts
        .iter_mut()
        .find(|(resource, _)| *resource == resource_runtime_id)
    {
        *existing += amount;
    } else {
        amounts.push((resource_runtime_id, amount));
    }
}

fn resource_amount(
    amounts: &[(ResourceRuntimeId, f32)],
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    amounts
        .iter()
        .find_map(|(resource, amount)| (*resource == resource_runtime_id).then_some(*amount))
        .unwrap_or(0.0)
}

struct DailyDemandSnapshot {
    vacant_household_slots: u32,
    total_household_count: u32,
    housed_household_count: u32,
    unhoused_household_count: u32,
    zero_budget_household_count: u32,
    persistent_exit_eligible_household_count: u32,
    unhoused_household_ratio: f32,
    zero_budget_household_ratio: f32,
    housing_availability: f32,
    household_affordability: f32,
    household_stock_stability: f32,
    commercial_capacity_deficit: f32,
    external_connection_available: f32,
    connected_border_count: u32,
    city_treasury_balance: f32,
    candidate_household_size: f32,
    immigrant_starter_savings_per_household: f32,
    candidate_daily_essential_cost: f32,
    unemployment_daily_benefit_per_member: f32,
    existing_unemployed_member_count: u32,
    open_job_slots: u32,
    average_open_job_wage_per_day: f32,
    /// Fraction of commercial input value sourced from OWA rather than local industrial.
    ///
    /// Computed from the daily `daily_owa_input_value` / `daily_local_input_value` accumulators
    /// on each active commercial building. 1.0 = all inputs imported from OWA; 0.0 = fully
    /// supplied by local industrial. Drives industrial spawning based on actual throughput
    /// rather than a headcount ratio, so one farm can partially satisfy multiple shops.
    commercial_owa_dependency: f32,
    // Raw counts needed for non-residential spawn gates.
    housed_resident_count: u32,
}

impl DailyDemandSnapshot {
    fn from_runtime(
        allocator: &BuildingAllocator,
        households: &HouseholdSystem,
        graph: &RegionGraph,
        config: &DemandConfig,
        treasury_balance: f64,
    ) -> Self {
        let mut total_household_slots = 0_u32;
        let mut occupied_household_slots = 0_u32;
        let mut existing_private_building_count = 0_u32;
        let mut total_commercial_owa_input = 0.0_f32;
        let mut total_commercial_local_input = 0.0_f32;
        let mut total_commercial_expected_input = 0.0_f32;
        let mut candidate_household_size_sum = 0.0_f32;
        let mut candidate_household_slot_count = 0_u32;
        let mut filled_job_count = 0_u32;
        let mut open_job_slots = 0_u32;
        let mut open_job_wage_sum = 0.0_f32;

        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let mut commercial_profile_output_resources = Vec::new();
        for profile in catalog.all_profiles() {
            if profile.kind != EconomyProfileRuntimeKind::Store {
                continue;
            }
            for output_port in &profile.outputs {
                add_resource_amount(
                    &mut commercial_profile_output_resources,
                    output_port.resource_runtime_id,
                    1.0,
                );
            }
        }
        let mut demand_sink_rates_by_resource = Vec::new();
        for profile in catalog.all_profiles() {
            if profile.kind != EconomyProfileRuntimeKind::DemandSink {
                continue;
            }
            for input_port in &profile.inputs {
                if resource_amount(
                    &commercial_profile_output_resources,
                    input_port.resource_runtime_id,
                ) <= 0.0
                {
                    continue;
                }
                add_resource_amount(
                    &mut demand_sink_rates_by_resource,
                    input_port.resource_runtime_id,
                    profile.consumption_rate_per_resident,
                );
            }
        }
        let mut daily_supply_cost_per_resident = 0.0_f32;
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let resource_price = catalog
                .unit_price_for_resource(resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!(
                        "resource '{resource_id}' used by household demand sink has no catalog price"
                    )
                });
            daily_supply_cost_per_resident +=
                consumption_rate_per_resident.max(0.0) * resource_price.max(0.0);
        }
        let daily_essential_cost_per_resident =
            daily_supply_cost_per_resident + tuning.households.utility_cost_per_member_per_day;
        let mut commercial_output_capacity_by_resource = Vec::new();

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken || building.economy_broken {
                continue;
            }

            let is_private_building = allocator
                .registry
                .get(&building.asset_id)
                .and_then(|entry| entry.manifest.building.as_ref())
                .map(|authored| authored.is_zoned_private())
                .unwrap_or(!matches!(building.zone_type, ZoneType::None));
            if is_private_building {
                existing_private_building_count = existing_private_building_count.saturating_add(1);
            }

            if matches!(building.zone_type, ZoneType::Residential) {
                let household_capacity = allocator.household_capacity(idx);
                total_household_slots = total_household_slots.saturating_add(household_capacity);
                let occupied = building.occupancy.min(household_capacity);
                occupied_household_slots = occupied_household_slots.saturating_add(occupied);
                let free_slots = household_capacity.saturating_sub(occupied);
                if free_slots > 0 {
                    let candidate_size =
                        candidate_household_size_from_flat_size(allocator.flat_size_m2(idx));
                    candidate_household_size_sum += candidate_size as f32 * free_slots as f32;
                    candidate_household_slot_count =
                        candidate_household_slot_count.saturating_add(free_slots);
                }
            }

            if !building.is_deserted
                && matches!(
                    building.zone_type,
                    ZoneType::Commercial | ZoneType::Industrial
                )
            {
                let worker_capacity = allocator.worker_capacity(idx);
                if worker_capacity > 0 {
                    let average_daily_wage = catalog
                        .profile_by_runtime_id(building.economy_profile_runtime_id)
                        .map(|profile| profile.average_daily_wage())
                        .unwrap_or(0.0);
                    let filled_workers = building.worker_count.min(worker_capacity);
                    filled_job_count = filled_job_count.saturating_add(filled_workers);
                    if average_daily_wage <= 0.1 {
                        continue;
                    }
                    let budget_capacity =
                        (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32;
                    let effective_capacity = worker_capacity.min(budget_capacity);
                    let open_slots = effective_capacity.saturating_sub(filled_workers);
                    open_job_slots = open_job_slots.saturating_add(open_slots);
                    open_job_wage_sum += open_slots as f32 * average_daily_wage.max(0.0);
                }
            }

            // Sum OWA vs local input spend across active commercial buildings.
            // Deserted buildings transact nothing; their accumulators stay at zero.
            if !building.is_deserted && matches!(building.zone_type, ZoneType::Commercial) {
                total_commercial_owa_input += building.daily_owa_input_value;
                total_commercial_local_input += building.daily_local_input_value;
                if let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                {
                    for output_port in &profile.outputs {
                        if resource_amount(
                            &demand_sink_rates_by_resource,
                            output_port.resource_runtime_id,
                        ) > 0.0
                        {
                            add_resource_amount(
                                &mut commercial_output_capacity_by_resource,
                                output_port.resource_runtime_id,
                                output_port.units_per_day,
                            );
                        }
                    }
                    for input_port in &profile.inputs {
                        let resource_price = catalog
                            .unit_price_for_resource(input_port.resource_runtime_id)
                            .unwrap_or_else(|| {
                                let resource_id = catalog
                                    .resource_id_for_runtime_id(input_port.resource_runtime_id)
                                    .unwrap_or("<unknown>");
                                panic!(
                                    "resource '{resource_id}' used by profile '{}' has no catalog price",
                                    profile.id
                                )
                            });
                        total_commercial_expected_input +=
                            input_port.units_per_day * resource_price;
                    }
                }
            }
        }

        let vacant_household_slots = total_household_slots.saturating_sub(occupied_household_slots);
        let candidate_household_size = if candidate_household_slot_count == 0 {
            0.0
        } else {
            candidate_household_size_sum / candidate_household_slot_count as f32
        };
        let immigrant_starter_savings_per_household =
            candidate_household_size * tuning.households.immigrant_starting_budget_per_member;
        let candidate_daily_essential_cost =
            candidate_household_size * daily_essential_cost_per_resident;
        let average_open_job_wage_per_day = if open_job_slots == 0 {
            0.0
        } else {
            open_job_wage_sum / open_job_slots as f32
        };

        let mut housed_resident_count = 0_u32;
        let mut housed_household_count = 0_u32;
        let mut unhoused_household_count = 0_u32;
        let mut zero_budget_household_count = 0_u32;
        let mut persistent_exit_eligible_household_count = 0_u32;
        let mut household_affordability_sum = 0.0;
        let mut household_stock_stability_sum = 0.0;

        for household in &households.households {
            if household.member_count == 0 {
                continue;
            }
            if household.budget <= EPSILON {
                zero_budget_household_count = zero_budget_household_count.saturating_add(1);
            }
            let is_housed = household.home_building_id < allocator.buildings.len()
                && !allocator.buildings[household.home_building_id].broken
                && !allocator.buildings[household.home_building_id].economy_broken;
            if is_housed {
                housed_household_count = housed_household_count.saturating_add(1);
                housed_resident_count =
                    housed_resident_count.saturating_add(household.member_count as u32);
                household_affordability_sum += clamp01(
                    household_reserve_days(&catalog, &tuning, household)
                        / config
                            .signal_normalization
                            .household_affordability_target_reserve_days,
                );
                household_stock_stability_sum += clamp01(
                    household.stock_days
                        / config
                            .signal_normalization
                            .household_stock_stability_target_days,
                );
            } else {
                unhoused_household_count = unhoused_household_count.saturating_add(1);
                let is_destitute = household.budget <= EPSILON
                    && household.stock_days
                        <= config.household_action.persistent_exit_destitute_stock_days;
                let destitute_exit_eligible = is_destitute
                    && household.unhoused_days_elapsed
                        >= config
                            .household_action
                            .persistent_exit_destitute_unhoused_days;
                let max_unhoused_exit_eligible = household.unhoused_days_elapsed
                    >= config.household_action.persistent_exit_max_unhoused_days;
                if destitute_exit_eligible || max_unhoused_exit_eligible {
                    persistent_exit_eligible_household_count =
                        persistent_exit_eligible_household_count.saturating_add(1);
                }
            }
        }

        let total_household_count = housed_household_count.saturating_add(unhoused_household_count);
        let housing_availability = if total_household_slots == 0 {
            0.0
        } else {
            clamp01(vacant_household_slots as f32 / total_household_slots as f32)
        };
        let household_affordability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_affordability_sum / housed_household_count as f32)
        };
        let household_stock_stability = if housed_household_count == 0 {
            1.0
        } else {
            clamp01(household_stock_stability_sum / housed_household_count as f32)
        };
        let mut total_commercial_consumer_demand = 0.0_f32;
        let mut unmet_commercial_consumer_demand = 0.0_f32;
        for &(resource_runtime_id, consumption_rate_per_resident) in &demand_sink_rates_by_resource
        {
            let consumer_demand = consumption_rate_per_resident * housed_resident_count as f32;
            if consumer_demand <= 0.0 {
                continue;
            }
            let placed_capacity =
                resource_amount(&commercial_output_capacity_by_resource, resource_runtime_id);
            total_commercial_consumer_demand += consumer_demand;
            unmet_commercial_consumer_demand += (consumer_demand - placed_capacity).max(0.0);
        }
        let commercial_capacity_deficit = if total_commercial_consumer_demand <= 0.0 {
            0.0
        } else {
            clamp01(unmet_commercial_consumer_demand / total_commercial_consumer_demand)
        };
        let connected_border_count = graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(idx, node)| {
                node.node_type == NodeType::Border
                    && graph.node_adjacency(*idx as u32).iter().any(|&edge_idx| {
                        let edge = graph.edge(edge_idx);
                        !edge.deleted
                            && edge.primary_type == TransitType::Road
                            && (edge.allowed_types & TransitFlags::CAR) != 0
                    })
            })
            .count() as u32;
        let external_connection_available = if connected_border_count > 0 { 1.0 } else { 0.0 };
        let unhoused_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(unhoused_household_count as f32 / total_household_count as f32)
        };
        let zero_budget_household_ratio = if total_household_count == 0 {
            0.0
        } else {
            clamp01(zero_budget_household_count as f32 / total_household_count as f32)
        };
        let existing_unemployed_member_count =
            housed_resident_count.saturating_sub(filled_job_count);

        // Fraction of commercial input value sourced from OWA vs local industrial.
        // Uses expected daily input cost as a minimum denominator so a tiny OWA
        // emergency import (e.g. one unit when the building budget is briefly
        // exhausted) does not register as full OWA dependency when local supply
        // exists and normal throughput resumes the next hour.
        let commercial_owa_dependency = {
            let actual_total = total_commercial_owa_input + total_commercial_local_input;
            let denom = actual_total.max(total_commercial_expected_input);
            if denom <= 0.0 {
                0.0
            } else {
                clamp01(total_commercial_owa_input / denom)
            }
        };

        debug_log!(
            "spawn",
            "daily_snapshot: border_nodes={} ext_conn={:.0} housing_avail={:.2} \
             unhoused_ratio={:.2} zero_budget_ratio={:.2} stock_stab={:.2} afford={:.2} \
             com_cap_def={:.2} owa_dep={:.2} treasury={:.0} cand_size={:.1} \
             open_jobs={} existing_unemployed={} private_buildings={}",
            connected_border_count,
            external_connection_available,
            housing_availability,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            household_stock_stability,
            household_affordability,
            commercial_capacity_deficit,
            commercial_owa_dependency,
            treasury_balance,
            candidate_household_size,
            open_job_slots,
            existing_unemployed_member_count,
            existing_private_building_count,
        );

        Self {
            vacant_household_slots,
            total_household_count,
            housed_household_count,
            unhoused_household_count,
            zero_budget_household_count,
            persistent_exit_eligible_household_count,
            unhoused_household_ratio,
            zero_budget_household_ratio,
            housing_availability,
            household_affordability,
            household_stock_stability,
            commercial_capacity_deficit,
            external_connection_available,
            connected_border_count,
            city_treasury_balance: treasury_balance as f32,
            candidate_household_size,
            immigrant_starter_savings_per_household,
            candidate_daily_essential_cost,
            unemployment_daily_benefit_per_member: tuning.unemployment_daily_benefit_per_member,
            existing_unemployed_member_count,
            open_job_slots,
            average_open_job_wage_per_day,
            commercial_owa_dependency,
            housed_resident_count,
        }
    }
}

fn candidate_household_size_from_flat_size(flat_size_m2: f32) -> u16 {
    if flat_size_m2 > 1.0 {
        ((flat_size_m2 / 40.0).ceil() as u16).clamp(1, 5)
    } else {
        2
    }
}

/// Returns true if the housed population is large enough to staff the spawning asset.
/// `available_unemployed` is the remaining workforce pool for this daily pass (starts
/// from `housed_resident_count` and is decremented by each approved spawn).
fn nonresidential_passes_labour_gate(
    allocator: &BuildingAllocator,
    asset_id: &str,
    available_unemployed: u32,
) -> bool {
    let required = allocator.worker_capacity_for_asset(asset_id);
    // Buildings with no workers (e.g. utility nodes) always pass.
    required == 0 || available_unemployed >= required
}

/// Returns true if the resident population can absorb more output from the spawning asset.
/// Compares total placed output capacity against total derived consumer demand.
fn nonresidential_passes_absorption_gate(
    allocator: &BuildingAllocator,
    catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
    asset_id: &str,
    housed_resident_count: u32,
) -> bool {
    use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
    // Resolve the candidate profile from the asset registry.
    let Some(profile_id) = allocator.registry.economy_profile(asset_id) else {
        // No economy profile binding → no capacity limit, pass.
        return true;
    };
    let Some(candidate_profile) = catalog.profile_for_id(profile_id) else {
        return true;
    };
    // Buildings with no declared outputs are not capacity-limited.
    if candidate_profile.outputs.is_empty() {
        return true;
    }
    let candidate_output_resource_ids: Vec<_> = candidate_profile
        .outputs
        .iter()
        .map(|p| p.resource_runtime_id)
        .collect();

    // Sum output capacity (units/day) already placed for matching resource types.
    // Deserted buildings are excluded: they produce nothing and must not block a replacement spawn.
    let placed_capacity: f32 = allocator
        .buildings
        .iter()
        .filter(|b| !b.broken && !b.economy_broken && !b.is_deserted)
        .filter_map(|b| {
            let p = catalog.profile_by_runtime_id(b.economy_profile_runtime_id)?;
            let overlaps = p
                .outputs
                .iter()
                .any(|port| candidate_output_resource_ids.contains(&port.resource_runtime_id));
            if overlaps {
                Some(p.outputs.iter().map(|port| port.units_per_day).sum::<f32>())
            } else {
                None
            }
        })
        .sum();

    // Derive consumer demand from housed residents and demand-sink consumption rates.
    let consumer_demand: f32 = catalog
        .all_profiles()
        .iter()
        .filter(|p| p.kind == EconomyProfileRuntimeKind::DemandSink)
        .filter(|p| {
            p.inputs
                .iter()
                .any(|port| candidate_output_resource_ids.contains(&port.resource_runtime_id))
        })
        .map(|p| p.consumption_rate_per_resident * housed_resident_count as f32)
        .sum();

    // If no demand-sink found for this resource, gate is not applicable → pass.
    if consumer_demand == 0.0 {
        return true;
    }
    placed_capacity < consumer_demand
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::economy::households::{
        Household, HouseholdSystem, REPLENISHMENT_STABLE,
    };
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{
        EdgeClass, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::zoning::ZoningSystem;
    use godot::prelude::{Vector2, Vector3};

    fn test_economy_runtime_id(zone_type: ZoneType) -> u16 {
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        match zone_type {
            ZoneType::Commercial => {
                catalog
                    .profile_for_id("grocery_basic")
                    .expect("grocery starter profile")
                    .runtime_id
            }
            ZoneType::Industrial => {
                catalog
                    .profile_for_id("food_processor_basic")
                    .expect("food processor starter profile")
                    .runtime_id
            }
            _ => 0,
        }
    }

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        asset_id: &str,
        zone_type: ZoneType,
    ) -> String {
        register_family_asset(allocator, asset_id, zone_type, None, 1)
    }

    fn register_family_asset(
        allocator: &mut BuildingAllocator,
        asset_id: &str,
        zone_type: ZoneType,
        asset_set: Option<&str>,
        level: u8,
    ) -> String {
        let (zone_class, household_capacity, worker_capacity) = match zone_type {
            ZoneType::Residential => (ZoneClass::Residential, Some(6), None),
            ZoneType::Commercial => (ZoneClass::Commercial, None, Some(4)),
            ZoneType::Industrial => (ZoneClass::Industrial, None, Some(4)),
            ZoneType::Office => (ZoneClass::Office, None, Some(4)),
            ZoneType::Mixed => (ZoneClass::Mixed, Some(4), Some(2)),
            ZoneType::None => panic!("test assets must use a real zone type"),
        };
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: asset_set.map(str::to_owned),
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                flat_size_m2: None,
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone_class),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile: match zone_type {
                    ZoneType::Commercial => Some("grocery_basic".to_owned()),
                    ZoneType::Industrial => Some("food_processor_basic".to_owned()),
                    _ => None,
                },
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        };
        allocator.registry.register("test", manifest, String::new());
        format!("test:{asset_id}")
    }

    fn building(
        zone_type: ZoneType,
        stock: f32,
        occupancy: u32,
        worker_count: u32,
        asset_id: String,
    ) -> Building {
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        let runtime_id = test_economy_runtime_id(zone_type);
        let mut resource_inventory = vec![0.0; catalog.resource_count()];
        if stock > 0.0
            && let Some(profile) = catalog.profile_by_runtime_id(runtime_id)
            && let Some(output_port) = profile.outputs.first()
        {
            resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
        }
        Building {
            center_x: 0.0,
            center_y: 0.0,
            width_cells: 2,
            depth_cells: 2,
            zone_profile_runtime_id: 0,
            parcel_id: 0,
            zone_type,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy,
            worker_count,
            asset_id,
            level: 1,
            broken: false,
            economy_profile_runtime_id: runtime_id,
            economy_broken: false,
            resource_inventory,
            revenue: 0.0,
            operating_budget: 500.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    fn housed_household(
        home_building_id: usize,
        member_count: u16,
        budget: f32,
        stock_days: f32,
    ) -> Household {
        Household {
            home_building_id,
            budget,
            stock: stock_days * member_count as f32,
            member_count,
            consumption_rate: 1.0,
            stock_days,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_hours: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_hours: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        }
    }

    fn unhoused_household(member_count: u16, budget: f32, stock_days: f32) -> Household {
        housed_household(usize::MAX, member_count, budget, stock_days)
    }

    fn graph_with_connected_border() -> RegionGraph {
        let mut graph = RegionGraph::new();
        let border = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
        let junction = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        graph.add_edge(Edge {
            start_node: border,
            end_node: junction,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 50.0,
            physical_length: 50.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        graph
    }

    fn empty_zoning() -> ZoningSystem {
        ZoningSystem::new(&WorldConfig::default())
    }

    fn vacant_admission_snapshot() -> DailyDemandSnapshot {
        DailyDemandSnapshot {
            vacant_household_slots: 10,
            total_household_count: 4,
            housed_household_count: 4,
            unhoused_household_count: 0,
            zero_budget_household_count: 0,
            persistent_exit_eligible_household_count: 0,
            unhoused_household_ratio: 0.0,
            zero_budget_household_ratio: 0.0,
            housing_availability: 1.0,
            household_affordability: 1.0,
            household_stock_stability: 1.0,
            commercial_capacity_deficit: 0.0,
            external_connection_available: 1.0,
            connected_border_count: 1,
            city_treasury_balance: 100_000.0,
            candidate_household_size: 2.0,
            immigrant_starter_savings_per_household: 30.0,
            candidate_daily_essential_cost: 56.0,
            unemployment_daily_benefit_per_member: 30.0,
            existing_unemployed_member_count: 0,
            open_job_slots: 0,
            average_open_job_wage_per_day: 0.0,
            commercial_owa_dependency: 0.0,
            housed_resident_count: 10,
        }
    }

    #[test]
    fn daily_pass_raises_commercial_and_industrial_pressure_on_shortages() {
        let mut allocator = BuildingAllocator::new();
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        // Simulate commercial building that has been buying inputs from OWA:
        // daily_owa_input_value > 0 drives industrial demand.
        let mut com = building(ZoneType::Commercial, 80.0, 0, 1, commercial_asset);
        com.daily_owa_input_value = 100.0;
        allocator.buildings.push(com);
        // occupancy=2 so resident_presence > 0, allowing organic commercial/industrial pressure.
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            2,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        // home_building_id=1 (residential is now at index 1 after commercial at 0)
        households
            .households
            .push(housed_household(1, 2, 120.0, 0.25));

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        assert!(demand.commercial > 0.0);
        assert!(demand.industrial > 0.0);
    }

    #[test]
    fn daily_pass_raises_commercial_pressure_when_residents_lack_shop_capacity() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            1,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(0, 5, 1_000.0, 3.0));

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        assert!(
            demand.commercial > 0.95,
            "commercial demand should anticipate missing shop capacity, got={:.3}",
            demand.commercial
        );
    }

    #[test]
    fn daily_pass_uses_short_run_purchase_power_for_missing_shop_capacity() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            1,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(0, 5, 140.0, 3.0));

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        assert!(
            demand.commercial > 0.95,
            "one reserve day should still represent immediate grocery buying power, got={:.3}",
            demand.commercial
        );
    }

    #[test]
    fn daily_pass_raises_residential_pressure_when_jobs_outrun_housing() {
        let mut allocator = BuildingAllocator::new();
        let industrial_asset =
            register_test_asset(&mut allocator, "industrial", ZoneType::Industrial);
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Industrial,
            300.0,
            0,
            1,
            industrial_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Commercial,
            500.0,
            0,
            1,
            commercial_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            5,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        for _ in 0..5 {
            households
                .households
                .push(housed_household(2, 1, 120.0, 3.0));
        }

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();
        demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        assert!(demand.residential > 0.50);
    }

    #[test]
    fn daily_pass_blocks_growth_without_external_connection() {
        let allocator = BuildingAllocator::new();
        let households = HouseholdSystem::new();
        let graph = RegionGraph::new();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        // No external connection means inflow_desire = 0.0 and removal_pressure = 0.0
        // (no households → unhoused_ratio = 0), so ResidentialGrowth is at equilibrium
        // (= 0.5) — no spawn pressure, no despawn pressure. Growth is blocked because
        // 0.5 is below the spawn threshold.
        assert!(
            demand.residential <= 0.50,
            "residential={}",
            demand.residential
        );
        assert_eq!(demand.commercial, 0.0);
        assert_eq!(demand.industrial, 0.0);
        assert_eq!(demand.households_to_admit_today, 0);
    }

    #[test]
    fn hourly_pass_produces_startup_household_admission_when_capacity_jobs_and_border_exist() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            0,
            0,
            residential_asset,
        ));
        allocator.buildings.push(building(
            ZoneType::Commercial,
            500.0,
            0,
            0,
            commercial_asset,
        ));

        let households = HouseholdSystem::new();
        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);

        assert!(demand.households_to_admit_today > 0);
    }

    #[test]
    fn hourly_admission_soft_damps_when_household_economy_is_failing() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            1,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        households.households.push(housed_household(0, 1, 0.0, 0.0));
        for _ in 0..3 {
            households.households.push(unhoused_household(1, 0.0, 0.0));
        }

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, -100.0);

        assert_eq!(
            demand.households_to_admit_today, 0,
            "vacancy alone must not keep admitting households while affordability is zero, many households are unhoused, and the treasury is negative"
        );
    }

    #[test]
    fn household_admission_diagnostics_record_pressure_breakdown() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            0,
            0,
            residential_asset,
        ));

        let households = HouseholdSystem::new();
        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_hourly_pass(&allocator, &households, &graph, &zoning, 1_000.0);
        let diagnostics = demand.last_admission_diagnostics;

        assert_eq!(diagnostics.total_household_count, 0);
        assert_eq!(
            diagnostics.vacant_household_slots,
            allocator.household_capacity(0)
        );
        assert_eq!(diagnostics.connected_border_count, 1);
        assert!(diagnostics.base_pressure > 0.0);
        assert_eq!(
            diagnostics.planned_households,
            demand.households_to_admit_today
        );

        demand.record_household_admission_execution(1);

        assert_eq!(demand.last_admission_diagnostics.launched_households, 1);
    }

    #[test]
    fn household_removal_diagnostics_record_failure_signal_counts() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            1,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(0, 1, 200.0, 3.0));
        households.households.push(unhoused_household(1, 0.0, 0.0));
        households.households.push(unhoused_household(1, 0.0, 0.0));

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households, &graph, &zoning, -100.0);
        let diagnostics = demand.last_removal_diagnostics;

        assert_eq!(diagnostics.total_household_count, 3);
        assert_eq!(diagnostics.housed_household_count, 1);
        assert_eq!(diagnostics.unhoused_household_count, 2);
        assert_eq!(diagnostics.zero_budget_household_count, 2);
        assert!((diagnostics.pressure - (2.0 / 3.0)).abs() < 1e-4);
        assert!((diagnostics.failure_pressure - (2.0 / 3.0)).abs() < 1e-4);
        assert_eq!(diagnostics.recent_failure_before, 0.0);
        assert!((diagnostics.recent_failure_after - (2.0 / 3.0)).abs() < 1e-4);
        assert!((demand.recent_household_failure_pressure - (2.0 / 3.0)).abs() < 1e-4);
        assert_eq!(
            diagnostics.planned_households,
            demand.households_to_remove_today
        );

        demand.record_household_removal_execution(2);

        assert_eq!(demand.last_removal_diagnostics.removed_households, 2);
    }

    #[test]
    fn persistent_exit_removes_failed_unhoused_tail_below_crisis_threshold() {
        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "residential", ZoneType::Residential);
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            7,
            0,
            residential_asset,
        ));

        let mut households = HouseholdSystem::new();
        for _ in 0..7 {
            households
                .households
                .push(housed_household(0, 1, 200.0, 3.0));
        }
        for _ in 0..8 {
            let mut household = unhoused_household(1, 0.0, 0.0);
            household.unhoused_days_elapsed = 2;
            households.households.push(household);
        }

        let graph = graph_with_connected_border();
        let zoning = empty_zoning();
        let mut demand = DemandSystem::new();

        demand.run_daily_pass(&allocator, &households, &graph, &zoning, -100.0);
        let diagnostics = demand.last_removal_diagnostics;

        assert_eq!(diagnostics.unhoused_household_count, 8);
        assert_eq!(diagnostics.total_household_count, 15);
        assert!(diagnostics.pressure < diagnostics.threshold);
        assert_eq!(diagnostics.normalized_action_pressure, 0.0);
        assert_eq!(diagnostics.persistent_exit_eligible_household_count, 8);
        assert_eq!(diagnostics.persistent_exit_planned_households, 2);
        assert_eq!(demand.households_to_remove_today, 2);
    }

    #[test]
    fn recent_failure_memory_damps_household_admission_pressure() {
        let mut healthy_demand = DemandSystem::new();
        let healthy = healthy_demand
            .update_pressure_channels_from_snapshot(&vacant_admission_snapshot())
            .admission_pressure;
        let mut cooling_demand = DemandSystem::new();
        cooling_demand.recent_household_failure_pressure = 0.8;
        let cooling_inputs =
            cooling_demand.update_pressure_channels_from_snapshot(&vacant_admission_snapshot());

        assert!(
            cooling_inputs.admission_pressure < healthy * 0.40,
            "recent failure memory should substantially reduce otherwise healthy vacancy admission"
        );
        assert_eq!(
            cooling_inputs.admission_diagnostics.recent_failure_pressure,
            0.8
        );
        assert!(cooling_inputs.admission_diagnostics.recent_failure_factor < 0.35);
    }

    #[test]
    fn admission_pressure_counts_zero_budget_households() {
        fn snapshot_with_zero_budget_ratio(
            zero_budget_household_ratio: f32,
        ) -> DailyDemandSnapshot {
            let mut snapshot = vacant_admission_snapshot();
            snapshot.total_household_count = 10;
            snapshot.housed_household_count = 10;
            snapshot.zero_budget_household_count =
                (zero_budget_household_ratio * 10.0).round() as u32;
            snapshot.zero_budget_household_ratio = zero_budget_household_ratio;
            snapshot
        }

        let mut healthy_demand = DemandSystem::new();
        let healthy_pressure = healthy_demand
            .update_pressure_channels_from_snapshot(&snapshot_with_zero_budget_ratio(0.0))
            .admission_pressure;
        let mut failing_demand = DemandSystem::new();
        let failing_pressure = failing_demand
            .update_pressure_channels_from_snapshot(&snapshot_with_zero_budget_ratio(0.8))
            .admission_pressure;

        assert!(
            failing_pressure < healthy_pressure,
            "zero-budget households must soft-damp admission pressure even when surviving housed households look affordable"
        );
    }

    #[test]
    fn move_in_acceptance_accounts_for_benefit_treasury_coverage() {
        let mut covered_snapshot = vacant_admission_snapshot();
        covered_snapshot.existing_unemployed_member_count = 100;
        covered_snapshot.city_treasury_balance = 100_000.0;

        let mut depleted_snapshot = vacant_admission_snapshot();
        depleted_snapshot.existing_unemployed_member_count = 100;
        depleted_snapshot.city_treasury_balance = 0.0;

        let mut covered_demand = DemandSystem::new();
        let covered_inputs =
            covered_demand.update_pressure_channels_from_snapshot(&covered_snapshot);
        let mut depleted_demand = DemandSystem::new();
        let depleted_inputs =
            depleted_demand.update_pressure_channels_from_snapshot(&depleted_snapshot);

        assert!(
            covered_inputs.admission_pressure > 0.9,
            "covered benefit runway should admit into available housing"
        );
        assert_eq!(
            depleted_inputs.admission_diagnostics.benefit_reliability,
            0.0
        );
        assert_eq!(
            depleted_inputs.admission_diagnostics.move_in_acceptance,
            0.0
        );
        assert_eq!(depleted_inputs.admission_pressure, 0.0);
    }

    #[test]
    fn open_jobs_make_move_in_viable_without_benefits() {
        let mut snapshot = vacant_admission_snapshot();
        snapshot.city_treasury_balance = 0.0;
        snapshot.open_job_slots = 2;
        snapshot.average_open_job_wage_per_day = 100.0;

        let mut demand = DemandSystem::new();
        let inputs = demand.update_pressure_channels_from_snapshot(&snapshot);

        assert_eq!(inputs.admission_diagnostics.expected_employed_members, 2.0);
        assert_eq!(inputs.admission_diagnostics.daily_deficit, 0.0);
        assert!(
            inputs.admission_pressure > 0.9,
            "budget-backed open jobs should make the candidate household viable without benefit treasury"
        );
    }

    #[test]
    fn snapshot_computes_owa_dependency_from_input_accumulators() {
        // Commercial building (grocery_basic profile) with 75 currency from OWA and 25 from local.
        // Expected daily input = 160 staple_food * 15.0/unit = 2400.
        // denom = max(actual=100, expected=2400) = 2400.
        // owa_dependency = 75 / 2400 = 0.03125.
        let mut allocator = BuildingAllocator::new();
        let commercial_asset =
            register_test_asset(&mut allocator, "commercial", ZoneType::Commercial);
        let mut com = building(ZoneType::Commercial, 40.0, 0, 1, commercial_asset);
        com.daily_owa_input_value = 75.0;
        com.daily_local_input_value = 25.0;
        allocator.buildings.push(com);

        let households = HouseholdSystem::new();
        let graph = graph_with_connected_border();
        let config = load_builtin_demand_config().expect("built-in demand config must load");

        let snapshot =
            DailyDemandSnapshot::from_runtime(&allocator, &households, &graph, &config, 1_000.0);

        assert!(
            (snapshot.commercial_owa_dependency - 0.03125).abs() < 1e-4,
            "owa_dependency must equal owa/max(actual,expected): got={:.6}",
            snapshot.commercial_owa_dependency
        );
    }

    #[test]
    fn residential_upgrade_requires_current_household_affordability_for_target_level() {
        let mut allocator = BuildingAllocator::new();
        let level_one = register_family_asset(
            &mut allocator,
            "res_level_1",
            ZoneType::Residential,
            Some("res_family"),
            1,
        );
        let _level_two = register_family_asset(
            &mut allocator,
            "res_level_2",
            ZoneType::Residential,
            Some("res_family"),
            2,
        );
        allocator.buildings.push(building(
            ZoneType::Residential,
            0.0,
            6,
            0,
            level_one.clone(),
        ));

        let mut households = HouseholdSystem::new();
        households
            .households
            .push(housed_household(0, 6, 200.0, 3.0));

        let demand = DemandSystem::new();
        let economy_tuning =
            load_runtime_economy_tuning().expect("runtime economy tuning must load");
        let residential_occupants =
            ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
        let low_affordability = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Residential,
            0.95,
        );
        assert!(low_affordability.upgrades.is_empty());

        households.households[0].budget = 1_200.0;
        let residential_occupants =
            ResidentialOccupantSnapshot::from_runtime(&allocator, &households);
        let high_affordability = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Residential,
            0.95,
        );
        assert_eq!(high_affordability.upgrades.len(), 1);
    }

    #[test]
    fn commercial_upgrade_requires_business_viability_not_only_pressure() {
        let mut allocator = BuildingAllocator::new();
        let level_one = register_family_asset(
            &mut allocator,
            "com_level_1",
            ZoneType::Commercial,
            Some("com_family"),
            1,
        );
        let _level_two = register_family_asset(
            &mut allocator,
            "com_level_2",
            ZoneType::Commercial,
            Some("com_family"),
            2,
        );
        let mut shop = building(ZoneType::Commercial, 50.0, 0, 1, level_one);
        shop.operating_budget = 20.0;
        allocator.buildings.push(shop);

        let households = HouseholdSystem::new();
        let demand = DemandSystem::new();
        let economy_tuning =
            load_runtime_economy_tuning().expect("runtime economy tuning must load");
        let residential_occupants =
            ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

        let weak_viability = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Commercial,
            0.95,
        );
        assert!(weak_viability.upgrades.is_empty());

        allocator.buildings[0].worker_count = 15;
        allocator.buildings[0].operating_budget = 6_000.0;
        let strong_viability = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Commercial,
            0.95,
        );
        assert_eq!(strong_viability.upgrades.len(), 1);
    }

    #[test]
    fn industrial_upgrade_uses_shipped_profile_viability_gates() {
        let mut allocator = BuildingAllocator::new();
        let level_one = register_family_asset(
            &mut allocator,
            "ind_level_1",
            ZoneType::Industrial,
            Some("ind_family"),
            1,
        );
        let _level_two = register_family_asset(
            &mut allocator,
            "ind_level_2",
            ZoneType::Industrial,
            Some("ind_family"),
            2,
        );
        let mut factory = building(ZoneType::Industrial, 50.0, 0, 10, level_one);
        factory.operating_budget = 4_000.0;
        allocator.buildings.push(factory);

        let households = HouseholdSystem::new();
        let demand = DemandSystem::new();
        let economy_tuning =
            load_runtime_economy_tuning().expect("runtime economy tuning must load");
        let residential_occupants =
            ResidentialOccupantSnapshot::from_runtime(&allocator, &households);

        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog must load");
        let starter_factory = catalog
            .profile_for_id("food_processor_basic")
            .expect("food processor starter profile");
        assert!(
            starter_factory.inputs.is_empty(),
            "shipped starter industrial profile is currently inputless"
        );

        let starter_headroom = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Industrial,
            0.95,
        );
        assert_eq!(starter_headroom.upgrades.len(), 1);

        if let Some(input_port) = starter_factory.inputs.first() {
            allocator.buildings[0].set_inventory_units(input_port.resource_runtime_id, 320.0);
        }
        if let Some(output_port) = starter_factory.outputs.first() {
            allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 50.0);
        }
        let same_profile = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Industrial,
            0.95,
        );
        assert_eq!(same_profile.upgrades.len(), 1);

        if let Some(output_port) = starter_factory.outputs.first() {
            allocator.buildings[0].set_inventory_units(output_port.resource_runtime_id, 630.0);
        }
        let jammed_output = demand.collect_existing_building_candidates(
            &allocator,
            &households,
            economy_tuning.as_ref(),
            &residential_occupants,
            ZoneType::Industrial,
            0.95,
        );
        assert!(jammed_output.upgrades.is_empty());
    }
}
