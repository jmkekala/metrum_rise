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

impl HouseholdSystem {
    pub(super) fn run_bankruptcy_check(&mut self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
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
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));

        // Phase 1: find operational utility provider buildings.
        let mut utility_provider_indices: Vec<usize> = Vec::new();
        let mut power_available = false;
        let mut water_available = false;
        let mut sewage_available = false;

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !is_staffed_utility_provider(building, profile) {
                continue;
            }
            utility_provider_indices.push(idx);
            match profile.utility_service.as_deref() {
                Some(UTILITY_SERVICE_POWER) => power_available = true,
                Some(UTILITY_SERVICE_WATER) => water_available = true,
                Some(UTILITY_SERVICE_SEWAGE) => sewage_available = true,
                _ => {}
            }
        }

        let all_local = power_available && water_available && sewage_available;
        let local_utility_total = local_utility_total_cost(&catalog);

        // Phase 2 (daily): charge each commercial/industrial building the full daily utility
        // cost unconditionally. Budget may go negative.
        let mut local_utility_revenue = 0.0f32;

        for building in &mut allocator.buildings {
            if building.is_deserted
                || building.broken
                || building.economy_broken
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                continue;
            }
            let (daily_cost, is_local) = match building.zone_type {
                ZoneType::Commercial => {
                    if all_local {
                        (local_utility_total, true)
                    } else {
                        (tuning.commercial_owa_utility_cost_per_day, false)
                    }
                }
                ZoneType::Industrial => {
                    if all_local {
                        (local_utility_total, true)
                    } else {
                        (tuning.industrial_owa_utility_cost_per_day, false)
                    }
                }
                _ => continue,
            };
            building.operating_budget -= daily_cost;
            if is_local {
                local_utility_revenue += daily_cost;
            }
        }

        // Phase 3: distribute local revenue to utility provider buildings.
        if local_utility_revenue > 0.0 && !utility_provider_indices.is_empty() {
            let share = local_utility_revenue / utility_provider_indices.len() as f32;
            for &idx in &utility_provider_indices {
                allocator.buildings[idx].revenue += share;
                allocator.buildings[idx].operating_budget += share;
            }
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
                    building_participates_in_budget_distress(&catalog, building)
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

fn is_staffed_utility_provider(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(
        profile.kind,
        EconomyProfileRuntimeKind::UtilityProducer | EconomyProfileRuntimeKind::UtilityProcessor
    ) && building.worker_count > 0
        && profile.worker_capacity > 0
}

fn building_participates_in_budget_distress(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> bool {
    if matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) {
        return true;
    }
    catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .is_some_and(|profile| {
            matches!(
                profile.kind,
                EconomyProfileRuntimeKind::UtilityProducer
                    | EconomyProfileRuntimeKind::UtilityProcessor
            )
        })
}

fn local_utility_total_cost(catalog: &RuntimeEconomyCatalog) -> f32 {
    [
        UTILITY_SERVICE_POWER,
        UTILITY_SERVICE_WATER,
        UTILITY_SERVICE_SEWAGE,
    ]
    .into_iter()
    .filter_map(|service| {
        catalog
            .all_profiles()
            .iter()
            .find(|profile| profile.utility_service.as_deref() == Some(service))
            .map(|profile| profile.unit_price_currency)
    })
    .sum()
}
