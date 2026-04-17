//! Household runtime state and the first-pass building-centric economy loop.
//!
//! The v0.1 foundation keeps households explicit, lightweight, and tied to
//! residential buildings without reviving per-agent grocery trips. This module
//! owns household stock/budget state, sub-daily building-side economy updates,
//! replenishment requests, and decision-utility-driven job assignment.

use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, baseline_private_zone_slot,
};
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

/// Household stock is healthy and no replenishment is pending.
pub const REPLENISHMENT_STABLE: u8 = 0;
/// Household stock fell below the trigger and needs a restock attempt.
pub const REPLENISHMENT_NEEDS: u8 = 1;
/// Household stock was replenished on this economy pass.
pub const REPLENISHMENT_RESERVED: u8 = 2;
/// Household has a reserved supply source and is waiting for pickup-side fulfillment.
pub const REPLENISHMENT_PICKUP_PENDING: u8 = 3;
/// Household stock was replenished on this economy pass.
pub const REPLENISHMENT_FULFILLED: u8 = 4;
/// Household is waiting before retrying another replenishment attempt.
pub const REPLENISHMENT_COOLDOWN: u8 = 5;

const HOUSEHOLD_CONSUMPTION_RATE: f32 = 1.0;
const HOUSEHOLD_TARGET_STOCK_DAYS: f32 = 3.0;
const HOUSEHOLD_TRIGGER_STOCK_DAYS: f32 = 1.5;
const IMMIGRANT_STARTING_STOCK_DAYS: f32 = 3.0;
// $15/member gives a 2-member household $30 starting — roughly 5 days of utility runway
// ($6/day) before wages arrive, while keeping income_pressure ≈ 0.44 at spawn so agents
// remain motivated to seek work.
const IMMIGRANT_STARTING_BUDGET_PER_MEMBER: f32 = 15.0;
// $3/member/day ≈ 6% of a single $100 wage, consistent with the business OWA utility bands
// ($8–12/day). Previously $2/member felt too low relative to wages.
const HOUSEHOLD_UTILITY_COST_PER_MEMBER: f32 = 3.0;
const HOUSEHOLD_STARTING_BUDGET: f32 = 10.0;

const UTILITY_COST_COMMERCIAL: f32 = 8.0;
const UTILITY_COST_INDUSTRIAL: f32 = 12.0;

// Local rates charged to consumers when all three utility buildings are present.
// Combined total (6.5/day) is lower than either OWA rate, making local utilities cheaper.
const UTILITY_LOCAL_POWER: f32 = 3.0;
const UTILITY_LOCAL_WATER: f32 = 2.0;
const UTILITY_LOCAL_SEWAGE: f32 = 1.5;
const UTILITY_LOCAL_TOTAL: f32 = UTILITY_LOCAL_POWER + UTILITY_LOCAL_WATER + UTILITY_LOCAL_SEWAGE;

const W_INCOME: f32 = 0.35;
const W_STOCK: f32 = 0.35;
const W_JOB: f32 = 0.20;
const W_COMMUTE: f32 = 0.10;
const GO_TO_WORK_THRESHOLD: f32 = 0.10;
// Days an agent is locked to a voluntarily-chosen job before they may switch.
const JOB_LOCK_DAYS: u8 = 7;
// Consecutive unpaid days that override the lock — lets agents escape a failing employer
// before the full lock expires.
const JOB_UNPAID_ABANDON_DAYS: u8 = 2;
const JOB_SEARCH_MAX_RING: i32 = 8;
const JOB_SEARCH_CANDIDATES: usize = 24;
const GROCERY_SEARCH_MAX_RING: i32 = 6;
const GROCERY_SEARCH_CANDIDATES: usize = 24;
const OPERATIONAL_HOURS_PER_DAY: f32 = 24.0;

/// Explicit household runtime record anchored to a residential building.
#[derive(Clone, Debug)]
pub struct Household {
    /// Residential building currently anchoring the household.
    pub home_building_id: usize,
    /// Shared household budget used for essentials in the first-pass loop.
    pub budget: f32,
    /// Current household stock buffer in `household_supplies`.
    pub stock: f32,
    /// Cached linked population count. Rebuilt from resident agents every economy pass.
    pub member_count: u16,
    /// Baseline daily consumption in `household_supplies / day / resident`.
    pub consumption_rate: f32,
    /// Cached derived stock horizon in days at the current consumption rate.
    pub stock_days: f32,
    /// Current replenishment state for diagnostics and cooldown handling.
    pub replenishment_state: u8,
    /// Remaining operational-hour cooldown steps before another replenishment retry.
    pub cooldown_hours: u16,
    /// Reserved source building for the current replenishment request, if any.
    pub reserved_store_building_id: usize,
    /// Reserved amount waiting for household pickup-side fulfillment.
    pub reserved_amount: f32,
    /// Reserved budget waiting to be transferred to the supplying store.
    pub reserved_total_cost: f32,
    /// Remaining operational-hour steps before the reserved pickup completes.
    pub pickup_eta_hours: u16,
    /// Consecutive daily stay-rule failures for the current home.
    pub stay_failure_days: u32,
    /// Stable authored cadence offset used for periodic replenishment checks.
    pub replenishment_offset_hours: u16,
    /// Days elapsed with at least one unemployed member. Resets to 0 when all members are
    /// employed. Incremented each daily tick while the household is benefit-eligible. Once
    /// this reaches `unemployment_max_days`, the household becomes emigration-eligible and
    /// benefit payments stop.
    pub unemployment_days_elapsed: u32,
}

/// Collection of explicit household records for the live simulation.
#[derive(Clone, Debug, Default)]
pub struct HouseholdSystem {
    /// All known households. Agents reference these by index.
    pub households: Vec<Household>,
}

impl HouseholdSystem {
    /// Creates an empty household system.
    pub fn new() -> Self {
        Self {
            households: Vec::new(),
        }
    }

    /// Clears all households.
    pub fn clear(&mut self) {
        self.households.clear();
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
        for household in &mut self.households {
            if let Some(&new_id) = mapping.get(&household.home_building_id) {
                household.home_building_id = new_id;
            }
            if let Some(&new_id) = mapping.get(&household.reserved_store_building_id) {
                household.reserved_store_building_id = new_id;
            }
        }
    }

    /// Invalidates references to a building that is being removed.
    pub fn invalidate_building(&mut self, removed_building: usize) {
        for household in &mut self.households {
            if household.home_building_id == removed_building {
                household.home_building_id = usize::MAX;
            }
            if household.reserved_store_building_id == removed_building {
                // Return reserved budget to the household.
                household.budget += household.reserved_total_cost;
                household.reserved_store_building_id = usize::MAX;
                household.reserved_amount = 0.0;
                household.reserved_total_cost = 0.0;
                household.replenishment_state = 0; // REPLENISHMENT_STABLE
            }
        }
    }

    /// Creates one immigrant household with shared starter savings and stock.
    pub(crate) fn admit_immigrant_household(
        &mut self,
        catalog: &RuntimeEconomyCatalog,
        home_building_id: usize,
        member_count: u16,
    ) -> usize {
        let profile = get_household_demand_profile(catalog);
        let consumption_rate = profile
            .map(|p| p.consumption_rate_per_resident)
            .unwrap_or(HOUSEHOLD_CONSUMPTION_RATE);
        let target_days = profile
            .map(|p| p.stock_target_days)
            .unwrap_or(HOUSEHOLD_TARGET_STOCK_DAYS);

        let member_count = member_count.max(1);
        self.households.push(Household {
            home_building_id,
            // Founding households arrive with modest savings so the first town
            // has a real incentive to take available jobs instead of idling on
            // a large abstract cash cushion.
            budget: IMMIGRANT_STARTING_BUDGET_PER_MEMBER * member_count as f32,
            stock: member_count as f32
                * consumption_rate
                * target_days.min(IMMIGRANT_STARTING_STOCK_DAYS),
            member_count,
            consumption_rate,
            stock_days: target_days.min(IMMIGRANT_STARTING_STOCK_DAYS),
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_hours: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_hours: 0,
            stay_failure_days: 0,
            unemployment_days_elapsed: 0,
            replenishment_offset_hours: stable_replenishment_offset_hours(
                home_building_id,
                self.households.len() as u32,
            ),
        });
        self.households.len() - 1
    }

    /// Runs one coarse operational-hour economy step.
    pub fn operational_hour_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        minute_of_day: u16,
    ) {
        self.ensure_agent_households(agents);
        self.rebuild_household_membership(agents);
        self.recount_worker_assignments(agents, allocator);
        self.run_building_economy(allocator);
        logistics.hourly_tick(allocator, transit_network, graph, minute_of_day);
        self.consume_household_stock(agents);
        self.run_household_replenishment(allocator, absolute_hour);
        self.assign_agent_workplaces(agents, allocator);
        self.sync_agent_money_from_households(agents);
    }

    /// Runs one daily settlement pass after the final operational-hour step of the day.
    ///
    /// Implements the four-step bankruptcy spec from `economy.md § Building Bankruptcy`:
    /// Step 1 — bankruptcy check, Step 2 — wages, Step 3 — utility cost, Step 4 — distress.
    pub fn daily_settlement_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        treasury_balance: &mut f64,
    ) {
        self.ensure_agent_households(agents);
        self.rebuild_household_membership(agents);
        self.recount_worker_assignments(agents, allocator);
        // Advance per-agent job-lock countdown once per day.
        for i in 0..agents.len() {
            if agents.job_lock_days[i] > 0 {
                agents.job_lock_days[i] -= 1;
            }
        }
        // Step 1: bankruptcy check — mark buildings that were in distress yesterday and are
        // still negative. Must run before wages so workers are ejected on the same day.
        self.run_bankruptcy_check(allocator);
        // Step 2: pay wages (budget does not go negative from this step).
        self.pay_daily_wages(agents, allocator);
        // Step 3: pay unemployment benefit to eligible households from the city treasury.
        self.pay_unemployment_benefits(agents, treasury_balance);
        // Steps 4 + 5: charge utility, then liquidate if still negative.
        self.settle_daily_utilities(allocator);
        self.resolve_household_housing(agents, allocator);
        self.assign_agent_workplaces(agents, allocator);
        self.sync_agent_money_from_households(agents);
    }

    /// Pays daily unemployment benefits to eligible households, drawing from the city treasury.
    ///
    /// Iterates all households with at least one unemployed member. Each eligible household
    /// receives `unemployed_members × unemployment_daily_benefit_per_member`. Disbursement
    /// stops once the treasury balance reaches zero. Households that have been unemployed for
    /// `unemployment_max_days` stop receiving benefit and become emigration-eligible via normal
    /// removal pressure.
    pub(crate) fn pay_unemployment_benefits(
        &mut self,
        agents: &AgentSystem,
        treasury_balance: &mut f64,
    ) {
        if *treasury_balance <= 0.0 {
            debug_log!(
                "economy",
                "unemployment_benefits: treasury_empty — disbursement skipped"
            );
            return;
        }
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let benefit_per_member = tuning.unemployment_daily_benefit_per_member;
        let max_days = tuning.unemployment_max_days;

        // Count unemployed members per household in one agent pass.
        let mut unemployed_per_household = vec![0u16; self.households.len()];
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }
            if agents.work_building[i] == usize::MAX {
                unemployed_per_household[hid] = unemployed_per_household[hid].saturating_add(1);
            }
        }

        let mut total_disbursed = 0.0f32;
        let mut households_paid = 0u32;
        let mut households_exhausted = 0u32;

        for (hid, household) in self.households.iter_mut().enumerate() {
            if household.member_count == 0 || household.home_building_id == usize::MAX {
                continue;
            }
            let unemployed = unemployed_per_household[hid];
            if unemployed == 0 {
                household.unemployment_days_elapsed = 0;
                continue;
            }
            if household.unemployment_days_elapsed >= max_days {
                // Benefit exhausted; household is emigration-eligible via removal pressure.
                households_exhausted += 1;
                continue;
            }
            household.unemployment_days_elapsed =
                household.unemployment_days_elapsed.saturating_add(1);
            if *treasury_balance <= 0.0 {
                continue;
            }
            let benefit = unemployed as f32 * benefit_per_member;
            let paid = if *treasury_balance >= benefit as f64 {
                household.budget += benefit;
                *treasury_balance -= benefit as f64;
                benefit
            } else {
                let remainder = *treasury_balance as f32;
                household.budget += remainder;
                *treasury_balance = 0.0;
                remainder
            };
            total_disbursed += paid;
            households_paid += 1;
        }

        debug_log!(
            "economy",
            "unemployment_benefits: paid={:.1} households={} exhausted={} treasury={:.0}",
            total_disbursed,
            households_paid,
            households_exhausted,
            *treasury_balance,
        );
    }

    /// Removes the already-decided demand-owned household outflow count from the settled
    /// household snapshot using the deterministic order defined in `economy.md`.
    pub(crate) fn execute_demand_household_removal(
        &mut self,
        households_to_remove_today: u32,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if households_to_remove_today == 0 || self.households.is_empty() {
            return;
        }

        let mut unhoused_candidates = Vec::new();
        let mut housed_candidates = Vec::new();
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        for (household_id, household) in self.households.iter().enumerate() {
            if household.member_count == 0 {
                continue;
            }
            let reserve_days = household_reserve_days(&catalog, household);
            let candidate = (household_id, reserve_days, household.stock_days);
            if household_is_housed(household, allocator) {
                housed_candidates.push(candidate);
            } else {
                unhoused_candidates.push(candidate);
            }
        }

        let candidate_order = |a: &(usize, f32, f32), b: &(usize, f32, f32)| {
            a.1.total_cmp(&b.1)
                .then_with(|| a.2.total_cmp(&b.2))
                .then_with(|| a.0.cmp(&b.0))
        };
        unhoused_candidates.sort_by(candidate_order);
        housed_candidates.sort_by(candidate_order);

        let mut ordered_households = Vec::with_capacity(
            unhoused_candidates
                .len()
                .saturating_add(housed_candidates.len()),
        );
        ordered_households.extend(unhoused_candidates.into_iter().map(|candidate| candidate.0));
        ordered_households.extend(housed_candidates.into_iter().map(|candidate| candidate.0));

        let removal_count = ordered_households
            .len()
            .min(households_to_remove_today as usize);
        if removal_count == 0 {
            return;
        }

        let mut selected_households: Vec<_> =
            ordered_households.into_iter().take(removal_count).collect();
        selected_households.sort_unstable_by(|a, b| b.cmp(a));

        debug_log!(
            "economy",
            "demand-owned household removal executing: households_to_remove_today={} selected={}",
            households_to_remove_today,
            selected_households.len()
        );

        for household_id in selected_households {
            self.remove_household_at_index(household_id, agents, allocator);
        }
    }

    fn ensure_agent_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            let home = agents.home_building[i];
            if home == usize::MAX {
                agents.household_id[i] = usize::MAX;
                continue;
            }
            let hid = agents.household_id[i];
            let needs_new = hid == usize::MAX
                || hid >= self.households.len()
                || self.households[hid].home_building_id != home;
            if needs_new {
                let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                    panic!("could not load built-in economy catalog during re-housing: {err}")
                });
                let profile = get_household_demand_profile(&catalog);
                let consumption_rate = profile
                    .map(|p| p.consumption_rate_per_resident)
                    .unwrap_or(HOUSEHOLD_CONSUMPTION_RATE);
                let target_days = profile
                    .map(|p| p.stock_target_days)
                    .unwrap_or(HOUSEHOLD_TARGET_STOCK_DAYS);

                let budget = agents.money[i].max(HOUSEHOLD_STARTING_BUDGET);
                self.households.push(Household {
                    home_building_id: home,
                    budget,
                    stock: target_days * consumption_rate,
                    member_count: 0,
                    consumption_rate,
                    stock_days: target_days,
                    replenishment_state: REPLENISHMENT_STABLE,
                    cooldown_hours: 0,
                    reserved_store_building_id: usize::MAX,
                    reserved_amount: 0.0,
                    reserved_total_cost: 0.0,
                    pickup_eta_hours: 0,
                    stay_failure_days: 0,
                    replenishment_offset_hours: stable_replenishment_offset_hours(
                        home,
                        self.households.len() as u32,
                    ),
                    unemployment_days_elapsed: 0,
                });
                agents.household_id[i] = self.households.len() - 1;
            }
        }
    }

    fn rebuild_household_membership(&mut self, agents: &AgentSystem) {
        for household in &mut self.households {
            household.member_count = 0;
        }
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid != usize::MAX && hid < self.households.len() {
                let household = &mut self.households[hid];
                household.member_count = household.member_count.saturating_add(1);
                household.home_building_id = agents.home_building[i];
            }
        }
        for household in &mut self.households {
            household.stock_days = stock_days(
                household.stock,
                household.member_count,
                household.consumption_rate,
            );
            if household.member_count == 0 {
                clear_replenishment_request(household);
            }
        }
    }

    fn recount_worker_assignments(
        &mut self,
        agents: &AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        for building in &mut allocator.buildings {
            building.worker_count = 0;
        }
        for i in 0..agents.len() {
            let work = agents.work_building[i];
            if work != usize::MAX && work < allocator.buildings.len() {
                allocator.buildings[work].worker_count =
                    allocator.buildings[work].worker_count.saturating_add(1);
            }
        }
    }

    /// Step 1 of the daily settlement sequence: mark bankrupt any building that ended yesterday
    /// in distress (budget negative after forced liquidation) and is still negative today.
    fn run_bankruptcy_check(&mut self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            if building.broken || building.economy_broken || building.is_deserted {
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
    fn settle_daily_utilities(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        // Phase 1: find operational utility provider buildings.
        let mut utility_provider_indices: Vec<usize> = Vec::new();
        let mut power_available = false;
        let mut water_available = false;
        let mut sewage_available = false;

        for (idx, building) in allocator.buildings.iter().enumerate() {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.edge_idx == usize::MAX
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if !matches!(
                profile.kind,
                EconomyProfileRuntimeKind::UtilityProducer
                    | EconomyProfileRuntimeKind::UtilityProcessor
            ) {
                continue;
            }
            utility_provider_indices.push(idx);
            match profile.utility_service.as_deref() {
                Some("power") => power_available = true,
                Some("water") => water_available = true,
                Some("sewage") => sewage_available = true,
                _ => {}
            }
        }

        let all_local = power_available && water_available && sewage_available;

        // Phase 2 (daily): charge each commercial/industrial building the full daily utility
        // cost unconditionally. Budget may go negative.
        let mut local_utility_revenue = 0.0f32;

        for building in &mut allocator.buildings {
            if building.is_deserted
                || building.broken
                || building.economy_broken
                || building.edge_idx == usize::MAX
            {
                continue;
            }
            let (daily_cost, is_local) = match building.zone_type {
                ZoneType::Commercial => {
                    if all_local {
                        (UTILITY_LOCAL_TOTAL, true)
                    } else {
                        (UTILITY_COST_COMMERCIAL, false)
                    }
                }
                ZoneType::Industrial => {
                    if all_local {
                        (UTILITY_LOCAL_TOTAL, true)
                    } else {
                        (UTILITY_COST_INDUSTRIAL, false)
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

        // Step 4: distress resolution — forced OWA liquidation for buildings that went negative.
        for building in &mut allocator.buildings {
            if building.is_deserted || building.broken || building.economy_broken {
                continue;
            }
            if !matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial
            ) {
                continue;
            }
            if building.operating_budget < 0.0 {
                forced_owa_liquidation(building, &catalog);
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

    fn run_building_economy(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        for idx in 0..allocator.buildings.len() {
            let zone = allocator.buildings[idx].zone_type;
            if allocator.buildings[idx].broken
                || allocator.buildings[idx].economy_broken
                || allocator.buildings[idx].is_deserted
            {
                continue;
            }
            let Some(profile) = economy_profile_for_building(&catalog, &allocator.buildings[idx])
            else {
                continue;
            };
            let worker_capacity = allocator.worker_capacity(idx).max(1);
            let staffing_factor = (allocator.buildings[idx].worker_count as f32
                / worker_capacity as f32)
                .clamp(0.0, 1.0);
            let input_factor =
                industrial_input_coverage_factor(&catalog, &allocator.buildings[idx]);
            let output_headroom_factor =
                industrial_output_headroom_factor(&catalog, &allocator.buildings[idx]);
            let throughput_factor = staffing_factor * input_factor * output_headroom_factor;

            let building = &mut allocator.buildings[idx];
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
        }
    }

    fn consume_household_stock(&mut self, agents: &mut AgentSystem) {
        for hid in 0..self.households.len() {
            let household = &mut self.households[hid];
            if household.member_count == 0 {
                continue;
            }
            let hourly_consumption = household.member_count as f32 * household.consumption_rate
                / OPERATIONAL_HOURS_PER_DAY;
            household.stock = (household.stock - hourly_consumption).max(0.0);
            let hourly_utility_cost = household.member_count as f32
                * HOUSEHOLD_UTILITY_COST_PER_MEMBER
                / OPERATIONAL_HOURS_PER_DAY;
            household.budget = (household.budget - hourly_utility_cost).max(0.0);
            household.stock_days = stock_days(
                household.stock,
                household.member_count,
                household.consumption_rate,
            );
            if matches!(
                household.replenishment_state,
                REPLENISHMENT_RESERVED | REPLENISHMENT_PICKUP_PENDING
            ) {
                continue;
            } else if household.replenishment_state == REPLENISHMENT_FULFILLED {
                if household.cooldown_hours > 0 {
                    household.cooldown_hours -= 1;
                }
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else if household.cooldown_hours > 0 {
                household.cooldown_hours -= 1;
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else {
                household.replenishment_state = REPLENISHMENT_STABLE;
            }

            if household.stock_days == 0.0 {
                for i in 0..agents.len() {
                    if agents.household_id[i] == hid {
                        agents.happiness[i] = (agents.happiness[i] - 4.0).clamp(0.0, 100.0);
                    }
                }
            }
        }
    }

    fn resolve_household_housing(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let config = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));

        for household_id in 0..self.households.len() {
            let household = &self.households[household_id];
            if household.member_count == 0 {
                continue;
            }

            let reserve_days = household_reserve_days(&catalog, household);
            let current_home = household.home_building_id;
            let is_housed = household_is_housed(household, allocator);

            if !is_housed {
                self.households[household_id].stay_failure_days = 0;
                if let Some(target_home) = self.find_affordable_home_for_household(
                    household_id,
                    reserve_days,
                    allocator,
                    None,
                    &config.households,
                ) {
                    self.relocate_household(
                        household_id,
                        usize::MAX,
                        target_home,
                        agents,
                        allocator,
                    );
                }
                continue;
            }

            let current_level = allocator.buildings[current_home].level;
            let stay_threshold = level_tuning_value(
                &config.households.residential_stay_min_reserve_days_by_level,
                current_level,
            );

            if reserve_days >= stay_threshold {
                self.households[household_id].stay_failure_days = 0;
                if let Some(target_home) = self.find_affordable_home_for_household(
                    household_id,
                    reserve_days,
                    allocator,
                    Some(current_home),
                    &config.households,
                ) && allocator.buildings[target_home].level > current_level
                {
                    self.relocate_household(
                        household_id,
                        current_home,
                        target_home,
                        agents,
                        allocator,
                    );
                }
                continue;
            }

            self.households[household_id].stay_failure_days = self.households[household_id]
                .stay_failure_days
                .saturating_add(1);
            if self.households[household_id].stay_failure_days
                < config.households.stay_failure_days_before_eviction
            {
                continue;
            }

            if let Some(target_home) = self.find_affordable_home_for_household(
                household_id,
                reserve_days,
                allocator,
                Some(current_home),
                &config.households,
            ) {
                self.relocate_household(household_id, current_home, target_home, agents, allocator);
            } else {
                self.evict_household(household_id, current_home, agents, allocator);
            }
        }
    }

    fn find_affordable_home_for_household(
        &self,
        household_id: usize,
        reserve_days: f32,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
        config: &crate::simulation::economy::definitions::HouseholdRuntimeTuning,
    ) -> Option<usize> {
        let _household = &self.households[household_id];
        let current_center = current_home.and_then(|building_idx| {
            allocator
                .buildings
                .get(building_idx)
                .map(|building| (building.center_x, building.center_y))
        });

        let mut candidates = Vec::new();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };
        for &building_idx in &allocator.vacancy_index[residential_slot] {
            if Some(building_idx) == current_home || building_idx >= allocator.buildings.len() {
                continue;
            }
            let building = &allocator.buildings[building_idx];
            if building.broken || building.economy_broken || building.pending_redevelopment {
                continue;
            }

            let free_slots = allocator
                .household_capacity(building_idx)
                .saturating_sub(building.occupancy);
            if free_slots == 0 {
                continue;
            }

            let move_in_threshold = level_tuning_value(
                &config.residential_move_in_min_reserve_days_by_level,
                building.level,
            );
            if reserve_days + f32::EPSILON < move_in_threshold {
                continue;
            }

            let distance = current_center.map_or(0.0, |(origin_x, origin_y)| {
                let dx = building.center_x - origin_x;
                let dy = building.center_y - origin_y;
                dx * dx + dy * dy
            });
            candidates.push((building_idx, building.level, distance));
        }

        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| {
                    left.2
                        .partial_cmp(&right.2)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.0.cmp(&right.0))
        });
        candidates.first().map(|candidate| candidate.0)
    }

    fn relocate_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        new_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if household_id >= self.households.len() || new_home >= allocator.buildings.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
        }
        allocator.claim_vacancy(new_home);

        self.households[household_id].home_building_id = new_home;
        self.households[household_id].stay_failure_days = 0;

        for agent_idx in 0..agents.len() {
            if agents.household_id[agent_idx] != household_id {
                continue;
            }
            agents.home_building[agent_idx] = new_home;
            if old_home < allocator.buildings.len() {
                if agents.current_building[agent_idx] == old_home {
                    agents.current_building[agent_idx] = new_home;
                    agents.target_building[agent_idx] = usize::MAX;
                    agents.planned_target_building[agent_idx] = usize::MAX;
                    agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
                }
                if agents.target_building[agent_idx] == old_home {
                    agents.target_building[agent_idx] = new_home;
                }
                if agents.planned_target_building[agent_idx] == old_home {
                    agents.planned_target_building[agent_idx] = new_home;
                }
            } else if agents.current_building[agent_idx] == usize::MAX
                && agents.target_building[agent_idx] == usize::MAX
            {
                agents.target_building[agent_idx] = new_home;
                agents.planned_target_building[agent_idx] = new_home;
                agents.activity[agent_idx] = 0;
            }
        }
    }

    fn evict_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if household_id >= self.households.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
        }

        let household = &mut self.households[household_id];
        household.home_building_id = usize::MAX;
        household.stay_failure_days = 0;
        clear_replenishment_request(household);

        for agent_idx in 0..agents.len() {
            if agents.household_id[agent_idx] != household_id {
                continue;
            }
            agents.home_building[agent_idx] = usize::MAX;
            if agents.current_building[agent_idx] == old_home {
                agents.current_building[agent_idx] = usize::MAX;
                agents.target_building[agent_idx] = usize::MAX;
                agents.planned_target_building[agent_idx] = usize::MAX;
                agents.transit[agent_idx] = TRANSIT_ACCESS_INGRESS;
            } else {
                if agents.target_building[agent_idx] == old_home {
                    agents.target_building[agent_idx] = usize::MAX;
                }
                if agents.planned_target_building[agent_idx] == old_home {
                    agents.planned_target_building[agent_idx] = usize::MAX;
                }
            }
        }
    }

    fn run_household_replenishment(
        &mut self,
        allocator: &mut BuildingAllocator,
        absolute_hour: u32,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let check_interval = u32::from(
            tuning
                .operational_clock
                .household_replenishment_check_interval_hours,
        );

        for hid in 0..self.households.len() {
            self.progress_household_replenishment(hid, allocator);
        }

        let profile = get_household_demand_profile(&catalog);
        let target_days = profile
            .map(|p| p.stock_target_days)
            .unwrap_or(HOUSEHOLD_TARGET_STOCK_DAYS);
        let trigger_days = profile
            .map(|p| p.reorder_threshold_days)
            .unwrap_or(HOUSEHOLD_TRIGGER_STOCK_DAYS);

        for hid in 0..self.households.len() {
            let household = &self.households[hid];
            if household.member_count == 0
                || household.home_building_id == usize::MAX
                || household.home_building_id >= allocator.buildings.len()
                || household.replenishment_state == REPLENISHMENT_RESERVED
                || household.replenishment_state == REPLENISHMENT_PICKUP_PENDING
                || household.cooldown_hours > 0
                || household.stock_days >= trigger_days
                || absolute_hour % check_interval
                    != u32::from(household.replenishment_offset_hours % check_interval as u16)
            {
                continue;
            }

            let Some(household_supply_resource) = household_supply_resource_runtime_id(&catalog)
            else {
                continue;
            };
            let home = &allocator.buildings[household.home_building_id];
            let candidates = allocator.find_nearby_buildings_by_zones(
                home.center_x,
                home.center_y,
                &[ZoneType::Commercial],
                GROCERY_SEARCH_MAX_RING,
                GROCERY_SEARCH_CANDIDATES,
            );

            let daily_consumption = household.member_count as f32 * household.consumption_rate;
            let target_stock = target_days * daily_consumption;
            let mut desired_amount = (target_stock - household.stock).max(0.0);
            let mut found_sale = None;

            for candidate in candidates {
                let store = &allocator.buildings[candidate];
                // A store can sell from existing inventory even when utility
                // service is temporarily unavailable — only broken or
                // economy_broken stores are excluded.
                if store.inventory_units(household_supply_resource) <= 0.0
                    || store.broken
                    || store.economy_broken
                {
                    continue;
                }
                let Some(store_profile) = economy_profile_for_building(&catalog, store) else {
                    continue;
                };
                if store_profile
                    .output_port(household_supply_resource)
                    .is_none()
                {
                    continue;
                }
                let available_stock = store.inventory_units(household_supply_resource);
                let amount = desired_amount.min(available_stock);
                let total_cost = amount * store_profile.unit_price_currency;
                if amount > 0.0 && household.budget >= total_cost {
                    found_sale = Some((candidate, amount, total_cost));
                    break;
                }
                desired_amount = desired_amount.min(available_stock);
            }

            let household = &mut self.households[hid];
            if let Some((store_idx, amount, total_cost)) = found_sale {
                let store = &mut allocator.buildings[store_idx];
                store.remove_inventory_units(household_supply_resource, amount);
                household.budget -= total_cost;
                household.reserved_store_building_id = store_idx;
                household.reserved_amount = amount;
                household.reserved_total_cost = total_cost;
                household.pickup_eta_hours = tuning.operational_clock.household_pickup_eta_hours;
                household.replenishment_state = REPLENISHMENT_RESERVED;
            } else {
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
                household.cooldown_hours = tuning
                    .operational_clock
                    .household_replenishment_retry_cooldown_hours;
            }
        }
    }

    fn progress_household_replenishment(&mut self, hid: usize, allocator: &mut BuildingAllocator) {
        let Some(household) = self.households.get_mut(hid) else {
            return;
        };
        match household.replenishment_state {
            REPLENISHMENT_RESERVED => {
                if household.pickup_eta_hours > 0 {
                    household.pickup_eta_hours -= 1;
                }
                household.replenishment_state = REPLENISHMENT_PICKUP_PENDING;
            }
            REPLENISHMENT_PICKUP_PENDING => {
                let store_idx = household.reserved_store_building_id;
                if store_idx == usize::MAX || store_idx >= allocator.buildings.len() {
                    let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
                        panic!("could not load built-in economy runtime tuning: {err}")
                    });
                    household.budget += household.reserved_total_cost;
                    clear_replenishment_request(household);
                    household.replenishment_state = REPLENISHMENT_COOLDOWN;
                    household.cooldown_hours = tuning
                        .operational_clock
                        .household_replenishment_retry_cooldown_hours;
                    return;
                }

                let store = &mut allocator.buildings[store_idx];
                store.revenue += household.reserved_total_cost;
                store.operating_budget += household.reserved_total_cost;
                household.stock += household.reserved_amount;
                household.stock_days = stock_days(
                    household.stock,
                    household.member_count,
                    household.consumption_rate,
                );
                household.replenishment_state = REPLENISHMENT_FULFILLED;
                household.cooldown_hours = 1;
                household.reserved_store_building_id = usize::MAX;
                household.reserved_amount = 0.0;
                household.reserved_total_cost = 0.0;
                household.pickup_eta_hours = 0;
            }
            _ => {}
        }
    }

    fn assign_agent_workplaces(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let profile = get_household_demand_profile(&catalog);
        let target_days = profile
            .map(|p| p.stock_target_days)
            .unwrap_or(HOUSEHOLD_TARGET_STOCK_DAYS);

        let mut reserved_workers: Vec<u32> =
            allocator.buildings.iter().map(|b| b.worker_count).collect();

        // Ejection pre-pass: immediately detach workers from newly-bankrupt buildings so they
        // enter the open job market this same day rather than waiting for unpaid-wage accumulation.
        for i in 0..agents.len() {
            let work = agents.work_building[i];
            if work == usize::MAX || work >= allocator.buildings.len() {
                continue;
            }
            if allocator.buildings[work].is_deserted {
                if work < reserved_workers.len() {
                    reserved_workers[work] = reserved_workers[work].saturating_sub(1);
                }
                agents.work_building[i] = usize::MAX;
                agents.job_lock_days[i] = 0;
                agents.consecutive_unpaid_days[i] = 0;
            }
        }

        for i in 0..agents.len() {
            if agents.transit[i] != TRANSIT_IN_BUILDING {
                continue;
            }

            let home_idx = agents.home_building[i];
            if home_idx == usize::MAX || home_idx >= allocator.buildings.len() {
                continue;
            }

            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }

            let household = &self.households[hid];
            let home = &allocator.buildings[home_idx];
            let mut candidates = allocator.find_nearby_buildings_by_zones(
                home.center_x,
                home.center_y,
                &[ZoneType::Industrial, ZoneType::Commercial],
                JOB_SEARCH_MAX_RING,
                JOB_SEARCH_CANDIDATES,
            );
            if agents.work_building[i] != usize::MAX
                && !candidates.contains(&agents.work_building[i])
            {
                candidates.push(agents.work_building[i]);
            }

            let income_pressure = household_income_pressure(&catalog, household);
            let stock_pressure = (1.0
                - (household.stock_days / target_days.max(0.1)).clamp(0.0, 1.0))
            .clamp(0.0, 1.0);

            let mut best_job = usize::MAX;
            let mut best_score = 0.0;
            for candidate in candidates {
                if candidate >= allocator.buildings.len() {
                    continue;
                }
                let building = &allocator.buildings[candidate];
                if building.is_deserted || building.broken || building.economy_broken {
                    continue;
                }
                if !matches!(
                    building.zone_type,
                    ZoneType::Industrial | ZoneType::Commercial
                ) {
                    continue;
                }

                // Budget-based hiring constraint: Only allow hiring if the building can afford
                // to pay at least the current staff plus this potential new worker for one day.
                let average_daily_wage = catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .map(|p| p.average_daily_wage())
                    .unwrap_or(0.0);

                let worker_capacity = allocator.worker_capacity(candidate);

                // Effective capacity is the floor of what the building can afford to pay right now,
                // clamped by its physical worker limits.
                let budget_capacity = if average_daily_wage > 0.1 {
                    (building.operating_budget / average_daily_wage).floor() as u32
                } else {
                    worker_capacity
                };
                let effective_capacity = worker_capacity.min(budget_capacity);

                if effective_capacity == 0 && agents.work_building[i] != candidate {
                    continue;
                }

                let already_assigned = agents.work_building[i] == candidate;
                let reserved = reserved_workers[candidate];
                let open_slots = if already_assigned {
                    // If already working here, we don't need a "new" budget slot,
                    // but we still respect the physical capacity.
                    worker_capacity.saturating_sub(reserved.saturating_sub(1))
                } else {
                    effective_capacity.saturating_sub(reserved)
                };
                if open_slots == 0 {
                    continue;
                }

                let commute_penalty = normalized_commute_penalty(home, building);
                let score = W_INCOME * income_pressure + W_STOCK * stock_pressure + W_JOB * 1.0
                    - W_COMMUTE * commute_penalty;
                if score > best_score {
                    best_score = score;
                    best_job = candidate;
                }
            }

            if best_job != usize::MAX && best_score >= GO_TO_WORK_THRESHOLD {
                let old_job = agents.work_building[i];
                // Allow switching only if: no current job, lock expired, or employer
                // has not paid for JOB_UNPAID_ABANDON_DAYS consecutive days.
                let can_switch = old_job == usize::MAX
                    || agents.job_lock_days[i] == 0
                    || agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS;
                if old_job != best_job && can_switch {
                    if old_job != usize::MAX && old_job < reserved_workers.len() {
                        reserved_workers[old_job] = reserved_workers[old_job].saturating_sub(1);
                    }
                    reserved_workers[best_job] = reserved_workers[best_job].saturating_add(1);
                    agents.work_building[i] = best_job;
                    agents.job_lock_days[i] = JOB_LOCK_DAYS;
                    debug_log!(
                        "economy",
                        "agent_idx={} accepted job building={} zone={:?} score={:.2} income_pressure={:.2} stock_pressure={:.2}",
                        i,
                        best_job,
                        allocator.buildings[best_job].zone_type,
                        best_score,
                        income_pressure,
                        stock_pressure
                    );
                }
            }
        }

        // Commit reserved counts back to buildings so production and demand metrics are accurate.
        for (idx, count) in reserved_workers.into_iter().enumerate() {
            allocator.buildings[idx].worker_count = count;
        }
    }

    fn sync_agent_money_from_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }
            let household = &self.households[hid];
            let per_member = if household.member_count > 0 {
                household.budget / household.member_count as f32
            } else {
                0.0
            };
            agents.money[i] = per_member.max(0.0);
        }
    }

    /// Pays wages into each employed agent's household budget.
    pub fn pay_daily_wages(&mut self, agents: &mut AgentSystem, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        for i in 0..agents.len() {
            let work = agents.work_building[i];
            let hid = agents.household_id[i];
            if work == usize::MAX || hid == usize::MAX {
                continue;
            }
            if work >= allocator.buildings.len() || hid >= self.households.len() {
                continue;
            }
            let Some(profile) = economy_profile_for_building(&catalog, &allocator.buildings[work])
            else {
                continue;
            };
            let wage = profile.average_daily_wage();
            if wage <= 0.0 {
                continue;
            }
            if allocator.buildings[work].operating_budget >= wage {
                allocator.buildings[work].operating_budget -= wage;
                self.households[hid].budget += wage;
                agents.consecutive_unpaid_days[i] = 0;
            } else {
                agents.consecutive_unpaid_days[i] =
                    agents.consecutive_unpaid_days[i].saturating_add(1);

                if agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS {
                    // Fire self from work.
                    agents.work_building[i] = usize::MAX;
                    agents.consecutive_unpaid_days[i] = 0;
                    debug_log!(
                        "economy",
                        "agent_idx={} fired self from insolvent building={} due to consecutive unpaid days",
                        i,
                        work
                    );
                }
            }
        }
        self.sync_agent_money_from_households(agents);
    }

    fn remove_household_at_index(
        &mut self,
        household_id: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if household_id >= self.households.len() {
            return;
        }

        let household = &self.households[household_id];
        if matches!(
            household.replenishment_state,
            REPLENISHMENT_RESERVED | REPLENISHMENT_PICKUP_PENDING
        ) {
            let store_idx = household.reserved_store_building_id;
            if store_idx < allocator.buildings.len() {
                if let Ok(catalog) = load_runtime_economy_catalog()
                    && let Some(resource_runtime_id) =
                        household_supply_resource_runtime_id(&catalog)
                {
                    allocator.buildings[store_idx]
                        .add_inventory_units(resource_runtime_id, household.reserved_amount);
                }
            }
        }

        let mut agent_indices = Vec::new();
        for agent_idx in 0..agents.len() {
            if agents.household_id[agent_idx] == household_id {
                agent_indices.push(agent_idx);
            }
        }
        agent_indices.sort_unstable_by(|a, b| b.cmp(a));

        debug_log!(
            "economy",
            "removing household_id={} members={} home_building={}",
            household_id,
            self.households[household_id].member_count,
            self.households[household_id].home_building_id
        );

        for agent_idx in agent_indices {
            agents.kill_agent(agent_idx, allocator);
        }

        // Release the household's residential slot if they had a home.
        let home_idx = self.households[household_id].home_building_id;
        if home_idx < allocator.buildings.len() {
            allocator.release_vacancy(home_idx);
        }

        let last_household_id = self.households.len() - 1;
        self.households.swap_remove(household_id);
        if household_id < self.households.len() {
            let mut mapping = std::collections::HashMap::with_capacity(1);
            mapping.insert(last_household_id, household_id);
            agents.remap_household_indices(&mapping);
        }
    }
}

fn stock_days(stock: f32, member_count: u16, consumption_rate: f32) -> f32 {
    let daily_consumption = member_count as f32 * consumption_rate;
    if daily_consumption <= 0.0 {
        0.0
    } else {
        stock / daily_consumption
    }
}

/// Sells all unreserved output inventory at OWA export prices, crediting `operating_budget`
/// immediately.
///
/// Called during Step 4 of the daily settlement when a building's budget goes negative after
/// utility payment. Bypasses the normal `min_shipment_units` buffer check. If inventory is
/// empty the function is a no-op and `budget_distress` will still be set by the caller.
///
/// Price = `catalog.unit_price_for_resource × owa_export_price_multiplier` (default 0.6×),
/// matching the normal OWA export path. Falls back to `profile.unit_price_currency` when no
/// catalog price is registered.
fn forced_owa_liquidation(building: &mut Building, catalog: &RuntimeEconomyCatalog) {
    let tuning = load_runtime_economy_tuning()
        .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
    let export_multiplier = tuning.owa_export_price_multiplier.clamp(0.0, 1.0);
    let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id) else {
        return;
    };
    for output_port in &profile.outputs {
        let available = building.inventory_units(output_port.resource_runtime_id);
        if available <= 0.0 {
            continue;
        }
        let unit_price = catalog
            .unit_price_for_resource(output_port.resource_runtime_id)
            .unwrap_or(profile.unit_price_currency)
            * export_multiplier;
        let revenue = available * unit_price;
        building.operating_budget += revenue;
        building.revenue += revenue;
        building.set_inventory_units(output_port.resource_runtime_id, 0.0);
    }
}

fn economy_profile_for_building<'a>(
    catalog: &'a RuntimeEconomyCatalog,
    building: &Building,
) -> Option<&'a EconomyProfileRuntime> {
    if building.economy_broken || building.economy_profile_runtime_id == 0 {
        return None;
    }
    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
}

/// Lookup helper for the authoritative household demand profile.
fn get_household_demand_profile<'a>(
    catalog: &'a RuntimeEconomyCatalog,
) -> Option<&'a EconomyProfileRuntime> {
    catalog.profile_for_id("basic_household_demand")
}

fn household_supply_resource_runtime_id(
    catalog: &RuntimeEconomyCatalog,
) -> Option<ResourceRuntimeId> {
    catalog.resource_runtime_id_for_id("household_supplies")
}

fn household_supply_unit_price(catalog: &RuntimeEconomyCatalog) -> f32 {
    household_supply_resource_runtime_id(catalog)
        .and_then(|resource_runtime_id| catalog.unit_price_for_resource(resource_runtime_id))
        .unwrap_or(0.0)
}

pub(crate) fn building_total_output_inventory(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    profile
        .outputs
        .iter()
        .map(|port| building.inventory_units(port.resource_runtime_id))
        .sum()
}

pub(crate) fn household_reserve_days(
    catalog: &RuntimeEconomyCatalog,
    household: &Household,
) -> f32 {
    let members = household.member_count.max(1) as f32;
    let daily_supply_cost =
        members * household.consumption_rate.max(0.0) * household_supply_unit_price(catalog);
    let daily_utility_cost = members * HOUSEHOLD_UTILITY_COST_PER_MEMBER;
    let daily_essential_cost = daily_supply_cost + daily_utility_cost;
    if daily_essential_cost <= 0.0 {
        0.0
    } else {
        (household.budget.max(0.0) / daily_essential_cost).max(0.0)
    }
}

fn household_is_housed(household: &Household, allocator: &BuildingAllocator) -> bool {
    household.home_building_id < allocator.buildings.len()
        && !allocator.buildings[household.home_building_id].broken
        && !allocator.buildings[household.home_building_id].economy_broken
}

fn clear_replenishment_request(household: &mut Household) {
    household.replenishment_state = REPLENISHMENT_STABLE;
    household.cooldown_hours = 0;
    household.reserved_store_building_id = usize::MAX;
    household.reserved_amount = 0.0;
    household.reserved_total_cost = 0.0;
    household.pickup_eta_hours = 0;
}

fn stable_replenishment_offset_hours(home_building_id: usize, household_seed: u32) -> u16 {
    let mixed = (home_building_id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(household_seed).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    (mixed % OPERATIONAL_HOURS_PER_DAY as u64) as u16
}

fn household_income_pressure(catalog: &RuntimeEconomyCatalog, household: &Household) -> f32 {
    let profile = get_household_demand_profile(catalog);
    let target_days = profile
        .map(|p| p.stock_target_days)
        .unwrap_or(HOUSEHOLD_TARGET_STOCK_DAYS);

    let daily_consumption = household.member_count.max(1) as f32 * household.consumption_rate;
    let reserve_target = daily_consumption * household_supply_unit_price(catalog) * target_days
        + household.member_count.max(1) as f32 * HOUSEHOLD_UTILITY_COST_PER_MEMBER * target_days;
    (1.0 - (household.budget / reserve_target.max(1.0)).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn normalized_commute_penalty(home: &Building, work: &Building) -> f32 {
    let dx = home.center_x - work.center_x;
    let dy = home.center_y - work.center_y;
    ((dx * dx + dy * dy).sqrt() / 2000.0).clamp(0.0, 1.0)
}

pub(crate) fn level_tuning_value(values: &[f32], level: u8) -> f32 {
    let index = level.saturating_sub(1) as usize;
    values
        .get(index)
        .copied()
        .or_else(|| values.last().copied())
        .unwrap_or(0.0)
}

pub(crate) fn building_operating_buffer_days(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    let daily_operating_cost = building.worker_count as f32 * profile.average_daily_wage()
        + match building.zone_type {
            ZoneType::Commercial => UTILITY_COST_COMMERCIAL,
            ZoneType::Industrial => UTILITY_COST_INDUSTRIAL,
            _ => 0.0,
        };
    if daily_operating_cost <= 0.0 {
        0.0
    } else {
        (building.operating_budget.max(0.0) / daily_operating_cost).max(0.0)
    }
}

pub(crate) fn building_staffing_ratio(
    allocator: &BuildingAllocator,
    building_idx: usize,
    building: &Building,
) -> f32 {
    let worker_capacity = allocator.worker_capacity(building_idx);
    if worker_capacity == 0 {
        0.0
    } else {
        (building.worker_count as f32 / worker_capacity as f32).clamp(0.0, 1.0)
    }
}

pub(crate) fn industrial_input_coverage_factor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    if profile.inputs.is_empty() {
        1.0
    } else {
        profile
            .inputs
            .iter()
            .map(|port| {
                if port.units_per_day <= 0.0 {
                    1.0
                } else {
                    (building.inventory_units(port.resource_runtime_id) / port.units_per_day)
                        .clamp(0.0, 1.0)
                }
            })
            .fold(1.0, f32::min)
    }
}

pub(crate) fn industrial_output_headroom_factor(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
) -> f32 {
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return 0.0;
    };
    if profile.outputs.is_empty() {
        1.0
    } else {
        profile
            .outputs
            .iter()
            .map(|port| {
                let output_capacity_units = profile.output_buffer_capacity_units_for(port);
                if !output_capacity_units.is_finite() || output_capacity_units <= 0.0 {
                    1.0
                } else {
                    let remaining_headroom = (output_capacity_units
                        - building.inventory_units(port.resource_runtime_id))
                    .max(0.0);
                    (remaining_headroom / output_capacity_units).clamp(0.0, 1.0)
                }
            })
            .fold(1.0, f32::min)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass,
    };
    use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    use godot::prelude::Vector2;

    fn test_economy_runtime_id(zone_type: ZoneType) -> u16 {
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        match zone_type {
            ZoneType::Commercial => {
                catalog
                    .profile_for_id("grocery_basic")
                    .expect("grocery starter profile")
                    .runtime_id
            }
            ZoneType::Industrial => {
                catalog
                    .profile_for_id("food_processor_basic")
                    .expect("food processor starter profile")
                    .runtime_id
            }
            _ => 0,
        }
    }

    fn make_building(center_x: f32, zone_type: ZoneType, asset_id: &str, stock: f32) -> Building {
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        let runtime_id = test_economy_runtime_id(zone_type);
        let mut resource_inventory = vec![0.0; catalog.resource_count()];
        if stock > 0.0
            && let Some(profile) = catalog.profile_by_runtime_id(runtime_id)
            && let Some(output_port) = profile.outputs.first()
        {
            resource_inventory[output_port.resource_runtime_id as usize - 1] = stock;
        }
        Building {
            center_x,
            center_y: 0.0,
            width_cells: 2,
            depth_cells: 2,
            zone_profile_runtime_id: 0,
            zone_type,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            is_deserted: false,
            budget_distress: false,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: asset_id.to_owned(),
            level: 1,
            broken: false,
            economy_profile_runtime_id: runtime_id,
            economy_broken: false,
            resource_inventory,
            revenue: 0.0,
            operating_budget: 500.0,
            shipment_cooldown_hours: 0,
            daily_owa_input_value: 0.0,
            daily_local_input_value: 0.0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
        let (household_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        allocator.registry.register(
            pack_id,
            AssetManifest {
                asset_id: asset_id.to_owned(),
                display_name: "Test".to_owned(),
                asset_set: None,
                tags: vec![],
                thumbnail: None,
                lods: vec![LodEntry {
                    file: "lod0.glb".to_owned(),
                    distance_min_m: 0.0,
                    distance_max_m: None,
                }],
                anchors: vec![Anchor {
                    anchor_type: AnchorType::Entrance,
                    name: "main".to_owned(),
                    position: [0.0, 0.0, 0.5],
                    forward: [0.0, 0.0, 1.0],
                }],
                building: Some(BuildingData {
                    flat_size_m2: None,
                    placement_mode: PlacementMode::ZonedPrivate,
                    zone_type: Some(zone),
                    density: Some("low".to_owned()),
                    lot_width_cells: 2,
                    lot_depth_cells: 2,
                    min_zone_width_cells: None,
                    min_zone_depth_cells: None,
                    level: 1,
                    household_capacity,
                    worker_capacity,
                    service_class: None,
                    economy_profile: match zone {
                        ZoneClass::Commercial => Some("grocery_basic".to_owned()),
                        ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
                        _ => None,
                    },
                    preview_scale: Some(1.0),
                }),
                prop: None,
                vehicle: None,
                character: None,
                pivot_offset: None,
            },
            String::new(),
        );
        format!("{pack_id}:{asset_id}")
    }

    #[test]
    fn household_replenishment_flows_through_reserved_and_pickup_states() {
        let mut households = HouseholdSystem::new();
        households.households.push(Household {
            home_building_id: 0,
            budget: 300.0,
            stock: 0.0,
            member_count: 2,
            consumption_rate: 1.0,
            stock_days: 0.0,
            replenishment_state: REPLENISHMENT_NEEDS,
            cooldown_hours: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_hours: 0,
            stay_failure_days: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        });

        let mut allocator = BuildingAllocator::new();
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "replenish_res",
            ZoneClass::Residential,
        );
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "replenish_com",
            ZoneClass::Commercial,
        );
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Commercial,
            &commercial_asset,
            50.0,
        ));
        allocator.rebuild_zone_index();

        households.run_household_replenishment(&mut allocator, 0);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_RESERVED
        );
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        let household_supplies = catalog
            .resource_runtime_id_for_id("household_supplies")
            .expect("household supplies resource");
        assert_eq!(
            allocator.buildings[1].inventory_units(household_supplies),
            40.0
        );
        assert_eq!(households.households[0].budget, 50.0);

        households.run_household_replenishment(&mut allocator, 0);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_PICKUP_PENDING
        );

        households.run_household_replenishment(&mut allocator, 0);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_FULFILLED
        );
        assert_eq!(households.households[0].stock, 10.0);
        assert_eq!(allocator.buildings[1].revenue, 250.0);
    }

    #[test]
    fn immigrant_household_assigns_nearby_work_during_founding() {
        let mut households = HouseholdSystem::new();
        let catalog = load_runtime_economy_catalog().expect("catalog");
        let hid = households.admit_immigrant_household(&catalog, 0, 2);
        households.households[hid].budget = 0.0;
        households.households[hid].stock = 1.0;
        households.households[hid].stock_days = 0.5;

        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "test", "res_house", ZoneClass::Residential);
        let industrial_asset =
            register_test_asset(&mut allocator, "test", "ind_shop", ZoneClass::Industrial);
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Industrial,
            &industrial_asset,
            0.0,
        ));
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
        let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
        for a in [a0, a1] {
            agents.household_id[a] = hid;
            agents.transit[a] = TRANSIT_IN_BUILDING;
            agents.current_building[a] = 0;
            agents.target_building[a] = usize::MAX;
            agents.current_node[a] = 0;
            agents.has_car[a] = true;
        }

        households.consume_household_stock(&mut agents);
        households.assign_agent_workplaces(&mut agents, &mut allocator);

        assert_eq!(agents.work_building[a0], 1);
        assert_eq!(agents.planned_activity[a0], 0);
        assert_eq!(agents.planned_target_building[a0], usize::MAX);
    }

    fn make_household(
        home_building_id: usize,
        member_count: u16,
        reserve_days: f32,
        stock_days: f32,
    ) -> Household {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let consumption_rate = 1.0;
        let daily_supply_cost =
            member_count.max(1) as f32 * consumption_rate * household_supply_unit_price(&catalog);
        let daily_utility_cost = member_count.max(1) as f32 * HOUSEHOLD_UTILITY_COST_PER_MEMBER;
        let daily_essential_cost = daily_supply_cost + daily_utility_cost;
        Household {
            home_building_id,
            budget: reserve_days * daily_essential_cost,
            stock: stock_days * member_count.max(1) as f32 * consumption_rate,
            member_count,
            consumption_rate,
            stock_days,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_hours: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_hours: 0,
            stay_failure_days: 0,
            replenishment_offset_hours: 0,
            unemployment_days_elapsed: 0,
        }
    }

    #[test]
    fn demand_household_removal_prioritizes_unhoused_households() {
        let mut households = HouseholdSystem::new();
        households.households.push(make_household(0, 1, 0.5, 1.0));
        households
            .households
            .push(make_household(usize::MAX, 1, 5.0, 5.0));
        households.households.push(make_household(1, 1, 2.0, 2.0));

        let mut allocator = BuildingAllocator::new();
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "removal_res_a",
            ZoneClass::Residential,
        );
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let housed_a = agents.spawn_housed_agent(0, 0.0, 0.0);
        let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
        let housed_b = agents.spawn_housed_agent(1, 0.0, 0.0);
        agents.household_id[housed_a] = 0;
        agents.household_id[unhoused] = 1;
        agents.home_building[unhoused] = usize::MAX;
        agents.target_building[unhoused] = usize::MAX;
        agents.household_id[housed_b] = 2;
        agents.recalculate_occupancy(&mut allocator);

        households.execute_demand_household_removal(1, &mut agents, &mut allocator);

        assert_eq!(households.households.len(), 2);
        assert_eq!(agents.len(), 2);
        assert!(
            agents
                .household_id
                .iter()
                .all(|&household_id| household_id < households.households.len())
        );
        assert!(agents.home_building.iter().all(|&home| home != usize::MAX));
    }

    #[test]
    fn demand_household_removal_uses_weaker_housed_households_after_unhoused_pool() {
        let mut households = HouseholdSystem::new();
        households.households.push(make_household(0, 1, 0.5, 0.5));
        households.households.push(make_household(1, 1, 5.0, 5.0));
        households
            .households
            .push(make_household(usize::MAX, 1, 4.0, 4.0));

        let mut allocator = BuildingAllocator::new();
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "removal_res_b",
            ZoneClass::Residential,
        );
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let weak_housed = agents.spawn_housed_agent(0, 0.0, 0.0);
        let strong_housed = agents.spawn_housed_agent(1, 0.0, 0.0);
        let unhoused = agents.spawn_housed_agent(0, 0.0, 0.0);
        agents.household_id[weak_housed] = 0;
        agents.household_id[strong_housed] = 1;
        agents.household_id[unhoused] = 2;
        agents.home_building[unhoused] = usize::MAX;
        agents.target_building[unhoused] = usize::MAX;
        agents.recalculate_occupancy(&mut allocator);

        households.execute_demand_household_removal(2, &mut agents, &mut allocator);

        assert_eq!(households.households.len(), 1);
        assert_eq!(agents.len(), 1);
        assert_eq!(households.households[0].home_building_id, 1);
        assert_eq!(agents.household_id[0], 0);
        assert_eq!(agents.home_building[0], 1);
        assert_eq!(allocator.buildings[0].occupancy, 0);
        assert_eq!(allocator.buildings[1].occupancy, 1);
    }

    #[test]
    fn unhoused_household_rehouses_into_affordable_vacant_home() {
        let mut households = HouseholdSystem::new();
        households
            .households
            .push(make_household(usize::MAX, 2, 12.0, 3.0));

        let mut allocator = BuildingAllocator::new();
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "rehouse_res",
            ZoneClass::Residential,
        );
        allocator.buildings.push(make_building(
            0.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
        ));
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
        let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
        for a in [a0, a1] {
            agents.household_id[a] = 0;
            agents.home_building[a] = usize::MAX;
            agents.current_building[a] = usize::MAX;
            agents.target_building[a] = usize::MAX;
            agents.planned_target_building[a] = usize::MAX;
            agents.transit[a] = TRANSIT_ACCESS_INGRESS;
        }

        households.resolve_household_housing(&mut agents, &mut allocator);

        assert_eq!(households.households[0].home_building_id, 0);
        assert_eq!(allocator.buildings[0].occupancy, 1);
        assert_eq!(households.households.len(), 1);
        assert_eq!(agents.home_building[a1], 0);
        assert_eq!(agents.target_building[a0], 0);
        assert_eq!(agents.target_building[a1], 0);
    }

    #[test]
    fn failed_stay_rule_evicts_household_when_no_affordable_home_exists() {
        let mut households = HouseholdSystem::new();
        let mut household = make_household(0, 2, 0.5, 1.0);
        household.stay_failure_days = 1;
        households.households.push(household);

        let mut allocator = BuildingAllocator::new();
        let residential_asset =
            register_test_asset(&mut allocator, "test", "evict_res", ZoneClass::Residential);
        let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0);
        home.level = 2;
        home.occupancy = 1;
        allocator.buildings.push(home);
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let a0 = agents.spawn_housed_agent(0, 0.0, 0.0);
        let a1 = agents.spawn_housed_agent(0, 0.0, 0.0);
        for a in [a0, a1] {
            agents.household_id[a] = 0;
            agents.home_building[a] = 0;
            agents.current_building[a] = 0;
            agents.target_building[a] = usize::MAX;
            agents.planned_target_building[a] = usize::MAX;
            agents.transit[a] = TRANSIT_IN_BUILDING;
        }

        households.resolve_household_housing(&mut agents, &mut allocator);

        assert_eq!(households.households.len(), 1);
        assert_eq!(households.households[0].home_building_id, usize::MAX);
        assert_eq!(households.households[0].stay_failure_days, 0);
        assert_eq!(allocator.buildings[0].occupancy, 0);
        assert_eq!(agents.home_building[a0], usize::MAX);
        assert_eq!(agents.home_building[a1], usize::MAX);
        assert_eq!(agents.current_building[a0], usize::MAX);
        assert_eq!(agents.current_building[a1], usize::MAX);
        assert_eq!(agents.transit[a0], TRANSIT_ACCESS_INGRESS);
        assert_eq!(agents.transit[a1], TRANSIT_ACCESS_INGRESS);
    }
}
