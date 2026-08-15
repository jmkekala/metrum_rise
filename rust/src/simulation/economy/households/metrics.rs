//! Shared household economy metrics and profile helpers.

use super::Household;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::MAX_ADULTS_PER_HOUSEHOLD;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    RuntimeEconomyTuning, RuntimeResourcePort, load_runtime_economy_tuning,
};
use crate::simulation::work_area::{
    profile_kind_uses_explicit_work_area, sanitize_work_area_scale,
    scaled_work_area_worker_capacity,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const HOUSEHOLD_DEMAND_PROFILE_ID: &str = "basic_household_demand";
const HOUSEHOLD_SUPPLY_RESOURCE_ID: &str = "household_supplies";
const MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE: u16 = 6;
const STARTER_IMMIGRANT_HOUSEHOLD_SIZE_BUCKETS: [u16; 16] =
    [1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 3, 3, 4, 5, 6];
const HOUSEHOLD_BASE_AREA_M2: f32 = 25.0;
const HOUSEHOLD_ADULT_AREA_M2: f32 = 22.0;
const HOUSEHOLD_CHILD_AREA_M2: f32 = 12.0;
const MIN_POSITIVE_VALUE: f32 = 0.000_1;
const EXPLICIT_WORK_AREA_OWA_MARKET_LOADS_PER_DAY: f32 = 1.0;
pub(crate) const UTILITY_SERVICE_POWER: &str = "power";
const UTILITY_SERVICE_WATER: &str = "water";
const UTILITY_SERVICE_SEWAGE: &str = "sewage";
const UTILITY_SERVICES: [&str; 3] = [
    UTILITY_SERVICE_POWER,
    UTILITY_SERVICE_WATER,
    UTILITY_SERVICE_SEWAGE,
];

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

pub(super) fn demand_sink_cash_cost_per_resident_excluding_resource(
    catalog: &RuntimeEconomyCatalog,
    excluded_resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    catalog
        .all_profiles()
        .iter()
        .filter(|profile| profile.kind == EconomyProfileRuntimeKind::DemandSink)
        .flat_map(|profile| {
            profile.inputs.iter().filter_map(move |input| {
                if input.resource_runtime_id == excluded_resource_runtime_id {
                    return None;
                }
                let resource_price = catalog
                    .unit_price_for_resource(input.resource_runtime_id)
                    .unwrap_or_else(|| {
                        let resource_id = catalog
                            .resource_id_for_runtime_id(input.resource_runtime_id)
                            .unwrap_or("<unknown>");
                        panic!(
                            "resource '{resource_id}' used by profile '{}' has no catalog price",
                            profile.id
                        )
                    });
                Some(profile.consumption_rate_per_resident.max(0.0) * resource_price.max(0.0))
            })
        })
        .sum()
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

fn is_service_scaled_commercial_store(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> bool {
    matches!(building.zone_type, ZoneType::Commercial)
        && matches!(profile.kind, EconomyProfileRuntimeKind::ServiceStore)
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

fn add_resource_amount(
    amounts_by_resource: &mut Vec<(ResourceRuntimeId, f32)>,
    resource_runtime_id: ResourceRuntimeId,
    amount: f32,
) {
    if amount <= 0.0 {
        return;
    }
    if let Some((_, total)) = amounts_by_resource
        .iter_mut()
        .find(|(existing_resource, _)| *existing_resource == resource_runtime_id)
    {
        *total += amount;
    } else {
        amounts_by_resource.push((resource_runtime_id, amount));
    }
}

fn resource_amount(
    amounts_by_resource: &[(ResourceRuntimeId, f32)],
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    amounts_by_resource
        .iter()
        .find(|(existing_resource, _)| *existing_resource == resource_runtime_id)
        .map(|(_, amount)| *amount)
        .unwrap_or(0.0)
}

fn service_store_demand_rates_by_resource(
    catalog: &RuntimeEconomyCatalog,
) -> Vec<(ResourceRuntimeId, f32)> {
    let mut demand_rates = Vec::new();
    let mut service_resources = Vec::new();
    for profile in catalog.all_profiles() {
        if profile.kind != EconomyProfileRuntimeKind::ServiceStore {
            continue;
        }
        for output in &profile.outputs {
            add_resource_amount(&mut service_resources, output.resource_runtime_id, 1.0);
        }
    }

    for profile in catalog.all_profiles() {
        if profile.kind != EconomyProfileRuntimeKind::DemandSink {
            continue;
        }
        for input in &profile.inputs {
            if resource_amount(&service_resources, input.resource_runtime_id) <= 0.0 {
                continue;
            }
            add_resource_amount(
                &mut demand_rates,
                input.resource_runtime_id,
                profile.consumption_rate_per_resident.max(0.0),
            );
        }
    }
    demand_rates
}

fn service_store_live_output_units_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
) -> Vec<(ResourceRuntimeId, f32)> {
    allocator
        .buildings
        .par_iter()
        .filter_map(|building| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                return None;
            }
            let profile = economy_profile_for_building(catalog, building)?;
            if !is_service_scaled_commercial_store(building, profile) {
                return None;
            }
            Some(profile.outputs.as_slice())
        })
        .fold(Vec::new, |mut local, outputs| {
            for output in outputs {
                add_resource_amount(
                    &mut local,
                    output.resource_runtime_id,
                    output.units_per_day.max(0.0),
                );
            }
            local
        })
        .reduce(Vec::new, |mut left, right| {
            for (resource_runtime_id, amount) in right {
                add_resource_amount(&mut left, resource_runtime_id, amount);
            }
            left
        })
}

fn housed_resident_count(households: &[Household], allocator: &BuildingAllocator) -> u32 {
    households
        .par_iter()
        .filter(|household| household_is_housed(household, allocator))
        .map(|household| u32::from(household.member_count))
        .sum()
}

fn service_store_activity_scale_by_resource(
    catalog: &RuntimeEconomyCatalog,
    households: &[Household],
    allocator: &BuildingAllocator,
) -> Vec<(ResourceRuntimeId, f32)> {
    let housed_residents = housed_resident_count(households, allocator) as f32;
    if housed_residents <= 0.0 {
        return Vec::new();
    }

    let demand_rates = service_store_demand_rates_by_resource(catalog);
    if demand_rates.is_empty() {
        return Vec::new();
    }

    let live_output_units = service_store_live_output_units_by_resource(catalog, allocator);
    let mut scales = Vec::with_capacity(demand_rates.len());
    for (resource_runtime_id, demand_rate_per_resident) in demand_rates {
        let output_units_per_day = resource_amount(&live_output_units, resource_runtime_id);
        if output_units_per_day <= MIN_POSITIVE_VALUE {
            continue;
        }
        let demand_units_per_day = housed_residents * demand_rate_per_resident.max(0.0);
        add_resource_amount(
            &mut scales,
            resource_runtime_id,
            (demand_units_per_day / output_units_per_day).clamp(0.0, 1.0),
        );
    }
    scales
}

fn service_store_profile_activity_scale(
    profile: &EconomyProfileRuntime,
    activity_scale_by_resource: &[(ResourceRuntimeId, f32)],
) -> f32 {
    profile
        .outputs
        .iter()
        .map(|output| resource_amount(activity_scale_by_resource, output.resource_runtime_id))
        .fold(0.0, f32::max)
        .clamp(0.0, 1.0)
}

fn physical_worker_capacity_for_profile(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> u32 {
    if profile_kind_uses_explicit_work_area(profile.kind) {
        scaled_work_area_worker_capacity(profile.worker_capacity, building.work_area_scale)
    } else {
        profile.worker_capacity
    }
}

fn buyer_input_activity_scale(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> f32 {
    let physical_capacity = physical_worker_capacity_for_profile(building, profile);
    if physical_capacity == 0 {
        return 0.0;
    }
    let active_capacity = if profile_kind_uses_explicit_work_area(profile.kind) {
        physical_capacity
    } else {
        active_worker_capacity_for_profile(catalog, building, profile)
    };
    if active_capacity == 0 {
        return 0.0;
    }
    let average_daily_wage = profile.average_daily_wage();
    let city_funded = allocator.is_city_service_building(building);
    let budget_capacity = if city_funded {
        active_capacity
    } else if average_daily_wage > 0.1 {
        (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32
    } else {
        active_capacity
    };
    let budget_backed_capacity = active_capacity.min(budget_capacity);
    (budget_backed_capacity as f32 / physical_capacity.max(1) as f32).clamp(0.0, 1.0)
}

fn local_input_demand_units_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
) -> Vec<(ResourceRuntimeId, f32)> {
    let mut demand_units = Vec::new();
    for building in &allocator.buildings {
        if building.broken
            || building.economy_broken
            || building.is_deserted
            || building.is_under_construction()
            || building.edge_idx == usize::MAX
        {
            continue;
        }
        let Some(profile) = economy_profile_for_building(catalog, building) else {
            continue;
        };
        if profile.inputs.is_empty() {
            continue;
        }
        let activity_scale = buyer_input_activity_scale(catalog, allocator, building, profile);
        if activity_scale <= MIN_POSITIVE_VALUE {
            continue;
        }
        for input in &profile.inputs {
            add_resource_amount(
                &mut demand_units,
                input.resource_runtime_id,
                input.units_per_day.max(0.0) * activity_scale,
            );
        }
    }
    demand_units
}

fn explicit_work_area_full_output_units_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
) -> Vec<(ResourceRuntimeId, f32)> {
    allocator
        .buildings
        .par_iter()
        .filter_map(|building| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                return None;
            }
            let profile = economy_profile_for_building(catalog, building)?;
            if !profile_kind_uses_explicit_work_area(profile.kind) || profile.outputs.is_empty() {
                return None;
            }
            Some((building, profile))
        })
        .fold(Vec::new, |mut local, (building, profile)| {
            for output in &profile.outputs {
                add_resource_amount(
                    &mut local,
                    output.resource_runtime_id,
                    scaled_output_units_per_day_for_building(building, profile, output),
                );
            }
            local
        })
        .reduce(Vec::new, |mut left, right| {
            for (resource_runtime_id, amount) in right {
                add_resource_amount(&mut left, resource_runtime_id, amount);
            }
            left
        })
}

fn explicit_work_area_activity_scale_by_resource(
    catalog: &RuntimeEconomyCatalog,
    allocator: &BuildingAllocator,
    tuning: &RuntimeEconomyTuning,
) -> Vec<(ResourceRuntimeId, f32)> {
    let full_output_units = explicit_work_area_full_output_units_by_resource(catalog, allocator);
    if full_output_units.is_empty() {
        return Vec::new();
    }
    let local_input_demand = local_input_demand_units_by_resource(catalog, allocator);
    let owa_market_units =
        tuning.logistics.truck_load_units.max(0.0) * EXPLICIT_WORK_AREA_OWA_MARKET_LOADS_PER_DAY;
    let mut scales = Vec::with_capacity(full_output_units.len());
    for (resource_runtime_id, output_units_per_day) in full_output_units {
        if output_units_per_day <= MIN_POSITIVE_VALUE {
            continue;
        }
        let market_units =
            resource_amount(&local_input_demand, resource_runtime_id) + owa_market_units;
        scales.push((
            resource_runtime_id,
            (market_units / output_units_per_day).clamp(0.0, 1.0),
        ));
    }
    scales
}

fn explicit_work_area_profile_activity_scale(
    profile: &EconomyProfileRuntime,
    activity_scale_by_resource: &[(ResourceRuntimeId, f32)],
) -> f32 {
    profile
        .outputs
        .iter()
        .map(|output| resource_amount(activity_scale_by_resource, output.resource_runtime_id))
        .fold(0.0, f32::max)
        .clamp(0.0, 1.0)
}

pub(crate) fn refresh_commercial_activity_floor(
    catalog: &RuntimeEconomyCatalog,
    households: &[Household],
    allocator: &mut BuildingAllocator,
) -> CommercialActivitySignal {
    let tuning = load_runtime_economy_tuning()
        .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
    let signal = commercial_activity_signal_for_city(catalog, households, allocator);
    let service_activity_scale_by_resource =
        service_store_activity_scale_by_resource(catalog, households, allocator);
    let explicit_activity_scale_by_resource =
        explicit_work_area_activity_scale_by_resource(catalog, allocator, tuning.as_ref());
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
        } else if is_service_scaled_commercial_store(building, profile) {
            building.commercial_activity_floor_scale =
                service_store_profile_activity_scale(profile, &service_activity_scale_by_resource);
        } else if profile_kind_uses_explicit_work_area(profile.kind) {
            building.commercial_activity_floor_scale = explicit_work_area_profile_activity_scale(
                profile,
                &explicit_activity_scale_by_resource,
            );
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
    if profile_kind_uses_explicit_work_area(profile.kind) {
        let physical_capacity =
            scaled_work_area_worker_capacity(worker_capacity, building.work_area_scale);
        let activity_scale = floor_scale.clamp(0.0, 1.0);
        if physical_capacity == 0 || activity_scale <= MIN_POSITIVE_VALUE {
            return 0;
        }
        return ((physical_capacity as f32) * activity_scale)
            .ceil()
            .clamp(1.0, physical_capacity as f32) as u32;
    }
    if is_service_scaled_commercial_store(building, profile) {
        let activity_scale = floor_scale.clamp(0.0, 1.0);
        if activity_scale <= MIN_POSITIVE_VALUE {
            return 0;
        }
        return ((worker_capacity as f32) * activity_scale)
            .ceil()
            .clamp(1.0, worker_capacity as f32) as u32;
    }
    if !is_sales_scaled_commercial_store(building, profile) {
        return worker_capacity;
    }
    let scaled_capacity = (worker_capacity as f32
        * commercial_activity_scale_with_floor(catalog, building, profile, floor_scale))
    .ceil() as u32;
    scaled_capacity.clamp(1, worker_capacity)
}

/// Returns demand-responsive worker capacity before integer staffing slots are rounded.
pub(crate) fn active_worker_capacity_equivalent_for_profile_with_floor_scale(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
) -> f32 {
    let worker_capacity = profile.worker_capacity;
    if worker_capacity == 0 {
        return 0.0;
    }
    if profile_kind_uses_explicit_work_area(profile.kind) {
        return worker_capacity as f32
            * sanitize_work_area_scale(building.work_area_scale)
            * floor_scale.clamp(0.0, 1.0);
    }
    if is_service_scaled_commercial_store(building, profile) {
        return worker_capacity as f32 * floor_scale.clamp(0.0, 1.0);
    }
    if !is_sales_scaled_commercial_store(building, profile) {
        return worker_capacity as f32;
    }
    worker_capacity as f32
        * commercial_activity_scale_with_floor(catalog, building, profile, floor_scale)
}

/// Returns one output port's daily rate after explicit work-area scaling.
pub(crate) fn scaled_output_units_per_day_for_building(
    building: &Building,
    profile: &EconomyProfileRuntime,
    output_port: &RuntimeResourcePort,
) -> f32 {
    output_port.units_per_day.max(0.0) * output_area_scale_for_building(building, profile)
}

/// Returns one output port's storage capacity after explicit work-area scaling.
pub(crate) fn scaled_output_buffer_capacity_units_for_building(
    building: &Building,
    profile: &EconomyProfileRuntime,
    output_port: &RuntimeResourcePort,
) -> f32 {
    let capacity = profile.output_buffer_capacity_units_for(output_port);
    if capacity.is_finite() {
        capacity * output_area_scale_for_building(building, profile)
    } else {
        capacity
    }
}

fn output_area_scale_for_building(building: &Building, profile: &EconomyProfileRuntime) -> f32 {
    if profile_kind_uses_explicit_work_area(profile.kind) {
        sanitize_work_area_scale(building.work_area_scale)
    } else {
        1.0
    }
}

fn scaled_service_worker_capacity(capacity: u32, funding_factor: f32) -> u32 {
    if capacity == 0 {
        return 0;
    }
    let funding_factor = funding_factor.clamp(0.0, 1.0);
    if funding_factor <= f32::EPSILON {
        0
    } else {
        ((capacity as f32) * funding_factor).ceil() as u32
    }
}

fn service_funding_factor_for_building(
    service_funding_by_building: &[f32],
    building_idx: usize,
) -> f32 {
    service_funding_by_building
        .get(building_idx)
        .copied()
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

pub(crate) fn service_funded_worker_capacity(
    capacity: u32,
    profile: &EconomyProfileRuntime,
    building_idx: usize,
    service_funding_by_building: &[f32],
) -> u32 {
    if profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER) {
        scaled_service_worker_capacity(
            capacity,
            service_funding_factor_for_building(service_funding_by_building, building_idx),
        )
    } else {
        capacity
    }
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
    building_operation_factors_with_floor_scale(
        catalog,
        building,
        profile,
        building.commercial_activity_floor_scale,
    )
}

/// Returns production factors using an explicit demand-responsive activity floor.
pub(crate) fn building_operation_factors_with_floor_scale(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
) -> BuildingOperationFactors {
    let active_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
        catalog,
        building,
        profile,
        floor_scale,
    );
    let effective_workers = building.worker_count.min(active_worker_capacity);
    let staffing_worker_capacity = physical_worker_capacity_for_profile(building, profile).max(1);
    let staffing_factor =
        (effective_workers as f32 / staffing_worker_capacity as f32).clamp(0.0, 1.0);
    let production_rate_scale = if profile_kind_uses_explicit_work_area(profile.kind) {
        sanitize_work_area_scale(building.work_area_scale)
    } else {
        1.0
    };
    let input_factor =
        hourly_input_availability_factor(profile, building, staffing_factor, production_rate_scale);
    let output_headroom_factor = hourly_output_headroom_factor(
        profile,
        building,
        staffing_factor * input_factor,
        production_rate_scale,
    );
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
        let capacity =
            scaled_output_buffer_capacity_units_for_building(building, profile, output_port);
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
    production_rate_scale: f32,
) -> f32 {
    if base_throughput_factor <= 0.0 || profile.inputs.is_empty() {
        return 1.0;
    }
    let production_rate_scale = production_rate_scale.max(0.0);
    profile
        .inputs
        .iter()
        .map(|port| {
            let hourly_required = port.units_per_day.max(0.0) * production_rate_scale
                / OPERATIONAL_HOURS_PER_DAY
                * base_throughput_factor;
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
    production_rate_scale: f32,
) -> f32 {
    if base_throughput_factor <= 0.0 || profile.outputs.is_empty() {
        return 1.0;
    }
    let production_rate_scale = production_rate_scale.max(0.0);
    profile
        .outputs
        .iter()
        .map(|port| {
            let hourly_output = port.units_per_day.max(0.0) * production_rate_scale
                / OPERATIONAL_HOURS_PER_DAY
                * base_throughput_factor;
            if hourly_output <= 0.0 {
                return 1.0;
            }
            let output_capacity_units =
                scaled_output_buffer_capacity_units_for_building(building, profile, port);
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
    let household_supply_resource = household_supply_resource_runtime_id(catalog);
    let daily_supply_cost =
        members * household.consumption_rate.max(0.0) * household_supply_unit_price(catalog);
    let daily_cash_demand_sink_cost = members
        * demand_sink_cash_cost_per_resident_excluding_resource(catalog, household_supply_resource);
    let daily_utility_cost = members * tuning.households.utility_cost_per_member_per_day;
    let daily_essential_cost = daily_supply_cost + daily_cash_demand_sink_cost + daily_utility_cost;
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

/// Returns the deterministic starter immigrant household size that fits one residential flat.
///
/// Residential capacity is a household slot count, not a requirement that a new household fills
/// the whole authored home. Starter admissions use a simple area model: each flat reserves a base
/// living area, a two-person household may fit as one adult plus one child-weighted member, and
/// larger households reserve two adult-equivalent members plus child-weighted extra members.
pub(crate) fn candidate_immigrant_household_size_from_flat_size(flat_size_m2: f32) -> Option<u16> {
    if flat_size_m2 <= 1.0 {
        return None;
    }
    let mut candidate_size = 1u16;
    for household_size in 2..=MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE {
        if starter_household_required_area_m2(household_size) > flat_size_m2 {
            break;
        }
        candidate_size = household_size;
    }
    Some(candidate_size)
}

/// Returns the deterministic starter immigrant household size for one claimable residential slot.
///
/// Flat size is treated as a maximum fit. The requested household comes from a small deterministic
/// starter mix so residential admission can create singles and couples instead of always filling a
/// home to its largest possible family size.
pub(crate) fn candidate_immigrant_household_size_for_vacancy(
    flat_size_m2: f32,
    home_building_id: usize,
    occupied_household_slots: u32,
) -> Option<u16> {
    let max_size = candidate_immigrant_household_size_from_flat_size(flat_size_m2)?;
    let seed = 0xD1B5_4A32_D192_ED03_u64
        .wrapping_add((home_building_id as u64).wrapping_mul(0xA24B_AED4_963E_E407))
        .wrapping_add(u64::from(occupied_household_slots).wrapping_mul(0x1656_67B1_9E37_79F9));
    let requested_size = STARTER_IMMIGRANT_HOUSEHOLD_SIZE_BUCKETS
        [stable_index(seed, STARTER_IMMIGRANT_HOUSEHOLD_SIZE_BUCKETS.len())];

    Some(requested_size.min(max_size).max(1))
}

fn starter_household_required_area_m2(household_size: u16) -> f32 {
    let size = household_size.max(1);
    let adult_equivalent_members = if size <= 2 {
        1.0
    } else {
        MAX_ADULTS_PER_HOUSEHOLD as f32
    };
    let child_equivalent_members = size as f32 - adult_equivalent_members;
    HOUSEHOLD_BASE_AREA_M2
        + adult_equivalent_members * HOUSEHOLD_ADULT_AREA_M2
        + child_equivalent_members * HOUSEHOLD_CHILD_AREA_M2
}

#[inline(always)]
fn stable_hash64(seed: u64) -> u64 {
    let mut x = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

#[inline(always)]
fn stable_index(seed: u64, len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    (stable_hash64(seed) as usize) % len
}

pub(crate) fn level_tuning_value(values: &[f32], level: u8) -> f32 {
    let index = level.saturating_sub(1) as usize;
    values
        .get(index)
        .copied()
        .or_else(|| values.last().copied())
        .unwrap_or(0.0)
}

fn owa_utility_cost_for_zone(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    zone_type: ZoneType,
) -> f32 {
    if !matches!(
        zone_type,
        ZoneType::Commercial | ZoneType::Office | ZoneType::Mixed | ZoneType::Industrial
    ) {
        return 0.0;
    }
    local_utility_daily_unit_cost(catalog) * tuning.owa_import_price_multiplier.max(0.0)
}

fn local_utility_daily_unit_cost(catalog: &RuntimeEconomyCatalog) -> f32 {
    let mut costs = [0.0f32; 3];
    for profile in catalog.all_profiles() {
        if let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
            && costs[service_idx] == 0.0
        {
            costs[service_idx] = profile.unit_price_currency.max(0.0);
        }
    }
    costs.iter().sum()
}

fn utility_service_index(service: Option<&str>) -> Option<usize> {
    let service = service?;
    UTILITY_SERVICES
        .iter()
        .position(|candidate| *candidate == service)
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
        + owa_utility_cost_for_zone(catalog, tuning, building.zone_type);
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
    let worker_capacity = if profile_kind_uses_explicit_work_area(profile.kind) {
        active_worker_capacity_for_profile(catalog, building, profile)
    } else {
        profile.worker_capacity
    };
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
                let output_capacity_units =
                    scaled_output_buffer_capacity_units_for_building(building, profile, port);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_size_capacity_counts_children_more_lightly_than_adults() {
        assert_eq!(
            candidate_immigrant_household_size_from_flat_size(65.5),
            Some(2)
        );
        assert_eq!(
            candidate_immigrant_household_size_from_flat_size(100.0),
            Some(4)
        );
    }

    #[test]
    fn flat_size_capacity_stays_bounded_for_large_homes() {
        assert_eq!(
            candidate_immigrant_household_size_from_flat_size(200.0),
            Some(MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE)
        );
    }

    #[test]
    fn flat_size_capacity_rejects_missing_area() {
        assert_eq!(candidate_immigrant_household_size_from_flat_size(0.0), None);
        assert_eq!(candidate_immigrant_household_size_from_flat_size(1.0), None);
    }

    #[test]
    fn vacancy_candidate_uses_starter_mix_capped_by_flat_capacity() {
        assert_eq!(
            candidate_immigrant_household_size_for_vacancy(65.5, 0, 0),
            Some(1)
        );

        let mut saw_single = false;
        let mut saw_two_person = false;
        for home_building_id in 0..64 {
            let size = candidate_immigrant_household_size_for_vacancy(65.5, home_building_id, 0)
                .expect("starter flat admits a household");
            assert!(size <= 2);
            saw_single |= size == 1;
            saw_two_person |= size == 2;
        }

        assert!(saw_single);
        assert!(saw_two_person);
        assert_eq!(
            candidate_immigrant_household_size_for_vacancy(200.0, 14, 0),
            Some(MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE)
        );
    }

    #[test]
    fn household_reserve_days_include_aggregate_service_cash_costs() {
        let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog()
            .expect("runtime economy catalog");
        let tuning = load_runtime_economy_tuning().expect("runtime economy tuning");
        let household_supply_resource = household_supply_resource_runtime_id(catalog.as_ref());
        let service_cost_per_resident = demand_sink_cash_cost_per_resident_excluding_resource(
            catalog.as_ref(),
            household_supply_resource,
        );
        assert!(service_cost_per_resident > 0.0);

        let household = Household {
            home_building_id: 0,
            budget: 1_000.0,
            stock: 0.0,
            member_count: 2,
            child_count: 0,
            adult_count: 2,
            elder_count: 0,
            consumption_rate: 1.0,
            stock_days: 0.0,
            replenishment_state: 0,
            cooldown_hours: 0,
            replenishment_failure_count: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            shopping_agent_id: usize::MAX,
            shopping_agent_schedule_seed: 0,
            shopping_timeout_hours_remaining: 0,
            replenishment_search_cursor: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        };

        let expected_daily_cost = household.member_count as f32
            * (household_supply_unit_price(catalog.as_ref())
                + service_cost_per_resident
                + tuning.households.utility_cost_per_member_per_day);
        let reserve_days = household_reserve_days(catalog.as_ref(), tuning.as_ref(), &household);
        assert!((reserve_days - household.budget / expected_daily_cost).abs() < 0.001);
    }
}
