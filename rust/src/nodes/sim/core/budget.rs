// SPDX-License-Identifier: GPL-2.0-only

//! City treasury, service policy, fiscal reporting, and economy diagnostics.

use super::state::SimCore;
use crate::debug_log;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, TRANSIT_IN_BUILDING, age_group_can_work,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::{
    active_worker_capacity_for_profile_with_floor_scale, effective_service_funding,
    scaled_output_buffer_capacity_units_for_building, scaled_output_units_per_day_for_building,
    service_funded_worker_capacity, write_electricity_funding_by_building,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::work_area::profile_kind_uses_explicit_work_area;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

/// Currency cost per meter of new road laid, deducted from the city treasury at placement.
pub(crate) const ROAD_BUILD_COST_PER_METER: f64 = 15.0;
/// Starter build cost per service-building lot cell, deducted from the city treasury at placement.
pub(crate) const SERVICE_BUILD_COST_PER_LOT_CELL: f64 = 2_500.0;
/// Currency upkeep per meter of road per day, settled from the city treasury each day.
pub(crate) const ROAD_UPKEEP_PER_METER_PER_DAY: f64 = 0.1;
/// Stable service policy id for the city electricity service.
pub(crate) const SERVICE_POLICY_ELECTRICITY: &str = "electricity";
/// Number of completed daily budget entries kept for UI trend graphs.
pub(crate) const ECONOMY_HISTORY_DAYS: usize = 180;

/// Total road length as a rolling sum over each day (`SIM-02-G`, `docs/simulation.md#snapshots`):
/// each slot folds the next 1/23,040 of the edges counted at the day's start, and the finished
/// sum replaces the last at the day's final slot. Daily road upkeep reads the last finished
/// sum, so no midnight pass scans the network. O(edges / slots per day) per slot.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RoadLengthRoll {
    /// Length of the last finished sweep, in metres.
    pub(crate) front_m: f64,
    /// Length folded so far by the current sweep, in whole millimetres: an integer sum does not
    /// depend on how a save regroups the remaining edges into slots.
    pub(crate) back_mm: i64,
    /// Edge count frozen at the current sweep's start; `None` until a sweep starts at a day's
    /// first slot.
    pub(crate) sweep_edges: Option<u64>,
    /// First edge the current sweep has not visited.
    pub(crate) next_edge: u64,
    /// Whether `front_m` holds a measurement. A world gets its first one at its first close.
    pub(crate) measured: bool,
}

impl RoadLengthRoll {
    /// Folds slot `slot`'s share of the edges. O(edges / slots per day).
    pub(crate) fn advance(
        &mut self,
        slot: u64,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        use crate::simulation::core::scheduler::SLOTS_PER_DAY;
        let chunk = slot % SLOTS_PER_DAY;
        if chunk == 0 {
            self.sweep_edges = Some(graph.edge_count() as u64);
            self.back_mm = 0;
            self.next_edge = 0;
        }
        let Some(swept) = self.sweep_edges else {
            return;
        };
        let edges = graph.edges();
        let end = ((chunk + 1) * swept / SLOTS_PER_DAY).min(edges.len() as u64);
        if end > self.next_edge {
            self.back_mm += edges[self.next_edge as usize..end as usize]
                .iter()
                .filter(|edge| !edge.deleted)
                .map(|edge| length_mm(edge.physical_length))
                .sum::<i64>();
            self.next_edge = end;
        }
        if chunk == SLOTS_PER_DAY - 1 {
            self.front_m = self.back_mm as f64 / 1000.0;
            self.measured = true;
        }
    }

    /// The frozen edge count and the next edge as a save stores them: saving drops deleted
    /// edges and keeps the rest in order, so each position loses the deleted edges below it
    /// and the sweep resumes on the same live edges. O(edges), at save.
    pub(crate) fn saved_positions(
        &self,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> (Option<u64>, u64) {
        let edges = graph.edges();
        let compact = |position: u64| {
            let below = &edges[..(position as usize).min(edges.len())];
            position - below.iter().filter(|edge| edge.deleted).count() as u64
        };
        (self.sweep_edges.map(compact), compact(self.next_edge))
    }

    /// The last finished length; a world without one measures its network once. O(1), or
    /// O(edges) at a world's first close.
    pub(crate) fn length_m(
        &mut self,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> f64 {
        if !self.measured {
            let mm: i64 = graph
                .edges()
                .iter()
                .filter(|edge| !edge.deleted)
                .map(|edge| length_mm(edge.physical_length))
                .sum();
            self.front_m = mm as f64 / 1000.0;
            self.measured = true;
        }
        self.front_m
    }
}

// An edge's length in whole millimetres, for order-independent sums.
fn length_mm(metres: f32) -> i64 {
    (f64::from(metres) * 1000.0).round() as i64
}

fn is_electricity_service(service_id: &str) -> bool {
    matches!(service_id, SERVICE_POLICY_ELECTRICITY | "power")
}

fn percent_for_log(rate: f32) -> f32 {
    rate * 100.0
}

/// Player-controlled city service funding policies.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CityServicePolicy {
    /// Electricity service funding ratio in `0.0..=1.0`.
    pub(crate) electricity_funding: f32,
}

impl Default for CityServicePolicy {
    fn default() -> Self {
        Self {
            electricity_funding: 1.0,
        }
    }
}

/// Completed daily accounting buckets shown by the Economy Overview.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DailyBudgetLedgerEntry {
    /// Operational day index this completed entry represents.
    pub(crate) day_index: u32,
    /// Total city income recorded for the day.
    pub(crate) income: f64,
    /// Total city expenses recorded for the day.
    pub(crate) expenses: f64,
    /// Net cashflow for the day.
    pub(crate) net: f64,
    /// Treasury balance after the daily ledger closed.
    pub(crate) treasury: f64,
    /// Combined tax income bucket.
    pub(crate) tax_income: f64,
    /// Wage income tax collected from household gross wages.
    pub(crate) income_tax: f64,
    /// Household purchase VAT collected from store shopping.
    pub(crate) household_vat: f64,
    /// Business profit tax collected from positive private operating-budget growth.
    pub(crate) business_profit_tax: f64,
    /// Daily private property tax collected during the day.
    pub(crate) property_tax: f64,
    /// Residential property tax collected during the day.
    pub(crate) residential_property_tax: f64,
    /// Commercial property tax collected during the day.
    pub(crate) commercial_property_tax: f64,
    /// Industrial property tax collected during the day.
    pub(crate) industrial_property_tax: f64,
    /// Local utility and service revenue collected by city-owned providers.
    pub(crate) utility_service_revenue: f64,
    /// Total household transfer expense paid from the treasury.
    pub(crate) benefits: f64,
    /// Unemployment benefit expense paid from the treasury.
    pub(crate) unemployment_benefits: f64,
    /// Pension expense paid from the treasury.
    pub(crate) pensions: f64,
    /// Child-support expense paid from the treasury.
    pub(crate) child_support: f64,
    /// City-owned service payroll expense.
    pub(crate) city_wages: f64,
    /// Treasury-funded service input and fuel purchase expense.
    pub(crate) fuel_input_purchases: f64,
    /// City-paid OWA fallback expense.
    pub(crate) imports_owa: f64,
    /// Construction, road upkeep, and internal service operating costs.
    pub(crate) construction_service_costs: f64,
    /// Electricity units produced during the day.
    pub(crate) power_produced: f64,
    /// Electricity units consumed from local producers during the day.
    pub(crate) power_consumed: f64,
    /// Electricity demand not served by local producers during the day.
    pub(crate) power_unmet: f64,
    /// Local electricity coverage ratio in `0.0..=1.0`.
    pub(crate) power_coverage: f64,
    /// Coal inventory currently held by city power providers.
    pub(crate) coal_inventory: f64,
    /// Estimated coal units consumed by city power providers during the day.
    pub(crate) coal_consumed: f64,
    /// City fuel/input cost attributable to electricity providers.
    pub(crate) electricity_fuel_cost: f64,
    /// City payroll cost attributable to electricity providers.
    pub(crate) electricity_wage_cost: f64,
    /// Local electricity revenue collected from consumers.
    pub(crate) electricity_revenue: f64,
    /// Electricity cash flow after input purchases, payroll and utility upkeep.
    pub(crate) electricity_net: f64,
    /// Bills for all electricity delivered citywide, including imported power and unpaid usage.
    pub(crate) electricity_consumption_cost: f64,
}
/// City-level fiscal ledger, separate from household budgets and building budgets.
///
/// The balance may go negative: deficits are an explicit fiscal state rather than
/// a blocked operation. Future debt/credit systems may add consequences later.
pub struct CityTreasury {
    /// Current balance in currency units. May be negative.
    pub balance: f64,
    /// Running total of all infrastructure build costs since game start.
    pub lifetime_build_cost: f64,
    /// Running total of all collected tax revenue since game start.
    pub lifetime_tax_revenue: f64,
    /// Road upkeep deducted in the most recent daily settlement.
    pub last_daily_upkeep: f64,
    /// Income tax collected in the most recently finalized fiscal day.
    pub last_daily_income_tax: f64,
    /// Household VAT collected in the most recently finalized fiscal day.
    pub last_daily_household_vat: f64,
    /// Business profit tax collected in the most recently finalized fiscal day.
    pub last_daily_business_profit_tax: f64,
    /// Property tax collected in the most recently finalized fiscal day.
    pub last_daily_property_tax: f64,
    /// Residential property tax collected in the most recently finalized fiscal day.
    pub last_daily_residential_property_tax: f64,
    /// Commercial property tax collected in the most recently finalized fiscal day.
    pub last_daily_commercial_property_tax: f64,
    /// Industrial property tax collected in the most recently finalized fiscal day.
    pub last_daily_industrial_property_tax: f64,
    /// Income tax collected since the last daily fiscal finalization.
    pub pending_income_tax: f64,
    /// Household VAT collected since the last daily fiscal finalization.
    pub pending_household_vat: f64,
    /// Business profit tax collected since the last daily fiscal finalization.
    pub pending_business_profit_tax: f64,
    /// Property tax collected since the last daily fiscal finalization.
    pub pending_property_tax: f64,
    /// Residential property tax collected since the last daily fiscal finalization.
    pub pending_residential_property_tax: f64,
    /// Commercial property tax collected since the last daily fiscal finalization.
    pub pending_commercial_property_tax: f64,
    /// Industrial property tax collected since the last daily fiscal finalization.
    pub pending_industrial_property_tax: f64,
}

impl Default for CityTreasury {
    /// Starts a new fiscal ledger with the validated authored treasury balance.
    fn default() -> Self {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        Self::new(tuning.startup_treasury_balance)
    }
}

impl CityTreasury {
    /// Initialises the treasury with the given startup balance.
    pub(crate) fn new(startup_balance: f64) -> Self {
        Self {
            balance: startup_balance,
            lifetime_build_cost: 0.0,
            lifetime_tax_revenue: 0.0,
            last_daily_upkeep: 0.0,
            last_daily_income_tax: 0.0,
            last_daily_household_vat: 0.0,
            last_daily_business_profit_tax: 0.0,
            last_daily_property_tax: 0.0,
            last_daily_residential_property_tax: 0.0,
            last_daily_commercial_property_tax: 0.0,
            last_daily_industrial_property_tax: 0.0,
            pending_income_tax: 0.0,
            pending_household_vat: 0.0,
            pending_business_profit_tax: 0.0,
            pending_property_tax: 0.0,
            pending_residential_property_tax: 0.0,
            pending_commercial_property_tax: 0.0,
            pending_industrial_property_tax: 0.0,
        }
    }

    /// Deducts an infrastructure build cost from the treasury. Balance may go negative.
    pub(crate) fn deduct_build_cost(&mut self, amount: f64) {
        self.balance -= amount;
        crate::debug::money::record(crate::debug::money::MoneyFlow::Construction, -amount);
        self.lifetime_build_cost += amount;
    }

    /// Reverses an infrastructure build cost for a placement cancelled before final commitment.
    pub(crate) fn refund_build_cost(&mut self, amount: f64) {
        self.balance += amount;
        crate::debug::money::record(crate::debug::money::MoneyFlow::Construction, amount);
        self.lifetime_build_cost = (self.lifetime_build_cost - amount).max(0.0);
    }

    /// Settles one day's infrastructure upkeep cost. Balance may go negative.
    pub(crate) fn settle_daily_upkeep(&mut self, amount: f64) {
        self.balance -= amount;
        crate::debug::money::record(crate::debug::money::MoneyFlow::RoadUpkeep, -amount);
        self.last_daily_upkeep = amount;
    }

    /// Records wage income tax withheld from household income.
    pub(crate) fn collect_income_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Income);
    }

    /// Records VAT collected from household shopping purchases.
    pub(crate) fn collect_household_vat(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::HouseholdVat);
    }

    /// Records tax collected from positive daily business profit.
    pub(crate) fn collect_business_profit_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessProfit);
    }

    /// Records daily property tax by zone.
    pub(crate) fn collect_property_tax_for_zone(&mut self, zone_type: ZoneType, amount: f64) {
        if amount <= 0.0 {
            return;
        }
        self.record_tax(amount, TaxBucket::Property);
        match zone_type {
            ZoneType::Residential => self.pending_residential_property_tax += amount,
            ZoneType::Commercial => self.pending_commercial_property_tax += amount,
            ZoneType::Industrial => self.pending_industrial_property_tax += amount,
            _ => {}
        }
    }

    /// Rolls the current pending fiscal window into daily reporting buckets.
    pub(crate) fn finalize_daily_tax_window(&mut self) {
        self.last_daily_income_tax = self.pending_income_tax;
        self.last_daily_household_vat = self.pending_household_vat;
        self.last_daily_business_profit_tax = self.pending_business_profit_tax;
        self.last_daily_property_tax = self.pending_property_tax;
        self.last_daily_residential_property_tax = self.pending_residential_property_tax;
        self.last_daily_commercial_property_tax = self.pending_commercial_property_tax;
        self.last_daily_industrial_property_tax = self.pending_industrial_property_tax;
        self.pending_income_tax = 0.0;
        self.pending_household_vat = 0.0;
        self.pending_business_profit_tax = 0.0;
        self.pending_property_tax = 0.0;
        self.pending_residential_property_tax = 0.0;
        self.pending_commercial_property_tax = 0.0;
        self.pending_industrial_property_tax = 0.0;
    }

    fn record_tax(&mut self, amount: f64, bucket: TaxBucket) {
        if amount <= 0.0 {
            return;
        }
        self.balance += amount;
        self.lifetime_tax_revenue += amount;
        match bucket {
            TaxBucket::Income => self.pending_income_tax += amount,
            TaxBucket::HouseholdVat => self.pending_household_vat += amount,
            TaxBucket::BusinessProfit => self.pending_business_profit_tax += amount,
            TaxBucket::Property => self.pending_property_tax += amount,
        }
    }
}

#[derive(Clone, Copy)]
enum TaxBucket {
    Income,
    HouseholdVat,
    BusinessProfit,
    Property,
}

/// City-wide household and job counts, read at the daily settlement boundary.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DailyCityFlowDiagnostics {
    pub(super) active_households: u32,
    pub(super) housed_households: u32,
    pub(super) unhoused_households: u32,
    pub(super) zero_budget_households: u32,
    pub(super) supplies_empty_households: u32,
    pub(super) supplies_low_households: u32,
    pub(super) total_household_slots: u32,
    pub(super) vacant_household_slots: u32,
    pub(super) resident_agents: u32,
    pub(super) child_agents: u32,
    pub(super) adult_agents: u32,
    pub(super) elder_agents: u32,
    pub(super) pending_household_carriers: u32,
    pub(super) employed_agents: u32,
    pub(super) unemployed_agents: u32,
    pub(super) commercial_job_capacity: u32,
    pub(super) commercial_filled_jobs: u32,
    pub(super) commercial_active_job_capacity: u32,
    pub(super) commercial_active_filled_jobs: u32,
    pub(super) industrial_job_capacity: u32,
    pub(super) industrial_filled_jobs: u32,
    pub(super) industrial_active_job_capacity: u32,
    pub(super) industrial_active_filled_jobs: u32,
    pub(super) service_active_job_capacity: u32,
    pub(super) service_active_filled_jobs: u32,
}

#[derive(Clone, Copy)]
enum DailyCityFlowWorkBucket {
    Commercial,
    Industrial,
    Service,
}

fn daily_city_flow_work_bucket(
    building_zone: ZoneType,
    profile: Option<&EconomyProfileRuntime>,
) -> Option<DailyCityFlowWorkBucket> {
    match building_zone {
        ZoneType::Commercial => Some(DailyCityFlowWorkBucket::Commercial),
        ZoneType::Industrial => Some(DailyCityFlowWorkBucket::Industrial),
        _ => {
            let profile = profile?;
            if profile_kind_uses_explicit_work_area(profile.kind) {
                Some(DailyCityFlowWorkBucket::Industrial)
            } else if matches!(
                profile.kind,
                EconomyProfileRuntimeKind::UtilityProducer
                    | EconomyProfileRuntimeKind::UtilityProcessor
            ) {
                Some(DailyCityFlowWorkBucket::Service)
            } else {
                None
            }
        }
    }
}

fn active_budget_backed_worker_capacity(
    allocator_city_funded: bool,
    building_operating_budget: f32,
    active_worker_capacity: u32,
    profile: &EconomyProfileRuntime,
) -> u32 {
    if active_worker_capacity == 0 {
        return 0;
    }
    let average_daily_wage = profile.average_daily_wage();
    let budget_capacity = if allocator_city_funded {
        active_worker_capacity
    } else if average_daily_wage > 0.1 {
        (building_operating_budget.max(0.0) / average_daily_wage).floor() as u32
    } else {
        active_worker_capacity
    };
    active_worker_capacity.min(budget_capacity)
}

impl SimCore {
    /// Applies the gameplay cheat grant and enables private-construction viability bypass.
    pub(crate) fn apply_money_and_max_demand_cheat(&mut self, money_amount: f64) -> f64 {
        if money_amount.is_finite() && money_amount > 0.0 {
            self.treasury.balance += money_amount;
        }
        self.demand.enable_max_demand_cheat();
        self.treasury.balance
    }

    /// Applies a live service funding policy change from the UI.
    pub(crate) fn set_service_funding(&mut self, service_id: &str, funding: f32) -> bool {
        if !is_electricity_service(service_id) || !funding.is_finite() {
            return false;
        }
        let previous = self.service_policy.electricity_funding;
        let requested = funding;
        let funding = funding.clamp(0.0, 1.0);
        self.service_policy.electricity_funding = funding;
        self.apply_service_funding_staffing_policy();
        self.follow_census_policy();
        debug_log!(
            "economy",
            "service policy change: id={} requested={:.3} old={:.3} new={:.3} day={} minute={} treasury={:.1}",
            SERVICE_POLICY_ELECTRICITY,
            requested,
            previous,
            self.service_policy.electricity_funding,
            self.time.day_index,
            self.time.minute_of_day,
            self.treasury.balance,
        );
        true
    }

    /// Applies a live fiscal policy change from the UI.
    pub(crate) fn set_fiscal_policy_value(&mut self, policy_id: &str, value: f32) -> bool {
        let previous_policy = self.fiscal_policy;
        let Some(previous_control) = previous_policy.control(policy_id) else {
            return false;
        };
        if !self.fiscal_policy.set_value(policy_id, value) {
            return false;
        }
        let new_control = self
            .fiscal_policy
            .control(policy_id)
            .unwrap_or(previous_control);
        self.follow_census_policy();
        self.demand.refresh_household_diagnostics(
            &self.allocator,
            &self.households,
            &self.region_graph,
            self.treasury.balance,
            &self.fiscal_policy,
        );
        self.log_fiscal_policy_change(policy_id, previous_control.value, value, new_control.value);
        true
    }

    /// Applies a live per-building service funding override from an inspector panel.
    pub(crate) fn set_building_service_funding_override_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        service_id: &str,
        funding: f32,
    ) -> bool {
        if !is_electricity_service(service_id) || !funding.is_finite() {
            return false;
        }
        let Some(building_idx) = self.allocator.nearest_building_idx_at(world_x, world_z) else {
            return false;
        };
        if !self.building_provides_service(building_idx, "power") {
            return false;
        }
        self.allocator
            .set_service_funding_override(building_idx, funding.clamp(0.0, 1.0));
        self.apply_service_funding_staffing_policy();
        self.follow_census_policy();
        true
    }

    pub(crate) fn effective_electricity_funding_for_building(&self, building_idx: usize) -> f32 {
        let Some(building) = self.allocator.buildings.get(building_idx) else {
            return self.service_policy.electricity_funding;
        };
        effective_service_funding(
            building.service_funding_override,
            self.service_policy.electricity_funding,
        )
    }

    pub(crate) fn electricity_funding_by_building(&self) -> Vec<f32> {
        let mut funding = vec![1.0; self.allocator.buildings.len()];
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return funding;
        };
        write_electricity_funding_by_building(
            &self.allocator.buildings,
            &catalog,
            self.service_policy.electricity_funding,
            &mut funding,
        );
        funding
    }

    /// Releases the service workers the live funding no longer pays for, at once. The funded
    /// jobs follow at the next census publication ([`Self::follow_census_policy`]).
    pub(crate) fn apply_service_funding_staffing_policy(&mut self) {
        let funding = self.electricity_funding_by_building();
        self.households.enforce_service_funding_staffing(
            &mut self.agents,
            &mut self.allocator,
            &funding,
        );
    }

    // A fiscal or service funding change reaches the census through its copy (`SIM-02-I`):
    // the census in progress restarts on the new values and the next hour's first slot
    // publishes it, with the construction permits proved on it.
    fn follow_census_policy(&mut self) {
        self.households
            .follow_census_policy(self.fiscal_policy, self.service_policy.electricity_funding);
    }

    /// Restores one placed service's fiscal input ceiling without rebuilding citywide jobs.
    pub(crate) fn register_service_input_capacity(&mut self, building_idx: usize) {
        let building = &self.allocator.buildings[building_idx];
        if self.allocator.is_city_service_building(building)
            && let Some(profile) = self
                .demand
                .runtime_catalog()
                .profile_by_runtime_id(building.economy_profile_runtime_id)
        {
            let funding = self.effective_electricity_funding_for_building(building_idx);
            self.households.job_funding.register_pending_service(
                building_idx,
                building,
                profile,
                funding,
            );
        }
    }

    /// The census the loaded game's readers see until the next hour's first slot publishes one
    /// (`SIM-02-I`). A save before the hour's actions slot kept the census that hour's first
    /// slot published, which the hour's planning slots, permit epoch, workforce reset and
    /// project reservations read: it is recomputed from its saved copy, or, with books that
    /// were not valid at the save, stays not valid so those readers skip as the saved game's
    /// did. Later in the hour only the derived jobs shown read it until the next publication,
    /// and it is a census of the loaded city.
    pub(crate) fn refresh_loaded_job_funding(&mut self) {
        let published = self.households.census_inputs.take_loaded_published();
        if let Some(Some(adopted)) = &published {
            self.households.restore_published_census(
                &self.allocator,
                &self.transit_network,
                &self.region_graph,
                adopted,
            );
            return;
        }
        let funding = self.electricity_funding_by_building();
        self.households.job_funding.refresh(
            &self.allocator,
            &self.households.households,
            &mut self.logistics,
            &self.transit_network,
            &self.region_graph,
            self.demand.runtime_catalog(),
            self.demand.runtime_tuning(),
            &self.fiscal_policy,
            &funding,
            self.treasury.balance,
            self.time.minute_of_day,
        );
        // A census of the live city; the hour's census still publishes at the next hour's
        // first slot (`SIM-02-I`).
        let published = match published {
            Some(None) => None,
            _ => Some(self.households.census_key(&self.transit_network)),
        };
        self.households.job_funding.set_published(published);
    }

    fn log_fiscal_policy_change(
        &self,
        policy_id: &str,
        previous_value: f32,
        requested_value: f32,
        new_value: f32,
    ) {
        let policy = self.fiscal_policy;
        let label = policy
            .control(policy_id)
            .map(|control| control.label)
            .unwrap_or(policy_id);
        debug_log!(
            "economy",
            "fiscal policy change: id={} label=\"{}\" requested={:.4} old={:.4} new={:.4} \
             day={} minute={} treasury={:.1} policy=(unemployment={:.1}/day \
             unemployment_days={} pension={:.1}/day child_support={:.1}/day income_tax={:.1}% \
             household_vat={:.1}% business_profit={:.1}% \
             property_tax_per_day=res:{:.1},com:{:.1},ind:{:.1},level_mult:{:.2})",
            policy_id,
            label,
            requested_value,
            previous_value,
            new_value,
            self.time.day_index,
            self.time.minute_of_day,
            self.treasury.balance,
            policy.unemployment_benefit_per_adult_per_day,
            policy.unemployment_max_days,
            policy.pension_per_elder_per_day,
            policy.child_support_per_child_per_day,
            percent_for_log(policy.income_tax_rate),
            percent_for_log(policy.household_vat_rate),
            percent_for_log(policy.business_profit_tax_rate),
            policy.residential_property_tax_per_home_per_day,
            policy.commercial_property_tax_per_building_per_day,
            policy.industrial_property_tax_per_building_per_day,
            policy.property_tax_level_multiplier,
        );
    }

    fn building_provides_service(&self, building_idx: usize, utility_service: &str) -> bool {
        let Some(building) = self.allocator.buildings.get(building_idx) else {
            return false;
        };
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return false;
        };
        catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .is_some_and(|profile| profile.utility_service.as_deref() == Some(utility_service))
    }

    pub(super) fn record_daily_budget_ledger(&mut self, day_index: u32) {
        let construction_delta =
            (self.treasury.lifetime_build_cost - self.budget_last_lifetime_build_cost).max(0.0);
        self.budget_last_lifetime_build_cost = self.treasury.lifetime_build_cost;

        let entry = self.build_budget_ledger_entry(day_index, construction_delta);
        self.budget_history.push_back(entry);
        while self.budget_history.len() > ECONOMY_HISTORY_DAYS {
            self.budget_history.pop_front();
        }

        debug_log!(
            "economy",
            "budget ledger: day={} income={:.1} expenses={:.1} net={:.1} treasury={:.1} power=produced:{:.1} consumed:{:.1} unmet:{:.1} funding={:.2}",
            entry.day_index,
            entry.income,
            entry.expenses,
            entry.net,
            entry.treasury,
            entry.power_produced,
            entry.power_consumed,
            entry.power_unmet,
            self.service_policy.electricity_funding,
        );
    }

    fn build_budget_ledger_entry(
        &self,
        day_index: u32,
        construction_delta: f64,
    ) -> DailyBudgetLedgerEntry {
        let tax_income = self.treasury.last_daily_income_tax
            + self.treasury.last_daily_household_vat
            + self.treasury.last_daily_business_profit_tax
            + self.treasury.last_daily_property_tax;
        // Transfers post into the day's totals as each household settles (`SIM-02-G`).
        let last_day = &self.households.last_day_totals;
        let benefits = last_day.transfers();
        let unemployment_benefits = last_day.unemployment_benefits;
        let pensions = last_day.pensions;
        let child_support = last_day.child_support;
        let wages = self.households.last_city_service_wages();
        let city_wages = f64::from(wages.total);
        let power = self.households.last_power_settlement();
        let electricity_revenue = f64::from(
            power.household_local_revenue
                + power.private_local_revenue
                + power.city_service_local_cost
                - power.power_auxiliary_local_cost,
        );
        let utility_service_revenue = f64::from(power.utility_local_revenue.max(0.0));
        let imports_owa = f64::from(power.city_service_utility_owa_cost.max(0.0));
        let construction_service_costs = construction_delta
            + self.treasury.last_daily_upkeep.max(0.0)
            + f64::from(power.city_service_utility_local_cost.max(0.0));
        let (coal_inventory, coal_consumed, electricity_fuel_cost, fuel_input_purchases) =
            self.city_service_daily_input_summary();
        let electricity_wage_cost = f64::from(wages.power);
        let power_consumed = f64::from(power.served_units.max(0.0));
        let power_unmet = f64::from((power.demand_units - power.served_units).max(0.0));
        let power_produced = f64::from(power.supply_units.max(0.0));
        let electricity_net = electricity_revenue
            - electricity_fuel_cost
            - electricity_wage_cost
            - f64::from(power.power_upkeep_utility_cost);

        let income = tax_income + utility_service_revenue;
        let expenses =
            benefits + city_wages + fuel_input_purchases + imports_owa + construction_service_costs;
        let net = income - expenses;

        DailyBudgetLedgerEntry {
            day_index,
            income,
            expenses,
            net,
            treasury: self.treasury.balance,
            tax_income,
            income_tax: self.treasury.last_daily_income_tax,
            household_vat: self.treasury.last_daily_household_vat,
            business_profit_tax: self.treasury.last_daily_business_profit_tax,
            property_tax: self.treasury.last_daily_property_tax,
            residential_property_tax: self.treasury.last_daily_residential_property_tax,
            commercial_property_tax: self.treasury.last_daily_commercial_property_tax,
            industrial_property_tax: self.treasury.last_daily_industrial_property_tax,
            utility_service_revenue,
            benefits,
            unemployment_benefits,
            pensions,
            child_support,
            city_wages,
            fuel_input_purchases,
            imports_owa,
            construction_service_costs,
            power_produced,
            power_consumed,
            power_unmet,
            power_coverage: f64::from(power.coverage.clamp(0.0, 1.0)),
            coal_inventory,
            coal_consumed,
            electricity_fuel_cost,
            electricity_wage_cost,
            electricity_revenue,
            electricity_net,
            electricity_consumption_cost: {
                let catalog = load_runtime_economy_catalog().expect("validated economy catalog");
                let tuning = load_runtime_economy_tuning().expect("validated economy tuning");
                let price =
                    crate::simulation::economy::households::local_utility_service_costs(&catalog)
                        [0];
                f64::from(price)
                    * (power_consumed + power_unmet * f64::from(tuning.owa_import_price_multiplier))
            },
        }
    }

    // Coal stock at the latest dispatch, coal burned, power-plant and all city-service input
    // purchases of the last closed day; each posts as it happens (`SIM-02-G`). O(1).
    fn city_service_daily_input_summary(&self) -> (f64, f64, f64, f64) {
        let inputs = self.allocator.last_city_input_day;
        (
            self.households.coal_inventory_kg,
            self.households.last_day_totals.coal_consumed_kg,
            inputs.power,
            inputs.total,
        )
    }

    pub(super) fn print_sim_console_summary(&self, day_index: u32, minute_of_day: u16) {
        let mut at_home = 0usize;
        let mut at_work = 0usize;
        let mut shopping = 0usize;
        let mut travelling = 0usize;
        let mut other = 0usize;

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != TRANSIT_IN_BUILDING {
                travelling += 1;
                continue;
            }

            match self.agents.activity[i] {
                0 => at_home += 1,
                1 => at_work += 1,
                2 => shopping += 1,
                _ => other += 1,
            }
        }

        let household_count = self
            .households
            .households
            .iter()
            .filter(|household| household.member_count > 0)
            .count();
        let hours = minute_of_day / 60;
        let minutes = minute_of_day % 60;

        println!(
            "[SIM_DEBUG] Day {} {:02}:{:02} admit={} remove={} buildings={} households={} agents={} states=(home={}, work={}, shopping={}, travelling={}, other={}) actions=spawn({}/{}/{}) upgrade({}/{}/{}) downgrade({}/{}/{}) despawn({}/{}/{})",
            day_index,
            hours,
            minutes,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.allocator.buildings.len(),
            household_count,
            self.agents.len(),
            at_home,
            at_work,
            shopping,
            travelling,
            other,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    pub(super) fn daily_city_flow_diagnostics(&self) -> DailyCityFlowDiagnostics {
        let mut diagnostics = DailyCityFlowDiagnostics::default();
        let catalog = self.demand.runtime_catalog();
        let mut service_funding_by_building = vec![1.0; self.allocator.buildings.len()];
        let city_funding = self.service_policy.electricity_funding;
        service_funding_by_building
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, value)| {
                let building = &self.allocator.buildings[idx];
                let Some(profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    return;
                };
                if profile.utility_service.as_deref() == Some("power") {
                    *value =
                        effective_service_funding(building.service_funding_override, city_funding);
                }
            });

        for (building_idx, building) in self.allocator.buildings.iter().enumerate() {
            let household_capacity = self.allocator.household_capacity(building_idx);
            if household_capacity > 0 {
                diagnostics.total_household_slots = diagnostics
                    .total_household_slots
                    .saturating_add(household_capacity);
                diagnostics.vacant_household_slots =
                    diagnostics.vacant_household_slots.saturating_add(
                        household_capacity
                            .saturating_sub(building.occupancy.min(household_capacity)),
                    );
            }

            let theoretical_profile =
                catalog.profile_by_runtime_id(building.economy_profile_runtime_id);
            let worker_capacity = self
                .allocator
                .worker_capacity_with_catalog(building_idx, catalog);
            let work_bucket = daily_city_flow_work_bucket(building.zone_type, theoretical_profile);
            if let Some(work_bucket) = work_bucket {
                match work_bucket {
                    DailyCityFlowWorkBucket::Commercial => {
                        diagnostics.commercial_job_capacity = diagnostics
                            .commercial_job_capacity
                            .saturating_add(worker_capacity);
                    }
                    DailyCityFlowWorkBucket::Industrial => {
                        diagnostics.industrial_job_capacity = diagnostics
                            .industrial_job_capacity
                            .saturating_add(worker_capacity);
                    }
                    DailyCityFlowWorkBucket::Service => {}
                }
            }

            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                continue;
            }
            let Some(profile) = theoretical_profile else {
                continue;
            };
            let Some(work_bucket) = work_bucket else {
                continue;
            };
            let active_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                building.commercial_activity_floor_scale,
            );
            let active_worker_capacity = service_funded_worker_capacity(
                active_worker_capacity,
                profile,
                building_idx,
                &service_funding_by_building,
            );
            let active_worker_capacity = active_budget_backed_worker_capacity(
                self.allocator.is_city_service_building(building),
                building.operating_budget,
                active_worker_capacity,
                profile,
            );
            let active_filled_jobs = building.worker_count.min(active_worker_capacity);
            match work_bucket {
                DailyCityFlowWorkBucket::Commercial => {
                    diagnostics.commercial_active_job_capacity = diagnostics
                        .commercial_active_job_capacity
                        .saturating_add(active_worker_capacity);
                    diagnostics.commercial_active_filled_jobs = diagnostics
                        .commercial_active_filled_jobs
                        .saturating_add(active_filled_jobs);
                }
                DailyCityFlowWorkBucket::Industrial => {
                    diagnostics.industrial_active_job_capacity = diagnostics
                        .industrial_active_job_capacity
                        .saturating_add(active_worker_capacity);
                    diagnostics.industrial_active_filled_jobs = diagnostics
                        .industrial_active_filled_jobs
                        .saturating_add(active_filled_jobs);
                }
                DailyCityFlowWorkBucket::Service => {
                    diagnostics.service_active_job_capacity = diagnostics
                        .service_active_job_capacity
                        .saturating_add(active_worker_capacity);
                    diagnostics.service_active_filled_jobs = diagnostics
                        .service_active_filled_jobs
                        .saturating_add(active_filled_jobs);
                }
            }
        }

        for household in &self.households.households {
            if household.member_count == 0 {
                continue;
            }
            diagnostics.active_households = diagnostics.active_households.saturating_add(1);
            if self
                .allocator
                .household_capacity(household.home_building_id)
                > 0
            {
                diagnostics.housed_households = diagnostics.housed_households.saturating_add(1);
            } else {
                diagnostics.unhoused_households = diagnostics.unhoused_households.saturating_add(1);
            }
            if household.budget <= f32::EPSILON {
                diagnostics.zero_budget_households =
                    diagnostics.zero_budget_households.saturating_add(1);
            }
            if household.stock_days <= f32::EPSILON {
                diagnostics.supplies_empty_households =
                    diagnostics.supplies_empty_households.saturating_add(1);
            }
            if household.stock_days <= 1.0 {
                diagnostics.supplies_low_households =
                    diagnostics.supplies_low_households.saturating_add(1);
            }
        }

        for agent_idx in 0..self.agents.len() {
            if self.agents.pending_household_size[agent_idx] > 0 {
                diagnostics.pending_household_carriers =
                    diagnostics.pending_household_carriers.saturating_add(1);
                continue;
            }
            let household_id = self.agents.household_id[agent_idx];
            if household_id == usize::MAX || household_id >= self.households.households.len() {
                continue;
            }
            diagnostics.resident_agents = diagnostics.resident_agents.saturating_add(1);
            match self.agents.age_group[agent_idx] {
                AGE_CHILD => {
                    diagnostics.child_agents = diagnostics.child_agents.saturating_add(1);
                }
                AGE_ADULT => {
                    diagnostics.adult_agents = diagnostics.adult_agents.saturating_add(1);
                }
                AGE_ELDER => {
                    diagnostics.elder_agents = diagnostics.elder_agents.saturating_add(1);
                }
                _ => {}
            }

            if !age_group_can_work(self.agents.age_group[agent_idx]) {
                continue;
            }

            let work_building = self.agents.work_building[agent_idx];
            if work_building >= self.allocator.buildings.len() {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }
            let worker_capacity = self
                .allocator
                .worker_capacity_with_catalog(work_building, catalog);
            if worker_capacity == 0 {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }

            diagnostics.employed_agents = diagnostics.employed_agents.saturating_add(1);
            let workplace = &self.allocator.buildings[work_building];
            match daily_city_flow_work_bucket(
                workplace.zone_type,
                catalog.profile_by_runtime_id(workplace.economy_profile_runtime_id),
            ) {
                Some(DailyCityFlowWorkBucket::Commercial) => {
                    diagnostics.commercial_filled_jobs =
                        diagnostics.commercial_filled_jobs.saturating_add(1);
                }
                Some(DailyCityFlowWorkBucket::Industrial) => {
                    diagnostics.industrial_filled_jobs =
                        diagnostics.industrial_filled_jobs.saturating_add(1);
                }
                _ => {}
            }
        }

        diagnostics
    }

    pub(super) fn log_daily_city_flow_diagnostics(&self, day_index: u32, removed_households: u32) {
        if !crate::debug::Channel::Economy.enabled() {
            return;
        }

        let diagnostics = self.daily_city_flow_diagnostics();
        let total_job_capacity = diagnostics
            .commercial_job_capacity
            .saturating_add(diagnostics.industrial_job_capacity);
        let filled_jobs = diagnostics
            .commercial_filled_jobs
            .saturating_add(diagnostics.industrial_filled_jobs);
        let open_jobs = total_job_capacity.saturating_sub(filled_jobs);
        let commercial_open_jobs = diagnostics
            .commercial_job_capacity
            .saturating_sub(diagnostics.commercial_filled_jobs);
        let industrial_open_jobs = diagnostics
            .industrial_job_capacity
            .saturating_sub(diagnostics.industrial_filled_jobs);
        let active_job_capacity = diagnostics
            .commercial_active_job_capacity
            .saturating_add(diagnostics.industrial_active_job_capacity)
            .saturating_add(diagnostics.service_active_job_capacity);
        let active_filled_jobs = diagnostics
            .commercial_active_filled_jobs
            .saturating_add(diagnostics.industrial_active_filled_jobs)
            .saturating_add(diagnostics.service_active_filled_jobs);
        let active_open_jobs = active_job_capacity.saturating_sub(active_filled_jobs);
        let commercial_active_open_jobs = diagnostics
            .commercial_active_job_capacity
            .saturating_sub(diagnostics.commercial_active_filled_jobs);
        let industrial_active_open_jobs = diagnostics
            .industrial_active_job_capacity
            .saturating_sub(diagnostics.industrial_active_filled_jobs);
        let service_active_open_jobs = diagnostics
            .service_active_job_capacity
            .saturating_sub(diagnostics.service_active_filled_jobs);
        let occupied_household_slots = diagnostics
            .total_household_slots
            .saturating_sub(diagnostics.vacant_household_slots);
        let net_households =
            self.debug_household_admissions_since_daily as i32 - removed_households as i32;

        debug_log!(
            "economy",
            "city flow diagnostics: day={} net_households={:+} admitted_since_daily={} \
             removed_today={} households={} housed={} unhoused={} zero_budget={} \
             supplies_empty={} supplies_low={} resident_agents={} pending_carriers={} \
             children={} adults={} elders={} employed={} unemployed={} jobs={}/{} theoretical_open_jobs={} \
             active_jobs={}/{} active_open_jobs={} commercial_jobs={}/{} commercial_theoretical_open={} \
             commercial_active_jobs={}/{} commercial_active_open={} industrial_jobs={}/{} \
             industrial_theoretical_open={} industrial_active_jobs={}/{} industrial_active_open={} \
             service_active_jobs={}/{} service_active_open={} \
             homes={}/{} vacant_homes={} treasury={:.0} taxes=(income={:.1} household_vat={:.1} \
             business_profit={:.1} property={:.1} lifetime={:.1})",
            day_index,
            net_households,
            self.debug_household_admissions_since_daily,
            removed_households,
            diagnostics.active_households,
            diagnostics.housed_households,
            diagnostics.unhoused_households,
            diagnostics.zero_budget_households,
            diagnostics.supplies_empty_households,
            diagnostics.supplies_low_households,
            diagnostics.resident_agents,
            diagnostics.pending_household_carriers,
            diagnostics.child_agents,
            diagnostics.adult_agents,
            diagnostics.elder_agents,
            diagnostics.employed_agents,
            diagnostics.unemployed_agents,
            filled_jobs,
            total_job_capacity,
            open_jobs,
            active_filled_jobs,
            active_job_capacity,
            active_open_jobs,
            diagnostics.commercial_filled_jobs,
            diagnostics.commercial_job_capacity,
            commercial_open_jobs,
            diagnostics.commercial_active_filled_jobs,
            diagnostics.commercial_active_job_capacity,
            commercial_active_open_jobs,
            diagnostics.industrial_filled_jobs,
            diagnostics.industrial_job_capacity,
            industrial_open_jobs,
            diagnostics.industrial_active_filled_jobs,
            diagnostics.industrial_active_job_capacity,
            industrial_active_open_jobs,
            diagnostics.service_active_filled_jobs,
            diagnostics.service_active_job_capacity,
            service_active_open_jobs,
            occupied_household_slots,
            diagnostics.total_household_slots,
            diagnostics.vacant_household_slots,
            self.treasury.balance,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
        );
    }

    /// Emits the completed day's economy diagnostics without changing settlement state.
    pub(super) fn print_daily_building_economy(&self, day_index: u32) {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        if !crate::debug::Channel::Economy.enabled() {
            return;
        }
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return;
        };
        let service_funding_by_building = self.electricity_funding_by_building();
        let reserved_outbound = self
            .logistics
            .reserved_outbound_view(catalog.resource_count());

        for (idx, b) in self.allocator.buildings.iter().enumerate() {
            if b.zone_type == ZoneType::Residential {
                continue;
            }
            let zone_tag = match b.zone_type {
                ZoneType::Residential => "RES",
                ZoneType::Commercial => "COM",
                ZoneType::Industrial => "IND",
                _ => "OTHER",
            };
            let worker_cap = self
                .allocator
                .worker_capacity_with_catalog(idx, catalog.as_ref());
            let profile = catalog.profile_by_runtime_id(b.economy_profile_runtime_id);
            let profile_id = profile.map(|p| p.id.as_str()).unwrap_or("none");
            let active_worker_capacity = profile
                .map(|p| {
                    active_worker_capacity_for_profile_with_floor_scale(
                        catalog.as_ref(),
                        b,
                        p,
                        b.commercial_activity_floor_scale,
                    )
                })
                .unwrap_or(0);
            let funded_worker_capacity = profile
                .map(|p| {
                    service_funded_worker_capacity(
                        active_worker_capacity,
                        p,
                        idx,
                        &service_funding_by_building,
                    )
                })
                .unwrap_or(0);
            let budget_worker_capacity = profile
                .map(|p| {
                    let wage = p.average_daily_wage();
                    let budget_capacity =
                        if self.allocator.is_city_service_building(b) || wage <= 0.1 {
                            funded_worker_capacity
                        } else {
                            (b.operating_budget.max(0.0) / wage).floor() as u32
                        };
                    funded_worker_capacity.min(budget_capacity)
                })
                .unwrap_or(0);

            // Build inventory snapshot string for all non-zero resources.
            let mut inv_parts = Vec::new();
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount <= 0.0 {
                    continue;
                }
                let rid = (slot + 1) as u16;
                let name = catalog.resource_id_for_runtime_id(rid).unwrap_or("?");
                // capacity from output port if available
                let cap =
                    if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                        p.outputs
                            .iter()
                            .find(|o| o.resource_runtime_id == rid)
                            .map(|o| scaled_output_buffer_capacity_units_for_building(b, p, o))
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                if cap > 0.0 {
                    inv_parts.push(format!("{}={:.1}/{:.1}", name, amount, cap));
                } else {
                    inv_parts.push(format!("{}={:.1}", name, amount));
                }
            }
            let inv_str = if inv_parts.is_empty() {
                "none".to_owned()
            } else {
                inv_parts.join(" ")
            };

            let mut export_parts = Vec::new();
            if let Some(p) = profile {
                for output in &p.outputs {
                    let current_inventory = b.inventory_units(output.resource_runtime_id);
                    let reserved = ShipmentSystem::reservation_slot_for_building(
                        idx,
                        output.resource_runtime_id,
                        catalog.resource_count(),
                    )
                    .and_then(|slot| reserved_outbound.get(slot).copied())
                    .unwrap_or(0.0);
                    let unreserved = (current_inventory - reserved).max(0.0);
                    if unreserved <= 0.0 {
                        continue;
                    }
                    let name = catalog
                        .resource_id_for_runtime_id(output.resource_runtime_id)
                        .unwrap_or("?");
                    let buffer = scaled_output_units_per_day_for_building(b, p, output);
                    let surplus = (unreserved - buffer).max(0.0);
                    export_parts.push(format!(
                        "{}:buffer={:.1} surplus={:.1}",
                        name, buffer, surplus
                    ));
                }
            }
            let export_str = if export_parts.is_empty() {
                "none".to_owned()
            } else {
                export_parts.join(" ")
            };

            // Daily I/O from profile (per-day throughput at full capacity).
            let mut io_parts = Vec::new();
            if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                for port in &p.inputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("-{:.1}{}/day", port.units_per_day, name));
                }
                for port in &p.outputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("+{:.1}{}/day", port.units_per_day, name));
                }
                if p.utility_service.as_deref() == Some("power") {
                    io_parts.push(format!(
                        "power_out_today={:.1}",
                        b.recent_power_service_units
                    ));
                }
            }
            let io_str = if io_parts.is_empty() {
                "none".to_owned()
            } else {
                io_parts.join(" ")
            };

            println!(
                "[ECON] Day {:>4} idx={} {} asset={} profile={} workers={}/{} capacity=[active={} profile={} budget={} area={:.3}] budget={:.1} revenue={:.1} distress={} broken={} io=[{}] inventory=[{}] export=[{}]",
                day_index,
                idx,
                zone_tag,
                b.asset_id,
                profile_id,
                b.worker_count,
                active_worker_capacity,
                active_worker_capacity,
                worker_cap,
                budget_worker_capacity,
                b.work_area_scale,
                b.operating_budget,
                b.revenue,
                if b.budget_distress { "Y" } else { "N" },
                if b.broken || b.economy_broken {
                    "Y"
                } else {
                    "N"
                },
                io_str,
                inv_str,
                export_str,
            );
        }

        let mut households_at_budget_floor = 0u32;
        let mut households_below_1d_supplies = 0u32;
        let mut households_below_2d_supplies = 0u32;
        let mut households_below_3d_supplies = 0u32;
        let mut total_wages_paid = 0.0f32;
        let mut total_household_shopping_spend = 0.0f32;
        let mut total_unemployment_benefits_paid = 0.0f32;
        let mut total_pensions_paid = 0.0f32;
        let mut total_child_support_paid = 0.0f32;
        let mut total_property_tax_paid = 0.0f32;
        let mut total_household_supply_use_cost = 0.0f32;
        let mut total_household_utility_cost = 0.0f32;

        for (idx, h) in self.households.households.iter().enumerate() {
            if h.member_count == 0 {
                continue;
            }
            let ledger = self
                .households
                .daily_ledgers()
                .get(idx)
                .copied()
                .unwrap_or_default();
            if h.budget <= f32::EPSILON {
                households_at_budget_floor += 1;
            }
            if h.stock_days < 1.0 {
                households_below_1d_supplies += 1;
            }
            if h.stock_days < 2.0 {
                households_below_2d_supplies += 1;
            }
            if h.stock_days < 3.0 {
                households_below_3d_supplies += 1;
            }
            total_wages_paid += ledger.wage_income;
            total_household_shopping_spend += ledger.shopping_spend;
            total_unemployment_benefits_paid += ledger.unemployment_benefit_income;
            total_pensions_paid += ledger.pension_income;
            total_child_support_paid += ledger.child_support_income;
            total_property_tax_paid += ledger.property_tax_paid;
            total_household_supply_use_cost += ledger.household_supply_consumption_cost;
            let household_utility_cost = ledger.power_consumption_cost
                + ledger.water_consumption_cost
                + ledger.sewage_consumption_cost;
            total_household_utility_cost += household_utility_cost;
            let home_asset = self
                .allocator
                .buildings
                .get(h.home_building_id)
                .map(|b| b.asset_id.as_str())
                .unwrap_or("none");

            let state_str = match h.replenishment_state {
                0 => "STABLE",
                1 => "NEEDS",
                2 => "WAITING_SHOPPER",
                3 => "SHOPPING_TO_STORE",
                4 => "SHOPPING_RETURNING",
                5 => "FULFILLED",
                6 => "COOLDOWN",
                7 => "FAILED_TERMINAL",
                _ => "UNKNOWN",
            };

            let ub_str = if h.unemployment_days_elapsed > 0 {
                format!(" ub={}d", h.unemployment_days_elapsed)
            } else {
                String::new()
            };
            println!(
                "[ECON] Day {:>4} HH:{:<2} home_idx={:<2} asset={} residents={} children={} adults={} elders={} budget={:<5.1} supplies={:<4.2}days state={}{} ledger=(before={:.1} wage={:.1} transfer={:.1} unemployment={:.1} pension={:.1} child_support={:.1} property_tax={:.1} shopping={:.1} power={:.1} water={:.1} sewage={:.1} utility={:.1} supply_use={:.1} utility_supply={:.1} after={:.1} unemployed_adults={} shopper_trips={}/{})",
                day_index,
                idx,
                h.home_building_id,
                home_asset,
                h.member_count,
                h.child_count,
                h.adult_count,
                h.elder_count,
                h.budget,
                h.stock_days,
                state_str,
                ub_str,
                ledger.budget_before,
                ledger.wage_income,
                ledger.transfer_income(),
                ledger.unemployment_benefit_income,
                ledger.pension_income,
                ledger.child_support_income,
                ledger.property_tax_paid,
                ledger.shopping_spend,
                ledger.power_consumption_cost,
                ledger.water_consumption_cost,
                ledger.sewage_consumption_cost,
                household_utility_cost,
                ledger.household_supply_consumption_cost,
                household_utility_cost + ledger.household_supply_consumption_cost,
                ledger.budget_after,
                ledger.unemployed_adults,
                ledger.shopper_trips_completed,
                ledger.shopper_trips_failed,
            );
        }
        println!(
            "[ECON] Day {:>4} household ledger summary: budget_floor={} supplies_below_1d={} supplies_below_2d={} supplies_below_3d={} wages_paid={:.1} shopping_spend={:.1} transfers_paid={:.1} unemployment_paid={:.1} pensions_paid={:.1} child_support_paid={:.1} property_tax_paid={:.1} utility_cost={:.1} supply_use_cost={:.1} utility_supply_cost={:.1}",
            day_index,
            households_at_budget_floor,
            households_below_1d_supplies,
            households_below_2d_supplies,
            households_below_3d_supplies,
            total_wages_paid,
            total_household_shopping_spend,
            total_unemployment_benefits_paid + total_pensions_paid + total_child_support_paid,
            total_unemployment_benefits_paid,
            total_pensions_paid,
            total_child_support_paid,
            total_property_tax_paid,
            total_household_utility_cost,
            total_household_supply_use_cost,
            total_household_utility_cost + total_household_supply_use_cost,
        );
        println!(
            "[ECON] Day {:>4} fiscal summary: income_tax={:.1} household_vat={:.1} business_profit_tax={:.1} property_tax={:.1} residential_property_tax={:.1} commercial_property_tax={:.1} industrial_property_tax={:.1} tax_total={:.1} lifetime_tax={:.1} road_upkeep={:.1} treasury={:.1}",
            day_index,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.last_daily_residential_property_tax,
            self.treasury.last_daily_commercial_property_tax,
            self.treasury.last_daily_industrial_property_tax,
            self.treasury.last_daily_income_tax
                + self.treasury.last_daily_household_vat
                + self.treasury.last_daily_business_profit_tax
                + self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
            self.treasury.last_daily_upkeep,
            self.treasury.balance,
        );
    }

    pub(super) fn collect_fiscal_revenue(&mut self, revenue: FiscalRevenue) {
        self.treasury.collect_income_tax(revenue.income_tax as f64);
        self.treasury
            .collect_household_vat(revenue.household_vat as f64);
        self.treasury
            .collect_business_profit_tax(revenue.business_profit_tax as f64);
        self.treasury.collect_property_tax_for_zone(
            ZoneType::Residential,
            revenue.residential_property_tax as f64,
        );
        self.treasury.collect_property_tax_for_zone(
            ZoneType::Commercial,
            revenue.commercial_property_tax as f64,
        );
        self.treasury.collect_property_tax_for_zone(
            ZoneType::Industrial,
            revenue.industrial_property_tax as f64,
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::nodes::sim::core::tests::fields::{add_farm_household, farm_fixture};

    #[test]
    fn rolling_road_length_sums_the_network_over_one_day() {
        use crate::simulation::core::scheduler::SLOTS_PER_DAY;
        let (core, _, _) = farm_fixture();
        let graph = &core.region_graph;
        let total: f64 = graph
            .edges()
            .iter()
            .filter(|edge| !edge.deleted)
            .map(|edge| f64::from(edge.physical_length))
            .sum();
        assert!(total > 0.0);
        let mut unmeasured = super::RoadLengthRoll::default();
        assert!((unmeasured.length_m(graph) - total).abs() < 1e-6 * total);
        let mut roll = super::RoadLengthRoll::default();
        // A sweep started mid-day finishes nothing; the next full day does.
        for slot in SLOTS_PER_DAY / 2..2 * SLOTS_PER_DAY {
            roll.advance(slot, graph);
            assert_eq!(roll.measured, slot >= 2 * SLOTS_PER_DAY - 1, "slot {slot}");
        }
        assert!((roll.front_m - total).abs() < 1e-6 * total);
    }

    #[test]
    fn a_road_length_sweep_resumes_identically_on_the_edges_a_save_keeps() {
        use crate::simulation::core::scheduler::SLOTS_PER_DAY;
        use crate::simulation::network::graph::{Edge, RegionGraph};
        use crate::simulation::network::types::NodeType;
        use godot::prelude::Vector3;
        // Saving drops deleted edges and renumbers the rest in order.
        let graph = |lengths: &[(f32, bool)]| {
            let mut graph = RegionGraph::new();
            let a = graph.add_node(Vector3::ZERO, NodeType::Junction);
            let b = graph.add_node(Vector3::new(10.0, 0.0, 0.0), NodeType::Junction);
            for &(length, deleted) in lengths {
                let id = graph.add_edge(Edge {
                    start_node: a,
                    end_node: b,
                    physical_length: length,
                    ..Edge::default()
                });
                graph.edge_mut(id).deleted = deleted;
            }
            graph
        };
        let lengths: Vec<(f32, bool)> = (0..9)
            .map(|i| (10.0 + i as f32 * 1.375, i == 1 || i == 7))
            .collect();
        let live = graph(&lengths);
        let saved = graph(
            &lengths
                .iter()
                .filter(|(_, deleted)| !deleted)
                .copied()
                .collect::<Vec<_>>(),
        );
        let mut straight = super::RoadLengthRoll::default();
        let half = SLOTS_PER_DAY / 2;
        for slot in 0..half {
            straight.advance(slot, &live);
        }
        let (sweep_edges, next_edge) = straight.saved_positions(&live);
        let mut resumed = super::RoadLengthRoll {
            sweep_edges,
            next_edge,
            ..straight
        };
        for slot in half..SLOTS_PER_DAY {
            straight.advance(slot, &live);
            resumed.advance(slot, &saved);
        }
        assert!(straight.measured && resumed.measured);
        assert_eq!(straight.front_m.to_bits(), resumed.front_m.to_bits());
    }

    #[test]
    fn the_daily_phase_closes_household_payments_without_a_logging_call() {
        use crate::simulation::core::phases::daily_phase;
        use crate::simulation::core::scheduler::SLOTS_PER_DAY;
        let (mut core, farm, polygon) = farm_fixture();
        core.commit_field_polygon_internal(farm, polygon).unwrap();
        let household = add_farm_household(&mut core, farm);
        core.households
            .restore_utility_payments(household, [3.0, 2.0, 1.0]);
        // The slots up to the one of the household's daily phase, where it settles its day.
        let handles = &mut core.households.handles;
        handles.register_appended(core.households.households.len());
        let phase = u64::from(daily_phase(handles.handle(household)));
        let next = core.time.absolute_slot() + 1;
        core.run_slots_through_for_test(
            next + (phase + SLOTS_PER_DAY - next % SLOTS_PER_DAY) % SLOTS_PER_DAY,
        );
        let ledger = &core.households.daily_ledgers()[household];
        assert_eq!(ledger.power_consumption_cost, 0.0);
        assert_eq!(ledger.water_consumption_cost, 0.0);
        assert_eq!(ledger.sewage_consumption_cost, 0.0);
        assert_eq!(
            ledger.budget_before,
            core.households.households[household].budget
        );
    }

    #[test]
    fn farm_diagnostics_count_filled_jobs_and_household_independently() {
        let (mut core, farm, polygon) = farm_fixture();
        core.commit_field_polygon_internal(farm, polygon).unwrap();
        add_farm_household(&mut core, farm);
        let diagnostics = core.daily_city_flow_diagnostics();
        assert_eq!(diagnostics.total_household_slots, 1);
        assert_eq!(diagnostics.housed_households, 1);
        assert_eq!(diagnostics.vacant_household_slots, 0);
        // The fixture commits 20 ha, covered by the grain farm's two-worker minimum.
        assert_eq!(diagnostics.industrial_job_capacity, 2);
        assert_eq!(
            diagnostics.industrial_filled_jobs,
            diagnostics.employed_agents
        );
        assert!(diagnostics.industrial_filled_jobs > 0);
        assert_eq!(diagnostics.commercial_filled_jobs, 0);
    }
    #[test]
    fn budget_counts_machinery_for_all_utilities_without_treating_it_as_coal() {
        let (mut core, farm, _) = farm_fixture();
        let catalog =
            crate::simulation::economy::definitions::load_runtime_economy_catalog().unwrap();
        let template = core.allocator.buildings[farm].clone();
        let before = core.build_budget_ledger_entry(1, 0.0);
        let mut providers = Vec::new();
        for (id, cost) in [
            ("power_plant_basic", 70.0),
            ("water_plant_basic", 35.0),
            ("wastewater_treatment_basic", 17.5),
        ] {
            let mut provider = template.clone();
            provider.economy_profile_runtime_id = catalog.profile_for_id(id).unwrap().runtime_id;
            core.allocator.buildings.push(provider);
            let provider = core.allocator.buildings.len() - 1;
            core.allocator.record_city_input_purchase(provider, cost);
            providers.push(provider);
        }
        // Coal is what the hourly dispatches burned and the stock they left, not machinery.
        core.households.coal_inventory_kg = 10.0;
        core.households.day_totals.coal_consumed_kg = 540.0;
        core.households.close_calendar_day();
        core.allocator.close_city_input_day();
        let ledger = core.build_budget_ledger_entry(1, 0.0);
        assert_eq!(ledger.fuel_input_purchases, 122.5);
        assert_eq!(ledger.electricity_fuel_cost, 70.0);
        assert_eq!(ledger.coal_inventory, 10.0);
        assert_eq!(ledger.coal_consumed, 540.0);
        assert_eq!(ledger.expenses - before.expenses, 122.5);
        assert_eq!(ledger.net - before.net, -122.5);
        // A refund nets against the purchases of the day it is paid back.
        for (provider, cost) in providers.into_iter().zip([70.0, 35.0, -35.0]) {
            core.allocator.record_city_input_purchase(provider, cost);
        }
        core.allocator.close_city_input_day();
        let refunded = core.build_budget_ledger_entry(1, 0.0);
        assert_eq!(refunded.fuel_input_purchases, 70.0);
        assert_eq!(refunded.expenses - ledger.expenses, -52.5);
    }
}
