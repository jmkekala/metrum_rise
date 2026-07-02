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
#[cfg(test)]
mod tests;
mod tick;

pub(crate) use data::DailyPowerSettlementSummary;
pub use data::{Household, HouseholdSystem};
pub use replenishment::{
    REPLENISHMENT_COOLDOWN, REPLENISHMENT_FAILED_TERMINAL, REPLENISHMENT_FULFILLED,
    REPLENISHMENT_NEEDS, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
    REPLENISHMENT_STABLE, REPLENISHMENT_WAITING_FOR_SHOPPER,
};

pub(crate) use metrics::{
    active_worker_capacity_for_profile_with_floor_scale, building_inventory_fill_ratio,
    building_operating_buffer_days, building_operation_factors, building_staffing_ratio,
    building_total_output_inventory, candidate_immigrant_household_size_from_flat_size,
    commercial_activity_signal_for_city, expected_adult_members_for_household_size,
    household_reserve_days, industrial_input_coverage_factor, industrial_output_headroom_factor,
    level_tuning_value, scaled_input_inventory_targets_for_building,
    service_funded_worker_capacity,
};
