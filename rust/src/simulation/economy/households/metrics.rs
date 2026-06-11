//! Shared household economy metrics and profile helpers.

use super::Household;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::MAX_ADULTS_PER_HOUSEHOLD;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
};
use crate::simulation::zoning::ZoneType;

const HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
const HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
const STARTER_IMMIGRANT_HOUSEHOLD_SIZE: u16 = 2;

pub(super) const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;

pub(super) fn stock_days(stock: f32, member_count: u16, consumption_rate: f32) -> f32 {
    let daily_consumption = member_count as f32 * consumption_rate;
    if daily_consumption <= 0.0 {
        0.0
    } else {
        stock / daily_consumption
    }
}

pub(super) fn economy_profile_for_building<'a>(
    catalog: &'a RuntimeEconomyCatalog,
    building: &Building,
) -> Option<&'a EconomyProfileRuntime> {
    if building.economy_broken || building.economy_profile_runtime_id == 0 {
        return None;
    }
    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
}

pub(super) fn household_demand_profile(catalog: &RuntimeEconomyCatalog) -> &EconomyProfileRuntime {
    catalog
        .profile_for_id(HOUSEHOLD_DEMAND_PROFILE_ID)
        .unwrap_or_else(|| {
            panic!(
                "runtime economy catalog missing required profile '{}'",
                HOUSEHOLD_DEMAND_PROFILE_ID
            )
        })
}

pub(super) fn household_supply_resource_runtime_id(
    catalog: &RuntimeEconomyCatalog,
) -> ResourceRuntimeId {
    catalog
        .resource_runtime_id_for_id(HOUSEHOLD_SUPPLY_RESOURCE_ID)
        .unwrap_or_else(|| {
            panic!(
                "runtime economy catalog missing required resource '{}'",
                HOUSEHOLD_SUPPLY_RESOURCE_ID
            )
        })
}

pub(super) fn household_supply_unit_price(catalog: &RuntimeEconomyCatalog) -> f32 {
    let resource_runtime_id = household_supply_resource_runtime_id(catalog);
    catalog
        .unit_price_for_resource(resource_runtime_id)
        .unwrap_or_else(|| {
            panic!(
                "runtime economy catalog missing unit price for '{}'",
                HOUSEHOLD_SUPPLY_RESOURCE_ID
            )
        })
}

pub(crate) fn building_total_output_inventory(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    profile
        .outputs
        .iter()
        .map(|port| building.inventory_units(port.resource_runtime_id))
        .sum()
}

pub(crate) fn household_reserve_days(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    household: &Household,
) -> f32 {
    let members = household.member_count.max(1) as f32;
    let daily_supply_cost =
        members * household.consumption_rate.max(0.0) * household_supply_unit_price(catalog);
    let daily_utility_cost = members * tuning.households.utility_cost_per_member_per_day;
    let daily_essential_cost = daily_supply_cost + daily_utility_cost;
    if daily_essential_cost <= 0.0 {
        0.0
    } else {
        (household.budget.max(0.0) / daily_essential_cost).max(0.0)
    }
}

pub(super) fn household_is_housed(household: &Household, allocator: &BuildingAllocator) -> bool {
    household_has_independent_member(household)
        && household.home_building_id < allocator.buildings.len()
        && !allocator.buildings[household.home_building_id].broken
        && !allocator.buildings[household.home_building_id].economy_broken
        && !allocator.buildings[household.home_building_id].is_deserted
        && allocator.buildings[household.home_building_id].is_operational()
}

pub(super) fn household_has_independent_member(household: &Household) -> bool {
    household.adult_count > 0 || household.elder_count > 0
}

/// Returns the deterministic expected adult worker count for a candidate immigrant household size.
pub(crate) fn expected_adult_members_for_household_size(household_size: f32) -> f32 {
    if household_size <= 1.0 {
        0.8
    } else {
        (1.0 + (household_size - 1.0).clamp(0.0, 1.0) * 0.55)
            .min(MAX_ADULTS_PER_HOUSEHOLD as f32)
            .min(household_size)
    }
}

/// Returns the deterministic starter immigrant household size that fits one residential flat.
///
/// Residential capacity is a household slot count, not a requirement that a new household fills
/// the whole authored home. Starter admissions therefore use a small baseline household capped by
/// the home's flat-size capacity.
pub(crate) fn candidate_immigrant_household_size_from_flat_size(flat_size_m2: f32) -> Option<u16> {
    if flat_size_m2 <= 1.0 {
        return None;
    }
    let flat_capacity = ((flat_size_m2 / 40.0).ceil() as u16).max(1);
    Some(flat_capacity.min(STARTER_IMMIGRANT_HOUSEHOLD_SIZE))
}

pub(crate) fn level_tuning_value(values: &[f32], level: u8) -> f32 {
    let index = level.saturating_sub(1) as usize;
    values
        .get(index)
        .copied()
        .or_else(|| values.last().copied())
        .unwrap_or(0.0)
}

fn owa_utility_cost_for_zone(tuning: &RuntimeEconomyTuning, zone_type: ZoneType) -> f32 {
    match zone_type {
        ZoneType::Commercial => tuning.commercial_owa_utility_cost_per_day,
        ZoneType::Industrial => tuning.industrial_owa_utility_cost_per_day,
        _ => 0.0,
    }
}

pub(crate) fn building_operating_buffer_days(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    let daily_operating_cost = building.worker_count as f32 * profile.average_daily_wage()
        + owa_utility_cost_for_zone(tuning, building.zone_type);
    if daily_operating_cost <= 0.0 {
        0.0
    } else {
        (building.operating_budget.max(0.0) / daily_operating_cost).max(0.0)
    }
}

pub(crate) fn building_staffing_ratio(catalog: &RuntimeEconomyCatalog, building: &Building) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    let worker_capacity = profile.worker_capacity;
    if worker_capacity == 0 {
        0.0
    } else {
        (building.worker_count as f32 / worker_capacity as f32).clamp(0.0, 1.0)
    }
}

pub(crate) fn industrial_input_coverage_factor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    if profile.inputs.is_empty() {
        1.0
    } else {
        profile
            .inputs
            .iter()
            .map(|port| {
                if port.units_per_day <= 0.0 {
                    1.0
                } else {
                    (building.inventory_units(port.resource_runtime_id) / port.units_per_day)
                        .clamp(0.0, 1.0)
                }
            })
            .fold(1.0, f32::min)
    }
}

pub(crate) fn industrial_output_headroom_factor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    if profile.outputs.is_empty() {
        1.0
    } else {
        profile
            .outputs
            .iter()
            .map(|port| {
                let output_capacity_units = profile.output_buffer_capacity_units_for(port);
                if !output_capacity_units.is_finite() || output_capacity_units <= 0.0 {
                    1.0
                } else {
                    let remaining_headroom = (output_capacity_units
                        - building.inventory_units(port.resource_runtime_id))
                    .max(0.0);
                    (remaining_headroom / output_capacity_units).clamp(0.0, 1.0)
                }
            })
            .fold(1.0, f32::min)
    }
}
