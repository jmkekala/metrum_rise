// SPDX-License-Identifier: GPL-2.0-only

//! Service-store sales through an hourly money pool (`SIM-02-I`, `docs/economy.md#services`).
//!
//! A housed household pays at its hourly phase, priced by the last finished hour's aggregates:
//! its members' share of the gross revenue that hour's housed residents and store capacity
//! allowed, up to its budget. The money joins this hour's collections. A service store at its
//! hourly phase draws its capacity share of each resource's pool, with the capacity it recorded
//! in the last finished hour, then records this hour's. The hour's last slot splits the
//! collections by resource as they were priced, adds them to what no store drew, and turns the
//! hour's residents and capacity into the next hour's prices and draw shares.
//!
//! O(1) per household and O(outputs) per store at their phases; O(profiles² + resources) at the
//! hour's end over the small catalog. No allocation once the resource vectors are sized.

use super::HouseholdSystem;
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, economy_profile_for_building,
    household_is_housed, service_store_demand_rates_by_resource,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::zoning::ZoneType;

/// Service-sales aggregates and money between hourly phases (`SIM-02-I`). Per-resource vectors
/// hold slot `resource_runtime_id - 1`. Saved, so a game loaded mid-hour continues identically.
#[derive(Clone, Debug, Default)]
pub(crate) struct ServiceSales {
    /// Housed residents the last finished hour counted; this hour's shares divide by it.
    pub(crate) residents: u32,
    /// Hourly gross revenue per resource those residents and the last hour's stores allowed;
    /// this hour's payments are priced by it.
    pub(crate) gross_by_resource: Vec<f32>,
    /// Sum of [`Self::gross_by_resource`].
    pub(crate) gross_revenue: f32,
    /// Store capacity per resource the last finished hour recorded; this hour's draws divide
    /// by it.
    pub(crate) capacity_by_resource: Vec<f32>,
    /// Money per resource the stores share this hour, fixed at the hour's start.
    pub(crate) drawable_by_resource: Vec<f64>,
    /// What of [`Self::drawable_by_resource`] no store has drawn yet.
    pub(crate) undrawn_by_resource: Vec<f64>,
    /// Housed residents counted so far this hour.
    pub(crate) hour_residents: u32,
    /// Store capacity per resource recorded so far this hour.
    pub(crate) hour_capacity_by_resource: Vec<f32>,
    /// Money households have paid so far this hour.
    pub(crate) hour_collected: f64,
    // Reused demand rates for the hour's end.
    demand_rates: Vec<(ResourceRuntimeId, f32)>,
}

impl ServiceSales {
    /// Money held between households and stores: this hour's collections and what stores have
    /// not drawn. Money-conservation checks count it with city cash.
    #[cfg(test)]
    pub(crate) fn money(&self) -> f64 {
        self.hour_collected + self.undrawn_by_resource.iter().sum::<f64>()
    }

    /// Validates a loaded state: equal per-resource lengths, finite money and non-negative
    /// prices and capacities.
    pub(crate) fn is_valid(&self) -> bool {
        let len = self.gross_by_resource.len();
        [
            self.capacity_by_resource.len(),
            self.drawable_by_resource.len(),
            self.undrawn_by_resource.len(),
            self.hour_capacity_by_resource.len(),
        ]
        .iter()
        .all(|other| *other == len)
            && self.gross_revenue.is_finite()
            && self.gross_revenue >= 0.0
            && self
                .gross_by_resource
                .iter()
                .chain(&self.capacity_by_resource)
                .chain(&self.hour_capacity_by_resource)
                .all(|v| v.is_finite() && *v >= 0.0)
            && self.hour_collected.is_finite()
            && self
                .drawable_by_resource
                .iter()
                .chain(&self.undrawn_by_resource)
                .all(|v| v.is_finite())
    }

    // Sizes the per-resource vectors for the catalog, keeping their values; never shrinks, so
    // no pooled money is dropped. O(resources) once; O(1) after.
    fn ensure_resources(&mut self, count: usize) {
        if self.gross_by_resource.len() >= count {
            return;
        }
        self.gross_by_resource.resize(count, 0.0);
        self.capacity_by_resource.resize(count, 0.0);
        self.drawable_by_resource.resize(count, 0.0);
        self.undrawn_by_resource.resize(count, 0.0);
        self.hour_capacity_by_resource.resize(count, 0.0);
    }

    // Draws `building`'s share of each resource's pool by the capacity it recorded in the last
    // finished hour, setting its activity from the draw as the citywide sale did. A draw never
    // exceeds what is left; the pool loses exactly what the store's budget gained.
    fn draw(&mut self, catalog: &RuntimeEconomyCatalog, building: &mut Building) {
        let factor = building.service_sales_capacity;
        if factor <= 0.0 {
            return;
        }
        let Some(recorded) =
            catalog.profile_by_runtime_id(building.service_sales_profile_runtime_id)
        else {
            return;
        };
        let full_hour_revenue = catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .map(|profile| {
                profile.base_rate_units_per_day / OPERATIONAL_HOURS_PER_DAY
                    * profile.unit_price_currency
            });
        for output in &recorded.outputs {
            let capacity = output_capacity(output.units_per_day, factor);
            let slot = output.resource_runtime_id as usize - 1;
            let total = self.capacity_by_resource[slot];
            if capacity <= 0.0 || total <= 0.0 {
                continue;
            }
            let share = (self.drawable_by_resource[slot] * f64::from(capacity) / f64::from(total))
                .min(self.undrawn_by_resource[slot]);
            let revenue = share as f32;
            if revenue <= 0.0 {
                continue;
            }
            let before = building.operating_budget;
            building.revenue += revenue;
            building.operating_budget += revenue;
            building.daily_household_sales_value += revenue;
            self.undrawn_by_resource[slot] -=
                f64::from(building.operating_budget) - f64::from(before);
            if let Some(full_hour_revenue) = full_hour_revenue.filter(|full| *full > 0.0) {
                building.hourly_economy_activity = (revenue / full_hour_revenue).clamp(0.0, 1.0);
            }
        }
    }

    // Records `building`'s capacity for the next hour's draw and adds it to this hour's
    // totals: an operating, connected commercial service store's staffed output.
    fn record_capacity(&mut self, catalog: &RuntimeEconomyCatalog, building: &mut Building) {
        building.service_sales_capacity = 0.0;
        let Some(profile) = selling_service_store(catalog, building) else {
            return;
        };
        let factor = building_operation_factors(catalog, building, profile).throughput_factor;
        if factor <= 0.0 {
            return;
        }
        building.service_sales_capacity = factor;
        building.service_sales_profile_runtime_id = profile.runtime_id;
        for output in &profile.outputs {
            let capacity = output_capacity(output.units_per_day, factor);
            if capacity > 0.0 {
                self.hour_capacity_by_resource[output.resource_runtime_id as usize - 1] += capacity;
            }
        }
    }

    // The hour's end: collections join the pool, split by the gross that priced them, and the
    // hour's residents and capacity price the next hour.
    pub(super) fn close_hour(&mut self, catalog: &RuntimeEconomyCatalog) {
        self.ensure_resources(catalog.resource_count());
        self.pool_collections();
        self.drawable_by_resource
            .copy_from_slice(&self.undrawn_by_resource);
        self.capacity_by_resource
            .copy_from_slice(&self.hour_capacity_by_resource);
        self.hour_capacity_by_resource.fill(0.0);
        self.residents = std::mem::take(&mut self.hour_residents);
        self.gross_by_resource.fill(0.0);
        self.gross_revenue = 0.0;
        if self.residents == 0 {
            return;
        }
        service_store_demand_rates_by_resource(catalog, &mut self.demand_rates);
        for &(resource_runtime_id, rate_per_resident_per_day) in &self.demand_rates {
            let hourly_demand_units = self.residents as f32 * rate_per_resident_per_day.max(0.0)
                / OPERATIONAL_HOURS_PER_DAY;
            let slot = resource_runtime_id as usize - 1;
            let capacity_units = self.capacity_by_resource[slot].max(0.0);
            if hourly_demand_units <= 0.0 || capacity_units <= 0.0 {
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
            if revenue > 0.0 {
                self.gross_revenue += revenue;
                self.gross_by_resource[slot] += revenue;
            }
        }
    }

    // Moves the hour's collections into the per-resource pools by the gross that priced them.
    // The last priced resource takes the remainder, so the split keeps every unit of money.
    fn pool_collections(&mut self) {
        let Some(last) = self
            .gross_by_resource
            .iter()
            .rposition(|gross| *gross > 0.0)
        else {
            // Nothing priced the hour, so nobody paid: a collection waits for the next price.
            return;
        };
        let total = f64::from(self.gross_revenue);
        let mut pooled = 0.0;
        for slot in 0..last {
            let gross = self.gross_by_resource[slot];
            if gross > 0.0 {
                let part = self.hour_collected * f64::from(gross) / total;
                self.undrawn_by_resource[slot] += part;
                pooled += part;
            }
        }
        self.undrawn_by_resource[last] += self.hour_collected - pooled;
        self.hour_collected = 0.0;
    }
}

// Hourly units a store's output port supplies at `factor` of its throughput.
fn output_capacity(units_per_day: f32, factor: f32) -> f32 {
    units_per_day.max(0.0) / OPERATIONAL_HOURS_PER_DAY * factor
}

// The profile of a service store that can sell this hour: operating, connected and commercial.
fn selling_service_store<'a>(
    catalog: &'a RuntimeEconomyCatalog,
    building: &Building,
) -> Option<&'a EconomyProfileRuntime> {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
        || !matches!(building.zone_type, ZoneType::Commercial)
    {
        return None;
    }
    economy_profile_for_building(catalog, building)
        .filter(|profile| profile.kind == EconomyProfileRuntimeKind::ServiceStore)
}

impl HouseholdSystem {
    /// Household `household_id`'s service purchases at its hourly phase (`SIM-02-I`): its
    /// members' share of the gross revenue the last finished hour priced, up to its budget,
    /// paid into this hour's collections. A housed household also counts toward the next
    /// hour's price. O(1).
    pub(super) fn buy_household_services(
        &mut self,
        household_id: usize,
        allocator: &BuildingAllocator,
    ) {
        let household = &mut self.households[household_id];
        if !household_is_housed(household, allocator) {
            return;
        }
        let sales = &mut self.service_sales;
        sales.hour_residents = sales
            .hour_residents
            .saturating_add(u32::from(household.member_count));
        if sales.residents == 0 {
            return;
        }
        let share = sales.gross_revenue * household.member_count as f32 / sales.residents as f32;
        if share <= 0.0 {
            return;
        }
        let paid = share.min(household.budget.max(0.0));
        if paid <= 0.0 {
            return;
        }
        let before = household.budget;
        household.budget -= paid;
        self.daily_ledgers[household_id].shopping_spend += paid;
        sales.hour_collected += f64::from(before) - f64::from(household.budget);
    }

    /// Building `building`'s service sales at its hourly phase, after production (`SIM-02-I`):
    /// a service store draws its share of the pool with the capacity it recorded in the last
    /// finished hour, which sets its activity for this hour's electricity, then records this
    /// hour's capacity. O(outputs).
    pub(crate) fn settle_store_service_sales(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        building: &mut Building,
    ) {
        let sales = &mut self.service_sales;
        sales.ensure_resources(catalog.resource_count());
        sales.draw(catalog, building);
        sales.record_capacity(catalog, building);
    }
}
