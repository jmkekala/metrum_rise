// SPDX-License-Identifier: GPL-2.0-only

//! Hourly electricity dispatch and physical demand, independent of consumer cash.
//! O(buildings + households), with reusable scratch, parallel independent work and fixed-order
//! chunk reductions. No per-resident scheduling, routing or allocations are introduced.

use super::metrics::{
    household_demand_profile, household_is_housed, physical_worker_capacity_for_profile,
};
use super::{DailyPowerSettlementSummary, HouseholdSystem};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, RuntimeEconomyCatalog, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;
use std::sync::atomic::{AtomicU32, Ordering};

const CHUNK: usize = 512;

/// Final bills of the buildings removed during the calendar day, kept apart from the day's
/// totals so undoing a removal restores exactly that building's share (`SIM-02-G`).
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemovedBuildingElectricity {
    /// Workplace payments and delivered energy of removed buildings' open windows.
    pub(crate) settlement: DailyPowerSettlementSummary,
}

impl RemovedBuildingElectricity {
    /// Validates saved physical totals and payments.
    pub(crate) fn is_valid(self) -> bool {
        let s = self.settlement;
        [
            s.demand_units,
            s.supply_units,
            s.served_units,
            s.coverage,
            s.household_local_revenue,
            s.private_local_revenue,
            s.city_service_local_cost,
            s.utility_local_revenue,
            s.city_service_utility_local_cost,
            s.city_service_utility_owa_cost,
            s.power_auxiliary_local_cost,
            s.power_upkeep_utility_cost,
        ]
        .iter()
        .all(|v| v.is_finite() && *v >= 0.0)
            && s.served_units <= s.demand_units + 0.001
            && s.coverage <= 1.0
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct HourlyBuildingElectricity {
    workplace_kwh: f32,
    residential_kwh: f32,
    capacity_kwh: f32,
    staffed_fraction: f32,
    // Coal a power plant would burn at full dispatch, and its coal stock before burning.
    capacity_coal_kg: f32,
    coal_stock_kg: f32,
}

fn active(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

impl HouseholdSystem {
    /// Bills a removed building's open electricity window in O(1), retaining the bill for the
    /// day's report. Call after freight refunds and before removal; undo restores this state
    /// with the building.
    pub(crate) fn close_building_electricity(
        &mut self,
        building: &mut Building,
        city_owned: bool,
        treasury: &mut f64,
    ) {
        let catalog = load_runtime_economy_catalog().expect("validated economy catalog");
        let tuning = load_runtime_economy_tuning().expect("validated economy tuning");
        let price = super::metrics::local_utility_service_costs(&catalog)[0];
        collect_building_electricity(
            building,
            city_owned,
            &catalog,
            price,
            tuning.owa_import_price_multiplier,
            treasury,
            &mut self.removed_electricity.settlement,
        );
        building.daily_electricity = Default::default();
        building.daily_power_service_units = 0.0;
        building.daily_power_served_units = 0.0;
        building.daily_city_funded_input_cost = 0.0;
    }

    pub(super) fn run_electricity_hour(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog().expect("validated economy catalog");
        let home = household_demand_profile(&catalog);
        let fallback = catalog
            .profile_for_id("building_electricity_default")
            .expect("authored electricity fallback");
        let coal = catalog
            .resource_runtime_id_for_id("coal")
            .expect("authored coal resource");
        self.ensure_daily_ledger_len();
        let count = allocator.buildings.len();
        self.electricity_residents_scratch
            .resize_with(count, || AtomicU32::new(0));
        self.electricity_residents_scratch
            .par_iter()
            .for_each(|n| n.store(0, Ordering::Relaxed));
        let residents = &self.electricity_residents_scratch;
        self.households.par_iter().for_each(|household| {
            if household_is_housed(household, allocator) {
                residents[household.home_building_id]
                    .fetch_add(u32::from(household.member_count), Ordering::Relaxed);
            }
        });
        self.electricity_hour_scratch
            .resize(count, HourlyBuildingElectricity::default());
        let registry = &allocator.registry;
        self.electricity_hour_scratch
            .par_iter_mut()
            .zip(&allocator.buildings)
            .enumerate()
            .for_each(|(idx, (hour, building))| {
                *hour = HourlyBuildingElectricity::default();
                if !active(building) {
                    return;
                }
                let population = residents[idx].load(Ordering::Relaxed);
                if population > 0 {
                    hour.residential_kwh =
                        home.electricity_demand_kwh_per_day(0.0, 0.0, population) / 24.0;
                }
                let authored = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
                let has_workplace = authored
                    .is_some_and(|p| p.kind != EconomyProfileRuntimeKind::DemandSink)
                    || registry.is_city_service_asset(&building.asset_id)
                    || matches!(
                        building.zone_type,
                        ZoneType::Commercial
                            | ZoneType::Industrial
                            | ZoneType::Office
                            | ZoneType::Mixed
                    );
                if !has_workplace {
                    return;
                }
                let profile = authored.unwrap_or(fallback);
                let mut activity = building.hourly_economy_activity;
                let capacity = physical_worker_capacity_for_profile(building, profile);
                let staffing = if capacity > 0 {
                    (building.worker_count as f32 / capacity as f32).min(1.0)
                } else {
                    0.0
                };
                if profile.utility_service.as_deref() == Some("power") {
                    hour.staffed_fraction = staffing;
                    let upkeep_factor = profile
                        .inputs
                        .iter()
                        .filter(|p| p.resource_runtime_id != coal)
                        .map(|p| {
                            let required = p.units_per_day * staffing / 24.0;
                            if required > 0.0 {
                                (building.inventory_units(p.resource_runtime_id) / required)
                                    .clamp(0.0, 1.0)
                            } else {
                                1.0
                            }
                        })
                        .fold(1.0, f32::min);
                    activity = staffing * upkeep_factor;
                    let fuel_per_kwh = profile
                        .input_port(coal)
                        .map_or(0.0, |p| p.units_per_day / profile.base_rate_units_per_day);
                    hour.capacity_kwh = if fuel_per_kwh > 0.0 {
                        (profile.power_capacity_kw * activity)
                            .min(building.inventory_units(coal).max(0.0) / fuel_per_kwh)
                    } else {
                        0.0
                    };
                    hour.capacity_coal_kg = hour.capacity_kwh * fuel_per_kwh;
                    hour.coal_stock_kg = building.inventory_units(coal).max(0.0);
                } else if authored.is_none()
                    || profile.kind == EconomyProfileRuntimeKind::Unsupported
                {
                    let slots = registry.worker_capacity(&building.asset_id);
                    activity = if slots > 0 {
                        (building.worker_count as f32 / slots as f32).min(1.0)
                    } else {
                        0.0
                    };
                }
                hour.workplace_kwh =
                    profile.electricity_demand_kwh_per_day(building.work_area_scale, activity, 0)
                        / 24.0;
            });

        // Indexed chunks fix floating-point addition order independently of Rayon scheduling.
        self.electricity_totals_scratch
            .resize(count.div_ceil(CHUNK), [0.0; 4]);
        self.electricity_totals_scratch
            .par_iter_mut()
            .zip(self.electricity_hour_scratch.par_chunks(CHUNK))
            .for_each(|(total, chunk)| {
                *total = [0.0; 4];
                for hour in chunk {
                    total[0] += f64::from(hour.workplace_kwh + hour.residential_kwh);
                    total[1] += f64::from(hour.capacity_kwh);
                    total[2] += f64::from(hour.capacity_coal_kg);
                    total[3] += f64::from(hour.coal_stock_kg);
                }
            });
        let [demand, capacity, capacity_coal, coal_stock] = self
            .electricity_totals_scratch
            .iter()
            .fold([0.0; 4], |a, b| {
                [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
            });
        let generated = demand.min(capacity);
        let dispatch = if capacity > 0.0 {
            (generated / capacity) as f32
        } else {
            0.0
        };
        let coverage = if demand > 0.0 {
            (generated / demand) as f32
        } else {
            0.0
        };
        // The budget report reads the day's burn and the stock left after this dispatch.
        let burned = capacity_coal * f64::from(dispatch);
        self.day_totals.coal_consumed_kg += burned;
        self.coal_inventory_kg = (coal_stock - burned).max(0.0);
        allocator
            .buildings
            .par_iter_mut()
            .zip(&self.electricity_hour_scratch)
            .for_each(|(building, hour)| {
                building.hourly_economy_activity = 0.0;
                building
                    .daily_electricity
                    .workplace
                    .record(hour.workplace_kwh, coverage);
                building
                    .daily_electricity
                    .residential
                    .record(hour.residential_kwh, coverage);
                if hour.staffed_fraction <= 0.0 {
                    return;
                }
                let profile = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .expect("power profile");
                let generated = hour.capacity_kwh * dispatch;
                for input in &profile.inputs {
                    let amount = if input.resource_runtime_id == coal {
                        generated * input.units_per_day / profile.base_rate_units_per_day
                    } else {
                        input.units_per_day * hour.staffed_fraction / 24.0
                    };
                    building.remove_inventory_units(input.resource_runtime_id, amount);
                }
                building.daily_power_service_units += generated;
                building.daily_power_served_units += generated;
            });
        self.households
            .par_iter()
            .zip(self.daily_ledgers.par_iter_mut())
            .for_each(|(household, ledger)| {
                if !household_is_housed(household, allocator) {
                    return;
                }
                let idx = household.home_building_id;
                let population = residents[idx].load(Ordering::Relaxed);
                if population > 0 {
                    let kwh = self.electricity_hour_scratch[idx].residential_kwh
                        * f32::from(household.member_count)
                        / population as f32;
                    ledger.electricity.record(kwh, coverage);
                }
            });
    }

    /// Bills household `household_id`'s electricity window at its daily phase (`SIM-02-G`): the
    /// bill joins its arrears, the budget pays what it can, and the local share goes to the
    /// treasury. O(1).
    pub(super) fn settle_household_electricity(
        &mut self,
        household_id: usize,
        price: f32,
        imports: f32,
        treasury: &mut f64,
    ) {
        let household = &mut self.households[household_id];
        let ledger = &mut self.daily_ledgers[household_id];
        let bill = ledger.electricity.bill(price, imports);
        household.power_arrears.local += bill.local;
        household.power_arrears.imported += bill.imported;
        let before = household.budget;
        let paid = household.power_arrears.pay(&mut household.budget);
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::OutsideUtilities,
            before,
            household.budget,
        );
        ledger.power_consumption_cost += paid.total();
        ledger.power_local_payment = paid.local;
        ledger.power_unpaid_cost = household.power_arrears.total();
        let power = &mut self.day_totals.power;
        power.household_local_revenue += paid.local;
        power.utility_local_revenue += paid.local;
        *treasury += f64::from(paid.local);
        crate::debug::money::record(
            crate::debug::money::MoneyFlow::OutsideUtilities,
            f64::from(paid.local),
        );
    }

    /// Bills `building`'s electricity window at its daily phase (`SIM-02-G`). A power plant
    /// first books its share of the last closed day's electricity receipts as revenue,
    /// pro rata to the energy it supplied in its own window. O(1).
    pub(super) fn settle_building_electricity_day(
        &mut self,
        building: &mut Building,
        city_owned: bool,
        catalog: &RuntimeEconomyCatalog,
        price: f32,
        imports: f32,
        treasury: &mut f64,
    ) {
        let last = self.last_day_totals.power;
        if last.supply_units > 0.0 {
            let receipts = last.household_local_revenue
                + last.private_local_revenue
                + last.city_service_local_cost
                - last.power_auxiliary_local_cost;
            building.revenue += receipts * building.daily_power_service_units / last.supply_units;
        }
        collect_building_electricity(
            building,
            city_owned,
            catalog,
            price,
            imports,
            treasury,
            &mut self.day_totals.power,
        );
    }

    /// Bills every household and building at once, as their daily phases would, and closes
    /// the day; returns the closed day's electricity and utility summary.
    #[cfg(test)]
    pub(super) fn settle_daily_electricity(
        &mut self,
        allocator: &mut BuildingAllocator,
        treasury: &mut f64,
    ) -> DailyPowerSettlementSummary {
        let catalog = load_runtime_economy_catalog().expect("validated economy catalog");
        let tuning = load_runtime_economy_tuning().expect("validated economy tuning");
        let price = super::metrics::local_utility_service_costs(&catalog)[0];
        let imports = tuning.owa_import_price_multiplier;
        self.ensure_daily_ledger_len();
        for household_id in 0..self.households.len() {
            self.settle_household_electricity(household_id, price, imports, treasury);
        }
        for building in &mut allocator.buildings {
            let city_owned = allocator.registry.is_city_service_asset(&building.asset_id);
            collect_building_electricity(
                building,
                city_owned,
                &catalog,
                price,
                imports,
                treasury,
                &mut self.day_totals.power,
            );
        }
        self.day_totals.power.city_service_utility_local_cost =
            self.day_totals.power.city_service_local_cost;
        self.close_calendar_day();
        // Same-day revenue shares, as the providers' phases would book them the next day.
        let last = self.last_day_totals.power;
        if last.supply_units > 0.0 {
            let receipts = last.household_local_revenue
                + last.private_local_revenue
                + last.city_service_local_cost
                - last.power_auxiliary_local_cost;
            for building in &mut allocator.buildings {
                building.revenue +=
                    receipts * building.daily_power_service_units / last.supply_units;
            }
        }
        last
    }
}

// Bills one building's electricity window into `summary`: city-owned buildings pay from the
// treasury, private ones from their budget, even into deficit. The local share of what was
// paid returns to the treasury. Shared by the daily phase and the final bill of a building that
// is removed. O(1).
fn collect_building_electricity(
    building: &mut Building,
    city_owned: bool,
    catalog: &RuntimeEconomyCatalog,
    price: f32,
    imports: f32,
    treasury: &mut f64,
    summary: &mut DailyPowerSettlementSummary,
) {
    let use_ = building.daily_electricity;
    summary.demand_units += use_.workplace.consumed_kwh + use_.residential.consumed_kwh;
    summary.supply_units += building.daily_power_service_units;
    summary.served_units += use_.workplace.local_kwh + use_.residential.local_kwh;
    let bill = use_.workplace.bill(price, imports);
    let receipts = if city_owned {
        *treasury -= f64::from(bill.total());
        crate::debug::money::record(
            crate::debug::money::MoneyFlow::OutsideUtilities,
            -f64::from(bill.total()),
        );
        summary.city_service_local_cost += bill.local;
        summary.city_service_utility_local_cost += bill.local;
        summary.city_service_utility_owa_cost += bill.imported;
        if catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .is_some_and(|p| p.utility_service.as_deref() == Some("power"))
        {
            summary.power_auxiliary_local_cost += bill.local;
            summary.power_upkeep_utility_cost += bill.imported;
        }
        bill.local
    } else {
        let paid_fraction = if bill.total() > 0.0 {
            (building.operating_budget.max(0.0) / bill.total()).min(1.0)
        } else {
            0.0
        };
        let receipts = bill.local * paid_fraction;
        summary.private_local_revenue += receipts;
        // Unpaid business costs become the existing operating deficit/distress signal.
        let before = building.operating_budget;
        building.operating_budget -= bill.total();
        crate::debug::money::record_change(
            crate::debug::money::MoneyFlow::OutsideUtilities,
            before,
            building.operating_budget,
        );
        receipts
    };
    summary.utility_local_revenue += receipts;
    *treasury += f64::from(receipts);
    crate::debug::money::record(
        crate::debug::money::MoneyFlow::OutsideUtilities,
        f64::from(receipts),
    );
}
