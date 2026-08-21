//! Household storage, settlement, and shared household economy helpers.

mod admission;
mod benefits;
mod building_economy;
mod data;
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
pub use replenishment::{
    REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
    REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
    REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER,
};

pub(crate) use metrics::{
    active_worker_capacity_equivalent_for_profile_with_floor_scale,
    active_worker_capacity_for_profile_with_floor_scale, building_inventory_fill_ratio,
    building_operating_buffer_days, building_operation_factors,
    building_operation_factors_with_floor_scale, building_staffing_ratio,
    building_total_output_inventory, candidate_immigrant_household_size_for_vacancy,
    candidate_immigrant_household_size_from_flat_size, commercial_activity_signal_for_city,
    household_reserve_days, industrial_input_coverage_factor, industrial_output_headroom_factor,
    level_tuning_value, scaled_input_inventory_targets_for_building,
    scaled_output_buffer_capacity_units_for_building, scaled_output_units_per_day_for_building,
    service_funded_worker_capacity,
};
