// SPDX-License-Identifier: GPL-2.0-only

//! Household storage, settlement, and shared household economy helpers.

mod admission;
mod benefits;
mod building_economy;
mod data;
mod electricity;
mod employment;
mod membership;
mod metrics;
mod relocation;
mod removal;
mod replenishment;
mod service_visits;
#[cfg(test)]
mod tests;
mod tick;

pub(crate) use data::{DailyPowerSettlementSummary, HouseholdBuildingUndo};
pub use data::{Household, HouseholdSystem};
pub(crate) use removal::HouseholdExitPolicy;
pub(crate) use replenishment::stable_replenishment_offset_hours;
pub use replenishment::{
    REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
    REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
    REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER,
};

#[cfg(test)]
pub(crate) use metrics::{applicant_workers, candidate_immigrant_household_size_from_flat_size};

pub(crate) use metrics::{
    OPERATIONAL_HOURS_PER_DAY, active_worker_capacity_for_profile_with_floor_scale,
    affordable_consumer_residents, consumer_basket_cost, visit_consumer_budgets,
    building_inventory_fill_ratio, building_operation_factors,
    building_staffing_ratio,
    applicant_offer, applicant_offer_limit, applicant_places_and_members,
    applicant_places_by_adults,
    building_total_output_inventory, candidate_immigrant_household_size_for_vacancy,
    commercial_activity_signal_for_city, consume_hourly_production_inputs,
    household_daily_utility_allowance, household_is_housed, household_reserve_days,
    industrial_input_coverage_factor, industrial_output_headroom_factor, level_tuning_value,
    local_utility_service_costs, funded_input_demand_scale, output_area_scale_for_building,
    physical_worker_capacity_for_profile, saleable_output_stock,
    scaled_input_inventory_targets_for_building, scaled_output_buffer_capacity_units_for_building,
    scaled_output_units_per_day_for_building, service_funded_worker_capacity,
};
