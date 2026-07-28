//! City treasury, service policy, fiscal reporting, and economy diagnostics.

use super::state::SimCore;
use crate::debug_log;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, TRANSIT_IN_BUILDING, age_group_can_work,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::definitions::{EconomyProfileRuntime, EconomyProfileRuntimeKind};
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::{
    active_worker_capacity_for_profile_with_floor_scale, commercial_activity_signal_for_city,
    service_funded_worker_capacity,
};
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

/// Currency cost per meter of new road laid, deducted from the city treasury at placement.
pub(crate) const ROAD_BUILD_COST_PER_METER: f64 = 50.0;
/// Starter build cost per service-building lot cell, deducted from the city treasury at placement.
pub(crate) const SERVICE_BUILD_COST_PER_LOT_CELL: f64 = 2_500.0;
/// Currency upkeep per meter of road per day, settled from the city treasury each day.
pub(crate) const ROAD_UPKEEP_PER_METER_PER_DAY: f64 = 0.1;
/// Stable service policy id for the city electricity service.
pub(crate) const SERVICE_POLICY_ELECTRICITY: &str = "electricity";
/// Number of completed daily budget entries kept for UI trend graphs.
pub(crate) const ECONOMY_HISTORY_DAYS: usize = 180;

fn is_electricity_service(service_id: &str) -> bool {
    matches!(service_id, SERVICE_POLICY_ELECTRICITY | "power")
}

fn effective_service_funding(building_override: f32, city_funding: f32) -> f32 {
    if building_override >= 0.0 {
        building_override.clamp(0.0, 1.0)
    } else {
        city_funding
    }
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
    /// Business purchase tax collected from input freight.
    pub(crate) business_purchase_tax: f64,
    /// Business profit tax collected from positive private operating-budget growth.
    pub(crate) business_profit_tax: f64,
    /// One-time private construction property tax collected during the day.
    pub(crate) property_tax: f64,
    /// Residential construction property tax collected during the day.
    pub(crate) residential_property_tax: f64,
    /// Commercial construction property tax collected during the day.
    pub(crate) commercial_property_tax: f64,
    /// Industrial construction property tax collected during the day.
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
    /// Estimated coal units bought for city power providers during the day.
    pub(crate) coal_bought: f64,
    /// Estimated coal units consumed by city power providers during the day.
    pub(crate) coal_consumed: f64,
    /// City fuel/input cost attributable to electricity providers.
    pub(crate) electricity_fuel_cost: f64,
    /// City payroll cost attributable to electricity providers.
    pub(crate) electricity_wage_cost: f64,
    /// Local electricity revenue collected from consumers.
    pub(crate) electricity_revenue: f64,
    /// Local electricity service balance after fuel and payroll.
    pub(crate) electricity_net: f64,
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
    /// Business purchase tax collected in the most recently finalized fiscal day.
    pub last_daily_business_purchase_tax: f64,
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
    /// Business purchase tax collected since the last daily fiscal finalization.
    pub pending_business_purchase_tax: f64,
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
            last_daily_business_purchase_tax: 0.0,
            last_daily_business_profit_tax: 0.0,
            last_daily_property_tax: 0.0,
            last_daily_residential_property_tax: 0.0,
            last_daily_commercial_property_tax: 0.0,
            last_daily_industrial_property_tax: 0.0,
            pending_income_tax: 0.0,
            pending_household_vat: 0.0,
            pending_business_purchase_tax: 0.0,
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
        self.lifetime_build_cost += amount;
    }

    /// Settles one day's infrastructure upkeep cost. Balance may go negative.
    pub(crate) fn settle_daily_upkeep(&mut self, amount: f64) {
        self.balance -= amount;
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

    /// Records tax collected from business input purchases.
    pub(crate) fn collect_business_purchase_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessPurchase);
    }

    /// Records tax collected from positive daily business profit.
    pub(crate) fn collect_business_profit_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessProfit);
    }

    /// Records one-time property tax from new private construction.
    pub(crate) fn collect_property_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Property);
    }

    /// Records one-time property tax from new private construction by zone.
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
        self.last_daily_business_purchase_tax = self.pending_business_purchase_tax;
        self.last_daily_business_profit_tax = self.pending_business_profit_tax;
        self.last_daily_property_tax = self.pending_property_tax;
        self.last_daily_residential_property_tax = self.pending_residential_property_tax;
        self.last_daily_commercial_property_tax = self.pending_commercial_property_tax;
        self.last_daily_industrial_property_tax = self.pending_industrial_property_tax;
        self.pending_income_tax = 0.0;
        self.pending_household_vat = 0.0;
        self.pending_business_purchase_tax = 0.0;
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
            TaxBucket::BusinessPurchase => self.pending_business_purchase_tax += amount,
            TaxBucket::BusinessProfit => self.pending_business_profit_tax += amount,
            TaxBucket::Property => self.pending_property_tax += amount,
        }
    }
}

#[derive(Clone, Copy)]
enum TaxBucket {
    Income,
    HouseholdVat,
    BusinessPurchase,
    BusinessProfit,
    Property,
}

#[derive(Clone, Copy, Debug, Default)]
struct DailyCityFlowDiagnostics {
    active_households: u32,
    housed_households: u32,
    unhoused_households: u32,
    zero_budget_households: u32,
    stock_empty_households: u32,
    stock_low_households: u32,
    total_household_slots: u32,
    vacant_household_slots: u32,
    resident_agents: u32,
    child_agents: u32,
    adult_agents: u32,
    elder_agents: u32,
    pending_household_carriers: u32,
    employed_agents: u32,
    unemployed_agents: u32,
    commercial_job_capacity: u32,
    commercial_filled_jobs: u32,
    commercial_active_job_capacity: u32,
    commercial_active_filled_jobs: u32,
    industrial_job_capacity: u32,
    industrial_filled_jobs: u32,
    industrial_active_job_capacity: u32,
    industrial_active_filled_jobs: u32,
    service_active_job_capacity: u32,
    service_active_filled_jobs: u32,
}

fn profile_offers_daily_city_flow_work(
    building_zone: ZoneType,
    profile: &EconomyProfileRuntime,
) -> bool {
    matches!(building_zone, ZoneType::Commercial | ZoneType::Industrial)
        || matches!(
            profile.kind,
            EconomyProfileRuntimeKind::UtilityProducer
                | EconomyProfileRuntimeKind::UtilityProcessor
        )
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
    /// Applies the gameplay cheat grant and pins all demand channels to maximum pressure.
    pub(crate) fn apply_money_and_max_demand_cheat(&mut self, money_amount: f64) -> f64 {
        if money_amount.is_finite() && money_amount > 0.0 {
            self.treasury.balance += money_amount;
        }
        self.demand.enable_max_demand_cheat();
        self.treasury.balance
    }

    /// Applies a live service funding policy change from the UI.
    pub(crate) fn set_service_funding(&mut self, service_id: &str, funding: f32) -> bool {
        if !is_electricity_service(service_id) {
            return false;
        }
        let previous = self.service_policy.electricity_funding;
        let requested = funding;
        let funding = funding.clamp(0.0, 1.0);
        self.service_policy.electricity_funding = funding;
        self.apply_service_funding_staffing_policy();
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
        let service_funding_by_building = self.electricity_funding_by_building();
        self.demand.refresh_pressure_channels_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            self.treasury.balance,
            &service_funding_by_building,
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
        if !is_electricity_service(service_id) {
            return false;
        }
        if self.allocator.dirty_index {
            self.allocator.rebuild_zone_index();
        }
        let Some(building_idx) = self.nearest_building_idx_at(world_x, world_z, 30.0) else {
            return false;
        };
        if !self.building_provides_service(building_idx, "power") {
            return false;
        }
        self.allocator.buildings[building_idx].service_funding_override = funding.clamp(0.0, 1.0);
        self.apply_service_funding_staffing_policy();
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
        let buildings = &self.allocator.buildings;
        let city_funding = self.service_policy.electricity_funding;
        funding.par_iter_mut().enumerate().for_each(|(idx, value)| {
            let building = &buildings[idx];
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                return;
            };
            if profile.utility_service.as_deref() == Some("power") {
                *value = effective_service_funding(building.service_funding_override, city_funding);
            }
        });
        funding
    }

    pub(crate) fn apply_service_funding_staffing_policy(&mut self) {
        let funding = self.electricity_funding_by_building();
        self.households.enforce_service_funding_staffing(
            &mut self.agents,
            &mut self.allocator,
            &funding,
        );
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
             household_vat={:.1}% business_purchase={:.1}% business_profit={:.1}% \
             property_tax=res:{:.0},com:{:.0},ind:{:.0},level_mult:{:.2})",
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
            percent_for_log(policy.business_purchase_tax_rate),
            percent_for_log(policy.business_profit_tax_rate),
            policy.residential_property_tax_base,
            policy.commercial_property_tax_base,
            policy.industrial_property_tax_base,
            policy.property_tax_level_multiplier,
        );
    }

    fn nearest_building_idx_at(&self, world_x: f32, world_z: f32, radius_m: f32) -> Option<usize> {
        let mut candidates = Vec::with_capacity(1);
        self.allocator
            .fill_nearby_buildings(world_x, world_z, 1, 1, &mut candidates, |_, _| true);
        let building_idx = candidates.into_iter().next()?;
        let building = &self.allocator.buildings[building_idx];
        let dx = building.center_x - world_x;
        let dz = building.center_y - world_z;
        (dx * dx + dz * dz < radius_m.max(0.0).powi(2)).then_some(building_idx)
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
            + self.treasury.last_daily_business_purchase_tax
            + self.treasury.last_daily_business_profit_tax
            + self.treasury.last_daily_property_tax;
        let benefits = self
            .households
            .daily_ledgers()
            .iter()
            .map(|ledger| f64::from(ledger.transfer_income().max(0.0)))
            .sum::<f64>();
        let unemployment_benefits = self
            .households
            .daily_ledgers()
            .iter()
            .map(|ledger| f64::from(ledger.unemployment_benefit_income.max(0.0)))
            .sum::<f64>();
        let pensions = self
            .households
            .daily_ledgers()
            .iter()
            .map(|ledger| f64::from(ledger.pension_income.max(0.0)))
            .sum::<f64>();
        let child_support = self
            .households
            .daily_ledgers()
            .iter()
            .map(|ledger| f64::from(ledger.child_support_income.max(0.0)))
            .sum::<f64>();
        let city_wages = f64::from(self.households.last_city_service_wage_cost().max(0.0));
        let power = self.households.last_power_settlement();
        let utility_service_revenue = f64::from(
            power.household_local_revenue
                + power.private_local_revenue
                + power.city_service_local_cost,
        );
        let imports_owa = f64::from(power.city_service_owa_cost.max(0.0));
        let construction_service_costs = construction_delta
            + self.treasury.last_daily_upkeep.max(0.0)
            + f64::from(power.city_service_local_cost.max(0.0));
        let (coal_inventory, coal_bought, coal_consumed, electricity_fuel_cost) =
            self.electricity_provider_daily_fuel_summary();
        let electricity_wage_cost = city_wages;
        let power_consumed = f64::from(power.served_units.max(0.0));
        let power_unmet = f64::from((power.demand_units - power.served_units).max(0.0));
        let power_produced = f64::from(power.supply_units.max(0.0));
        let electricity_revenue = utility_service_revenue;
        let electricity_net = electricity_revenue - electricity_fuel_cost - electricity_wage_cost;

        let income = tax_income + utility_service_revenue;
        let expenses = benefits
            + city_wages
            + electricity_fuel_cost
            + imports_owa
            + construction_service_costs;
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
            business_purchase_tax: self.treasury.last_daily_business_purchase_tax,
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
            fuel_input_purchases: electricity_fuel_cost,
            imports_owa,
            construction_service_costs,
            power_produced,
            power_consumed,
            power_unmet,
            power_coverage: f64::from(power.coverage.clamp(0.0, 1.0)),
            coal_inventory,
            coal_bought,
            coal_consumed,
            electricity_fuel_cost,
            electricity_wage_cost,
            electricity_revenue,
            electricity_net,
        }
    }

    fn electricity_provider_daily_fuel_summary(&self) -> (f64, f64, f64, f64) {
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return (0.0, 0.0, 0.0, 0.0);
        };
        let coal_runtime_id = catalog.resource_runtime_id_for_id("coal");
        let mut coal_inventory = 0.0f64;
        let mut coal_consumed = 0.0f64;
        let mut fuel_cost = 0.0f64;

        // Fiscal reports keep index-order floating-point accumulation deterministic.
        for building in &self.allocator.buildings {
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.utility_service.as_deref() != Some("power") {
                continue;
            }
            fuel_cost += f64::from(building.daily_city_funded_input_cost.max(0.0));
            if let Some(coal_runtime_id) = coal_runtime_id {
                coal_inventory += f64::from(building.inventory_units(coal_runtime_id).max(0.0));
            }
            for input_port in &profile.inputs {
                if Some(input_port.resource_runtime_id) != coal_runtime_id {
                    continue;
                }
                if profile.base_rate_units_per_day <= f32::EPSILON {
                    continue;
                }
                let produced_ratio =
                    building.daily_power_service_units.max(0.0) / profile.base_rate_units_per_day;
                coal_consumed += f64::from(input_port.units_per_day.max(0.0) * produced_ratio);
            }
        }

        let coal_bought = coal_runtime_id
            .and_then(|resource| catalog.unit_price_for_resource(resource))
            .filter(|unit_price| *unit_price > f32::EPSILON)
            .map(|unit_price| fuel_cost / f64::from(unit_price))
            .unwrap_or(0.0);

        (coal_inventory, coal_bought, coal_consumed, fuel_cost)
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
            "[SIM_DEBUG] Day {} {:02}:{:02} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} buildings={} households={} agents={} states=(home={}, work={}, shopping={}, travelling={}, other={}) actions=spawn({}/{}/{}) upgrade({}/{}/{}) downgrade({}/{}/{}) despawn({}/{}/{})",
            day_index,
            hours,
            minutes,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
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

    fn daily_city_flow_diagnostics(&self) -> DailyCityFlowDiagnostics {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        let mut diagnostics = DailyCityFlowDiagnostics::default();
        let catalog = load_runtime_economy_catalog().ok();
        let commercial_activity_floor_scale = catalog
            .as_ref()
            .map(|catalog| {
                commercial_activity_signal_for_city(
                    catalog.as_ref(),
                    &self.households.households,
                    &self.allocator,
                )
                .activity_floor_scale
            })
            .unwrap_or(0.0);
        let mut service_funding_by_building = vec![1.0; self.allocator.buildings.len()];
        if let Some(catalog) = catalog.as_ref() {
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
                        *value = effective_service_funding(
                            building.service_funding_override,
                            city_funding,
                        );
                    }
                });
        }

        for (building_idx, building) in self.allocator.buildings.iter().enumerate() {
            if matches!(building.zone_type, ZoneType::Residential) {
                let household_capacity = self.allocator.household_capacity(building_idx);
                diagnostics.total_household_slots = diagnostics
                    .total_household_slots
                    .saturating_add(household_capacity);
                diagnostics.vacant_household_slots =
                    diagnostics.vacant_household_slots.saturating_add(
                        household_capacity
                            .saturating_sub(building.occupancy.min(household_capacity)),
                    );
            }

            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(building_idx, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(building_idx));
            match building.zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_job_capacity = diagnostics
                        .commercial_job_capacity
                        .saturating_add(worker_capacity);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_job_capacity = diagnostics
                        .industrial_job_capacity
                        .saturating_add(worker_capacity);
                }
                _ => {}
            }

            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
            {
                continue;
            }
            let Some((catalog, profile)) = catalog.as_ref().and_then(|catalog| {
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map(|profile| (catalog.as_ref(), profile))
            }) else {
                continue;
            };
            if !profile_offers_daily_city_flow_work(building.zone_type, profile) {
                continue;
            }
            let active_worker_capacity = active_worker_capacity_for_profile_with_floor_scale(
                catalog,
                building,
                profile,
                commercial_activity_floor_scale,
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
            match building.zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_active_job_capacity = diagnostics
                        .commercial_active_job_capacity
                        .saturating_add(active_worker_capacity);
                    diagnostics.commercial_active_filled_jobs = diagnostics
                        .commercial_active_filled_jobs
                        .saturating_add(active_filled_jobs);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_active_job_capacity = diagnostics
                        .industrial_active_job_capacity
                        .saturating_add(active_worker_capacity);
                    diagnostics.industrial_active_filled_jobs = diagnostics
                        .industrial_active_filled_jobs
                        .saturating_add(active_filled_jobs);
                }
                _ => {
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
            let live_home = self
                .allocator
                .buildings
                .get(household.home_building_id)
                .is_some_and(|building| {
                    !building.broken
                        && !building.economy_broken
                        && !building.is_deserted
                        && building.is_operational()
                });
            if live_home {
                diagnostics.housed_households = diagnostics.housed_households.saturating_add(1);
            } else {
                diagnostics.unhoused_households = diagnostics.unhoused_households.saturating_add(1);
            }
            if household.budget <= f32::EPSILON {
                diagnostics.zero_budget_households =
                    diagnostics.zero_budget_households.saturating_add(1);
            }
            if household.stock_days <= f32::EPSILON {
                diagnostics.stock_empty_households =
                    diagnostics.stock_empty_households.saturating_add(1);
            }
            if household.stock_days <= 1.0 {
                diagnostics.stock_low_households =
                    diagnostics.stock_low_households.saturating_add(1);
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
            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(work_building, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(work_building));
            if worker_capacity == 0 {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }

            diagnostics.employed_agents = diagnostics.employed_agents.saturating_add(1);
            match self.allocator.buildings[work_building].zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_filled_jobs =
                        diagnostics.commercial_filled_jobs.saturating_add(1);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_filled_jobs =
                        diagnostics.industrial_filled_jobs.saturating_add(1);
                }
                _ => {}
            }
        }

        diagnostics
    }

    pub(super) fn log_daily_city_flow_diagnostics(&self, day_index: u32, removed_households: u32) {
        if !crate::debug::category_enabled("economy") {
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
             stock_empty={} stock_low={} resident_agents={} pending_carriers={} \
             children={} adults={} elders={} employed={} unemployed={} jobs={}/{} theoretical_open_jobs={} \
             active_jobs={}/{} active_open_jobs={} commercial_jobs={}/{} commercial_theoretical_open={} \
             commercial_active_jobs={}/{} commercial_active_open={} industrial_jobs={}/{} \
             industrial_theoretical_open={} industrial_active_jobs={}/{} industrial_active_open={} \
             service_active_jobs={}/{} service_active_open={} \
             homes={}/{} vacant_homes={} treasury={:.0} taxes=(income={:.1} household_vat={:.1} \
             business_purchase={:.1} business_profit={:.1} property={:.1} lifetime={:.1})",
            day_index,
            net_households,
            self.debug_household_admissions_since_daily,
            removed_households,
            diagnostics.active_households,
            diagnostics.housed_households,
            diagnostics.unhoused_households,
            diagnostics.zero_budget_households,
            diagnostics.stock_empty_households,
            diagnostics.stock_low_households,
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
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
        );
    }

    fn print_daily_building_economy(&mut self, day_index: u32) {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        if !crate::debug::category_enabled("economy") {
            self.households.reset_daily_ledgers();
            return;
        }
        let Ok(catalog) = load_runtime_economy_catalog() else {
            self.households.reset_daily_ledgers();
            return;
        };

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
            let _resident_cap = self.allocator.household_capacity(idx);
            let profile_id = catalog
                .profile_by_runtime_id(b.economy_profile_runtime_id)
                .map(|p| p.id.as_str())
                .unwrap_or("none");

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
                            .map(|o| p.output_buffer_capacity_units_for(o))
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
                "[ECON] Day {:>4} idx={} {} asset={} profile={} workers={}/{} budget={:.1} revenue={:.1} distress={} broken={} io=[{}] inventory=[{}]",
                day_index,
                idx,
                zone_tag,
                b.asset_id,
                profile_id,
                b.worker_count,
                worker_cap,
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
            );
        }

        let mut households_at_budget_floor = 0u32;
        let mut households_below_1d_stock = 0u32;
        let mut households_below_2d_stock = 0u32;
        let mut households_below_3d_stock = 0u32;
        let mut total_wages_paid = 0.0f32;
        let mut total_household_shopping_spend = 0.0f32;
        let mut total_benefits_paid = 0.0f32;
        let mut total_unemployment_benefits_paid = 0.0f32;
        let mut total_pensions_paid = 0.0f32;
        let mut total_child_support_paid = 0.0f32;
        let mut total_utility_stock_cost = 0.0f32;
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
                households_below_1d_stock += 1;
            }
            if h.stock_days < 2.0 {
                households_below_2d_stock += 1;
            }
            if h.stock_days < 3.0 {
                households_below_3d_stock += 1;
            }
            total_wages_paid += ledger.wage_income;
            total_household_shopping_spend += ledger.shopping_spend;
            total_benefits_paid += ledger.transfer_income();
            total_unemployment_benefits_paid += ledger.unemployment_benefit_income;
            total_pensions_paid += ledger.pension_income;
            total_child_support_paid += ledger.child_support_income;
            total_utility_stock_cost += ledger.utility_stock_consumption_cost;
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
                "[ECON] Day {:>4} HH:{:<2} home_idx={:<2} asset={} residents={} children={} adults={} elders={} budget={:<5.1} stock={:<4.2}days state={}{} ledger=(before={:.1} wage={:.1} transfer={:.1} unemployment={:.1} pension={:.1} child_support={:.1} shopping={:.1} power={:.1} water={:.1} sewage={:.1} utility={:.1} stock_use={:.1} utility_stock={:.1} after={:.1} unemployed_adults={} shopper_trips={}/{})",
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
                ledger.shopping_spend,
                ledger.power_consumption_cost,
                ledger.water_consumption_cost,
                ledger.sewage_consumption_cost,
                household_utility_cost,
                ledger.household_supply_consumption_cost,
                ledger.utility_stock_consumption_cost,
                ledger.budget_after,
                ledger.unemployed_adults,
                ledger.shopper_trips_completed,
                ledger.shopper_trips_failed,
            );
        }
        println!(
            "[ECON] Day {:>4} household ledger summary: budget_floor={} stock_below_1d={} stock_below_2d={} stock_below_3d={} wages_paid={:.1} shopping_spend={:.1} transfers_paid={:.1} unemployment_paid={:.1} pensions_paid={:.1} child_support_paid={:.1} utility_cost={:.1} stock_use_cost={:.1} utility_stock_cost={:.1}",
            day_index,
            households_at_budget_floor,
            households_below_1d_stock,
            households_below_2d_stock,
            households_below_3d_stock,
            total_wages_paid,
            total_household_shopping_spend,
            total_benefits_paid,
            total_unemployment_benefits_paid,
            total_pensions_paid,
            total_child_support_paid,
            total_household_utility_cost,
            total_household_supply_use_cost,
            total_utility_stock_cost,
        );
        println!(
            "[ECON] Day {:>4} fiscal summary: income_tax={:.1} household_vat={:.1} business_purchase_tax={:.1} business_profit_tax={:.1} property_tax={:.1} residential_property_tax={:.1} commercial_property_tax={:.1} industrial_property_tax={:.1} tax_total={:.1} lifetime_tax={:.1} road_upkeep={:.1} treasury={:.1}",
            day_index,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.last_daily_residential_property_tax,
            self.treasury.last_daily_commercial_property_tax,
            self.treasury.last_daily_industrial_property_tax,
            self.treasury.last_daily_income_tax
                + self.treasury.last_daily_household_vat
                + self.treasury.last_daily_business_purchase_tax
                + self.treasury.last_daily_business_profit_tax
                + self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
            self.treasury.last_daily_upkeep,
            self.treasury.balance,
        );
        self.households.reset_daily_ledgers();
    }

    pub(super) fn collect_fiscal_revenue(&mut self, revenue: FiscalRevenue) {
        self.treasury.collect_income_tax(revenue.income_tax as f64);
        self.treasury
            .collect_household_vat(revenue.household_vat as f64);
        self.treasury
            .collect_business_purchase_tax(revenue.business_purchase_tax as f64);
        self.treasury
            .collect_business_profit_tax(revenue.business_profit_tax as f64);
        self.treasury
            .collect_property_tax(revenue.property_tax as f64);
    }

    /// Called once per in-game day by the tick loop to emit per-building economy lines.
    pub fn print_daily_building_economy_for_day(&mut self, day_index: u32) {
        self.print_daily_building_economy(day_index);
    }
}
