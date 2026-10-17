// SPDX-License-Identifier: GPL-2.0-only

//! Hourly electricity dispatch and physical demand, independent of consumer cash.
//!
//! Each building and household records its own hour at its hourly phase (`SIM-02-I`,
//! `docs/simulation.md#phases`): metered kWh with the coverage of the last finished hour, and a
//! power plant's generation at that hour's dispatch of its capacity. The phases fold demand,
//! capacity and coal into the hour's totals in phase order, and the hour's last slot turns them
//! into the next hour's dispatch and coverage. O(ports) per building and O(1) per household at
//! their phases, O(1) at the hour's end; no allocation.

use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, household_demand_profile, household_is_housed,
    physical_worker_capacity_for_profile,
};
use super::{DailyPowerSettlementSummary, HouseholdSystem};
use crate::assets::AssetRegistry;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::zoning::ZoneType;

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

/// The city's electricity between hourly phases (`SIM-02-I`): the last finished hour's dispatch
/// and coverage, which every phase of this hour reads, and the totals this hour's phases have
/// recorded so far. Saved, so a game loaded mid-hour continues identically.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ElectricityHour {
    /// Share of its capacity each plant generates this hour: the last hour's generation over
    /// its capacity.
    pub(crate) dispatch: f32,
    /// Share of this hour's metered kWh recorded as local: the last hour's generation over its
    /// demand.
    pub(crate) coverage: f32,
    /// Workplace and residential demand recorded so far this hour, in kWh.
    pub(crate) demand_kwh: f64,
    /// Capacity the plants offered so far this hour, in kWh.
    pub(crate) capacity_kwh: f64,
    /// Coal the plants burned so far this hour.
    pub(crate) coal_burned_kg: f64,
    /// Coal left at the plants that ran their phase this hour.
    pub(crate) coal_stock_kg: f64,
}

impl ElectricityHour {
    /// Validates a loaded state: shares in `0..=1` and finite, non-negative totals.
    pub(crate) fn is_valid(&self) -> bool {
        (0.0..=1.0).contains(&self.dispatch)
            && (0.0..=1.0).contains(&self.coverage)
            && [
                self.demand_kwh,
                self.capacity_kwh,
                self.coal_burned_kg,
                self.coal_stock_kg,
            ]
            .iter()
            .all(|v| v.is_finite() && *v >= 0.0)
    }
}

/// The catalog profiles every electricity phase of one slot reads, looked up once per slot.
pub(crate) struct ElectricityProfiles<'a> {
    catalog: &'a RuntimeEconomyCatalog,
    home: &'a EconomyProfileRuntime,
    fallback: &'a EconomyProfileRuntime,
    coal: ResourceRuntimeId,
}

impl<'a> ElectricityProfiles<'a> {
    /// Looks up the household, fallback workplace and coal definitions. O(profiles).
    pub(crate) fn new(catalog: &'a RuntimeEconomyCatalog) -> Self {
        Self {
            catalog,
            home: household_demand_profile(catalog),
            fallback: catalog
                .profile_for_id("building_electricity_default")
                .expect("authored electricity fallback"),
            coal: catalog
                .resource_runtime_id_for_id("coal")
                .expect("authored coal resource"),
        }
    }
}

// One building's workplace hour: its demand and, for a power plant, its staffed share, its
// capacity and the coal that capacity would burn.
#[derive(Clone, Copy, Debug, Default)]
struct WorkplaceHour {
    workplace_kwh: f32,
    power_plant: bool,
    capacity_kwh: f32,
    staffed_fraction: f32,
    capacity_coal_kg: f32,
}

fn active(building: &Building) -> bool {
    !building.broken
        && !building.economy_broken
        && !building.is_deserted
        && !building.is_under_construction()
        && building.edge_idx != usize::MAX
}

// The hourly dispatch's rules for one building, from the activity its phase recorded and its
// staffing, upkeep stock and coal. O(ports).
fn workplace_hour(
    profiles: &ElectricityProfiles,
    building: &Building,
    registry: &AssetRegistry,
) -> WorkplaceHour {
    let mut hour = WorkplaceHour::default();
    if !active(building) {
        return hour;
    }
    let catalog = profiles.catalog;
    let coal = profiles.coal;
    let authored = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
    let has_workplace = authored.is_some_and(|p| p.kind != EconomyProfileRuntimeKind::DemandSink)
        || registry.is_city_service_asset(&building.asset_id)
        || matches!(
            building.zone_type,
            ZoneType::Commercial | ZoneType::Industrial | ZoneType::Office | ZoneType::Mixed
        );
    if !has_workplace {
        return hour;
    }
    let profile = authored.unwrap_or(profiles.fallback);
    let mut activity = building.hourly_economy_activity;
    let capacity = physical_worker_capacity_for_profile(building, profile);
    let staffing = if capacity > 0 {
        (building.worker_count as f32 / capacity as f32).min(1.0)
    } else {
        0.0
    };
    if profile.utility_service.as_deref() == Some("power") {
        hour.power_plant = true;
        hour.staffed_fraction = staffing;
        let upkeep_factor = profile
            .inputs
            .iter()
            .filter(|p| p.resource_runtime_id != coal)
            .map(|p| {
                let required = p.units_per_day * staffing / 24.0;
                if required > 0.0 {
                    (building.inventory_units(p.resource_runtime_id) / required).clamp(0.0, 1.0)
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
    } else if authored.is_none() || profile.kind == EconomyProfileRuntimeKind::Unsupported {
        let slots = registry.worker_capacity(&building.asset_id);
        activity = if slots > 0 {
            (building.worker_count as f32 / slots as f32).min(1.0)
        } else {
            0.0
        };
    }
    hour.workplace_kwh =
        profile.electricity_demand_kwh_per_day(building.work_area_scale, activity, 0) / 24.0;
    hour
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

    /// Building `building_idx`'s electricity at its hourly phase, after its production and
    /// sales (`SIM-02-I`): its workplace kWh recorded with the last hour's coverage, and a
    /// power plant's generation at the last hour's dispatch of its capacity, burning coal and
    /// consuming staffed upkeep as the hourly dispatch did. Folds its demand, capacity and coal
    /// into the hour's totals and resets the hour's activity. O(ports).
    pub(crate) fn run_building_electricity(
        &mut self,
        profiles: &ElectricityProfiles,
        allocator: &mut BuildingAllocator,
        building_idx: usize,
    ) {
        let registry = &allocator.registry;
        let building = &mut allocator.buildings[building_idx];
        let row = workplace_hour(profiles, building, registry);
        let hour = &mut self.electricity_hour;
        building.hourly_economy_activity = 0.0;
        building
            .daily_electricity
            .workplace
            .record(row.workplace_kwh, hour.coverage);
        hour.demand_kwh += f64::from(row.workplace_kwh);
        if !row.power_plant {
            return;
        }
        hour.capacity_kwh += f64::from(row.capacity_kwh);
        if row.staffed_fraction > 0.0 {
            let profile = profiles
                .catalog
                .profile_by_runtime_id(building.economy_profile_runtime_id)
                .expect("power profile");
            let generated = row.capacity_kwh * hour.dispatch;
            for input in &profile.inputs {
                let amount = if input.resource_runtime_id == profiles.coal {
                    generated * input.units_per_day / profile.base_rate_units_per_day
                } else {
                    input.units_per_day * row.staffed_fraction / 24.0
                };
                building.remove_inventory_units(input.resource_runtime_id, amount);
            }
            building.daily_power_service_units += generated;
            building.daily_power_served_units += generated;
            hour.coal_burned_kg += f64::from(row.capacity_coal_kg) * f64::from(hour.dispatch);
        }
        hour.coal_stock_kg += f64::from(building.inventory_units(profiles.coal).max(0.0));
    }

    /// Household `household_id`'s electricity at its hourly phase (`SIM-02-I`): its share of
    /// its home's residential load, recorded in its ledger and its home's meter with the last
    /// hour's coverage and folded into the hour's demand. The home's standby load is split
    /// equally among its household places (`Building::occupancy`, a count kept in O(1) per
    /// move) and the per-resident load follows the members, so the home's total matches the
    /// hourly dispatch's while every place is housed. O(1).
    pub(super) fn record_household_electricity(
        &mut self,
        profiles: &ElectricityProfiles,
        household_id: usize,
        allocator: &mut BuildingAllocator,
    ) {
        let household = &self.households[household_id];
        if !household_is_housed(household, allocator) {
            return;
        }
        let home = &mut allocator.buildings[household.home_building_id];
        if !active(home) {
            return;
        }
        let rates = &profiles.home.electricity;
        let kwh = (rates.idle_kwh_per_day / home.occupancy.max(1) as f32
            + rates.per_resident_kwh_per_day * f32::from(household.member_count))
            / OPERATIONAL_HOURS_PER_DAY;
        let coverage = self.electricity_hour.coverage;
        home.daily_electricity.residential.record(kwh, coverage);
        self.daily_ledgers[household_id]
            .electricity
            .record(kwh, coverage);
        self.electricity_hour.demand_kwh += f64::from(kwh);
    }

    /// Ends the hour's electricity at its last slot (`SIM-02-I`): the hour's generation, the
    /// least of its demand and capacity, over each becomes the next hour's dispatch and
    /// coverage, and the coal it burned and the stock left go to the day's report. O(1).
    pub(super) fn close_electricity_hour(&mut self) {
        let hour = &mut self.electricity_hour;
        let generated = hour.demand_kwh.min(hour.capacity_kwh);
        hour.dispatch = if hour.capacity_kwh > 0.0 {
            (generated / hour.capacity_kwh) as f32
        } else {
            0.0
        };
        hour.coverage = if hour.demand_kwh > 0.0 {
            (generated / hour.demand_kwh) as f32
        } else {
            0.0
        };
        // The budget report reads the day's burn and the stock left after the hour's dispatch.
        self.day_totals.coal_consumed_kg += hour.coal_burned_kg;
        self.coal_inventory_kg = hour.coal_stock_kg;
        hour.demand_kwh = 0.0;
        hour.capacity_kwh = 0.0;
        hour.coal_burned_kg = 0.0;
        hour.coal_stock_kg = 0.0;
    }

    /// Every building's and household's electricity phase in dense order, then the hour's end,
    /// as one hour of slots would run them.
    #[cfg(test)]
    pub(super) fn run_electricity_hour(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog().expect("validated economy catalog");
        let profiles = ElectricityProfiles::new(&catalog);
        self.ensure_daily_ledger_len();
        for building_idx in 0..allocator.buildings.len() {
            self.run_building_electricity(&profiles, allocator, building_idx);
        }
        for household_id in 0..self.households.len() {
            self.record_household_electricity(&profiles, household_id, allocator);
        }
        self.close_electricity_hour();
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
