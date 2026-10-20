// SPDX-License-Identifier: GPL-2.0-only

//! Shared household economy metrics and profile helpers.

use super::{Household, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{
    HouseholdAgeComposition, MAX_ADULTS_PER_HOUSEHOLD, household_age_composition,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, HOUSEHOLD_DEMAND_PROFILE_ID,
    HOUSEHOLD_SUPPLY_RESOURCE_ID, ResourceRuntimeId, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    RuntimeResourcePort,
};
use crate::simulation::economy::reduction::ordered_fold;
use crate::simulation::economy::resource_totals::add_resource_amount;
use crate::simulation::work_area::{
    minimum_work_area_workers, profile_kind_uses_explicit_work_area, sanitize_work_area_scale,
    scaled_work_area_worker_capacity,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const MAX_STARTER_IMMIGRANT_HOUSEHOLD_SIZE: u16 = 6;
const STARTER_IMMIGRANT_HOUSEHOLD_SIZE_BUCKETS: [u16; 16] =
    [1, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 3, 3, 4, 5, 6];
const HOUSEHOLD_BASE_AREA_M2: f32 = 25.0;
const HOUSEHOLD_ADULT_AREA_M2: f32 = 22.0;
const HOUSEHOLD_CHILD_AREA_M2: f32 = 12.0;
const MIN_POSITIVE_VALUE: f32 = 0.000_1;
const ACTIVITY_CHUNK_SIZE: usize = 1024;
pub(crate) const UTILITY_SERVICE_POWER: &str = "power";
const UTILITY_SERVICE_WATER: &str = "water";
const UTILITY_SERVICE_SEWAGE: &str = "sewage";
const UTILITY_SERVICES: [&str; 3] = [
    UTILITY_SERVICE_POWER,
    UTILITY_SERVICE_WATER,
    UTILITY_SERVICE_SEWAGE,
];

/// Shared conversion from authored daily recipe rates to operational-hour production.
pub(crate) const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CommercialActivitySignal {
    pub(crate) household_supply_resource_runtime_id: ResourceRuntimeId,
    pub(crate) demand_units_per_day: f32,
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

/// Household-sized recurring baskets financed by wages and permanent transfers.
/// Each family's desired category shares scale together within its own disposable income.
/// Savings and temporary benefits are bridges, not recurring sales promised to a new employer.
/// O(households + profile ports), allocation-free. The ordered pass is deliberate: households
/// share a finite treasury in the same unemployment/pension/child-support order as settlement.
pub(crate) fn affordable_consumer_residents(
    households: &[Household],
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    policy: &crate::simulation::economy::fiscal::CityFiscalPolicy,
    treasury_balance: f64,
) -> f64 {
    let terms = ConsumerBudgetTerms::new(catalog, tuning, policy);
    let mut treasury = treasury_balance.max(0.0);
    let mut equivalent_residents = 0.0;
    for household in households {
        if let Some(residents) =
            terms.affordable_residents(household, allocator, policy, &mut treasury)
        {
            equivalent_residents += residents;
        }
    }
    equivalent_residents
}

/// Catalog and policy terms of one consumer budget walk: the gross basket and the imported
/// utility bills. A rolling snapshot builds them per chunk and walks its households a chunk at
/// a time with its own running treasury (`SIM-02-I`). O(profile ports), allocation-free.
#[derive(Clone, Copy)]
pub(crate) struct ConsumerBudgetTerms {
    basket: f64,
    base_utility: f32,
    member_utility: f32,
}

impl ConsumerBudgetTerms {
    /// Terms at the current catalog, tuning and policy.
    pub(crate) fn new(
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        policy: &crate::simulation::economy::fiscal::CityFiscalPolicy,
    ) -> Self {
        let base_utility = household_daily_utility_allowance(catalog, tuning, 0.0)
            * tuning.owa_import_price_multiplier;
        let member_utility = household_daily_utility_allowance(catalog, tuning, 1.0)
            * tuning.owa_import_price_multiplier
            - base_utility;
        Self {
            basket: consumer_basket_cost(catalog, policy.household_vat_rate),
            base_utility,
            member_utility,
        }
    }

    /// One household's residents whose basket its recurring budget pays for, after it claims
    /// its transfers from `treasury` in settlement order; `None` without members or a home,
    /// whose claims still draw on `treasury`. O(1).
    pub(crate) fn affordable_residents(
        &self,
        household: &Household,
        allocator: &BuildingAllocator,
        policy: &crate::simulation::economy::fiscal::CityFiscalPolicy,
        treasury: &mut f64,
    ) -> Option<f64> {
        let spending = self.budget(household, allocator, policy, treasury)?;
        let members = f64::from(household.member_count);
        Some(if self.basket > 0.0 {
            (spending / self.basket).min(members)
        } else {
            members
        })
    }

    // One household's step of the walk: its transfer claims on `treasury`, then, when housed,
    // its recurring goods budget including VAT.
    fn budget(
        &self,
        household: &Household,
        allocator: &BuildingAllocator,
        policy: &crate::simulation::economy::fiscal::CityFiscalPolicy,
        treasury: &mut f64,
    ) -> Option<f64> {
        use crate::simulation::economy::fiscal::{daily_property_tax, tax_amount};
        if household.member_count == 0 {
            return None;
        }
        let housed = household_is_housed(household, allocator);
        let unemployed = household
            .adult_count
            .saturating_sub(household.employed_adults);
        if housed && household.unemployment_days_elapsed < policy.unemployment_max_days {
            super::benefits::pay_transfer_claim(
                treasury,
                f32::from(unemployed) * policy.unemployment_benefit_per_adult_per_day.max(0.0),
            );
        }
        let pension = super::benefits::pay_transfer_claim(
            treasury,
            f32::from(household.elder_count) * policy.pension_per_elder_per_day.max(0.0),
        );
        let child_support = super::benefits::pay_transfer_claim(
            treasury,
            f32::from(household.child_count) * policy.child_support_per_child_per_day.max(0.0),
        );
        if !housed {
            return None;
        }
        let members = f64::from(household.member_count);
        let gross = household.gross_daily_wages.max(0.0);
        let income = f64::from(gross - tax_amount(gross, policy.income_tax_rate))
            + f64::from(pension)
            + f64::from(child_support);
        let home = &allocator.buildings[household.home_building_id];
        let bills = f64::from(self.base_utility)
            + members * f64::from(self.member_utility)
            + f64::from(daily_property_tax(
                ZoneType::Residential,
                home.level,
                policy,
            ));
        Some((income - bills).max(0.0))
    }
}

/// Gross cost of one resident's authored daily consumer basket at catalog prices.
/// The value includes household VAT and excludes utilities and property taxes.
pub(crate) fn consumer_basket_cost(catalog: &RuntimeEconomyCatalog, vat: f32) -> f64 {
    let mut basket = 0.0_f64;
    for profile in catalog.all_profiles() {
        if profile.kind == EconomyProfileRuntimeKind::DemandSink {
            for input in &profile.inputs {
                basket += f64::from(profile.consumption_rate_per_resident)
                    * f64::from(
                        catalog
                            .unit_price_for_resource(input.resource_runtime_id)
                            .unwrap_or(0.0),
                    );
            }
        }
    }
    basket *= f64::from(1.0 + vat);
    basket
}

/// Visits each housed family's recurring goods budget after taxes, utilities and property, for
/// the households in `range`. The callback receives household index, desired resident count and
/// gross spending including VAT. Savings and temporary benefits do not enter the budget; finite
/// benefit claims draw on the walk's running `treasury` (start it at the balance clamped to
/// zero) in settlement's deterministic household order, so consecutive ranges visited in order
/// visit exactly what one walk over every household does, bit for bit (`SIM-02-K`).
/// O(range + profile ports), allocation-free.
#[allow(clippy::too_many_arguments)]
pub(crate) fn visit_consumer_budgets_in(
    households: &[Household],
    range: std::ops::Range<usize>,
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    policy: &crate::simulation::economy::fiscal::CityFiscalPolicy,
    treasury: &mut f64,
    mut visitor: impl FnMut(usize, u16, f64),
) {
    let terms = ConsumerBudgetTerms::new(catalog, tuning, policy);
    let first = range.start;
    for (offset, household) in households[range].iter().enumerate() {
        if let Some(spending) = terms.budget(household, allocator, policy, treasury) {
            visitor(first + offset, household.member_count, spending);
        }
    }
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

#[cfg(test)]
pub(crate) fn commercial_activity_signal_for_city(
    catalog: &RuntimeEconomyCatalog,
    households: &[Household],
    allocator: &BuildingAllocator,
) -> CommercialActivitySignal {
    let terms = CommercialActivityTerms::new(catalog);
    commercial_activity_sums_for_city(catalog, terms, households, allocator).signal(terms)
}

/// The sums of [`commercial_activity_signal_for_city`] in fixed parallel chunks and ordered
/// merges, households and buildings each in dense order. O(households + buildings).
pub(crate) fn commercial_activity_sums_for_city(
    catalog: &RuntimeEconomyCatalog,
    terms: CommercialActivityTerms,
    households: &[Household],
    allocator: &BuildingAllocator,
) -> CommercialActivitySums {
    let merge =
        |left: CommercialActivitySums, right: CommercialActivitySums| CommercialActivitySums {
            daily_consumption: left.daily_consumption + right.daily_consumption,
            stock_gap: left.stock_gap + right.stock_gap,
            retail_stock: left.retail_stock + right.retail_stock,
        };
    let housed = ordered_fold(
        households,
        ACTIVITY_CHUNK_SIZE,
        CommercialActivitySums::default,
        |sums, _, household| sums.add_household(terms, household, allocator),
        merge,
    );
    let retail = ordered_fold(
        &allocator.buildings,
        ACTIVITY_CHUNK_SIZE,
        CommercialActivitySums::default,
        |sums, _, building| sums.add_building(terms, catalog, building),
        merge,
    );
    CommercialActivitySums {
        retail_stock: retail.retail_stock,
        ..housed
    }
}

/// Catalog terms of the commercial activity signal, looked up once per fold.
#[derive(Clone, Copy)]
pub(crate) struct CommercialActivityTerms {
    stock_target_days: f32,
    household_supply: ResourceRuntimeId,
}

impl CommercialActivityTerms {
    /// The household demand profile's stock target and the household supply resource.
    pub(crate) fn new(catalog: &RuntimeEconomyCatalog) -> Self {
        Self {
            stock_target_days: household_demand_profile(catalog).stock_target_days,
            household_supply: household_supply_resource_runtime_id(catalog),
        }
    }
}

/// Sums behind [`commercial_activity_signal_for_city`], which a rolling snapshot folds entity
/// by entity (`SIM-02-I`): housed households' daily supply consumption and their stock gap net
/// of shoppers' reservations, and the household supplies stocked at operating sales-scaled
/// stores.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CommercialActivitySums {
    pub(crate) daily_consumption: f32,
    pub(crate) stock_gap: f32,
    pub(crate) retail_stock: f32,
}

impl CommercialActivitySums {
    /// Adds a household; one without an operational home adds nothing. O(1).
    pub(crate) fn add_household(
        &mut self,
        terms: CommercialActivityTerms,
        household: &Household,
        allocator: &BuildingAllocator,
    ) {
        if !household_is_housed(household, allocator) {
            return;
        }
        let daily_consumption = household.member_count as f32 * household.consumption_rate.max(0.0);
        let target_stock = daily_consumption * terms.stock_target_days.max(0.0);
        let incoming = if matches!(
            household.replenishment_state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        ) {
            household.reserved_amount.max(0.0)
        } else {
            0.0
        };
        let stock_gap = (target_stock - household.stock.max(0.0) - incoming).max(0.0);
        self.daily_consumption += daily_consumption;
        self.stock_gap += stock_gap;
    }

    /// Adds a building's household supplies when it is an operating sales-scaled store. O(1).
    pub(crate) fn add_building(
        &mut self,
        terms: CommercialActivityTerms,
        catalog: &RuntimeEconomyCatalog,
        building: &Building,
    ) {
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
        if !is_sales_scaled_commercial_store(building, profile) {
            return;
        }
        self.retail_stock += building.inventory_units(terms.household_supply);
    }

    /// Daily consumption plus the stock gap that retail stock does not cover, recovered over
    /// the stock target. O(1).
    pub(crate) fn signal(&self, terms: CommercialActivityTerms) -> CommercialActivitySignal {
        // A household shortage is not new production demand when its goods already
        // exist at a shop or in a shopper's reservation. Recover only the remaining gap.
        let demand_units_per_day = self.daily_consumption
            + (self.stock_gap - self.retail_stock).max(0.0) / terms.stock_target_days.max(1.0);
        CommercialActivitySignal {
            household_supply_resource_runtime_id: terms.household_supply,
            demand_units_per_day,
        }
    }
}

/// Fills `rates` with each service resource's daily purchases per resident, summed over the
/// demand-sink profiles in first-seen order: the household side of service-store sales.
/// O(profiles²) over the small catalog; reuses `rates`.
pub(super) fn service_store_demand_rates_by_resource(
    catalog: &RuntimeEconomyCatalog,
    rates: &mut Vec<(ResourceRuntimeId, f32)>,
) {
    rates.clear();
    let profiles = catalog.all_profiles();
    for profile in profiles {
        if profile.kind != EconomyProfileRuntimeKind::DemandSink {
            continue;
        }
        for input in &profile.inputs {
            let sold_by_service_stores = profiles.iter().any(|store| {
                store.kind == EconomyProfileRuntimeKind::ServiceStore
                    && store
                        .outputs
                        .iter()
                        .any(|output| output.resource_runtime_id == input.resource_runtime_id)
            });
            if sold_by_service_stores {
                add_resource_amount(
                    rates,
                    input.resource_runtime_id,
                    profile.consumption_rate_per_resident.max(0.0),
                );
            }
        }
    }
}

/// Returns whole physical worker slots before market or payroll limits, including field area.
pub(crate) fn physical_worker_capacity_for_profile(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> u32 {
    if profile_kind_uses_explicit_work_area(profile.kind) {
        scaled_work_area_worker_capacity(
            profile.workers_per_hectare,
            building.work_area_scale,
            minimum_work_area_workers(profile.kind),
            profile.area_worker_capacity_max,
        )
    } else {
        profile.worker_capacity
    }
}

/// A workplace's activity cap from its funded census `activity` (`DEM-02`): goods and service
/// stores, and explicit work areas; zero for any other building and for one that cannot
/// operate. The census already sizes each to the reachable customers, buyers or exports it
/// reserved and can finance; a citywide demand ratio would also cap the businesses those
/// customers chose. Each building sets its own at its hourly phase (`SIM-02-I`). O(1).
pub(crate) fn commercial_activity_floor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    activity: f32,
) -> f32 {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
    {
        return 0.0;
    }
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    if is_sales_scaled_commercial_store(building, profile)
        || is_service_scaled_commercial_store(building, profile)
        || profile_kind_uses_explicit_work_area(profile.kind)
    {
        activity.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Sets every building's activity cap from `store_activity` (indexed by building) at once, as
/// their hourly phases would.
#[cfg(test)]
pub(crate) fn refresh_commercial_activity_floor(
    catalog: &RuntimeEconomyCatalog,
    allocator: &mut BuildingAllocator,
    store_activity: &[f32],
) {
    for (index, building) in allocator.buildings.iter_mut().enumerate() {
        let activity = store_activity.get(index).copied().unwrap_or(0.0);
        building.commercial_activity_floor_scale =
            commercial_activity_floor(catalog, building, activity);
    }
}

fn commercial_activity_scale_with_floor(
    _catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
) -> f32 {
    if !is_sales_scaled_commercial_store(building, profile) {
        return 1.0;
    }
    // A bulk household purchase transfers existing stock; its one-day receipt
    // must not open recurring jobs above the store's funded recurring activity.
    floor_scale.clamp(0.0, 1.0)
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
        let physical_capacity = physical_worker_capacity_for_profile(building, profile);
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

/// Physical recipe scale, independent of current staffing, stock and customer activity.
pub(crate) fn output_area_scale_for_building(
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> f32 {
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

/// A service's funding ratio: its own override when set (`>= 0`), else the city's.
pub(crate) fn effective_service_funding(building_override: f32, city_funding: f32) -> f32 {
    if building_override >= 0.0 {
        building_override.clamp(0.0, 1.0)
    } else {
        city_funding
    }
}

/// Writes each building's electricity service funding into `out`: the effective funding of
/// power producers, `1.0` for every other building. O(buildings), parallel, and
/// allocation-free once `out` has grown.
pub(crate) fn write_electricity_funding_by_building(
    buildings: &[Building],
    catalog: &RuntimeEconomyCatalog,
    city_funding: f32,
    out: &mut Vec<f32>,
) {
    out.clear();
    out.resize(buildings.len(), 1.0);
    // Main-thread funding changes reach here outside any pool (CODE-17).
    crate::thread_pools::install(|| {
        out.par_iter_mut()
            .zip(buildings)
            .for_each(|(value, building)| {
                if catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .is_some_and(|profile| {
                        profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER)
                    })
                {
                    *value =
                        effective_service_funding(building.service_funding_override, city_funding);
                }
            });
    });
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
    /// Current staffed/input-backed production ratio before output-buffer throttling.
    pub(crate) output_capacity_factor: f32,
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

// A store's funded staff, rounded up to whole workers, may refill its shelf above its funded
// sales until it holds `stock_target_days` of them; bulk pantry purchases otherwise find it
// sold out. It opens no job and stays within the authored output buffer. O(output ports).
fn shelf_below_target(
    building: &Building,
    profile: &EconomyProfileRuntime,
    floor_scale: f32,
    production_rate_scale: f32,
) -> bool {
    let days = profile.stock_target_days.max(0.0) * floor_scale.clamp(0.0, 1.0);
    profile.outputs.iter().any(|port| {
        building.inventory_units(port.resource_runtime_id)
            < profile.net_output_units_per_day(port) * production_rate_scale * days
    })
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
    let mut staffing_factor =
        (effective_workers as f32 / staffing_worker_capacity as f32).clamp(0.0, 1.0);
    let production_rate_scale = output_area_scale_for_building(building, profile);
    if is_sales_scaled_commercial_store(building, profile)
        && !shelf_below_target(building, profile, floor_scale, production_rate_scale)
    {
        staffing_factor = staffing_factor.min(floor_scale.clamp(0.0, 1.0));
    }
    let input_factor =
        hourly_input_availability_factor(profile, building, staffing_factor, production_rate_scale);
    let output_capacity_factor = staffing_factor * input_factor;
    let output_headroom_factor = hourly_output_headroom_factor(
        profile,
        building,
        output_capacity_factor,
        production_rate_scale,
    );
    let throughput_factor = output_capacity_factor * output_headroom_factor;
    BuildingOperationFactors {
        active_worker_capacity,
        effective_workers,
        input_factor,
        output_headroom_factor,
        output_capacity_factor,
        throughput_factor,
    }
}

/// Recurring purchases for an accepted operation, capped by current staff and explicit area.
/// The caller supplies a financially validated activity; staff or stock alone cannot create it.
pub(crate) fn funded_input_demand_scale(
    building: &Building,
    profile: &EconomyProfileRuntime,
    funded_activity: f32,
) -> f32 {
    let capacity = physical_worker_capacity_for_profile(building, profile).max(1);
    funded_activity
        .min(building.worker_count as f32 / capacity as f32)
        .clamp(0.0, 1.0)
        * output_area_scale_for_building(building, profile)
}

/// Consumes recipe inputs exactly once for the realized hourly production scale.
/// Work-area callers include hectares and any final reserve/output truncation in the scale.
pub(crate) fn consume_hourly_production_inputs(
    building: &mut Building,
    profile: &EconomyProfileRuntime,
    realized_scale: f32,
) {
    for port in &profile.inputs {
        let units = port.units_per_day / OPERATIONAL_HOURS_PER_DAY * realized_scale;
        if units > 0.0 {
            building.remove_inventory_units(port.resource_runtime_id, units);
        }
    }
}

/// Returns a fill ratio across distinct resource buffers, counting shared input/output stock once.
pub(crate) fn building_inventory_fill_ratio(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> Option<f32> {
    let mut inventory_units = 0.0f32;
    let mut capacity_units = 0.0f32;

    for input_port in &profile.inputs {
        let (mut target_units, _, _) =
            scaled_input_inventory_targets_for_building(catalog, building, profile, input_port);
        if let Some(output) = profile.output_port(input_port.resource_runtime_id) {
            let output_capacity =
                scaled_output_buffer_capacity_units_for_building(building, profile, output);
            if output_capacity.is_finite() {
                target_units = target_units.max(output_capacity);
            }
        }
        if target_units <= MIN_POSITIVE_VALUE {
            continue;
        }
        inventory_units += building
            .inventory_units(input_port.resource_runtime_id)
            .min(target_units);
        capacity_units += target_units;
    }

    for output_port in &profile.outputs {
        if profile
            .input_port(output_port.resource_runtime_id)
            .is_some()
        {
            continue;
        }
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
            // Inputs are consumed before output enters this same inventory slot.
            let hourly_output = profile.net_output_units_per_day(port) * production_rate_scale
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
    let mut scale = output_area_scale_for_building(building, profile);
    if profile.utility_service.as_deref() == Some("power") {
        // Reserve fuel for staffed capacity, not the entire plant. Dispatch can use less,
        // while the existing shipment-batch floor still keeps deliveries consolidated.
        let capacity = physical_worker_capacity_for_profile(building, profile).max(1);
        scale *= (building.worker_count as f32 / capacity as f32).min(1.0);
    }
    if is_sales_scaled_commercial_store(building, profile) {
        scale *= commercial_activity_scale_with_floor(
            catalog,
            building,
            profile,
            building.commercial_activity_floor_scale,
        );
    }
    profile.input_inventory_targets(input_port, scale)
}

/// Output stock available after retaining a producer's own same-resource input reserve.
/// Local orders, scheduled exports and distress sales must use the same protection.
pub(crate) fn saleable_output_stock(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
    resource: ResourceRuntimeId,
) -> f32 {
    let own_reserve = profile.input_port(resource).map_or(0.0, |input| {
        scaled_input_inventory_targets_for_building(catalog, building, profile, input).0
    });
    (building.inventory_units(resource) - own_reserve).max(0.0)
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
    let daily_utility_cost = household_daily_utility_allowance(catalog, tuning, members);
    let daily_essential_cost = daily_supply_cost + daily_cash_demand_sink_cost + daily_utility_cost;
    if daily_essential_cost <= 0.0 {
        0.0
    } else {
        ((household.budget - household.power_arrears.total()).max(0.0) / daily_essential_cost)
            .max(0.0)
    }
}

/// Conservative full-home utility allowance for admission, reserves and job seeking.
/// Forecasts reserve the entire base because future occupants of an offered flat are not known.
pub(crate) fn household_daily_utility_allowance(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    members: f32,
) -> f32 {
    let home = household_demand_profile(catalog);
    members * tuning.households.water_sewage_cost_per_member_per_day
        + (home.electricity.idle_kwh_per_day + members * home.electricity.per_resident_kwh_per_day)
            * local_utility_service_costs(catalog)[0]
}

/// Whether a household with an independent adult or elder has an operational home.
pub(crate) fn household_is_housed(household: &Household, allocator: &BuildingAllocator) -> bool {
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

/// Offers examined for a batch of `arrivals`: four per arrival, at least sixteen for a small
/// town. Admission and the housing need walk share this bound.
pub(crate) fn applicant_offer_limit(arrivals: usize) -> usize {
    arrivals.saturating_mul(4).max(16)
}

/// The deterministic outside household offered one residential place: size and ages.
///
/// `home_seed` is the home's building index (the index it will have once placed while it is
/// queued, `queued_home_seed`; a proposed home's parcel id before it is queued) and
/// `household_seed` the offer's position in the applicant stream. Admission, housing need and
/// the workforce calendar read the same stream. `None` where the flat fits no household. O(1).
pub(crate) fn applicant_offer(
    flat_size_m2: f32,
    home_seed: usize,
    household_seed: usize,
) -> Option<(u16, HouseholdAgeComposition)> {
    let size = candidate_immigrant_household_size_for_vacancy(
        flat_size_m2,
        home_seed,
        household_seed as u32,
    )?;
    Some((
        size,
        household_age_composition(home_seed, household_seed, size),
    ))
}

/// Places `slots` of one home counted by the adults of the applicant offered each place,
/// indexed `0..=MAX_ADULTS_PER_HOUSEHOLD`. A flat that fits no household counts nowhere.
/// O(places).
pub(crate) fn applicant_places_by_adults(
    flat_size_m2: f32,
    home_seed: usize,
    slots: std::ops::Range<u32>,
) -> [u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1] {
    applicant_places_and_members(flat_size_m2, home_seed, slots).0
}

/// [`applicant_places_by_adults`] together with the applicants' total members. O(places).
pub(crate) fn applicant_places_and_members(
    flat_size_m2: f32,
    home_seed: usize,
    slots: std::ops::Range<u32>,
) -> ([u32; MAX_ADULTS_PER_HOUSEHOLD as usize + 1], u32) {
    let mut places = [0; MAX_ADULTS_PER_HOUSEHOLD as usize + 1];
    let mut members = 0;
    for slot in slots {
        if let Some((size, composition)) = applicant_offer(flat_size_m2, home_seed, slot as usize) {
            places[usize::from(composition.adult_count)] += 1;
            members += u32::from(size);
        }
    }
    (places, members)
}

/// Adults the applicants for places `slots` of one home would bring. O(places).
#[cfg(test)]
pub(crate) fn applicant_workers(
    flat_size_m2: f32,
    home_seed: usize,
    slots: std::ops::Range<u32>,
) -> u32 {
    applicant_places_by_adults(flat_size_m2, home_seed, slots)
        .iter()
        .enumerate()
        .map(|(adults, &places)| adults as u32 * places)
        .sum()
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

pub(super) use crate::simulation::core::stable_hash64;

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

/// Authored daily unit prices in the shared power, water, sewage order.
pub(crate) fn local_utility_service_costs(catalog: &RuntimeEconomyCatalog) -> [f32; 3] {
    let mut costs = [0.0f32; 3];
    for profile in catalog.all_profiles() {
        if let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
            && costs[service_idx] == 0.0
        {
            costs[service_idx] = profile.unit_price_currency.max(0.0);
        }
    }
    costs
}

/// Maps an authored utility name into the shared power, water, sewage order.
pub(super) fn utility_service_index(service: Option<&str>) -> Option<usize> {
    let service = service?;
    UTILITY_SERVICES
        .iter()
        .position(|candidate| *candidate == service)
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
    use crate::simulation::economy::definitions::load_runtime_economy_tuning;

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
            power_arrears: Default::default(),
            budget: 1_000.0,
            stock: 0.0,
            member_count: 2,
            child_count: 0,
            adult_count: 2,
            employed_adults: 0,
            gross_daily_wages: 0.0,
            shopping_has_car: false,
            elder_count: 0,
            consumption_rate: 1.0,
            stock_days: 0.0,
            replenishment_state: 0,
            cooldown_hours: 0,
            replenishment_failure_count: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            shopping_agent: crate::simulation::economy::agents::AgentHandle::NONE,
            shopping_timeout_hours_remaining: 0,
            replenishment_search_cursor: 0,
            stay_failure_days: 0,
            unhoused_days_elapsed: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        };

        let expected_daily_cost = household.member_count as f32
            * (household_supply_unit_price(catalog.as_ref()) + service_cost_per_resident)
            + household_daily_utility_allowance(
                catalog.as_ref(),
                tuning.as_ref(),
                household.member_count as f32,
            );
        let reserve_days = household_reserve_days(catalog.as_ref(), tuning.as_ref(), &household);
        assert!((reserve_days - household.budget / expected_daily_cost).abs() < 0.001);
    }
}
