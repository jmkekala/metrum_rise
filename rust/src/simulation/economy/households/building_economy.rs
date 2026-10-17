// SPDX-License-Identifier: GPL-2.0-only

//! Building-side production, utility settlement, bankruptcy, and liquidation.

use super::HouseholdSystem;
use super::data::DailyHouseholdLedger;
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, UTILITY_SERVICE_POWER, building_operation_factors,
    commercial_activity_floor, consume_hourly_production_inputs, economy_profile_for_building,
    local_utility_service_costs, saleable_output_stock,
    scaled_output_buffer_capacity_units_for_building, utility_service_index,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{
    CityFiscalPolicy, FiscalRevenue, daily_property_tax, tax_amount,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::zoning::ZoneType;
use std::sync::Arc;

const UTILITY_SERVICE_POWER_INDEX: usize = 0;
const UTILITY_SERVICE_WATER_INDEX: usize = 1;
const UTILITY_SERVICE_SEWAGE_INDEX: usize = 2;
const UTILITY_SERVICE_COUNT: usize = 3;
const LIQUIDATION_UNIT_PRICE_EPSILON: f32 = 0.0001;
const CITY_SERVICE_UTILITY_DEMAND_UNITS_PER_DAY: f32 = 1.0;

impl HouseholdSystem {
    /// Settles building `building_idx`'s 24-hour window at its daily phase (`SIM-02-G`), in the
    /// order of `economy.md § Building Bankruptcy`: the bankruptcy check, property tax, water,
    /// sewage and electricity bills, distress liquidation, then business profit tax. The caller
    /// reports the window and then rolls its accumulators. Workers are paid at their own
    /// phases. O(ports).
    pub(crate) fn settle_building_day(
        &mut self,
        building_idx: usize,
        allocator: &mut BuildingAllocator,
        day: &DaySettlement,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: &mut f64,
    ) -> FiscalRevenue {
        let catalog = &*day.catalog;
        check_building_bankruptcy(allocator, building_idx);
        let mut revenue =
            charge_building_property_tax(allocator, building_idx, catalog, fiscal_policy);
        self.settle_building_utilities(allocator, building_idx, day, treasury_balance);
        let city_owned = allocator
            .registry
            .is_city_service_asset(&allocator.buildings[building_idx].asset_id);
        self.settle_building_electricity_day(
            &mut allocator.buildings[building_idx],
            city_owned,
            catalog,
            day.prices[UTILITY_SERVICE_POWER_INDEX],
            day.tuning.owa_import_price_multiplier,
            treasury_balance,
        );
        resolve_building_distress(
            building_idx,
            &mut allocator.buildings[building_idx],
            catalog,
            day.tuning.owa_distress_liquidation_multiplier,
        );
        revenue.business_profit_tax = settle_building_profit_tax(
            &mut allocator.buildings[building_idx],
            catalog,
            fiscal_policy.business_profit_tax_rate,
        );
        revenue
    }

    // Water and sewage of one building at the last closed day's coverage: a private consumer
    // pays the local share and the import price for the rest, a staffed city service pays from
    // the treasury, and local payments return to the treasury as utility revenue. A provider
    // counts itself for the next day's coverage and books its share of the last day's revenue.
    // O(1).
    fn settle_building_utilities(
        &mut self,
        allocator: &mut BuildingAllocator,
        building_idx: usize,
        day: &DaySettlement,
        treasury_balance: &mut f64,
    ) {
        let city_service = allocator.is_city_service_building(&allocator.buildings[building_idx]);
        let building = &mut allocator.buildings[building_idx];
        if !is_active_utility_consumer(building) {
            return;
        }
        let catalog = &*day.catalog;
        let imports = day.tuning.owa_import_price_multiplier.max(0.0);
        let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
        let totals = &mut self.day_totals;
        let mut local_revenue = 0.0f32;
        if is_private_utility_consumer(building, profile) {
            let demand = profile.map_or([1.0; 3], |p| {
                p.utility_demand_units_per_day(building.work_area_scale)
            });
            let mut daily_cost = 0.0f32;
            for (service_idx, &service_demand) in demand.iter().enumerate().skip(1) {
                let units = day.prices[service_idx].max(0.0) * service_demand;
                let coverage = day.coverage[service_idx];
                let local_cost = units * coverage;
                daily_cost += local_cost + units * imports * (1.0 - coverage);
                totals.utility_local_revenue[service_idx] += local_cost;
                local_revenue += local_cost;
            }
            let before = building.operating_budget;
            building.operating_budget -= daily_cost;
            crate::debug::money::record_change(
                crate::debug::money::MoneyFlow::OutsideUtilities,
                before,
                building.operating_budget,
            );
        } else if city_service && building.worker_count > 0 {
            for service_idx in 1..UTILITY_SERVICE_COUNT {
                let units = CITY_SERVICE_UTILITY_DEMAND_UNITS_PER_DAY * day.prices[service_idx];
                let coverage = day.coverage[service_idx];
                let local_cost = units * coverage;
                let owa_cost = units * imports * (1.0 - coverage);
                *treasury_balance -= f64::from(local_cost + owa_cost);
                crate::debug::money::record(
                    crate::debug::money::MoneyFlow::OutsideUtilities,
                    -f64::from(local_cost + owa_cost),
                );
                totals.utility_local_revenue[service_idx] += local_cost;
                totals.power.city_service_utility_local_cost += local_cost;
                totals.power.city_service_utility_owa_cost += owa_cost;
                local_revenue += local_cost;
            }
        }
        if local_revenue > 0.0 {
            totals.power.utility_local_revenue += local_revenue;
            *treasury_balance += f64::from(local_revenue);
            crate::debug::money::record(
                crate::debug::money::MoneyFlow::OutsideUtilities,
                f64::from(local_revenue),
            );
        }
        let Some(profile) = profile else {
            return;
        };
        if is_operational_utility_provider(catalog, building, profile)
            && let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
            && service_idx != UTILITY_SERVICE_POWER_INDEX
        {
            totals.utility_providers[service_idx] += 1;
            // Provider revenue is telemetry; local utility fees fund the treasury.
            building.revenue += day.provider_share[service_idx];
        }
        if city_service
            && building.worker_count > 0
            && profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER)
        {
            totals.power.power_upkeep_utility_cost += day.plant_upkeep;
        }
    }

    /// Water and sewage of household `household_id`'s ledger window at the last closed day's
    /// coverage: the local share of its bills goes to the treasury, and missing coverage costs an
    /// import surcharge the budget pays as far as it can. O(1).
    pub(super) fn settle_household_utilities(
        &mut self,
        household_id: usize,
        day: &DaySettlement,
        treasury_balance: &mut f64,
    ) {
        let imports = day.tuning.owa_import_price_multiplier;
        for service_idx in 1..UTILITY_SERVICE_COUNT {
            let coverage = day.coverage[service_idx];
            let bill =
                household_utility_bill_for_service(&self.daily_ledgers[household_id], service_idx);
            let local_revenue = bill * coverage;
            if local_revenue > 0.0 {
                self.day_totals.utility_local_revenue[service_idx] += local_revenue;
                self.day_totals.power.utility_local_revenue += local_revenue;
                *treasury_balance += f64::from(local_revenue);
                crate::debug::money::record(
                    crate::debug::money::MoneyFlow::OutsideUtilities,
                    f64::from(local_revenue),
                );
            }
            let surcharge_multiplier = household_owa_surcharge_multiplier(coverage, imports);
            if surcharge_multiplier <= 0.0 {
                continue;
            }
            let household = &mut self.households[household_id];
            let surcharge = (bill * surcharge_multiplier).min(household.budget.max(0.0));
            if surcharge <= 0.0 {
                continue;
            }
            let before = household.budget;
            household.budget = (household.budget - surcharge).max(0.0);
            crate::debug::money::record_change(
                crate::debug::money::MoneyFlow::OutsideUtilities,
                before,
                household.budget,
            );
            add_household_utility_cost_for_service(
                &mut self.daily_ledgers[household_id],
                service_idx,
                surcharge,
            );
        }
    }

    /// Residential property tax of household `household_id` at its daily phase, paid as far as
    /// its budget allows. O(1).
    pub(super) fn charge_household_property_tax(
        &mut self,
        household_id: usize,
        allocator: &BuildingAllocator,
        fiscal_policy: &CityFiscalPolicy,
    ) -> f32 {
        let household = &mut self.households[household_id];
        if household.member_count == 0 {
            return 0.0;
        }
        let Some(home) = allocator.buildings.get(household.home_building_id) else {
            return 0.0;
        };
        if !is_taxable_residential_home(home) {
            return 0.0;
        }
        let tax = daily_property_tax(ZoneType::Residential, home.level, fiscal_policy);
        let paid = tax.min(household.budget.max(0.0));
        if paid <= 0.0 {
            return 0.0;
        }
        household.budget -= paid;
        self.daily_ledgers[household_id].property_tax_paid += paid;
        paid
    }

    /// Bankruptcy check of every building at once, as their daily phases would run it.
    #[cfg(test)]
    pub(super) fn run_bankruptcy_check(&mut self, allocator: &mut BuildingAllocator) {
        for building_idx in 0..allocator.buildings.len() {
            check_building_bankruptcy(allocator, building_idx);
        }
    }

    /// Water, sewage and electricity bills of every household and building at once, then
    /// distress liquidation, as their daily phases would run them; coverage and provider
    /// revenue read the providers operating now, as if they had settled the day before.
    #[cfg(test)]
    pub(super) fn settle_daily_utilities(
        &mut self,
        allocator: &mut BuildingAllocator,
        _logistics: &ShipmentSystem,
        treasury_balance: &mut f64,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.ensure_daily_ledger_len();
        let mut providers = [0u32; UTILITY_SERVICE_COUNT];
        for building in &allocator.buildings {
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if is_active_utility_consumer(building)
                && is_operational_utility_provider(&catalog, building, profile)
                && let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
            {
                providers[service_idx] += 1;
            }
        }
        let day = DaySettlement::new(
            Arc::clone(&catalog),
            Arc::clone(&tuning),
            providers,
            [0.0; UTILITY_SERVICE_COUNT],
        );
        for household_id in 0..self.households.len() {
            self.settle_household_utilities(household_id, &day, treasury_balance);
        }
        for building_idx in 0..allocator.buildings.len() {
            self.settle_building_utilities(allocator, building_idx, &day, treasury_balance);
        }
        let revenue = self.day_totals.utility_local_revenue;
        let summary = self.settle_daily_electricity(allocator, treasury_balance);
        self.last_day_totals.power = summary;
        // Same-day revenue shares, as the providers' phases would book them the next day.
        let shares = DaySettlement::new(catalog, Arc::clone(&tuning), providers, revenue);
        for building in &mut allocator.buildings {
            if let Some(profile) = shares
                .catalog
                .profile_by_runtime_id(building.economy_profile_runtime_id)
                && is_active_utility_consumer(building)
                && is_operational_utility_provider(&shares.catalog, building, profile)
                && let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
                && service_idx != UTILITY_SERVICE_POWER_INDEX
            {
                building.revenue += shares.provider_share[service_idx];
            }
        }
        for building_idx in 0..allocator.buildings.len() {
            resolve_building_distress(
                building_idx,
                &mut allocator.buildings[building_idx],
                &shares.catalog,
                tuning.owa_distress_liquidation_multiplier,
            );
        }
    }

    /// Property tax of every household and building at once, as their daily phases would
    /// collect it.
    #[cfg(test)]
    pub(super) fn settle_daily_property_tax(
        &mut self,
        allocator: &mut BuildingAllocator,
        fiscal_policy: &CityFiscalPolicy,
    ) -> FiscalRevenue {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.ensure_daily_ledger_len();
        let mut revenue = FiscalRevenue::default();
        for household_id in 0..self.households.len() {
            revenue.residential_property_tax +=
                self.charge_household_property_tax(household_id, allocator, fiscal_policy);
        }
        for building_idx in 0..allocator.buildings.len() {
            let building =
                charge_building_property_tax(allocator, building_idx, &catalog, fiscal_policy);
            revenue.commercial_property_tax += building.commercial_property_tax;
            revenue.industrial_property_tax += building.industrial_property_tax;
        }
        revenue
    }

    /// Business profit tax of every building at once, as their daily phases would collect it.
    #[cfg(test)]
    pub(super) fn settle_business_profit_tax(
        &mut self,
        allocator: &mut BuildingAllocator,
        tax_rate: f32,
    ) -> f32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        allocator
            .buildings
            .iter_mut()
            .map(|building| settle_building_profit_tax(building, &catalog, tax_rate))
            .sum()
    }

    /// Every building's hourly phase in dense order (activity floor from the census,
    /// production, service sales, electricity), every household's service purchases and
    /// electricity, then the hour's end, as one hour of slots would run them. Farm and mine
    /// output, which the simulation's phases add, is left to the caller.
    #[cfg(test)]
    pub(super) fn run_building_hour_for_test(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let profiles = super::electricity::ElectricityProfiles::new(&catalog);
        self.ensure_daily_ledger_len();
        // The hour's census, as its pass would publish it.
        let activities = self.job_funding.activities();
        for (building_idx, building) in allocator.buildings.iter_mut().enumerate() {
            building.funded_activity = activities.get(building_idx).copied().unwrap_or(0.0);
        }
        for building_idx in 0..allocator.buildings.len() {
            let building = &mut allocator.buildings[building_idx];
            refresh_activity_floor(&catalog, building);
            produce_building_hour(&catalog, building);
            self.settle_store_service_sales(&catalog, building);
            self.run_building_electricity(&profiles, allocator, building_idx);
        }
        for household_id in 0..self.households.len() {
            self.buy_household_services(household_id, allocator);
            self.record_household_electricity(&profiles, household_id, allocator);
        }
        self.close_city_hour(&catalog);
    }
}

/// `building`'s activity cap at its hourly phase, before production (`SIM-02-I`): from its
/// funded share of output in the hour's census, as the census published it. O(1).
pub(crate) fn refresh_activity_floor(catalog: &RuntimeEconomyCatalog, building: &mut Building) {
    building.commercial_activity_floor_scale =
        commercial_activity_floor(catalog, building, building.funded_activity);
}

/// What every daily settlement in one slot shares (`SIM-02-G`): catalog and tuning, utility
/// prices, and the water and sewage coverage and provider revenue shares the last closed day
/// left. Built once per slot in O(profiles).
pub(crate) struct DaySettlement {
    pub(crate) catalog: Arc<RuntimeEconomyCatalog>,
    pub(crate) tuning: Arc<RuntimeEconomyTuning>,
    // Local unit price of power, water and sewage.
    pub(super) prices: [f32; UTILITY_SERVICE_COUNT],
    // One where an operational provider settled in the last closed day: water and sewage
    // supply always meets demand once a provider operates.
    coverage: [f32; UTILITY_SERVICE_COUNT],
    // The last closed day's local revenue per provider that settled in it.
    provider_share: [f32; UTILITY_SERVICE_COUNT],
    // Water and sewage upkeep of one staffed power plant, for the electricity report.
    plant_upkeep: f32,
}

impl DaySettlement {
    fn new(
        catalog: Arc<RuntimeEconomyCatalog>,
        tuning: Arc<RuntimeEconomyTuning>,
        providers: [u32; UTILITY_SERVICE_COUNT],
        local_revenue: [f32; UTILITY_SERVICE_COUNT],
    ) -> Self {
        let prices = local_utility_service_costs(&catalog);
        let mut coverage = [0.0; UTILITY_SERVICE_COUNT];
        let mut provider_share = [0.0; UTILITY_SERVICE_COUNT];
        for service_idx in 1..UTILITY_SERVICE_COUNT {
            if providers[service_idx] > 0 {
                coverage[service_idx] = 1.0;
                provider_share[service_idx] =
                    local_revenue[service_idx].max(0.0) / providers[service_idx] as f32;
            }
        }
        let imports = tuning.owa_import_price_multiplier;
        let plant_upkeep = (1..UTILITY_SERVICE_COUNT)
            .map(|service_idx| {
                let coverage = coverage[service_idx];
                prices[service_idx] * (coverage + (1.0 - coverage) * imports)
            })
            .sum();
        Self {
            catalog,
            tuning,
            prices,
            coverage,
            provider_share,
            plant_upkeep,
        }
    }
}

impl HouseholdSystem {
    /// The inputs this slot's daily settlements share, from the last closed day.
    pub(crate) fn day_settlement(&self) -> DaySettlement {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let last = &self.last_day_totals;
        DaySettlement::new(
            catalog,
            tuning,
            last.utility_providers,
            last.utility_local_revenue,
        )
    }
}

// Step 1 of `economy.md § Building Bankruptcy`: a building that ended its last window in
// distress and is still negative is deserted. O(1).
fn check_building_bankruptcy(allocator: &mut BuildingAllocator, building_idx: usize) {
    let building = &allocator.buildings[building_idx];
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || allocator.registry.is_city_service_asset(&building.asset_id)
    {
        return;
    }
    if building.budget_distress && building_window_insolvent(building) {
        debug_log!(
            "economy",
            "building asset={} bankrupt: budget_distress=true budget={:.2}",
            building.asset_id,
            building.operating_budget
        );
        allocator.mark_building_deserted(building_idx);
    }
}

// Daily property tax of a private commercial or industrial site. O(1).
fn charge_building_property_tax(
    allocator: &mut BuildingAllocator,
    building_idx: usize,
    catalog: &RuntimeEconomyCatalog,
    fiscal_policy: &CityFiscalPolicy,
) -> FiscalRevenue {
    let mut revenue = FiscalRevenue::default();
    let is_city_service = allocator
        .registry
        .is_city_service_asset(&allocator.buildings[building_idx].asset_id);
    let building = &mut allocator.buildings[building_idx];
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
    let Some(tax_zone) =
        private_nonresidential_property_tax_zone(building, profile, is_city_service)
    else {
        return revenue;
    };
    let tax = daily_property_tax(tax_zone, building.level, fiscal_policy);
    if tax <= 0.0 {
        return revenue;
    }
    building.operating_budget -= tax;
    match tax_zone {
        ZoneType::Commercial => revenue.commercial_property_tax = tax,
        ZoneType::Industrial => revenue.industrial_property_tax = tax,
        _ => {}
    }
    revenue
}

// Step 4 of `economy.md § Building Bankruptcy`: a negative budget sells all unreserved output
// at distress price and marks distress; otherwise distress clears. Outside the logistics hour
// no shipment holds unloaded source stock (`SIM-02-E` loads cargo when it is created), so
// nothing is reserved. O(output ports).
// A window is insolvent when it ends in deficit or left a wage unpaid for lack of cash after
// selling output. The midnight payroll ran just before the bills, so an employer that could not
// pay its workers ended the day negative; wages at each worker's phase leave no such moment, so
// the unpaid wages stand in for it (`SIM-02-G`).
fn building_window_insolvent(building: &Building) -> bool {
    building.operating_budget < 0.0 || building.unpaid_cash_wages_in_window > 0
}

fn resolve_building_distress(
    building_idx: usize,
    building: &mut Building,
    catalog: &RuntimeEconomyCatalog,
    liquidation_multiplier: f32,
) {
    if building.is_deserted
        || building.broken
        || building.economy_broken
        || building.is_under_construction()
    {
        return;
    }
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
    if !building_participates_in_budget_distress(building, profile) {
        return;
    }
    if building.operating_budget < 0.0 {
        liquidate_outputs_until_budget(
            building_idx,
            building,
            catalog,
            &[],
            catalog.resource_count(),
            liquidation_multiplier,
            f32::INFINITY,
        );
        building.budget_distress = true;
        debug_log!(
            "economy",
            "building asset={} in distress: budget={:.2} after liquidation",
            building.asset_id,
            building.operating_budget
        );
    } else {
        building.budget_distress = building.unpaid_cash_wages_in_window > 0;
    }
}

// Tax on a private business's positive budget growth over its last window; the baseline then
// restarts, so the next window taxes only its own growth. Never debits below zero. O(1).
fn settle_building_profit_tax(
    building: &mut Building,
    catalog: &RuntimeEconomyCatalog,
    tax_rate: f32,
) -> f32 {
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
    let tracked_business = is_profit_tracked_private_business(building, profile);
    let mut tax = 0.0;
    if tracked_business {
        let baseline = if building.profit_tax_budget_baseline.is_finite() {
            building.profit_tax_budget_baseline
        } else {
            building.operating_budget
        };
        let profit = building.operating_budget - baseline;
        building.last_day_profit = profit;
        let taxable = !building.broken && !building.economy_broken && !building.is_deserted;
        if taxable && tax_rate > 0.0 {
            tax = tax_amount(profit, tax_rate).min(building.operating_budget.max(0.0));
            if tax > 0.0 {
                building.operating_budget -= tax;
            } else {
                tax = 0.0;
            }
        }
    } else {
        building.last_day_profit = 0.0;
    }
    building.profit_tax_budget_baseline = building.operating_budget;
    tax
}

/// One operational hour of production at `building`, run at its hourly phase (`SIM-02-F`):
/// inputs consumed and outputs stocked up to the output buffer. Power plants generate in their
/// electricity phase, explicit work areas through their sites and service stores sell through
/// the service-sales pool (`SIM-02-I`). O(ports).
pub(crate) fn produce_building_hour(catalog: &RuntimeEconomyCatalog, building: &mut Building) {
    let zone = building.zone_type;
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
    if crate::simulation::work_area::profile_kind_uses_explicit_work_area(profile.kind) {
        return;
    }
    if profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER) {
        // Electricity dispatch consumes fuel only for generation demanded this hour.
        return;
    }
    let factors = building_operation_factors(catalog, building, profile);
    let throughput_factor = factors.throughput_factor;
    if profile.kind != EconomyProfileRuntimeKind::ServiceStore {
        building.hourly_economy_activity = throughput_factor;
    }

    consume_hourly_production_inputs(building, profile, throughput_factor);
    if matches!(zone, ZoneType::Commercial | ZoneType::Industrial)
        && profile.kind != EconomyProfileRuntimeKind::ServiceStore
    {
        for output_port in &profile.outputs {
            let hourly_output_units =
                output_port.units_per_day / OPERATIONAL_HOURS_PER_DAY * throughput_factor;
            if hourly_output_units <= 0.0 {
                continue;
            }
            let current = building.inventory_units(output_port.resource_runtime_id);
            let capacity =
                scaled_output_buffer_capacity_units_for_building(building, profile, output_port);
            building.set_inventory_units(
                output_port.resource_runtime_id,
                (current + hourly_output_units).min(capacity),
            );
        }
    }
}

/// Sells output through emergency OWA liquidation while retaining shipment and upkeep reserves.
pub(super) fn liquidate_outputs_until_budget(
    building_idx: usize,
    building: &mut Building,
    catalog: &RuntimeEconomyCatalog,
    reserved_outbound: &[f32],
    resource_count: usize,
    export_multiplier: f32,
    target_budget: f32,
) {
    let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else {
        return;
    };
    if profile.kind == EconomyProfileRuntimeKind::ServiceStore {
        return;
    }
    let target_budget = if target_budget.is_finite() {
        target_budget.max(building.operating_budget)
    } else {
        f32::INFINITY
    };
    for output_port in &profile.outputs {
        if building.operating_budget >= target_budget {
            return;
        }
        let reserved = ShipmentSystem::reservation_slot_for_building(
            building_idx,
            output_port.resource_runtime_id,
            resource_count,
        )
        .and_then(|slot| reserved_outbound.get(slot).copied())
        .unwrap_or(0.0);
        let available =
            (saleable_output_stock(catalog, building, profile, output_port.resource_runtime_id)
                - reserved)
                .max(0.0);
        if available <= 0.0 {
            continue;
        }
        let unit_price = catalog
            .unit_price_for_resource(output_port.resource_runtime_id)
            .unwrap_or_else(|| {
                let resource_id = catalog
                    .resource_id_for_runtime_id(output_port.resource_runtime_id)
                    .unwrap_or("<unknown>");
                panic!(
                    "resource '{resource_id}' used by profile '{}' has no catalog price",
                    profile.id
                )
            })
            * export_multiplier;
        if unit_price <= LIQUIDATION_UNIT_PRICE_EPSILON {
            continue;
        }
        let needed_budget = (target_budget - building.operating_budget).max(0.0);
        let sold_units = if target_budget.is_finite() {
            available.min(needed_budget / unit_price)
        } else {
            available
        };
        if sold_units <= 0.0 {
            continue;
        }
        let revenue = sold_units * unit_price;
        let before = building.operating_budget;
        building.operating_budget += revenue;
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::OutsideSales,
            before,
            building.operating_budget,
        );
        building.revenue += revenue;
        building.remove_inventory_units(output_port.resource_runtime_id, sold_units);
    }
}

fn is_profit_tracked_private_business(
    building: &Building,
    profile: Option<&EconomyProfileRuntime>,
) -> bool {
    let zone_tracked = matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    );
    let explicit_area_tracked = profile.is_some_and(|profile| {
        matches!(
            profile.kind,
            EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
        )
    });
    (zone_tracked || explicit_area_tracked)
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

fn is_active_utility_consumer(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

fn is_taxable_residential_home(building: &Building) -> bool {
    matches!(building.zone_type, ZoneType::Residential)
        && !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

fn private_nonresidential_property_tax_zone(
    building: &Building,
    profile: Option<&EconomyProfileRuntime>,
    is_city_service: bool,
) -> Option<ZoneType> {
    if is_city_service
        || building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
    {
        return None;
    }
    if matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) {
        return Some(building.zone_type);
    }
    if is_explicit_private_industry_profile(profile) {
        return Some(ZoneType::Industrial);
    }
    None
}

fn is_operational_utility_provider(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    profile: &EconomyProfileRuntime,
) -> bool {
    if !matches!(
        profile.kind,
        EconomyProfileRuntimeKind::UtilityProducer | EconomyProfileRuntimeKind::UtilityProcessor
    ) || profile.worker_capacity == 0
    {
        return false;
    }
    if profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER) {
        return building.daily_power_service_units > 0.0;
    }
    building.worker_count > 0
        && building_operation_factors(catalog, building, profile).throughput_factor > 0.0
}

fn building_participates_in_budget_distress(
    building: &Building,
    profile: Option<&EconomyProfileRuntime>,
) -> bool {
    is_private_utility_consumer(building, profile)
}

fn is_private_utility_consumer_zone(zone_type: ZoneType) -> bool {
    matches!(
        zone_type,
        ZoneType::Commercial | ZoneType::Office | ZoneType::Mixed | ZoneType::Industrial
    )
}

fn is_private_utility_consumer(
    building: &Building,
    profile: Option<&EconomyProfileRuntime>,
) -> bool {
    is_private_utility_consumer_zone(building.zone_type)
        || is_explicit_private_industry_profile(profile)
}

fn is_explicit_private_industry_profile(profile: Option<&EconomyProfileRuntime>) -> bool {
    profile.is_some_and(|profile| {
        matches!(
            profile.kind,
            EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
        )
    })
}

fn household_utility_bill_for_service(ledger: &DailyHouseholdLedger, service_idx: usize) -> f32 {
    match service_idx {
        UTILITY_SERVICE_POWER_INDEX => ledger.power_consumption_cost.max(0.0),
        UTILITY_SERVICE_WATER_INDEX => ledger.water_consumption_cost.max(0.0),
        UTILITY_SERVICE_SEWAGE_INDEX => ledger.sewage_consumption_cost.max(0.0),
        _ => 0.0,
    }
}

fn add_household_utility_cost_for_service(
    ledger: &mut DailyHouseholdLedger,
    service_idx: usize,
    cost: f32,
) {
    let cost = cost.max(0.0);
    match service_idx {
        UTILITY_SERVICE_POWER_INDEX => ledger.power_consumption_cost += cost,
        UTILITY_SERVICE_WATER_INDEX => ledger.water_consumption_cost += cost,
        UTILITY_SERVICE_SEWAGE_INDEX => ledger.sewage_consumption_cost += cost,
        _ => {}
    }
}

fn household_owa_surcharge_multiplier(coverage: f32, owa_import_price_multiplier: f32) -> f32 {
    let missing_coverage = 1.0 - coverage.clamp(0.0, 1.0);
    (owa_import_price_multiplier.max(0.0) - 1.0).max(0.0) * missing_coverage
}
