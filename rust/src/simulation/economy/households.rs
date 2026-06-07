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

pub use data::{Household, HouseholdSystem};
pub use replenishment::{
    REPLENISHMENT_COOLDOWN, REPLENISHMENT_FULFILLED, REPLENISHMENT_NEEDS,
    REPLENISHMENT_PICKUP_PENDING, REPLENISHMENT_RESERVED, REPLENISHMENT_STABLE,
};

pub(crate) use metrics::{
    building_operating_buffer_days, building_staffing_ratio, building_total_output_inventory,
    household_reserve_days, industrial_input_coverage_factor, industrial_output_headroom_factor,
    level_tuning_value,
};
