//! Shared household economy metrics and profile helpers.

use super::Household;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::MAX_ADULTS_PER_HOUSEHOLD;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning, RuntimeResourcePort,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
const HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
const STARTER_IMMIGRANT_HOUSEHOLD_SIZE: u16 = 2;
const MIN_POSITIVE_VALUE: f32 = 0.000_1;

pub(super) const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommercialActivitySignal {
    pub(crate) household_supply_resource_runtime_id: ResourceRuntimeId,
    pub(crate) demand_units_per_day: f32,
    pub(crate) activity_floor_scale: f32,
}

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

pub(crate) fn profile_output_value_per_day(
    catalog: &RuntimeEconomyCatalog,
    profile: &EconomyProfileRuntime,
) -> f32 {
    profile
        .outputs
        .iter()
        .map(|port| {
            catalog
                .unit_price_for_resource(port.resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!(
                        "resource '{resource_id}' used by profile '{}' has no catalog price",
                        profile.id
                    )
                })
                * port.units_per_day.max(0.0)
        })
        .sum()
}

fn is_sales_scaled_commercial_store(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(building.zone_type, ZoneType::Commercial)
        && matches!(profile.kind, EconomyProfileRuntimeKind::Store)
}

fn commercial_household_supply_output_units_per_day(
    resource_runtime_id: ResourceRuntimeId,
    profile: &EconomyProfileRuntime,
) -> f32 {
    profile
        .outputs
        .iter()
        .filter(|port| port.resource_runtime_id == resource_runtime_id)
        .map(|port| port.units_per_day.max(0.0))
        .sum()
}

pub(crate) fn commercial_activity_signal_for_city(
    catalog: &RuntimeEconomyCatalog,
    households: &[Household],
    allocator: &BuildingAllocator,
) -> CommercialActivitySignal {
    let household_profile = household_demand_profile(catalog);
    let household_supply_resource = household_supply_resource_runtime_id(catalog);
    let recovery_days = household_profile.stock_target_days.max(1.0);
    let (daily_consumption, stock_gap) = households
        .par_iter()
        .filter(|household| household_is_housed(household, allocator))
        .map(|household| {
            let daily_consumption =
                household.member_count as f32 * household.consumption_rate.max(0.0);
            let target_stock = daily_consumption * household_profile.stock_target_days.max(0.0);
            let stock_gap = (target_stock - household.stock.max(0.0)).max(0.0);
            (daily_consumption, stock_gap)
        })
        .reduce(
            || (0.0, 0.0),
            |left, right| (left.0 + right.0, left.1 + right.1),
        );
    let demand_units_per_day = daily_consumption + stock_gap / recovery_days;

    let live_output_units_per_day: f32 = allocator
        .buildings
        .par_iter()
        .filter_map(|building| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                return None;
            }
            let profile = economy_profile_for_building(catalog, building)?;
            if !is_sales_scaled_commercial_store(building, profile) {
                return None;
            }
            Some(commercial_household_supply_output_units_per_day(
                household_supply_resource,
                profile,
            ))
        })
        .sum();
    let activity_floor_scale = if live_output_units_per_day <= MIN_POSITIVE_VALUE {
        0.0
    } else {
        (demand_units_per_day / live_output_units_per_day).clamp(0.0, 1.0)
    };

    CommercialActivitySignal {
        household_supply_resource_runtime_id: household_supply_resource,
        demand_units_per_day,
        activity_floor_scale,
    }
}

pub(crate) fn refresh_commercial_activity_floor(
    catalog: &RuntimeEconomyCatalog,
    households: &[Household],
    allocator: &mut BuildingAllocator,
) -> CommercialActivitySignal {
    let signal = commercial_activity_signal_for_city(catalog, households, allocator);
    allocator.buildings.par_iter_mut().for_each(|building| {
        building.commercial_activity_floor_scale = 0.0;
        if building.broken
            || building.economy_broken
            || building.is_deserted
            || building.is_under_construction()
        {
            return;
        }
        let Some(profile) = economy_profile_for_building(catalog, building) else {
            return;
        };
        if is_sales_scaled_commercial_store(building, profile) {
            building.commercial_activity_floor_scale = signal.activity_floor_scale;
        }
    });
    signal
}

fn commercial_sales_activity_scale(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> f32 {
    if !is_sales_scaled_commercial_store(building, profile) {
        return 1.0;
    }
    let full_output_value = profile_output_value_per_day(catalog, profile);
    if full_output_value <= MIN_POSITIVE_VALUE {
        return 1.0;
    }
    (building
        .daily_household_sales_value
        .max(building.recent_household_sales_value)
        .max(0.0)
        / full_output_value)
        .clamp(0.0, 1.0)
}

fn commercial_activity_scale_with_floor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
) -> f32 {
    if !is_sales_scaled_commercial_store(building, profile) {
        return 1.0;
    }
    commercial_sales_activity_scale(catalog, building, profile)
        .max(floor_scale.max(0.0))
        .clamp(0.0, 1.0)
}

pub(crate) fn active_worker_capacity_for_profile(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> u32 {
    active_worker_capacity_for_profile_with_floor_scale(
        catalog,
        building,
        profile,
        building.commercial_activity_floor_scale,
    )
}

pub(crate) fn active_worker_capacity_for_profile_with_floor_scale(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
) -> u32 {
    let worker_capacity = profile.worker_capacity;
    if worker_capacity == 0 {
        return 0;
    }
    if !is_sales_scaled_commercial_store(building, profile) {
        return worker_capacity;
    }
    let scaled_capacity = (worker_capacity as f32
        * commercial_activity_scale_with_floor(catalog, building, profile, floor_scale))
    .ceil() as u32;
    scaled_capacity.clamp(1, worker_capacity)
}

/// Cheap live production factors for one building/profile pair.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BuildingOperationFactors {
    /// Demand-responsive worker slots the building is currently willing to staff.
    pub(crate) active_worker_capacity: u32,
    /// Workers that can contribute this hour after active-capacity throttling.
    pub(crate) effective_workers: u32,
    /// Input-stock contribution to the current hourly throughput.
    pub(crate) input_factor: f32,
    /// Output-buffer contribution to the current hourly throughput.
    pub(crate) output_headroom_factor: f32,
    /// Final current-hour throughput ratio against authored full capacity.
    pub(crate) throughput_factor: f32,
}

/// Returns the same production factors used by the hourly building economy tick.
pub(crate) fn building_operation_factors(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> BuildingOperationFactors {
    let authored_worker_capacity = profile.worker_capacity.max(1);
    let active_worker_capacity = active_worker_capacity_for_profile(catalog, building, profile);
    let effective_workers = building.worker_count.min(active_worker_capacity);
    let staffing_factor =
        (effective_workers as f32 / authored_worker_capacity as f32).clamp(0.0, 1.0);
    let input_factor = hourly_input_availability_factor(profile, building, staffing_factor);
    let output_headroom_factor =
        hourly_output_headroom_factor(profile, building, staffing_factor * input_factor);
    let throughput_factor = staffing_factor * input_factor * output_headroom_factor;
    BuildingOperationFactors {
        active_worker_capacity,
        effective_workers,
        input_factor,
        output_headroom_factor,
        throughput_factor,
    }
}

/// Returns a weighted fill ratio across the profile's tracked input and output buffers.
pub(crate) fn building_inventory_fill_ratio(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> Option<f32> {
    let mut inventory_units = 0.0f32;
    let mut capacity_units = 0.0f32;

    for input_port in &profile.inputs {
        let (target_units, _, _) =
            scaled_input_inventory_targets_for_building(catalog, building, profile, input_port);
        if target_units <= MIN_POSITIVE_VALUE {
            continue;
        }
        inventory_units += building
            .inventory_units(input_port.resource_runtime_id)
            .min(target_units);
        capacity_units += target_units;
    }

    for output_port in &profile.outputs {
        let capacity = profile.output_buffer_capacity_units_for(output_port);
        if !capacity.is_finite() || capacity <= MIN_POSITIVE_VALUE {
            continue;
        }
        inventory_units += building
            .inventory_units(output_port.resource_runtime_id)
            .min(capacity);
        capacity_units += capacity;
    }

    if capacity_units <= MIN_POSITIVE_VALUE {
        None
    } else {
        Some((inventory_units / capacity_units).clamp(0.0, 1.0))
    }
}

fn hourly_input_availability_factor(
    profile: &EconomyProfileRuntime,
    building: &Building,
    base_throughput_factor: f32,
) -> f32 {
    if base_throughput_factor <= 0.0 || profile.inputs.is_empty() {
        return 1.0;
    }
    profile
        .inputs
        .iter()
        .map(|port| {
            let hourly_required =
                port.units_per_day.max(0.0) / OPERATIONAL_HOURS_PER_DAY * base_throughput_factor;
            if hourly_required <= 0.0 {
                1.0
            } else {
                (building.inventory_units(port.resource_runtime_id) / hourly_required)
                    .clamp(0.0, 1.0)
            }
        })
        .fold(1.0, f32::min)
}

fn hourly_output_headroom_factor(
    profile: &EconomyProfileRuntime,
    building: &Building,
    base_throughput_factor: f32,
) -> f32 {
    if base_throughput_factor <= 0.0 || profile.outputs.is_empty() {
        return 1.0;
    }
    profile
        .outputs
        .iter()
        .map(|port| {
            let hourly_output =
                port.units_per_day.max(0.0) / OPERATIONAL_HOURS_PER_DAY * base_throughput_factor;
            if hourly_output <= 0.0 {
                return 1.0;
            }
            let output_capacity_units = profile.output_buffer_capacity_units_for(port);
            if !output_capacity_units.is_finite() || output_capacity_units <= 0.0 {
                return 1.0;
            }
            let remaining_headroom = (output_capacity_units
                - building.inventory_units(port.resource_runtime_id))
            .max(0.0);
            (remaining_headroom / hourly_output).clamp(0.0, 1.0)
        })
        .fold(1.0, f32::min)
}

pub(crate) fn scaled_input_inventory_targets_for_building(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    input_port: &RuntimeResourcePort,
) -> (f32, f32, f32) {
    let base_target = profile.inventory_target_units_for(input_port);
    let base_reorder = profile.inventory_reorder_units_for(input_port);
    let base_critical = profile.inventory_critical_units_for(input_port);
    if base_target <= 0.0 || !is_sales_scaled_commercial_store(building, profile) {
        return (base_target, base_reorder, base_critical);
    }

    let worker_capacity = profile.worker_capacity.max(1) as f32;
    let active_capacity = active_worker_capacity_for_profile(catalog, building, profile) as f32;
    let activity_scale = (active_capacity / worker_capacity).clamp(0.0, 1.0);
    let min_target = profile.min_shipment_units.min(base_target).max(0.0);
    let target_units = (base_target * activity_scale).clamp(min_target, base_target);
    let reorder_units = if base_reorder <= 0.0 {
        0.0
    } else {
        (base_reorder * activity_scale).clamp(min_target.min(target_units), target_units)
    };
    let critical_units = (base_critical * activity_scale).clamp(0.0, target_units);
    (target_units, reorder_units, critical_units)
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
