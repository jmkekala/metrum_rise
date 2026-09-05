// SPDX-License-Identifier: GPL-2.0-only

//! Building-side production, utility settlement, bankruptcy, and liquidation.

use super::data::DailyHouseholdLedger;
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, economy_profile_for_building,
    household_is_housed, refresh_commercial_activity_floor,
    scaled_output_buffer_capacity_units_for_building,
};
use super::{DailyPowerSettlementSummary, HouseholdSystem};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{
    CityFiscalPolicy, FiscalRevenue, daily_property_tax, tax_amount,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const UTILITY_SERVICE_POWER: &str = "power";
const UTILITY_SERVICE_WATER: &str = "water";
const UTILITY_SERVICE_SEWAGE: &str = "sewage";
const UTILITY_SERVICES: [&str; UTILITY_SERVICE_COUNT] = [
    UTILITY_SERVICE_POWER,
    UTILITY_SERVICE_WATER,
    UTILITY_SERVICE_SEWAGE,
];
const UTILITY_SERVICE_POWER_INDEX: usize = 0;
const UTILITY_SERVICE_WATER_INDEX: usize = 1;
const UTILITY_SERVICE_SEWAGE_INDEX: usize = 2;
const UTILITY_SERVICE_COUNT: usize = 3;
const UTILITY_PRICE_EPSILON: f32 = 0.0001;
const POWER_UNIT_EPSILON: f32 = 0.0001;
const LIQUIDATION_UNIT_PRICE_EPSILON: f32 = 0.0001;
const NON_RESIDENTIAL_UTILITY_DEMAND_UNITS_PER_DAY: f32 = 1.0;
const CITY_SERVICE_UTILITY_DEMAND_UNITS_PER_DAY: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default)]
struct UtilityServiceSettlement {
    demand_units: f32,
    supply_units: f32,
    coverage: f32,
    household_base_bill: f32,
    household_local_revenue: f32,
    household_owa_cost: f32,
    household_owa_surcharge: f32,
    private_local_revenue: f32,
    private_owa_cost: f32,
    city_service_demand_units: f32,
    city_service_local_cost: f32,
    city_service_owa_cost: f32,
}

#[derive(Clone, Copy, Debug)]
struct ServiceStoreCapacity {
    building_idx: usize,
    resource_runtime_id: ResourceRuntimeId,
    capacity_units_per_hour: f32,
}

impl HouseholdSystem {
    pub(super) fn run_bankruptcy_check(&mut self, allocator: &mut BuildingAllocator) {
        let registry = &allocator.registry;
        for building in &mut allocator.buildings {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || registry.is_city_service_asset(&building.asset_id)
            {
                continue;
            }
            if building.budget_distress && building.operating_budget < 0.0 {
                building.is_deserted = true;
                debug_log!(
                    "economy",
                    "building asset={} bankrupt: budget_distress=true budget={:.2}",
                    building.asset_id,
                    building.operating_budget
                );
            }
        }
    }

    /// Steps 3 + 4: deduct daily utility costs then perform forced OWA liquidation for any
    /// building whose budget went negative.
    ///
    /// Phase 1 finds active utility providers, Phase 2 settles each service against local
    /// coverage or OWA fallback, and Phase 3 distributes local revenue to providers.
    pub(super) fn settle_daily_utilities(
        &mut self,
        allocator: &mut BuildingAllocator,
        logistics: &ShipmentSystem,
        treasury_balance: &mut f64,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.ensure_daily_ledger_len();

        for building in &mut allocator.buildings {
            building.daily_power_served_units = 0.0;
        }

        // Phase 1: find operational utility provider buildings.
        let mut utility_provider_indices: [Vec<usize>; UTILITY_SERVICE_COUNT] =
            [Vec::new(), Vec::new(), Vec::new()];
        let local_utility_costs = local_utility_service_costs(&catalog);
        let mut power_supply_units = 0.0f32;
        let mut private_utility_demand_units_by_service = [0.0f32; UTILITY_SERVICE_COUNT];
        let mut city_service_utility_demand_units_by_service = [0.0f32; UTILITY_SERVICE_COUNT];

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if !is_active_utility_consumer(building) {
                continue;
            }
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
            if is_private_utility_consumer(building, profile) {
                for units in &mut private_utility_demand_units_by_service {
                    *units += NON_RESIDENTIAL_UTILITY_DEMAND_UNITS_PER_DAY;
                }
            } else if allocator.is_city_service_building(building) && building.worker_count > 0 {
                for units in &mut city_service_utility_demand_units_by_service {
                    *units += CITY_SERVICE_UTILITY_DEMAND_UNITS_PER_DAY;
                }
            }
            let Some(profile) = profile else {
                continue;
            };
            if !is_operational_utility_provider(&catalog, building, profile) {
                continue;
            }
            if let Some(service_idx) = utility_service_index(profile.utility_service.as_deref()) {
                if service_idx == UTILITY_SERVICE_POWER_INDEX {
                    let produced_units = building.daily_power_service_units.max(0.0);
                    if produced_units <= 0.0 {
                        continue;
                    }
                    power_supply_units += produced_units;
                }
                utility_provider_indices[service_idx].push(idx);
            }
        }

        // Phase 2 (daily): households, private buildings, and city services all resolve
        // through the same per-service local-or-OWA pricing model.
        let mut local_utility_revenue_by_service = [0.0f32; UTILITY_SERVICE_COUNT];
        let household_bills_by_service = [
            household_utility_bill_by_service(&self.daily_ledgers, UTILITY_SERVICE_POWER_INDEX),
            household_utility_bill_by_service(&self.daily_ledgers, UTILITY_SERVICE_WATER_INDEX),
            household_utility_bill_by_service(&self.daily_ledgers, UTILITY_SERVICE_SEWAGE_INDEX),
        ];
        let mut utility_settlements = [UtilityServiceSettlement::default(); UTILITY_SERVICE_COUNT];
        for service_idx in 0..UTILITY_SERVICE_COUNT {
            let unit_price = local_utility_costs[service_idx];
            let household_demand_units =
                utility_demand_units_from_cost(household_bills_by_service[service_idx], unit_price);
            let private_demand_units = private_utility_demand_units_by_service[service_idx];
            let city_service_demand_units =
                city_service_utility_demand_units_by_service[service_idx];
            let demand_units =
                household_demand_units + private_demand_units + city_service_demand_units;
            let supply_units = if service_idx == UTILITY_SERVICE_POWER_INDEX {
                power_supply_units
            } else if utility_provider_indices[service_idx].is_empty() {
                0.0
            } else {
                demand_units
            };
            utility_settlements[service_idx] = utility_service_settlement(
                supply_units,
                household_bills_by_service[service_idx],
                private_demand_units,
                city_service_demand_units,
                unit_price,
                tuning.owa_import_price_multiplier,
            );
        }
        for service_idx in 0..UTILITY_SERVICE_COUNT {
            let household_owa_surcharge = self.apply_household_utility_owa_surcharge(
                service_idx,
                utility_settlements[service_idx].coverage,
                tuning.owa_import_price_multiplier,
            );
            utility_settlements[service_idx].household_owa_surcharge = household_owa_surcharge;
            if utility_settlements[service_idx].household_local_revenue > 0.0 {
                local_utility_revenue_by_service[service_idx] +=
                    utility_settlements[service_idx].household_local_revenue;
            }
            if utility_settlements[service_idx].city_service_local_cost > 0.0 {
                local_utility_revenue_by_service[service_idx] +=
                    utility_settlements[service_idx].city_service_local_cost;
                *treasury_balance -=
                    utility_settlements[service_idx].city_service_local_cost as f64;
            }
            if utility_settlements[service_idx].city_service_owa_cost > 0.0 {
                *treasury_balance -= utility_settlements[service_idx].city_service_owa_cost as f64;
            }
        }
        let served_power_units = utility_settlements[UTILITY_SERVICE_POWER_INDEX].demand_units
            * utility_settlements[UTILITY_SERVICE_POWER_INDEX].coverage;
        if power_supply_units > POWER_UNIT_EPSILON && served_power_units > 0.0 {
            for &idx in &utility_provider_indices[UTILITY_SERVICE_POWER_INDEX] {
                let produced_units = allocator.buildings[idx].daily_power_service_units.max(0.0);
                let served_units = served_power_units * (produced_units / power_supply_units);
                allocator.buildings[idx].daily_power_served_units =
                    served_units.clamp(0.0, produced_units);
            }
        }

        for building in &mut allocator.buildings {
            if !is_active_utility_consumer(building) {
                continue;
            }
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
            if !is_private_utility_consumer(building, profile) {
                continue;
            }
            let mut daily_cost = 0.0f32;
            for service_idx in 0..UTILITY_SERVICE_COUNT {
                let settlement = &mut utility_settlements[service_idx];
                let unit_price = local_utility_costs[service_idx].max(0.0);
                let demand_units = NON_RESIDENTIAL_UTILITY_DEMAND_UNITS_PER_DAY;
                let coverage = settlement.coverage.clamp(0.0, 1.0);
                let local_cost = unit_price * demand_units * coverage;
                let owa_cost = unit_price
                    * demand_units
                    * tuning.owa_import_price_multiplier.max(0.0)
                    * (1.0 - coverage);
                daily_cost += local_cost + owa_cost;
                local_utility_revenue_by_service[service_idx] += local_cost;
                settlement.private_local_revenue += local_cost;
                settlement.private_owa_cost += owa_cost;
            }
            building.operating_budget -= daily_cost;
        }

        // Phase 3: local utility fees go to the city treasury; provider `revenue`
        // remains telemetry and does not fund municipal payroll.
        let mut local_utility_revenue_total = 0.0f32;
        for service_idx in 0..UTILITY_SERVICE_COUNT {
            let revenue = local_utility_revenue_by_service[service_idx];
            if revenue <= 0.0 || utility_provider_indices[service_idx].is_empty() {
                continue;
            }
            local_utility_revenue_total += revenue;
            let share = revenue / utility_provider_indices[service_idx].len() as f32;
            for &idx in &utility_provider_indices[service_idx] {
                allocator.buildings[idx].revenue += share;
            }
        }
        *treasury_balance += local_utility_revenue_total as f64;
        let city_service_utility_local_cost_total = utility_settlements
            .iter()
            .map(|settlement| settlement.city_service_local_cost.max(0.0))
            .sum::<f32>();
        let city_service_utility_owa_cost_total = utility_settlements
            .iter()
            .map(|settlement| settlement.city_service_owa_cost.max(0.0))
            .sum::<f32>();
        let power_settlement = utility_settlements[UTILITY_SERVICE_POWER_INDEX];
        if power_settlement.demand_units > 0.0 || power_settlement.supply_units > 0.0 {
            debug_log!(
                "economy",
                "power settlement: demand_units={:.2} production_units={:.2} served_units={:.2} coverage={:.3} providers={} household_bill={:.2} household_local={:.2} household_owa={:.2} private_local={:.2} private_owa={:.2} city_service_demand={:.2} city_service_local={:.2} city_service_owa={:.2}",
                power_settlement.demand_units,
                power_settlement.supply_units,
                served_power_units,
                power_settlement.coverage,
                utility_provider_indices[UTILITY_SERVICE_POWER_INDEX].len(),
                power_settlement.household_base_bill + power_settlement.household_owa_surcharge,
                power_settlement.household_local_revenue,
                power_settlement.household_owa_cost,
                power_settlement.private_local_revenue,
                power_settlement.private_owa_cost,
                power_settlement.city_service_demand_units,
                power_settlement.city_service_local_cost,
                power_settlement.city_service_owa_cost,
            );
        }
        self.last_power_settlement = DailyPowerSettlementSummary {
            demand_units: power_settlement.demand_units,
            supply_units: power_settlement.supply_units,
            served_units: served_power_units,
            coverage: power_settlement.coverage,
            household_local_revenue: power_settlement.household_local_revenue,
            private_local_revenue: power_settlement.private_local_revenue,
            city_service_local_cost: power_settlement.city_service_local_cost,
            utility_local_revenue: local_utility_revenue_total,
            city_service_utility_local_cost: city_service_utility_local_cost_total,
            city_service_utility_owa_cost: city_service_utility_owa_cost_total,
        };

        let resource_count = catalog.resource_count();
        let reserved_outbound = logistics.reserved_outbound_view(resource_count);

        // Step 4: distress resolution — forced OWA liquidation for buildings that went negative.
        for idx in 0..allocator.buildings.len() {
            let participates = {
                let building = &allocator.buildings[idx];
                if building.is_deserted
                    || building.broken
                    || building.economy_broken
                    || building.is_under_construction()
                {
                    false
                } else {
                    let profile =
                        catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
                    building_participates_in_budget_distress(building, profile)
                }
            };
            if !participates {
                continue;
            }
            let building = &mut allocator.buildings[idx];
            if building.operating_budget < 0.0 {
                forced_owa_liquidation(
                    idx,
                    building,
                    &catalog,
                    &reserved_outbound,
                    resource_count,
                    tuning.owa_distress_liquidation_multiplier,
                );
                building.budget_distress = true;
                debug_log!(
                    "economy",
                    "building asset={} in distress: budget={:.2} after liquidation",
                    building.asset_id,
                    building.operating_budget
                );
            } else {
                building.budget_distress = false;
            }
        }
    }

    /// Collects modeled daily property tax from occupied homes and private non-residential sites.
    pub(super) fn settle_daily_property_tax(
        &mut self,
        allocator: &mut BuildingAllocator,
        fiscal_policy: &CityFiscalPolicy,
    ) -> FiscalRevenue {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.ensure_daily_ledger_len();
        let mut revenue = FiscalRevenue::default();

        for (household_id, household) in self.households.iter_mut().enumerate() {
            if household.member_count == 0 {
                continue;
            }
            let Some(home) = allocator.buildings.get(household.home_building_id) else {
                continue;
            };
            if !is_taxable_residential_home(home) {
                continue;
            }
            let tax = daily_property_tax(ZoneType::Residential, home.level, fiscal_policy);
            let paid = tax.min(household.budget.max(0.0));
            if paid <= 0.0 {
                continue;
            }
            household.budget -= paid;
            if let Some(ledger) = self.daily_ledgers.get_mut(household_id) {
                ledger.property_tax_paid += paid;
            }
            revenue.residential_property_tax += paid;
        }

        for building_idx in 0..allocator.buildings.len() {
            let is_city_service = allocator
                .registry
                .is_city_service_asset(&allocator.buildings[building_idx].asset_id);
            let building = &mut allocator.buildings[building_idx];
            let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
            let Some(tax_zone) =
                private_nonresidential_property_tax_zone(building, profile, is_city_service)
            else {
                continue;
            };
            let tax = daily_property_tax(tax_zone, building.level, fiscal_policy);
            if tax <= 0.0 {
                continue;
            }
            building.operating_budget -= tax;
            match tax_zone {
                ZoneType::Commercial => revenue.commercial_property_tax += tax,
                ZoneType::Industrial => revenue.industrial_property_tax += tax,
                _ => {}
            }
        }

        revenue
    }

    /// Collects daily business profit tax from positive commercial/industrial budget growth.
    ///
    /// The baseline is reset after each daily settlement so the tax applies to today's net
    /// operating-budget increase only, after wages, utilities, freight, shopping, and liquidation.
    /// It never debits below zero; distress handling has already run for the day.
    pub(super) fn settle_business_profit_tax(
        &mut self,
        allocator: &mut BuildingAllocator,
        tax_rate: f32,
    ) -> f32 {
        let mut total_tax = 0.0f32;
        let catalog = load_runtime_economy_catalog().ok();
        for building in &mut allocator.buildings {
            let profile = catalog.as_ref().and_then(|catalog| {
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            });
            let tracked_business = is_profit_tracked_private_business(building, profile);
            let taxable = tracked_business
                && !building.broken
                && !building.economy_broken
                && !building.is_deserted;

            if tracked_business {
                let baseline = if building.profit_tax_budget_baseline.is_finite() {
                    building.profit_tax_budget_baseline
                } else {
                    building.operating_budget
                };
                let profit = building.operating_budget - baseline;
                building.last_day_profit = profit;
                if taxable {
                    let tax = if tax_rate > 0.0 {
                        tax_amount(profit, tax_rate).min(building.operating_budget.max(0.0))
                    } else {
                        0.0
                    };
                    if tax > 0.0 {
                        building.operating_budget -= tax;
                        total_tax += tax;
                    }
                }
            } else {
                building.last_day_profit = 0.0;
            }
            building.profit_tax_budget_baseline = building.operating_budget;
        }
        total_tax
    }

    pub(super) fn run_building_economy(
        &mut self,
        allocator: &mut BuildingAllocator,
        owa_exports_available: bool,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        refresh_commercial_activity_floor(
            &catalog,
            &self.households,
            allocator,
            owa_exports_available,
        );
        self.settle_hourly_service_store_sales(allocator, &catalog);
        allocator.buildings.par_iter_mut().for_each(|building| {
            let zone = building.zone_type;
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                return;
            }
            let Some(profile) = economy_profile_for_building(&catalog, building) else {
                return;
            };
            if profile.kind == EconomyProfileRuntimeKind::FieldProducer {
                return;
            }
            let factors = building_operation_factors(&catalog, building, profile);
            let throughput_factor = factors.throughput_factor;
            if profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER)
                && profile.base_rate_units_per_day > 0.0
            {
                let hourly_power_units =
                    profile.base_rate_units_per_day / OPERATIONAL_HOURS_PER_DAY * throughput_factor;
                if hourly_power_units > 0.0 {
                    building.daily_power_service_units += hourly_power_units;
                }
            }

            for input_port in &profile.inputs {
                let hourly_input_units =
                    input_port.units_per_day / OPERATIONAL_HOURS_PER_DAY * throughput_factor;
                if hourly_input_units > 0.0 {
                    building
                        .remove_inventory_units(input_port.resource_runtime_id, hourly_input_units);
                }
            }
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
                    let capacity = scaled_output_buffer_capacity_units_for_building(
                        building,
                        profile,
                        output_port,
                    );
                    building.set_inventory_units(
                        output_port.resource_runtime_id,
                        (current + hourly_output_units).min(capacity),
                    );
                }
            }
        });
    }

    fn settle_hourly_service_store_sales(
        &mut self,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let demand_rates = service_store_demand_rates_by_resource(catalog);
        if demand_rates.is_empty() {
            return;
        }
        let housed_residents: u32 = self
            .households
            .par_iter()
            .filter(|household| household_is_housed(household, allocator))
            .map(|household| u32::from(household.member_count))
            .sum();
        if housed_residents == 0 {
            return;
        }
        let capacities = service_store_hourly_capacities(allocator, catalog);
        if capacities.is_empty() {
            return;
        }

        let mut capacity_by_resource = Vec::new();
        for capacity in &capacities {
            add_resource_amount(
                &mut capacity_by_resource,
                capacity.resource_runtime_id,
                capacity.capacity_units_per_hour,
            );
        }

        let mut gross_revenue_by_resource = Vec::new();
        let mut gross_revenue = 0.0_f32;
        for &(resource_runtime_id, rate_per_resident_per_day) in &demand_rates {
            let hourly_demand_units = housed_residents as f32 * rate_per_resident_per_day.max(0.0)
                / OPERATIONAL_HOURS_PER_DAY;
            if hourly_demand_units <= 0.0 {
                continue;
            }
            let capacity_units =
                resource_amount(&capacity_by_resource, resource_runtime_id).max(0.0);
            if capacity_units <= 0.0 {
                continue;
            }
            let served_units = hourly_demand_units.min(capacity_units);
            let unit_price = catalog
                .unit_price_for_resource(resource_runtime_id)
                .unwrap_or_else(|| {
                    let resource_id = catalog
                        .resource_id_for_runtime_id(resource_runtime_id)
                        .unwrap_or("<unknown>");
                    panic!("service resource '{resource_id}' has no catalog price")
                });
            let revenue = served_units * unit_price.max(0.0);
            if revenue <= 0.0 {
                continue;
            }
            gross_revenue += revenue;
            add_resource_amount(&mut gross_revenue_by_resource, resource_runtime_id, revenue);
        }
        if gross_revenue <= 0.0 {
            return;
        }

        let paid_revenue =
            self.charge_households_for_aggregate_service_sales(allocator, gross_revenue);
        if paid_revenue <= 0.0 {
            return;
        }
        let paid_scale = (paid_revenue / gross_revenue).clamp(0.0, 1.0);
        for capacity in capacities {
            let total_resource_capacity =
                resource_amount(&capacity_by_resource, capacity.resource_runtime_id);
            if total_resource_capacity <= 0.0 {
                continue;
            }
            let resource_revenue =
                resource_amount(&gross_revenue_by_resource, capacity.resource_runtime_id);
            if resource_revenue <= 0.0 {
                continue;
            }
            let building_revenue = resource_revenue * paid_scale * capacity.capacity_units_per_hour
                / total_resource_capacity;
            if building_revenue <= 0.0 {
                continue;
            }
            if let Some(building) = allocator.buildings.get_mut(capacity.building_idx) {
                building.revenue += building_revenue;
                building.operating_budget += building_revenue;
                building.daily_household_sales_value += building_revenue;
            }
        }
    }

    fn charge_households_for_aggregate_service_sales(
        &mut self,
        allocator: &BuildingAllocator,
        gross_revenue: f32,
    ) -> f32 {
        self.ensure_daily_ledger_len();
        let total_housed_residents: u32 = self
            .households
            .iter()
            .filter(|household| household_is_housed(household, allocator))
            .map(|household| u32::from(household.member_count))
            .sum();
        if total_housed_residents == 0 {
            return 0.0;
        }

        let mut paid_total = 0.0_f32;
        for household_id in 0..self.households.len() {
            if !household_is_housed(&self.households[household_id], allocator) {
                continue;
            }
            let share = gross_revenue * self.households[household_id].member_count as f32
                / total_housed_residents as f32;
            if share <= 0.0 {
                continue;
            }
            let paid = share.min(self.households[household_id].budget.max(0.0));
            if paid <= 0.0 {
                continue;
            }
            self.households[household_id].budget -= paid;
            if let Some(ledger) = self.daily_ledgers.get_mut(household_id) {
                ledger.shopping_spend += paid;
            }
            paid_total += paid;
        }
        paid_total
    }

    fn apply_household_utility_owa_surcharge(
        &mut self,
        service_idx: usize,
        coverage: f32,
        owa_import_price_multiplier: f32,
    ) -> f32 {
        let surcharge_multiplier =
            household_owa_surcharge_multiplier(coverage, owa_import_price_multiplier);
        if surcharge_multiplier <= 0.0 {
            return 0.0;
        }
        let mut surcharge_total = 0.0f32;
        for household_id in 0..self.households.len().min(self.daily_ledgers.len()) {
            let base_bill =
                household_utility_bill_for_service(&self.daily_ledgers[household_id], service_idx);
            let surcharge = base_bill * surcharge_multiplier;
            if surcharge <= 0.0 {
                continue;
            }
            self.households[household_id].budget =
                (self.households[household_id].budget - surcharge).max(0.0);
            let ledger = &mut self.daily_ledgers[household_id];
            add_household_utility_cost_for_service(ledger, service_idx, surcharge);
            ledger.utility_stock_consumption_cost += surcharge;
            surcharge_total += surcharge;
        }
        surcharge_total
    }
}

fn service_store_demand_rates_by_resource(
    catalog: &RuntimeEconomyCatalog,
) -> Vec<(ResourceRuntimeId, f32)> {
    let mut service_outputs = Vec::new();
    for profile in catalog.all_profiles() {
        if profile.kind != EconomyProfileRuntimeKind::ServiceStore {
            continue;
        }
        for output in &profile.outputs {
            add_resource_amount(&mut service_outputs, output.resource_runtime_id, 1.0);
        }
    }
    if service_outputs.is_empty() {
        return Vec::new();
    }

    let mut demand_rates = Vec::new();
    for profile in catalog.all_profiles() {
        if profile.kind != EconomyProfileRuntimeKind::DemandSink {
            continue;
        }
        for input in &profile.inputs {
            if resource_amount(&service_outputs, input.resource_runtime_id) <= 0.0 {
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

fn service_store_hourly_capacities(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) -> Vec<ServiceStoreCapacity> {
    let mut capacities = allocator
        .buildings
        .par_iter()
        .enumerate()
        .fold(Vec::new, |mut local, (building_idx, building)| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
                || !matches!(building.zone_type, ZoneType::Commercial)
            {
                return local;
            }
            let Some(profile) = economy_profile_for_building(catalog, building) else {
                return local;
            };
            if profile.kind != EconomyProfileRuntimeKind::ServiceStore {
                return local;
            }
            let factors = building_operation_factors(catalog, building, profile);
            if factors.throughput_factor <= 0.0 {
                return local;
            }
            for output in &profile.outputs {
                let capacity_units_per_hour = output.units_per_day.max(0.0)
                    / OPERATIONAL_HOURS_PER_DAY
                    * factors.throughput_factor;
                if capacity_units_per_hour <= 0.0 {
                    continue;
                }
                local.push(ServiceStoreCapacity {
                    building_idx,
                    resource_runtime_id: output.resource_runtime_id,
                    capacity_units_per_hour,
                });
            }
            local
        })
        .reduce(Vec::new, |mut left, mut right| {
            left.append(&mut right);
            left
        });
    capacities
        .sort_unstable_by_key(|capacity| (capacity.resource_runtime_id, capacity.building_idx));
    capacities
}

fn add_resource_amount(
    amounts: &mut Vec<(ResourceRuntimeId, f32)>,
    resource_runtime_id: ResourceRuntimeId,
    amount: f32,
) {
    if amount <= 0.0 {
        return;
    }
    if let Some((_, existing)) = amounts
        .iter_mut()
        .find(|(resource, _)| *resource == resource_runtime_id)
    {
        *existing += amount;
    } else {
        amounts.push((resource_runtime_id, amount));
    }
}

fn resource_amount(
    amounts: &[(ResourceRuntimeId, f32)],
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    amounts
        .iter()
        .find_map(|(resource, amount)| (*resource == resource_runtime_id).then_some(*amount))
        .unwrap_or(0.0)
}

/// Sells unreserved output inventory through the emergency OWA liquidation path.
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
            (building.inventory_units(output_port.resource_runtime_id) - reserved).max(0.0);
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
        building.operating_budget += revenue;
        building.revenue += revenue;
        building.remove_inventory_units(output_port.resource_runtime_id, sold_units);
    }
}

fn forced_owa_liquidation(
    building_idx: usize,
    building: &mut Building,
    catalog: &RuntimeEconomyCatalog,
    reserved_outbound: &[f32],
    resource_count: usize,
    export_multiplier: f32,
) {
    liquidate_outputs_until_budget(
        building_idx,
        building,
        catalog,
        reserved_outbound,
        resource_count,
        export_multiplier,
        f32::INFINITY,
    );
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

fn utility_demand_units_from_cost(cost: f32, unit_price: f32) -> f32 {
    if unit_price <= UTILITY_PRICE_EPSILON {
        0.0
    } else {
        cost.max(0.0) / unit_price
    }
}

fn utility_service_settlement(
    supply_units: f32,
    household_base_bill: f32,
    private_demand_units: f32,
    city_service_demand_units: f32,
    unit_price: f32,
    owa_import_price_multiplier: f32,
) -> UtilityServiceSettlement {
    let household_demand_units = utility_demand_units_from_cost(household_base_bill, unit_price);
    let demand_units = household_demand_units + private_demand_units + city_service_demand_units;
    let coverage = if demand_units <= 0.0 {
        0.0
    } else {
        (supply_units.max(0.0) / demand_units).clamp(0.0, 1.0)
    };
    let unit_price = unit_price.max(0.0);
    let owa_multiplier = owa_import_price_multiplier.max(0.0);
    let missing_coverage = 1.0 - coverage;
    let household_local_revenue = household_base_bill.max(0.0) * coverage;
    let household_owa_cost = household_base_bill.max(0.0) * owa_multiplier * missing_coverage;
    let household_owa_surcharge =
        household_base_bill.max(0.0) * household_owa_surcharge_multiplier(coverage, owa_multiplier);
    let city_service_local_cost = city_service_demand_units * unit_price * coverage;
    let city_service_owa_cost =
        city_service_demand_units * unit_price * owa_multiplier * missing_coverage;
    UtilityServiceSettlement {
        demand_units,
        supply_units: supply_units.max(0.0),
        coverage,
        household_base_bill,
        household_local_revenue,
        household_owa_cost,
        household_owa_surcharge,
        private_local_revenue: 0.0,
        private_owa_cost: 0.0,
        city_service_demand_units,
        city_service_local_cost,
        city_service_owa_cost,
    }
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

fn household_utility_bill_by_service(ledgers: &[DailyHouseholdLedger], service_idx: usize) -> f32 {
    ledgers
        .iter()
        .map(|ledger| household_utility_bill_for_service(ledger, service_idx))
        .sum()
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

fn local_utility_service_costs(catalog: &RuntimeEconomyCatalog) -> [f32; UTILITY_SERVICE_COUNT] {
    let mut costs = [0.0; UTILITY_SERVICE_COUNT];
    for profile in catalog.all_profiles() {
        if let Some(service_idx) = utility_service_index(profile.utility_service.as_deref())
            && costs[service_idx] == 0.0
        {
            costs[service_idx] = profile.unit_price_currency.max(0.0);
        }
    }
    costs
}

fn utility_service_index(service: Option<&str>) -> Option<usize> {
    let service = service?;
    UTILITY_SERVICES
        .iter()
        .position(|candidate| *candidate == service)
}
