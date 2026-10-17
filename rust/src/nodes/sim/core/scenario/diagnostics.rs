// SPDX-License-Identifier: GPL-2.0-only

//! Read-only accounting for the small headless growth fixture, never the live game.
//!
//! Observe actual payments before household removal and daily resets. Track businesses by
//! parcel and build generation, since allocator indices move. Serial ordered accumulation is
//! deliberate here: this opt-in village report needs reproducible floating-point totals.
//! Sampling costs O((H + B) log B) per economy/lifecycle boundary and O(A + H) per settlement;
//! retained history is O(days × profiles + all observed firms), with O(B + A) live snapshots
//! and shortage details. Removed firms retain their final capital and operating outcomes.
//! This opt-in diagnostic is not a scaling fixture.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Write};

use super::super::frame::EconomyObserver;
use super::super::state::SimCore;
use super::GrowthDayRecord;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::economy::definitions::EconomyProfileRuntimeKind;
use crate::simulation::economy::demand::{
    BuildingActionDiagnosticsByUse, HouseholdAdmissionDiagnostics, HouseholdRemovalDiagnostics,
};
use crate::simulation::economy::households::{
    REPLENISHMENT_SHOPPING_TO_STORE, active_worker_capacity_for_profile_with_floor_scale,
};
use crate::simulation::zoning::ZoneType;

// Explicit sites have no parcel. Their fixed position identifies them across swap-removal;
// ordinary zoned businesses retain their parcel/generation identity through level changes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum BusinessIdentity {
    Parcel(u64, u32),
    Explicit(u32, u32, u32),
}

// What a business sample attributes (`SIM-02-G`): an hourly sample also counts staffed hours.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sample {
    Hourly,
    Interim,
    Boundary,
}

// Supplies reserved at `building_idx` by shoppers on their way. O(households).
fn reserved_supplies(core: &SimCore, building_idx: usize) -> f64 {
    core.households
        .households
        .iter()
        .filter(|household| {
            household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
                && household.reserved_store_building_id == building_idx
        })
        .map(|household| f64::from(household.reserved_amount))
        .sum()
}

fn business_identity(building: &Building) -> BusinessIdentity {
    if building.parcel_id != 0 {
        BusinessIdentity::Parcel(building.parcel_id, building.build_generation)
    } else {
        BusinessIdentity::Explicit(
            building.center_x.to_bits(),
            building.center_y.to_bits(),
            building.build_generation,
        )
    }
}

#[derive(Clone, Copy, Default)]
struct BusinessSnapshot {
    profile: u16,
    cash: f64,
    revenue: f64,
    inputs_received: f64,
    household_sales: f64,
    supply_stock: f64,
    deserted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct BusinessDay {
    sales: f64,
    local_sales: f64,
    export_sales: f64,
    export_units: f64,
    export_deliveries: u32,
    import_deliveries: u32,
    imported_inputs: f64,
    local_inputs: f64,
    input_purchases: f64,
    delivered_freight_cost: f64,
    inputs_received: f64,
    cash_change: f64,
    startup_cash: f64,
    replacement_startup_cash: f64,
    removed_cash: f64,
    cash: f64,
    active: u32,
    deserted: u32,
    negative_cash: u32,
    closures: u32,
    removals: u32,
    payroll: f64,
    supplied_units: f64,
    produced_units: f64,
    supply_stock: f64,
    input_stock: f64,
    staffed_hours: f64,
    capacity_hours: f64,
    sample_hours: u32,
    area_hectares: f64,
    remaining_reserve: f64,
    extracted_units: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct HouseholdDay {
    wages: f64,
    transfers: f64,
    unpaid_transfers: f64,
    unemployment: f64,
    shopping: f64,
    utilities: f64,
    property: f64,
    cash: f64,
    deficit: u32,
    empty_stock: u32,
    empty_by_replenishment_state: [u32; 8],
    empty_with_daily_supply_cash: u32,
    shortage_stranded_at_old_work: u32,
    zero_cash: u32,
    support_expired: u32,
    shopper_failures: u32,
    unemployed: u32,
    max_cash_residual: f64,
    daily_consumption: f64,
    pantry_gap: f64,
}

impl HouseholdDay {
    fn bills(&self) -> f64 {
        self.shopping + self.utilities + self.property
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
struct DayDiagnostics {
    businesses: BTreeMap<u16, BusinessDay>,
    household: HouseholdDay,
    payroll: f64,
    launched: u32,
    taxes: f64,
    upkeep: f64,
    other_city_net: f64,
    construction: f64,
    recurring_net: f64,
    city_residual: f64,
    omitted_transfers: f64,
    admission: HouseholdAdmissionDiagnostics,
    removal: HouseholdRemovalDiagnostics,
    construction_decisions: BuildingActionDiagnosticsByUse,
}

/// A diagnosed run of the existing growth scenario, formatted as a compact failure timeline.
pub struct GrowthReport {
    /// The same end-of-settlement records returned by the unobserved runner.
    pub days: Vec<GrowthDayRecord>,
    details: Vec<DayDiagnostics>,
    current: DayDiagnostics,
    businesses: BTreeMap<BusinessIdentity, BusinessSnapshot>,
    business_outcomes: BTreeMap<BusinessIdentity, GrowthBusinessOutcome>,
    profile_names: BTreeMap<u16, String>,
    opening_treasury: f64,
    opening_build_cost: f64,
    opening_business_cash: f64,
    previous_treasury: f64,
    previous_build_cost: f64,
    unrecorded_opening_build_cost: f64,
    support_days: u32,
    stranded_members: String,
    // Shortage lines of the household windows closed since the last calendar close.
    stranded_today: String,
}

/// Per-firm results over the observed run, retained after bankruptcy and removal.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GrowthBusinessOutcome {
    /// Authored economy profile; firms sharing a profile retain separate outcomes.
    pub profile_id: String,
    /// Parcel identity; zero denotes an explicitly placed site.
    pub parcel_id: u64,
    /// Construction generation distinguishes replacement businesses on the same site.
    pub generation: u32,
    /// Site centre in world metres, also identifying explicit sites.
    pub centre: [f32; 2],
    /// Initial cash already present when the observer was attached.
    pub opening_cash: f64,
    /// Owner capital provided after attachment, including replacements.
    pub startup_cash: f64,
    /// Cash earned or lost after expenses, excluding owner capital and removal.
    pub operating_cash_change: f64,
    /// Latest cash balance, retained at removal for closed sites.
    pub ending_cash: f64,
    /// Sum of assigned workers sampled each live operational hour.
    pub staffed_worker_hours: f64,
    /// Sum of demand-responsive worker capacity over the same live hours.
    pub capacity_worker_hours: f64,
    /// Number of live operational hours sampled; excludes construction and desertion.
    pub operating_hours: u32,
    /// Whether this firm became deserted during observation.
    pub bankrupt: bool,
    /// Whether its building was removed during observation.
    pub removed: bool,
}

/// Observed cash flows for one business profile over a requested trailing period.
#[derive(Debug, Default)]
pub struct GrowthBusinessCashFlow {
    /// Paid sales, including any distress liquidation.
    pub sales: f64,
    /// Actual wages paid, before withheld household income tax.
    pub payroll: f64,
    /// Operating cash change after all costs and taxes, excluding startup and removals.
    pub operating_cash_change: f64,
    /// New owner capital, including sites built on previously unused parcels.
    pub startup_cash: f64,
    /// Owner capital injected on previously demolished parcels; a subset of startup cash.
    pub replacement_startup_cash: f64,
    /// Receipts from physically delivered sales to local buildings.
    pub local_sales: f64,
    /// Receipts from physically delivered regional exports.
    pub export_sales: f64,
    /// Net trade costs paid: input commitments, their freight and delivered export freight,
    /// including cancellation refunds.
    pub input_purchases: f64,
    /// Carrier fees on delivered jobs (a subset of trade costs; input payment may precede delivery).
    pub delivered_freight_cost: f64,
    /// Successfully delivered export loads; each owns one physical carrier.
    pub export_deliveries: u32,
    /// Successfully delivered imported input loads; excludes local deliveries.
    pub import_deliveries: u32,
    /// Businesses newly marked deserted during this period.
    pub bankruptcies: u32,
}

impl GrowthReport {
    pub(super) fn new(core: &SimCore) -> Self {
        let mut report = Self {
            days: Vec::new(),
            details: Vec::new(),
            current: DayDiagnostics::default(),
            businesses: BTreeMap::new(),
            business_outcomes: BTreeMap::new(),
            profile_names: BTreeMap::new(),
            opening_treasury: core.treasury.balance,
            opening_build_cost: core.treasury.lifetime_build_cost,
            opening_business_cash: 0.0,
            previous_treasury: core.treasury.balance,
            previous_build_cost: core.treasury.lifetime_build_cost,
            unrecorded_opening_build_cost: core.treasury.lifetime_build_cost
                - core.budget_last_lifetime_build_cost,
            support_days: core.demand.runtime_tuning().unemployment_max_days,
            stranded_members: String::new(),
            stranded_today: String::new(),
        };
        report.sample_businesses(core, Sample::Boundary);
        report.opening_business_cash = report.businesses.values().map(|b| b.cash).sum();
        for outcome in report.business_outcomes.values_mut() {
            outcome.opening_cash = outcome.startup_cash;
            outcome.startup_cash = 0.0;
        }
        report.current = DayDiagnostics::default();
        report
    }

    // Samples one business's cash, sales and stock since its last sample; `None` for a building
    // that is not a private business. Each building's daily settlement is bracketed by an
    // interim sample, which attributes purchases and output as an hourly one does, and a
    // boundary sample, which sees only the settlement's charges (`SIM-02-G`).
    fn sample_business(
        &mut self,
        core: &SimCore,
        building_idx: usize,
        kind: Sample,
        reserved: f64,
    ) -> Option<BusinessIdentity> {
        let catalog = core.demand.runtime_catalog();
        let supply_resource = catalog
            .resource_runtime_id_for_id("household_supplies")
            .unwrap();
        let building = &core.allocator.buildings[building_idx];
        let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
        let explicit_producer = profile.is_some_and(|p| {
            matches!(
                p.kind,
                EconomyProfileRuntimeKind::FieldProducer | EconomyProfileRuntimeKind::Extractor
            )
        });
        if (!matches!(
            building.zone_type,
            ZoneType::Commercial | ZoneType::Industrial
        ) && !explicit_producer)
            || core.allocator.is_city_service_building(building)
        {
            return None;
        }
        let key = business_identity(building);
        let now = BusinessSnapshot {
            profile: building.economy_profile_runtime_id,
            cash: f64::from(building.operating_budget),
            revenue: f64::from(building.revenue),
            inputs_received: f64::from(building.daily_owa_input_value)
                + f64::from(building.daily_local_input_value),
            household_sales: f64::from(building.daily_household_sales_value),
            supply_stock: f64::from(building.inventory_units(supply_resource)) + reserved,
            deserted: building.is_deserted,
        };
        self.profile_names.entry(now.profile).or_insert_with(|| {
            core.demand
                .runtime_catalog()
                .profile_by_runtime_id(now.profile)
                .map(|profile| profile.id.clone())
                .unwrap_or_else(|| "unresolved".into())
        });
        let day = self.current.businesses.entry(now.profile).or_default();
        let outcome = self
            .business_outcomes
            .entry(key)
            .or_insert_with(|| GrowthBusinessOutcome {
                profile_id: self.profile_names[&now.profile].clone(),
                parcel_id: building.parcel_id,
                generation: building.build_generation,
                centre: [building.center_x, building.center_y],
                ..Default::default()
            });
        outcome.ending_cash = now.cash;
        if let Some(before) = self.businesses.insert(key, now) {
            let sales = now.revenue - before.revenue;
            let cash_change = now.cash - before.cash;
            day.sales += sales;
            day.cash_change += cash_change;
            outcome.operating_cash_change += cash_change;
            outcome.bankrupt |= now.deserted && !before.deserted;
            day.inputs_received += now.inputs_received - before.inputs_received;
            if kind != Sample::Boundary {
                day.input_purchases += sales - cash_change;
                if kind == Sample::Hourly
                    && !building.is_deserted
                    && !building.is_under_construction()
                    && let Some(profile) = profile
                {
                    day.staffed_hours += f64::from(building.worker_count);
                    let capacity = f64::from(active_worker_capacity_for_profile_with_floor_scale(
                        catalog,
                        building,
                        profile,
                        building.commercial_activity_floor_scale,
                    ));
                    day.capacity_hours += capacity;
                    day.sample_hours += 1;
                    outcome.staffed_worker_hours += f64::from(building.worker_count);
                    outcome.capacity_worker_hours += capacity;
                    outcome.operating_hours += 1;
                }
                if let Some(profile) = catalog.profile_by_runtime_id(now.profile)
                    && profile.kind == EconomyProfileRuntimeKind::Store
                    && profile.output_port(supply_resource).is_some()
                {
                    let sold = (now.household_sales - before.household_sales)
                        / f64::from(profile.unit_price_currency);
                    day.supplied_units += sold;
                    day.produced_units += now.supply_stock - before.supply_stock + sold;
                }
            }
            day.closures += u32::from(now.deserted && !before.deserted);
        } else {
            // Placement is observed before the first production/freight tick.
            day.startup_cash += now.cash;
            outcome.startup_cash += now.cash;
            // Parcel generation advances on demolition. This subset identifies
            // fresh capital at rebuilt sites, including after observer attachment.
            if building.build_generation > 0 {
                day.replacement_startup_cash += now.cash;
            }
            day.sales += now.revenue;
            day.inputs_received += now.inputs_received;
        }
        Some(key)
    }

    // Hourly private cash has only sales receipts and freight commitments/refunds. Daily
    // payroll, property, utility and profit-tax charges run at a separate observed boundary.
    // Hence sales - cash delta measures net purchases, without treating delivered stock as
    // a second payment or reimplementing freight prices.
    fn sample_businesses(&mut self, core: &SimCore, kind: Sample) {
        // Reservations leave shop stock before pickup/payment. Include unpaid reservations
        // in its stock conservation identity so dispatch is not mistaken for a paid sale.
        let mut reserved = BTreeMap::<usize, f64>::new();
        for household in &core.households.households {
            if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE {
                *reserved
                    .entry(household.reserved_store_building_id)
                    .or_default() += f64::from(household.reserved_amount);
            }
        }
        let mut seen = BTreeSet::new();
        for building_idx in 0..core.allocator.buildings.len() {
            let reserved = reserved.get(&building_idx).copied().unwrap_or(0.0);
            if let Some(key) = self.sample_business(core, building_idx, kind, reserved) {
                assert!(seen.insert(key), "duplicate village business identity");
            }
        }
        self.businesses.retain(|key, before| {
            if seen.contains(key) {
                true
            } else {
                let day = self.current.businesses.entry(before.profile).or_default();
                day.removals += 1;
                day.removed_cash += before.cash;
                self.business_outcomes.get_mut(key).unwrap().removed = true;
                false
            }
        });
    }

    /// Returns individual firms in stable site order, including failed and removed firms.
    pub fn business_outcomes(&self) -> impl Iterator<Item = &GrowthBusinessOutcome> {
        self.business_outcomes.values()
    }

    /// Largest daily mismatch between measured treasury movement and the observed flow buckets.
    /// This includes transfers to households which leave before the UI budget ledger closes.
    pub fn max_city_cash_residual(&self) -> f64 {
        self.details
            .iter()
            .map(|day| day.city_residual.abs())
            .fold(0.0, f64::max)
    }

    /// Largest mismatch between business cash balances and operating/startup/removal flows.
    pub fn max_business_cash_residual(&self) -> f64 {
        let mut before = self.opening_business_cash;
        let mut max: f64 = 0.0;
        for day in &self.details {
            let after: f64 = day.businesses.values().map(|b| b.cash).sum();
            let change: f64 = day
                .businesses
                .values()
                .map(|b| b.cash_change + b.startup_cash - b.removed_cash)
                .sum();
            max = max.max((after - before - change).abs());
            before = after;
        }
        max
    }

    /// Largest per-household mismatch between its ledger cash movement and income minus bills.
    pub fn max_household_cash_residual(&self) -> f64 {
        self.details
            .iter()
            .map(|day| day.household.max_cash_residual)
            .fold(0.0, f64::max)
    }

    /// Sums one profile's observed flows for at most `last_days` completed settlements.
    pub fn business_cash_flow(&self, profile: &str, last_days: usize) -> GrowthBusinessCashFlow {
        let mut total = GrowthBusinessCashFlow::default();
        let Some((&id, _)) = self
            .profile_names
            .iter()
            .find(|(_, name)| name.as_str() == profile)
        else {
            return total;
        };
        for day in self.details.iter().rev().take(last_days) {
            if let Some(b) = day.businesses.get(&id) {
                total.sales += b.sales;
                total.payroll += b.payroll;
                total.operating_cash_change += b.cash_change;
                total.startup_cash += b.startup_cash;
                total.replacement_startup_cash += b.replacement_startup_cash;
                total.local_sales += b.local_sales;
                total.export_sales += b.export_sales;
                total.input_purchases += b.input_purchases;
                total.delivered_freight_cost += b.delivered_freight_cost;
                total.export_deliveries += b.export_deliveries;
                total.import_deliveries += b.import_deliveries;
                total.bankruptcies += b.closures;
            }
        }
        total
    }

    /// Counts empty household pantries at daily settlement over the trailing period.
    pub fn empty_household_days(&self, last_days: usize) -> u32 {
        self.details
            .iter()
            .rev()
            .take(last_days)
            .map(|d| d.household.empty_stock)
            .sum()
    }

    /// Treasury receipts minus recurring expenses, excluding initial and construction capital.
    pub fn recurring_city_balance(&self, last_days: usize) -> f64 {
        self.details
            .iter()
            .rev()
            .take(last_days)
            .map(|d| d.recurring_net)
            .sum()
    }

    /// Eligible household benefits not paid because treasury cash was unavailable.
    pub fn unpaid_household_transfers(&self, last_days: usize) -> f64 {
        self.details
            .iter()
            .rev()
            .take(last_days)
            .map(|d| d.household.unpaid_transfers)
            .sum()
    }

    /// True when repeated observers captured identical accounting as well as identical city state.
    pub fn has_same_accounting(&self, other: &Self) -> bool {
        self.days == other.days
            && self.details == other.details
            && self.business_outcomes == other.business_outcomes
            && self.stranded_members == other.stranded_members
            && self.opening_treasury == other.opening_treasury
            && self.opening_build_cost == other.opening_build_cost
    }

    /// Formats every daily observation for detailed investigation or an external text artifact.
    pub fn daily_report(&self) -> String {
        let mut text = String::new();
        self.write_header(&mut text).unwrap();
        for firm in self.business_outcomes() {
            writeln!(text, "Firm {} parcel {}/{} at {:?}: operating-hours {} staffing {:.0}/{:.0} worker-hours; cash {:.2}, operating-change {:+.2}, opening-capital {:.2}, owner-startup {:.2}, bankrupt {} removed {}",
                firm.profile_id, firm.parcel_id, firm.generation, firm.centre,
                firm.operating_hours, firm.staffed_worker_hours, firm.capacity_worker_hours,
                firm.ending_cash, firm.operating_cash_change, firm.opening_cash, firm.startup_cash,
                firm.bankrupt, firm.removed).unwrap();
        }
        text.push_str(&self.stranded_members);
        for index in 0..self.days.len() {
            self.write_day(&mut text, index).unwrap();
        }
        text
    }

    fn write_header(&self, out: &mut impl Write) -> fmt::Result {
        writeln!(
            out,
            "Village accounting: opening treasury {:.2}, infrastructure already paid {:.2}; unemployment window {} days.",
            self.opening_treasury, self.opening_build_cost, self.support_days
        )?;
        writeln!(
            out,
            "Private cash already present at observer attachment: {:.2}; excluded from operating cash changes.",
            self.opening_business_cash
        )?;
        writeln!(
            out,
            "Flows end at each daily settlement; the first observation may start partway through a day. City counts are after departures; HH accounting/counts are before departures. Launch counts are carriers, not arrivals."
        )?;
        writeln!(
            out,
            "Business purchases are inferred from hourly sales minus cash change (net freight commitments/refunds, with sub-unit f32 rounding). Received = delivered value, not consumption. Sales include liquidation; cash change excludes startup and removed balances. Replacement is the startup subset on previously demolished parcels. Payroll is actual gross private wages."
        )?;
        writeln!(
            out,
            "HH bills = shopping reservations/refunds + paid utilities + property; cash-limited payments are not full needs. Growth gates are the last midnight decision, not a full-day rejection count."
        )?;
        writeln!(
            out,
            "Reconciliation: max city residual {:.4}; transfers omitted by existing UI ledger {:.2} total; business {:.4}; household {:.4}.",
            self.max_city_cash_residual(),
            self.details
                .iter()
                .map(|day| day.omitted_transfers)
                .sum::<f64>(),
            self.max_business_cash_residual(),
            self.max_household_cash_residual()
        )?;
        writeln!(
            out,
            "Run totals: taxes {:.2}, transfers {:.2}, roads {:.2}, recurring net {:+.2}, private payroll {:.2}, unpaid eligible transfers {:.2}.",
            self.details.iter().map(|d| d.taxes).sum::<f64>(),
            self.details
                .iter()
                .map(|d| d.household.transfers)
                .sum::<f64>(),
            self.details.iter().map(|d| d.upkeep).sum::<f64>(),
            self.details.iter().map(|d| d.recurring_net).sum::<f64>(),
            self.details.iter().map(|d| d.payroll).sum::<f64>(),
            self.unpaid_household_transfers(usize::MAX)
        )?;
        for (profile, name) in &self.profile_names {
            let mut total = BusinessDay::default();
            for day in &self.details {
                if let Some(b) = day.businesses.get(profile) {
                    total.sales += b.sales;
                    total.input_purchases += b.input_purchases;
                    total.delivered_freight_cost += b.delivered_freight_cost;
                    total.export_deliveries += b.export_deliveries;
                    total.import_deliveries += b.import_deliveries;
                    total.startup_cash += b.startup_cash;
                    total.replacement_startup_cash += b.replacement_startup_cash;
                    total.cash_change += b.cash_change;
                    total.closures += b.closures;
                    total.local_sales += b.local_sales;
                    total.export_sales += b.export_sales;
                    total.export_units += b.export_units;
                    total.imported_inputs += b.imported_inputs;
                    total.local_inputs += b.local_inputs;
                    total.payroll += b.payroll;
                }
            }
            writeln!(
                out,
                "  total {name}: sales {:.2}, purchases {:.2}, startup {:.2} (replacement {:.2}), cash-change {:+.2}, bankruptcies {}",
                total.sales,
                total.input_purchases,
                total.startup_cash,
                total.replacement_startup_cash,
                total.cash_change,
                total.closures
            )?;
            writeln!(
                out,
                "    trade local-sales {:.2}, export-sales {:.2} / {:.2} units, delivered-inputs imported/local {:.2}/{:.2}, payroll {:.2}, other-expenses {:.2}, delivered-freight {:.2}, export/import-loads {}/{}",
                total.local_sales,
                total.export_sales,
                total.export_units,
                total.imported_inputs,
                total.local_inputs,
                total.payroll,
                total.sales - total.input_purchases - total.payroll - total.cash_change,
                total.delivered_freight_cost,
                total.export_deliveries,
                total.import_deliveries
            )?;
        }
        Ok(())
    }

    fn write_day(&self, out: &mut impl Write, index: usize) -> fmt::Result {
        let state = &self.days[index];
        let d = &self.details[index];
        let h = &d.household;
        writeln!(
            out,
            "day {:>3}: HH {} residents {} employed {}/{} jobs; launched {} departed {}; treasury {:.2}",
            state.day,
            state.households,
            state.residents,
            state.employed,
            state.jobs,
            d.launched,
            d.removal.removed_households,
            state.treasury
        )?;
        writeln!(
            out,
            "  HH wages {:.2} transfers {:.2} (unemployment {:.2}) bills {:.2} [shop {:.2} utility {:.2} property {:.2}] cash {:.2}; deficit {} empty {} cash-zero {} unemployed {} support-expired {} shopper-failures {}",
            h.wages,
            h.transfers,
            h.unemployment,
            h.bills(),
            h.shopping,
            h.utilities,
            h.property,
            h.cash,
            h.deficit,
            h.empty_stock,
            h.zero_cash,
            h.unemployed,
            h.support_expired,
            h.shopper_failures
        )?;
        let r = &d.removal;
        writeln!(
            out,
            "  exits unhoused {} cash-zero {} eligible {} planned/removed {}/{}",
            r.unhoused_household_count,
            r.zero_budget_household_count,
            r.persistent_exit_eligible_household_count,
            r.planned_households,
            r.removed_households
        )?;
        writeln!(
            out,
            "  city taxes {:.2} - transfers {:.2} - upkeep {:.2} + other-net {:.2} = recurring {:+.2}; construction {:.2}; residual {:+.4}; omitted-transfers {:.2}; private payroll {:.2}",
            d.taxes,
            h.transfers,
            d.upkeep,
            d.other_city_net,
            d.recurring_net,
            d.construction,
            d.city_residual,
            d.omitted_transfers,
            d.payroll
        )?;
        for (profile, b) in &d.businesses {
            writeln!(
                out,
                "  {} active {} sales {:.2} purchases {:.2} received {:.2} cash-change {:+.2} cash {:.2} startup {:.2} replacement {:.2} negative {} closed {} removed {} removed-cash {:.2} deserted {}",
                self.profile_names[profile],
                b.active,
                b.sales,
                b.input_purchases,
                b.inputs_received,
                b.cash_change,
                b.cash,
                b.startup_cash,
                b.replacement_startup_cash,
                b.negative_cash,
                b.closures,
                b.removals,
                b.removed_cash,
                b.deserted
            )?;
            writeln!(
                out,
                "    trade local-sales {:.2}, export-sales {:.2} / {:.2} units, delivered-inputs imported/local {:.2}/{:.2}, payroll {:.2}, other-expenses {:.2}, delivered-freight {:.2}, export/import-loads {}/{}",
                b.local_sales,
                b.export_sales,
                b.export_units,
                b.imported_inputs,
                b.local_inputs,
                b.payroll,
                b.sales - b.input_purchases - b.payroll - b.cash_change,
                b.delivered_freight_cost,
                b.export_deliveries,
                b.import_deliveries
            )?;
            if b.sample_hours > 0 {
                writeln!(
                    out,
                    "    inputs-stock {:.2}; mean workers/cap {:.2}/{:.2}",
                    b.input_stock,
                    b.staffed_hours / f64::from(b.sample_hours),
                    b.capacity_hours / f64::from(b.sample_hours),
                )?;
            }
            if self.profile_names[profile] == "grocery_basic" {
                writeln!(
                    out,
                    "    grocery-units produced {:.2} sold {:.2} stock+reserved {:.2}",
                    b.produced_units, b.supplied_units, b.supply_stock,
                )?;
            }
            if b.area_hectares > 0.0 || b.extracted_units > 0.0 {
                writeln!(
                    out,
                    "    productive hectares {:.4}; coal reserve {:.2}, cumulative extraction {:.2}",
                    b.area_hectares, b.remaining_reserve, b.extracted_units
                )?;
            }
        }
        writeln!(
            out,
            "  household consumption/day {:.2} pantry gap {:.2}; empty by replenishment state {:?}; empty with daily supply cash {}; stranded at old work {}",
            h.daily_consumption,
            h.pantry_gap,
            h.empty_by_replenishment_state,
            h.empty_with_daily_supply_cash,
            h.shortage_stranded_at_old_work
        )?;
        let a = &d.admission;
        let gate = if a.connected_border_count == 0 {
            "no-border"
        } else if a.vacant_household_slots == 0 {
            "no-vacant-home"
        } else if a.planned_households == 0 {
            "no-execution-capacity"
        } else if a.examined_candidates > 0 && a.income_rejections == a.examined_candidates {
            "applicant-income"
        } else if a.launched_households == 0 && a.worker_housing_rejections > 0 {
            "homes-reserved-for-workers"
        } else if a.launched_households == 0 {
            "carrier-launch"
        } else {
            "launched"
        };
        writeln!(
            out,
            "  admission {gate}: vacancy {} funded/physical {}/{} unfunded {} local-unemployed {} live-job-slots {} examined/income-rejected {}/{} worker-housing-rejected {} planned/launched {}/{}",
            a.vacant_household_slots,
            a.funded_worker_capacity,
            a.physical_worker_capacity,
            a.open_jobs_unfunded,
            a.existing_unemployed_member_count,
            a.move_in_job_slots,
            a.examined_candidates,
            a.income_rejections,
            a.worker_housing_rejections,
            a.planned_households,
            a.launched_households
        )?;
        for (zone, b) in [
            ("R", d.construction_decisions.residential),
            ("C", d.construction_decisions.commercial),
            ("I", d.construction_decisions.industrial),
        ] {
            let gate = if b.spawn_candidates == 0 {
                "no-placement-candidate"
            } else if b.spawn_selected > 0 {
                "selected"
            } else if b.spawn_selected == 0 && b.spawn_rejected_absorption > 0 {
                "output-absorption"
            } else if b.spawn_selected == 0 && b.spawn_rejected_viability > 0 {
                "market-cannot-fund-operation"
            } else if b.spawn_selected == 0 && b.spawn_rejected_workforce > 0 {
                "workforce-unavailable-by-completion"
            } else if zone == "R" && b.housing_needed_slots == 0 {
                "committed-housing-sufficient"
            } else {
                "no-new-project"
            };
            writeln!(
                out,
                "  build {zone} {gate}: candidates {} housing-needed-slots {} planned/selected {}/{} absorption-rejected {} viability-rejected {} workforce-rejected {}; despawn candidates {} credit {:.3}->{:.3} selected {}",
                b.spawn_candidates,
                b.housing_needed_slots,
                b.spawn_planned,
                b.spawn_selected,
                b.spawn_rejected_absorption,
                b.spawn_rejected_viability,
                b.spawn_rejected_workforce,
                b.despawn_candidates,
                b.despawn_credit_before,
                b.despawn_credit_after,
                b.despawn_selected
            )?;
        }
        Ok(())
    }
}

impl EconomyObserver for GrowthReport {
    fn freight_delivered(
        &mut self,
        shipment: &crate::simulation::economy::logistics::Shipment,
        allocator: &crate::simulation::buildings::allocator::BuildingAllocator,
    ) {
        use crate::simulation::economy::logistics::ShipmentEndpoint;
        if let ShipmentEndpoint::Building(source) = shipment.source {
            let day = self
                .current
                .businesses
                .entry(allocator.buildings[source].economy_profile_runtime_id)
                .or_default();
            if matches!(shipment.destination, ShipmentEndpoint::OwaBorder(_)) {
                day.export_sales += f64::from(shipment.total_cost);
                day.export_units += f64::from(shipment.amount);
                day.export_deliveries += 1;
                day.delivered_freight_cost += f64::from(shipment.freight_cost);
            } else {
                day.local_sales += f64::from(shipment.total_cost - shipment.freight_cost);
            }
        }
        if let ShipmentEndpoint::Building(destination) = shipment.destination {
            let building = &allocator.buildings[destination];
            if allocator.is_city_service_building(building) {
                return;
            }
            let day = self
                .current
                .businesses
                .entry(building.economy_profile_runtime_id)
                .or_default();
            day.delivered_freight_cost += f64::from(shipment.freight_cost);
            if matches!(shipment.source, ShipmentEndpoint::OwaBorder(_)) {
                day.imported_inputs += f64::from(shipment.total_cost);
                day.import_deliveries += 1;
            } else {
                day.local_inputs += f64::from(shipment.total_cost);
            }
        }
    }

    fn buildings_changed(&mut self, core: &SimCore) {
        self.sample_businesses(core, Sample::Boundary);
    }

    fn hourly_economy(&mut self, core: &SimCore) {
        self.sample_businesses(core, Sample::Hourly);
    }

    fn wage_paid(
        &mut self,
        core: &SimCore,
        payment: crate::simulation::economy::households::WagePayment,
    ) {
        if payment.city_funded {
            return;
        }
        let building = &core.allocator.buildings[payment.building];
        let wage = f64::from(payment.gross);
        // The wage left the snapshot's cash too, so later samples do not count it again.
        if let Some(snapshot) = self.businesses.get_mut(&business_identity(building)) {
            snapshot.cash -= wage;
            let day = self.current.businesses.entry(snapshot.profile).or_default();
            day.payroll += wage;
            day.cash_change -= wage;
            if let Some(outcome) = self.business_outcomes.get_mut(&business_identity(building)) {
                outcome.operating_cash_change -= wage;
            }
        }
        self.current.payroll += wage;
    }

    fn building_day_opened(&mut self, core: &SimCore, building_idx: usize) {
        let reserved = reserved_supplies(core, building_idx);
        self.sample_business(core, building_idx, Sample::Interim, reserved);
    }

    fn building_day_closed(&mut self, core: &SimCore, building_idx: usize) {
        let reserved = reserved_supplies(core, building_idx);
        if let Some(key) = self.sample_business(core, building_idx, Sample::Boundary, reserved)
            && let Some(snapshot) = self.businesses.get_mut(&key)
        {
            // The building's window accumulators restart right after this callback.
            snapshot.inputs_received = 0.0;
            snapshot.household_sales = 0.0;
        }
    }

    fn household_day_closed(&mut self, core: &SimCore, household_id: usize) {
        let household = &core.households.households[household_id];
        let ledger = &core.households.daily_ledgers()[household_id];
        let h = &mut self.current.household;
        h.wages += f64::from(ledger.wage_income);
        h.transfers += f64::from(ledger.transfer_income());
        h.unpaid_transfers +=
            f64::from((ledger.transfer_claim - ledger.transfer_income()).max(0.0));
        h.unemployment += f64::from(ledger.unemployment_benefit_income);
        h.shopping += f64::from(ledger.shopping_spend);
        let utilities = ledger.power_consumption_cost
            + ledger.water_consumption_cost
            + ledger.sewage_consumption_cost;
        h.utilities += f64::from(utilities);
        h.property += f64::from(ledger.property_tax_paid);
        let income = f64::from(ledger.wage_income) + f64::from(ledger.transfer_income());
        let bills = f64::from(ledger.shopping_spend)
            + f64::from(utilities)
            + f64::from(ledger.property_tax_paid);
        h.max_cash_residual = h.max_cash_residual.max(
            (f64::from(ledger.budget_after) - f64::from(ledger.budget_before) - (income - bills))
                .abs(),
        );
        h.cash += f64::from(household.budget);
        h.deficit += u32::from(
            ledger.wage_income + ledger.transfer_income()
                < ledger.shopping_spend + utilities + ledger.property_tax_paid,
        );
        h.empty_stock += u32::from(household.stock <= f32::EPSILON && household.member_count > 0);
        if household.stock <= f32::EPSILON && household.member_count > 0 {
            h.empty_by_replenishment_state[household.replenishment_state as usize] += 1;
            writeln!(self.stranded_today, "Shortage household {household_id}: budget {:.2} wages {:.2} transfers {:.2} shopping {:.2} home {} state {} failed {} shopper {} store {} amount {:.2} reserved-cost {:.2} cooldown {}",
                household.budget, ledger.wage_income, ledger.transfer_income(), ledger.shopping_spend,
                household.home_building_id, household.replenishment_state, household.replenishment_failure_count,
                core.agents.resolve_agent(household.shopping_agent).unwrap_or(usize::MAX), household.reserved_store_building_id, household.reserved_amount,
                household.reserved_total_cost, household.cooldown_hours).unwrap();
        }
        h.zero_cash += u32::from(household.budget <= f32::EPSILON && household.member_count > 0);
        h.support_expired += u32::from(
            household.unemployment_days_elapsed >= self.support_days
                && ledger.unemployed_adults > 0
                && ledger.unemployment_benefit_income <= f32::EPSILON,
        );
        h.shopper_failures += u32::from(ledger.shopper_trips_failed);
        h.unemployed += u32::from(ledger.unemployed_adults);
        let consumption = f64::from(household.member_count) * f64::from(household.consumption_rate);
        let supply_price = core
            .demand
            .runtime_catalog()
            .profile_for_id("grocery_basic")
            .unwrap()
            .unit_price_currency;
        h.empty_with_daily_supply_cash += u32::from(
            household.stock <= f32::EPSILON
                && f64::from(household.budget)
                    >= consumption
                        * f64::from(supply_price * (1.0 + core.fiscal_policy.household_vat_rate)),
        );
        h.daily_consumption += consumption;
        h.pantry_gap += (consumption
            * f64::from(
                core.demand
                    .runtime_catalog()
                    .profile_for_id("basic_household_demand")
                    .unwrap()
                    .stock_target_days,
            )
            - f64::from(household.stock))
        .max(0.0);
    }

    fn day_closed(&mut self, core: &SimCore) {
        self.sample_businesses(core, Sample::Boundary);
        let last_day = &core.households.last_day_totals;
        assert!(
            (self.current.payroll - last_day.private_payroll).abs() < 0.1
                && (self
                    .current
                    .businesses
                    .values()
                    .map(|day| day.payroll)
                    .sum::<f64>()
                    - self.current.payroll)
                    .abs()
                    < 0.1,
            "private payroll observer must reconcile actual paid wages"
        );
        self.stranded_members = std::mem::take(&mut self.stranded_today);
        for agent in 0..core.agents.len() {
            if core
                .households
                .households
                .get(core.agents.household_id[agent])
                .is_some_and(|household| household.stock <= f32::EPSILON)
            {
                let a = &core.agents;
                writeln!(self.stranded_members,
                    "Shortage member {agent}: household {} age {} transit {} activity {} home {} current {} work {} target {} planned {} replan {} mode {} edge {}",
                    a.household_id[agent], a.age_group[agent], a.transit[agent], a.activity[agent],
                    a.home_building[agent], a.current_building[agent], a.work_building[agent],
                    a.target_building[agent], a.planned_target_building[agent], a.next_replan_time[agent], a.transit_mode[agent], a.current_edge[agent]).unwrap();
            }
            if core.agents.transit[agent] == crate::simulation::economy::agents::TRANSIT_IN_BUILDING
                && core.agents.activity[agent] == crate::simulation::economy::agents::ACTIVITY_WORK
                && core.agents.current_building[agent] != core.agents.home_building[agent]
                && core.agents.current_building[agent] != core.agents.work_building[agent]
                && core
                    .households
                    .households
                    .get(core.agents.household_id[agent])
                    .is_some_and(|household| household.stock <= f32::EPSILON)
            {
                self.current.household.shortage_stranded_at_old_work += 1;
                let a = &core.agents;
                writeln!(self.stranded_members, "Stranded agent {agent}: age {} home {} current {} work {} target {} planned {} activity {}", a.age_group[agent], a.home_building[agent], a.current_building[agent], a.work_building[agent], a.target_building[agent], a.planned_target_building[agent], a.activity[agent]).unwrap();
            }
        }

        for (building_idx, building) in core.allocator.buildings.iter().enumerate() {
            let key = business_identity(building);
            if let Some(snapshot) = self.businesses.get_mut(&key) {
                let b = self.current.businesses.entry(snapshot.profile).or_default();
                b.cash += snapshot.cash;
                b.active += u32::from(!building.is_deserted && !building.is_under_construction());
                b.deserted += u32::from(building.is_deserted);
                b.negative_cash += u32::from(snapshot.cash < 0.0 && !building.is_deserted);
                b.supply_stock += snapshot.supply_stock;
                if let Some(site) = core.resource_extraction.site_for_building(building_idx) {
                    b.remaining_reserve += f64::from(site.remaining_reserve_units());
                    b.extracted_units += f64::from(site.extracted_units);
                }
                if let Some(profile) = core
                    .demand
                    .runtime_catalog()
                    .profile_by_runtime_id(snapshot.profile)
                {
                    if matches!(
                        profile.kind,
                        EconomyProfileRuntimeKind::FieldProducer
                            | EconomyProfileRuntimeKind::Extractor
                    ) {
                        b.area_hectares += f64::from(building.work_area_scale);
                    }
                    b.input_stock += profile
                        .inputs
                        .iter()
                        .map(|input| f64::from(building.inventory_units(input.resource_runtime_id)))
                        .sum::<f64>();
                }
            }
        }
        let (admission, removal, construction) = core.demand.scenario_diagnostics();
        self.current.admission = *admission;
        self.current.removal = *removal;
        self.current.construction_decisions = *construction;
        self.current.launched = core.debug_household_admissions_since_daily;
        let ledger = core
            .budget_history
            .back()
            .expect("closed daily city ledger");
        let d = &mut self.current;
        d.taxes = ledger.tax_income;
        d.upkeep = core.treasury.last_daily_upkeep;
        d.construction = core.treasury.lifetime_build_cost - self.previous_build_cost;
        // The first UI ledger includes the roads built before this report started. Strip that
        // separate initial capital expense using its existing lifetime-cost cursor.
        let ledger_construction = if self.details.is_empty() {
            d.construction + self.unrecorded_opening_build_cost
        } else {
            d.construction
        };
        d.other_city_net = ledger.utility_service_revenue
            - ledger.city_wages
            - ledger.fuel_input_purchases
            - ledger.imports_owa
            - (ledger.construction_service_costs - ledger_construction - d.upkeep);
        // Transfers post to the calendar day as they are paid; household windows straddle it.
        d.recurring_net = d.taxes - ledger.benefits - d.upkeep + d.other_city_net;
        d.city_residual =
            core.treasury.balance - self.previous_treasury - (d.recurring_net - d.construction);
        d.omitted_transfers = d.household.transfers - ledger.benefits;
        self.previous_treasury = core.treasury.balance;
        self.previous_build_cost = core.treasury.lifetime_build_cost;
        self.details.push(std::mem::take(&mut self.current));
    }
}

impl fmt::Display for GrowthReport {
    fn fmt(&self, out: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.write_header(out)?;
        let mut selected: BTreeSet<usize> = (0..self.days.len()).step_by(30).collect();
        let events: [(&str, Option<usize>); 7] = [
            (
                "first household cash deficit",
                self.details.iter().position(|d| d.household.deficit > 0),
            ),
            (
                "first household stockout",
                self.details
                    .iter()
                    .position(|d| d.household.empty_stock > 0),
            ),
            (
                "first negative business cash",
                self.details
                    .iter()
                    .position(|d| d.businesses.values().any(|b| b.negative_cash > 0)),
            ),
            (
                "first business bankruptcy",
                self.details
                    .iter()
                    .position(|d| d.businesses.values().any(|b| b.closures > 0)),
            ),
            (
                "first expired unemployment support",
                self.details
                    .iter()
                    .position(|d| d.household.support_expired > 0),
            ),
            (
                "first household departure",
                self.details
                    .iter()
                    .position(|d| d.removal.removed_households > 0),
            ),
            (
                "treasury exhausted",
                self.days.iter().position(|d| d.treasury <= 0.0),
            ),
        ];
        for (label, index) in events {
            if let Some(index) = index {
                selected.insert(index);
                writeln!(out, "event day {}: {label}", self.days[index].day)?;
            }
        }
        for (profile, name) in &self.profile_names {
            let active = |index: usize| {
                self.details[index]
                    .businesses
                    .get(profile)
                    .map_or(0, |b| b.active)
            };
            if !self.details.is_empty()
                && active(self.details.len() - 1) == 0
                && let Some(index) =
                    (1..self.details.len()).rfind(|&i| active(i - 1) > 0 && active(i) == 0)
            {
                selected.insert(index);
                writeln!(
                    out,
                    "event day {}: final loss of active {name} in this run",
                    self.days[index].day
                )?;
            }
        }
        if let Some(last) = self.days.len().checked_sub(1) {
            selected.insert(last);
        }
        for index in selected {
            self.write_day(out, index)?;
        }
        Ok(())
    }
}
