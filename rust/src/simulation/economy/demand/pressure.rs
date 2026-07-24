//! Demand pressure channel computation.

use super::config::DemandConfig;
use super::credits::clamp01;
use super::diagnostics::{HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics};
use super::snapshot::DailyDemandSnapshot;
use super::system::DemandSystem;
use super::types::{DemandUse, EPSILON};
use crate::simulation::economy::households::expected_adult_members_for_household_size;
use crate::simulation::zoning::ZoneType;

#[derive(Clone, Copy, Debug)]
pub(super) struct DemandPressureInputs {
    pub(super) admission_pressure: f32,
    pub(super) removal_pressure: f32,
    pub(super) admission_diagnostics: HouseholdAdmissionDiagnostics,
    pub(super) removal_diagnostics: HouseholdRemovalDiagnostics,
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct MoveInAcceptance {
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

impl DemandSystem {
    pub(super) fn update_pressure_channels_from_snapshot(
        &mut self,
        snapshot: &DailyDemandSnapshot,
    ) -> DemandPressureInputs {
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

        // Admission pressure is incoming household pull. Vacant homes cap execution only; they do
        // not decide whether households want to enter the city.
        let incoming_household_pressure = clamp01(snapshot.incoming_household_need);
        let admission_base_pressure = ext_conn * incoming_household_pressure;
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
        let construction_move_in = compute_construction_move_in_acceptance(&self.config, snapshot);
        let residential_construction_viability =
            clamp01(construction_move_in.acceptance) * clamp01(admission_failure_factor);
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
            incoming_household_need: snapshot.incoming_household_need,
            open_job_household_pull: snapshot.open_job_household_pull,
            regional_growth_household_pull: snapshot.regional_growth_household_pull,
            household_affordability: snapshot.household_affordability,
            move_in_acceptance: clamp01(move_in.acceptance),
            construction_move_in_acceptance: clamp01(construction_move_in.acceptance),
            construction_move_in_search_runway_days: construction_move_in.search_runway_days,
            construction_move_in_runway_factor: clamp01(construction_move_in.runway_factor),
            residential_construction_viability,
            move_in_search_runway_days: move_in.search_runway_days,
            move_in_runway_factor: clamp01(move_in.runway_factor),
            candidate_household_size: move_in.candidate_household_size,
            candidate_effective_workers: move_in.candidate_effective_workers,
            open_job_slots: snapshot.open_job_slots,
            physical_worker_capacity: snapshot.physical_worker_capacity,
            funded_worker_capacity: snapshot.funded_worker_capacity,
            open_jobs_unfunded: snapshot.open_jobs_unfunded,
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

        // Residential demand follows net migration balance. Incoming household pull raises both
        // admission pressure and the desire for more residential capacity; existing vacancy only
        // satisfies that pressure and caps actual move-ins.
        let inflow_desire =
            clamp01(ext_conn * incoming_household_pressure * residential_construction_viability);
        let net_residential = (inflow_desire - removal_pressure).clamp(-1.0, 1.0);
        self.residential = net_residential * 0.5 + 0.5;

        // Commercial: residents need both existing stocked households and enough shop output
        // capacity to keep those stocks stable. Uses short-run purchase power rather than the
        // long-run reserve target so starter cities can spawn shops before household stockout.
        self.commercial = clamp01(commercial_need * household_purchase_power * ext_conn);
        // Industrial: paper input-capacity gaps plus actual commercial OWA reliance. Spawn volume
        // still uses committed missing input capacity, so this pressure can flag a failing local
        // supply chain without blindly duplicating already committed factories.
        let industrial_need = snapshot
            .industrial_input_capacity_deficit
            .max(snapshot.commercial_owa_dependency);
        self.industrial = clamp01(industrial_need * ext_conn);

        DemandPressureInputs {
            admission_pressure,
            removal_pressure,
            admission_diagnostics,
            removal_diagnostics,
        }
    }

    pub(super) fn pressure_for_use(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    // Net growth/decline pressure for display, in −1.0..+1.0.
    //
    // Uses the low-density profile thresholds as reference, with the same hysteresis margin state
    // used by action planning:
    // - Positive: channel is above the active spawn threshold (city wants to grow this use)
    // - Negative: channel is below the active despawn threshold (city wants to shrink this use)
    // - Zero: channel is in the dead zone between thresholds (no pressure either way)
    pub(super) fn net_pressure_for(&self, use_kind: DemandUse) -> f32 {
        if self.cheat_max_demands_enabled {
            return 1.0;
        }
        let channel = self.pressure_for_use(use_kind);
        let zone_type = match use_kind {
            DemandUse::Residential => ZoneType::Residential,
            DemandUse::Commercial => ZoneType::Commercial,
            DemandUse::Industrial => ZoneType::Industrial,
        };
        let Some(profile) = self.config.profile_for_zone_density(zone_type, "low") else {
            return 0.0;
        };
        let spawn_t = if self.spawn_hysteresis_active.get(use_kind) {
            (profile.spawn_threshold - profile.hysteresis_margin).max(0.0)
        } else {
            profile.spawn_threshold
        };
        let despawn_t = if self.despawn_hysteresis_active.get(use_kind) {
            (profile.despawn_threshold + profile.hysteresis_margin).min(1.0)
        } else {
            profile.despawn_threshold
        };
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
}

pub(super) fn compute_move_in_acceptance(
    config: &DemandConfig,
    snapshot: &DailyDemandSnapshot,
) -> MoveInAcceptance {
    compute_move_in_acceptance_for(config, snapshot, false)
}

pub(super) fn compute_construction_move_in_acceptance(
    config: &DemandConfig,
    snapshot: &DailyDemandSnapshot,
) -> MoveInAcceptance {
    compute_move_in_acceptance_for(config, snapshot, false)
}

pub(super) fn compute_move_in_acceptance_for(
    config: &DemandConfig,
    snapshot: &DailyDemandSnapshot,
    require_vacant_household_slot: bool,
) -> MoveInAcceptance {
    if (require_vacant_household_slot && snapshot.vacant_household_slots == 0)
        || snapshot.candidate_household_size <= EPSILON
    {
        return MoveInAcceptance::default();
    }

    let candidate_household_size = snapshot.candidate_household_size.max(1.0);
    let candidate_effective_workers =
        expected_adult_members_for_household_size(candidate_household_size).max(EPSILON);
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
