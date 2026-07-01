//! Building-side production, utility settlement, bankruptcy, and liquidation.

use super::HouseholdSystem;
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, economy_profile_for_building,
    refresh_commercial_activity_floor,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::tax_amount;
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
const POWER_PRICE_EPSILON: f32 = 0.0001;
const POWER_UNIT_EPSILON: f32 = 0.0001;
const NON_RESIDENTIAL_POWER_DEMAND_UNITS_PER_DAY: f32 = 1.0;
const CITY_SERVICE_POWER_DEMAND_UNITS_PER_DAY: f32 = 1.0;

#[derive(Clone, Copy, Debug, Default)]
struct PowerSettlement {
    demand_units: f32,
    supply_units: f32,
    coverage: f32,
    household_bill: f32,
    household_local_revenue: f32,
    private_local_revenue: f32,
    private_owa_cost: f32,
    city_service_demand_units: f32,
    city_service_local_cost: f32,
    city_service_owa_cost: f32,
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
    /// Phase 1 (find utility providers) and Phase 3 (distribute local revenue to providers)
    /// are retained from the old hourly system. Phase 2 is now a flat daily deduction.
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
        let mut private_power_demand_units = 0.0f32;
        let mut city_service_power_demand_units = 0.0f32;

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if !is_active_utility_consumer(building) {
                continue;
            }
            if private_utility_owa_cost_per_day(
                building.zone_type,
                tuning.commercial_owa_utility_cost_per_day,
                tuning.industrial_owa_utility_cost_per_day,
            )
            .is_some()
            {
                private_power_demand_units += NON_RESIDENTIAL_POWER_DEMAND_UNITS_PER_DAY;
            } else if allocator.is_city_service_building(building) && building.worker_count > 0 {
                city_service_power_demand_units += CITY_SERVICE_POWER_DEMAND_UNITS_PER_DAY;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
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

        // Phase 2 (daily): charge private non-residential buildings their daily utility
        // cost unconditionally. Budget may go negative.
        let mut local_utility_revenue_by_service = [0.0f32; UTILITY_SERVICE_COUNT];
        let power_unit_price = local_utility_costs[UTILITY_SERVICE_POWER_INDEX];
        let household_power_bill = self
            .daily_ledgers
            .iter()
            .map(|ledger| ledger.power_consumption_cost.max(0.0))
            .sum::<f32>();
        let household_water_bill = self
            .daily_ledgers
            .iter()
            .map(|ledger| ledger.water_consumption_cost.max(0.0))
            .sum::<f32>();
        let household_sewage_bill = self
            .daily_ledgers
            .iter()
            .map(|ledger| ledger.sewage_consumption_cost.max(0.0))
            .sum::<f32>();
        let household_power_demand_units =
            power_demand_units_from_cost(household_power_bill, power_unit_price);
        let mut power_settlement = power_settlement(
            power_supply_units,
            household_power_demand_units,
            household_power_bill,
            private_power_demand_units,
            city_service_power_demand_units,
            power_unit_price,
            tuning.owa_import_price_multiplier,
        );
        if power_settlement.household_local_revenue > 0.0 {
            local_utility_revenue_by_service[UTILITY_SERVICE_POWER_INDEX] +=
                power_settlement.household_local_revenue;
        }
        if household_water_bill > 0.0
            && !utility_provider_indices[UTILITY_SERVICE_WATER_INDEX].is_empty()
        {
            local_utility_revenue_by_service[UTILITY_SERVICE_WATER_INDEX] += household_water_bill;
        }
        if household_sewage_bill > 0.0
            && !utility_provider_indices[UTILITY_SERVICE_SEWAGE_INDEX].is_empty()
        {
            local_utility_revenue_by_service[UTILITY_SERVICE_SEWAGE_INDEX] += household_sewage_bill;
        }
        if power_settlement.city_service_local_cost > 0.0 {
            local_utility_revenue_by_service[UTILITY_SERVICE_POWER_INDEX] +=
                power_settlement.city_service_local_cost;
            *treasury_balance -= power_settlement.city_service_local_cost as f64;
        }
        if power_settlement.city_service_owa_cost > 0.0 {
            *treasury_balance -= power_settlement.city_service_owa_cost as f64;
        }
        let served_power_units = power_settlement.demand_units * power_settlement.coverage;
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
            let Some(owa_total) = private_utility_owa_cost_per_day(
                building.zone_type,
                tuning.commercial_owa_utility_cost_per_day,
                tuning.industrial_owa_utility_cost_per_day,
            ) else {
                continue;
            };
            let owa_per_service = owa_total / UTILITY_SERVICE_COUNT as f32;
            let mut daily_cost = 0.0f32;
            for service_idx in 0..UTILITY_SERVICE_COUNT {
                if service_idx == UTILITY_SERVICE_POWER_INDEX {
                    let power_demand_units = NON_RESIDENTIAL_POWER_DEMAND_UNITS_PER_DAY;
                    let local_cost =
                        power_unit_price * power_demand_units * power_settlement.coverage;
                    let owa_cost = owa_per_service * (1.0 - power_settlement.coverage);
                    daily_cost += local_cost + owa_cost;
                    local_utility_revenue_by_service[service_idx] += local_cost;
                    power_settlement.private_local_revenue += local_cost;
                    power_settlement.private_owa_cost += owa_cost;
                    continue;
                }
                if utility_provider_indices[service_idx].is_empty() {
                    daily_cost += owa_per_service;
                } else {
                    let local_cost = local_utility_costs[service_idx];
                    daily_cost += local_cost;
                    local_utility_revenue_by_service[service_idx] += local_cost;
                }
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
        if power_settlement.demand_units > 0.0 || power_settlement.supply_units > 0.0 {
            debug_log!(
                "economy",
                "power settlement: demand_units={:.2} production_units={:.2} served_units={:.2} coverage={:.3} providers={} household_bill={:.2} household_local={:.2} private_local={:.2} private_owa={:.2} city_service_demand={:.2} city_service_local={:.2} city_service_owa={:.2}",
                power_settlement.demand_units,
                power_settlement.supply_units,
                served_power_units,
                power_settlement.coverage,
                utility_provider_indices[UTILITY_SERVICE_POWER_INDEX].len(),
                power_settlement.household_bill,
                power_settlement.household_local_revenue,
                power_settlement.private_local_revenue,
                power_settlement.private_owa_cost,
                power_settlement.city_service_demand_units,
                power_settlement.city_service_local_cost,
                power_settlement.city_service_owa_cost,
            );
        }

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
                    building_participates_in_budget_distress(building)
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
        for building in &mut allocator.buildings {
            let tracked_business = matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial
            ) && !building.is_under_construction()
                && building.edge_idx != usize::MAX;
            let taxable = matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial
            ) && !building.broken
                && !building.economy_broken
                && !building.is_deserted
                && !building.is_under_construction()
                && building.edge_idx != usize::MAX;

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

    pub(super) fn run_building_economy(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        refresh_commercial_activity_floor(&catalog, &self.households, allocator);
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
            if matches!(zone, ZoneType::Commercial | ZoneType::Industrial) {
                for output_port in &profile.outputs {
                    let hourly_output_units =
                        output_port.units_per_day / OPERATIONAL_HOURS_PER_DAY * throughput_factor;
                    if hourly_output_units <= 0.0 {
                        continue;
                    }
                    let current = building.inventory_units(output_port.resource_runtime_id);
                    let capacity = profile.output_buffer_capacity_units_for(output_port);
                    building.set_inventory_units(
                        output_port.resource_runtime_id,
                        (current + hourly_output_units).min(capacity),
                    );
                }
            }
        });
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
    let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else {
        return;
    };
    for output_port in &profile.outputs {
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
        let revenue = available * unit_price;
        building.operating_budget += revenue;
        building.revenue += revenue;
        building.remove_inventory_units(output_port.resource_runtime_id, available);
    }
}

fn is_active_utility_consumer(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
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

fn power_demand_units_from_cost(cost: f32, unit_price: f32) -> f32 {
    if unit_price <= POWER_PRICE_EPSILON {
        0.0
    } else {
        cost.max(0.0) / unit_price
    }
}

fn power_settlement(
    supply_units: f32,
    household_demand_units: f32,
    household_bill: f32,
    private_demand_units: f32,
    city_service_demand_units: f32,
    unit_price: f32,
    owa_import_price_multiplier: f32,
) -> PowerSettlement {
    let demand_units = household_demand_units + private_demand_units + city_service_demand_units;
    let coverage = if demand_units <= 0.0 {
        0.0
    } else {
        (supply_units.max(0.0) / demand_units).clamp(0.0, 1.0)
    };
    let household_local_revenue = household_bill.max(0.0) * coverage;
    let city_service_local_cost = city_service_demand_units * unit_price.max(0.0) * coverage;
    let city_service_owa_cost = city_service_demand_units
        * unit_price.max(0.0)
        * owa_import_price_multiplier.max(0.0)
        * (1.0 - coverage);
    PowerSettlement {
        demand_units,
        supply_units: supply_units.max(0.0),
        coverage,
        household_bill,
        household_local_revenue,
        private_local_revenue: 0.0,
        private_owa_cost: 0.0,
        city_service_demand_units,
        city_service_local_cost,
        city_service_owa_cost,
    }
}

fn building_participates_in_budget_distress(building: &Building) -> bool {
    private_utility_owa_cost_per_day(building.zone_type, 0.0, 0.0).is_some()
}

fn private_utility_owa_cost_per_day(
    zone_type: ZoneType,
    commercial_owa_cost_per_day: f32,
    industrial_owa_cost_per_day: f32,
) -> Option<f32> {
    match zone_type {
        ZoneType::Commercial | ZoneType::Office | ZoneType::Mixed => {
            Some(commercial_owa_cost_per_day)
        }
        ZoneType::Industrial => Some(industrial_owa_cost_per_day),
        ZoneType::None | ZoneType::Residential => None,
    }
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
