//! Household runtime state and the first-pass building-centric economy loop.
//!
//! The v0.1 foundation keeps households explicit, lightweight, and tied to
//! residential buildings without reviving per-agent grocery trips. This module
//! owns household stock/budget state, simple building-side economic updates,
//! daily replenishment requests, and decision-utility-driven work/home planning.

use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, baseline_private_zone_slot,
};
use crate::simulation::economy::agents::{
    AgentSystem, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
};
use crate::simulation::economy::definitions::load_runtime_economy_tuning;
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
const IMMIGRANT_STARTING_BUDGET_PER_MEMBER: f32 = 12.0;
const HOUSEHOLD_SUPPLY_UNIT_PRICE: f32 = 6.0;
const HOUSEHOLD_UTILITY_COST_PER_MEMBER: f32 = 2.0;
const HOUSEHOLD_STARTING_BUDGET: f32 = 100.0;
/// Default household size admitted by the first-pass immigration flow.
pub const DEFAULT_IMMIGRANT_HOUSEHOLD_SIZE: u16 = 2;

const COMMERCIAL_BASE_RATE: f32 = 200.0;
const INDUSTRIAL_BASE_RATE: f32 = 160.0;
const INDUSTRIAL_INPUT_UNITS_PER_OUTPUT: f32 = 1.0;
const INDUSTRIAL_OUTPUT_STORAGE_CAP_UNITS: f32 = 640.0;
const STARTUP_OPERATING_FLOAT: f32 = 500.0;

const WAGE_INDUSTRIAL: f32 = 100.0;
const WAGE_COMMERCIAL: f32 = 90.0;

const UTILITY_COST_COMMERCIAL: f32 = 8.0;
const UTILITY_COST_INDUSTRIAL: f32 = 12.0;

const W_INCOME: f32 = 0.35;
const W_STOCK: f32 = 0.35;
const W_JOB: f32 = 0.20;
const W_COMMUTE: f32 = 0.10;
const GO_TO_WORK_THRESHOLD: f32 = 0.45;
const JOB_SEARCH_MAX_RING: i32 = 2;
const JOB_SEARCH_CANDIDATES: usize = 8;
const GROCERY_SEARCH_MAX_RING: i32 = 2;
const GROCERY_SEARCH_CANDIDATES: usize = 8;
const HOUSEHOLD_PICKUP_DELAY_DAYS: u8 = 1;

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
    /// Remaining daily cooldown steps before another replenishment retry.
    pub cooldown_days: u8,
    /// Reserved source building for the current replenishment request, if any.
    pub reserved_store_building_id: usize,
    /// Reserved amount waiting for household pickup-side fulfillment.
    pub reserved_amount: f32,
    /// Reserved budget waiting to be transferred to the supplying store.
    pub reserved_total_cost: f32,
    /// Remaining daily steps before the reserved pickup completes.
    pub pickup_eta_days: u8,
    /// Consecutive daily stay-rule failures for the current home.
    pub stay_failure_days: u32,
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

    /// Creates one immigrant household with shared starter savings and stock.
    pub fn admit_immigrant_household(
        &mut self,
        home_building_id: usize,
        member_count: u16,
    ) -> usize {
        let member_count = member_count.max(1);
        self.households.push(Household {
            home_building_id,
            // Founding households arrive with modest savings so the first town
            // has a real incentive to take available jobs instead of idling on
            // a large abstract cash cushion.
            budget: IMMIGRANT_STARTING_BUDGET_PER_MEMBER * member_count as f32,
            stock: member_count as f32 * HOUSEHOLD_CONSUMPTION_RATE * IMMIGRANT_STARTING_STOCK_DAYS,
            member_count,
            consumption_rate: HOUSEHOLD_CONSUMPTION_RATE,
            stock_days: IMMIGRANT_STARTING_STOCK_DAYS,
            replenishment_state: REPLENISHMENT_STABLE,
            cooldown_days: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_days: 0,
            stay_failure_days: 0,
        });
        self.households.len() - 1
    }

    /// Runs the first-pass daily economy update.
    pub fn daily_tick(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.ensure_agent_households(agents);
        self.rebuild_household_membership(agents);
        self.recount_worker_assignments(agents, allocator);
        self.ensure_building_startup_float(allocator);
        self.resolve_building_utilities(allocator);
        self.run_building_economy(allocator);
        logistics.daily_tick(allocator, transit_network, graph);
        self.pay_daily_wages(agents, allocator);
        self.consume_household_stock(agents);
        self.resolve_household_housing(agents, allocator);
        self.run_household_replenishment(allocator);
        self.plan_agent_work_and_return_trips(agents, allocator);
        self.sync_agent_money_from_households(agents);
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
        for (household_id, household) in self.households.iter().enumerate() {
            if household.member_count == 0 {
                continue;
            }
            let reserve_days = household_reserve_days(household);
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
                let budget = agents.money[i].max(HOUSEHOLD_STARTING_BUDGET);
                self.households.push(Household {
                    home_building_id: home,
                    budget,
                    stock: HOUSEHOLD_TARGET_STOCK_DAYS * HOUSEHOLD_CONSUMPTION_RATE,
                    member_count: 0,
                    consumption_rate: HOUSEHOLD_CONSUMPTION_RATE,
                    stock_days: HOUSEHOLD_TARGET_STOCK_DAYS,
                    replenishment_state: REPLENISHMENT_STABLE,
                    cooldown_days: 0,
                    reserved_store_building_id: usize::MAX,
                    reserved_amount: 0.0,
                    reserved_total_cost: 0.0,
                    pickup_eta_days: 0,
                    stay_failure_days: 0,
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

    fn ensure_building_startup_float(&mut self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            if !matches!(
                building.zone_type,
                ZoneType::Commercial | ZoneType::Industrial
            ) {
                continue;
            }
            if building.operating_budget == 0.0 && building.revenue == 0.0 {
                building.operating_budget = STARTUP_OPERATING_FLOAT;
            }
        }
    }

    fn resolve_building_utilities(&mut self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            let utility_cost = match building.zone_type {
                ZoneType::Commercial => UTILITY_COST_COMMERCIAL,
                ZoneType::Industrial => UTILITY_COST_INDUSTRIAL,
                _ => {
                    building.utility_service_available = true;
                    continue;
                }
            };
            if building.edge_idx == usize::MAX || building.broken {
                building.utility_service_available = false;
                continue;
            }
            if building.operating_budget >= utility_cost {
                building.operating_budget -= utility_cost;
                building.utility_service_available = true;
            } else {
                building.utility_service_available = false;
            }
        }
    }

    fn run_building_economy(&mut self, allocator: &mut BuildingAllocator) {
        for idx in 0..allocator.buildings.len() {
            let zone = allocator.buildings[idx].zone_type;
            let worker_capacity = allocator.worker_capacity(idx).max(1);
            let staffing_factor = (allocator.buildings[idx].worker_count as f32
                / worker_capacity as f32)
                .clamp(0.0, 1.0);
            let utility_factor = if allocator.buildings[idx].utility_service_available {
                1.0
            } else {
                0.0
            };
            let input_factor = if zone == ZoneType::Industrial {
                industrial_input_coverage_factor(&allocator.buildings[idx])
            } else {
                1.0
            };
            let output_headroom_factor = if zone == ZoneType::Industrial {
                industrial_output_headroom_factor(&allocator.buildings[idx])
            } else {
                1.0
            };
            let throughput = match zone {
                ZoneType::Commercial => COMMERCIAL_BASE_RATE,
                ZoneType::Industrial => INDUSTRIAL_BASE_RATE,
                _ => 0.0,
            } * staffing_factor
                * input_factor
                * output_headroom_factor
                * utility_factor;

            let building = &mut allocator.buildings[idx];
            match zone {
                ZoneType::Commercial => {}
                ZoneType::Industrial => {
                    let consumed_inputs = throughput * INDUSTRIAL_INPUT_UNITS_PER_OUTPUT;
                    building.input_stock = (building.input_stock - consumed_inputs).max(0.0);
                    building.stock =
                        (building.stock + throughput).min(INDUSTRIAL_OUTPUT_STORAGE_CAP_UNITS);
                }
                _ => {}
            }
        }
    }

    fn consume_household_stock(&mut self, agents: &mut AgentSystem) {
        for hid in 0..self.households.len() {
            let household = &mut self.households[hid];
            if household.member_count == 0 {
                continue;
            }
            let daily_consumption = household.member_count as f32 * household.consumption_rate;
            household.stock = (household.stock - daily_consumption).max(0.0);
            let utility_cost = household.member_count as f32 * HOUSEHOLD_UTILITY_COST_PER_MEMBER;
            household.budget = (household.budget - utility_cost).max(0.0);
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
                if household.cooldown_days > 0 {
                    household.cooldown_days -= 1;
                }
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else if household.cooldown_days > 0 {
                household.cooldown_days -= 1;
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
            } else if household.stock_days < HOUSEHOLD_TRIGGER_STOCK_DAYS {
                household.replenishment_state = REPLENISHMENT_NEEDS;
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
        let config = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));

        for household_id in 0..self.households.len() {
            let household = &self.households[household_id];
            if household.member_count == 0 {
                continue;
            }

            let reserve_days = household_reserve_days(household);
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
        let household = &self.households[household_id];
        let required_slots = household.member_count.max(1) as u32;
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
            if building.broken || building.pending_redevelopment {
                continue;
            }

            let free_slots = allocator
                .resident_capacity(building_idx)
                .saturating_sub(building.occupancy);
            if free_slots < required_slots {
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

        let member_count = self.households[household_id].member_count as u32;
        if old_home < allocator.buildings.len() {
            for _ in 0..member_count {
                allocator.release_vacancy(old_home);
            }
        }
        for _ in 0..member_count {
            allocator.claim_vacancy(new_home);
        }

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

        let member_count = self.households[household_id].member_count as u32;
        if old_home < allocator.buildings.len() {
            for _ in 0..member_count {
                allocator.release_vacancy(old_home);
            }
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

    fn run_household_replenishment(&mut self, allocator: &mut BuildingAllocator) {
        for hid in 0..self.households.len() {
            self.progress_household_replenishment(hid, allocator);
        }

        for hid in 0..self.households.len() {
            let household = &self.households[hid];
            if household.member_count == 0
                || household.replenishment_state != REPLENISHMENT_NEEDS
                || household.home_building_id == usize::MAX
                || household.home_building_id >= allocator.buildings.len()
            {
                continue;
            }

            let home = &allocator.buildings[household.home_building_id];
            let candidates = allocator.find_nearby_buildings_by_zones(
                home.center_x,
                home.center_y,
                &[ZoneType::Commercial],
                GROCERY_SEARCH_MAX_RING,
                GROCERY_SEARCH_CANDIDATES,
            );

            let daily_consumption = household.member_count as f32 * household.consumption_rate;
            let target_stock = HOUSEHOLD_TARGET_STOCK_DAYS * daily_consumption;
            let mut desired_amount = (target_stock - household.stock).max(0.0);
            let mut found_sale = None;

            for candidate in candidates {
                let store = &allocator.buildings[candidate];
                if store.stock <= 0.0 || !store.utility_service_available {
                    continue;
                }
                let amount = desired_amount.min(store.stock);
                let total_cost = amount * HOUSEHOLD_SUPPLY_UNIT_PRICE;
                if amount > 0.0 && household.budget >= total_cost {
                    found_sale = Some((candidate, amount, total_cost));
                    break;
                }
                desired_amount = desired_amount.min(store.stock);
            }

            let household = &mut self.households[hid];
            if let Some((store_idx, amount, total_cost)) = found_sale {
                let store = &mut allocator.buildings[store_idx];
                store.stock -= amount;
                household.budget -= total_cost;
                household.reserved_store_building_id = store_idx;
                household.reserved_amount = amount;
                household.reserved_total_cost = total_cost;
                household.pickup_eta_days = HOUSEHOLD_PICKUP_DELAY_DAYS;
                household.replenishment_state = REPLENISHMENT_RESERVED;
            } else {
                household.replenishment_state = REPLENISHMENT_COOLDOWN;
                household.cooldown_days = 1;
            }
        }
    }

    fn progress_household_replenishment(&mut self, hid: usize, allocator: &mut BuildingAllocator) {
        let Some(household) = self.households.get_mut(hid) else {
            return;
        };
        match household.replenishment_state {
            REPLENISHMENT_RESERVED => {
                if household.pickup_eta_days > 0 {
                    household.pickup_eta_days -= 1;
                }
                household.replenishment_state = REPLENISHMENT_PICKUP_PENDING;
            }
            REPLENISHMENT_PICKUP_PENDING => {
                let store_idx = household.reserved_store_building_id;
                if store_idx == usize::MAX || store_idx >= allocator.buildings.len() {
                    household.budget += household.reserved_total_cost;
                    clear_replenishment_request(household);
                    household.replenishment_state = REPLENISHMENT_COOLDOWN;
                    household.cooldown_days = 1;
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
                household.cooldown_days = 1;
                household.reserved_store_building_id = usize::MAX;
                household.reserved_amount = 0.0;
                household.reserved_total_cost = 0.0;
                household.pickup_eta_days = 0;
            }
            _ => {}
        }
    }

    fn plan_agent_work_and_return_trips(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        let mut reserved_workers: Vec<u32> =
            allocator.buildings.iter().map(|b| b.worker_count).collect();

        for i in 0..agents.len() {
            agents.planned_activity[i] = 0;
            agents.planned_target_building[i] = usize::MAX;

            if agents.transit[i] != TRANSIT_IN_BUILDING {
                continue;
            }

            let home_idx = agents.home_building[i];
            let current_idx = agents.current_building[i];
            if home_idx == usize::MAX || home_idx >= allocator.buildings.len() {
                continue;
            }

            if current_idx == agents.work_building[i]
                && current_idx != usize::MAX
                && current_idx < allocator.buildings.len()
            {
                agents.planned_activity[i] = 0;
                agents.planned_target_building[i] = home_idx;
                continue;
            }

            if current_idx != home_idx {
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

            let income_pressure = household_income_pressure(household);
            let stock_pressure = (1.0
                - (household.stock_days / HOUSEHOLD_TARGET_STOCK_DAYS).clamp(0.0, 1.0))
            .clamp(0.0, 1.0);

            let mut best_job = usize::MAX;
            let mut best_score = 0.0;
            for candidate in candidates {
                if candidate >= allocator.buildings.len() {
                    continue;
                }
                let building = &allocator.buildings[candidate];
                if !matches!(
                    building.zone_type,
                    ZoneType::Industrial | ZoneType::Commercial
                ) {
                    continue;
                }

                let worker_capacity = allocator.worker_capacity(candidate);
                if worker_capacity == 0 {
                    continue;
                }
                let already_assigned = agents.work_building[i] == candidate;
                let reserved = reserved_workers[candidate];
                let open_slots = if already_assigned {
                    worker_capacity.saturating_sub(reserved.saturating_sub(1))
                } else {
                    worker_capacity.saturating_sub(reserved)
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
                if old_job != best_job {
                    if old_job != usize::MAX && old_job < reserved_workers.len() {
                        reserved_workers[old_job] = reserved_workers[old_job].saturating_sub(1);
                    }
                    reserved_workers[best_job] = reserved_workers[best_job].saturating_add(1);
                    agents.work_building[i] = best_job;
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
                agents.planned_activity[i] = 1;
                agents.planned_target_building[i] = best_job;
            }
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
        for i in 0..agents.len() {
            let work = agents.work_building[i];
            let hid = agents.household_id[i];
            if work == usize::MAX || hid == usize::MAX {
                continue;
            }
            if work >= allocator.buildings.len() || hid >= self.households.len() {
                continue;
            }
            let wage = match allocator.buildings[work].zone_type {
                ZoneType::Industrial => WAGE_INDUSTRIAL,
                ZoneType::Commercial => WAGE_COMMERCIAL,
                _ => 0.0,
            };
            if wage <= 0.0 {
                continue;
            }
            if allocator.buildings[work].operating_budget >= wage {
                allocator.buildings[work].operating_budget -= wage;
                self.households[hid].budget += wage;
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
                allocator.buildings[store_idx].stock += household.reserved_amount;
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

pub(crate) fn household_reserve_days(household: &Household) -> f32 {
    let members = household.member_count.max(1) as f32;
    let daily_supply_cost =
        members * household.consumption_rate.max(0.0) * HOUSEHOLD_SUPPLY_UNIT_PRICE;
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
}

fn clear_replenishment_request(household: &mut Household) {
    household.replenishment_state = REPLENISHMENT_STABLE;
    household.cooldown_days = 0;
    household.reserved_store_building_id = usize::MAX;
    household.reserved_amount = 0.0;
    household.reserved_total_cost = 0.0;
    household.pickup_eta_days = 0;
}

fn household_income_pressure(household: &Household) -> f32 {
    let daily_consumption = household.member_count.max(1) as f32 * household.consumption_rate;
    let reserve_target =
        daily_consumption * HOUSEHOLD_SUPPLY_UNIT_PRICE * HOUSEHOLD_TARGET_STOCK_DAYS
            + household.member_count.max(1) as f32
                * HOUSEHOLD_UTILITY_COST_PER_MEMBER
                * HOUSEHOLD_TARGET_STOCK_DAYS;
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

pub(crate) fn building_operating_buffer_days(building: &Building) -> f32 {
    let daily_operating_cost = match building.zone_type {
        ZoneType::Commercial => {
            building.worker_count as f32 * WAGE_COMMERCIAL + UTILITY_COST_COMMERCIAL
        }
        ZoneType::Industrial => {
            building.worker_count as f32 * WAGE_INDUSTRIAL + UTILITY_COST_INDUSTRIAL
        }
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

pub(crate) fn industrial_input_coverage_factor(building: &Building) -> f32 {
    let daily_input_need = INDUSTRIAL_BASE_RATE * INDUSTRIAL_INPUT_UNITS_PER_OUTPUT;
    if daily_input_need <= 0.0 {
        0.0
    } else {
        (building.input_stock / daily_input_need).clamp(0.0, 1.0)
    }
}

pub(crate) fn industrial_output_headroom_factor(building: &Building) -> f32 {
    let remaining_headroom = (INDUSTRIAL_OUTPUT_STORAGE_CAP_UNITS - building.stock).max(0.0);
    (remaining_headroom / INDUSTRIAL_OUTPUT_STORAGE_CAP_UNITS).clamp(0.0, 1.0)
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

    fn make_building(
        center_x: f32,
        zone_type: ZoneType,
        asset_id: &str,
        stock: f32,
        utility: bool,
    ) -> Building {
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
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: asset_id.to_owned(),
            level: 1,
            broken: false,
            stock,
            input_stock: 0.0,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: utility,
            shipment_cooldown_days: 0,
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
        let (residents_capacity, worker_capacity) = match zone {
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
                    placement_mode: PlacementMode::ZonedPrivate,
                    zone_type: Some(zone),
                    density: Some("low".to_owned()),
                    lot_width_cells: 2,
                    lot_depth_cells: 2,
                    min_zone_width_cells: None,
                    min_zone_depth_cells: None,
                    level: 1,
                    residents_capacity,
                    worker_capacity,
                    service_class: None,
                    economy_profile: None,
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
            budget: 200.0,
            stock: 0.0,
            member_count: 2,
            consumption_rate: 1.0,
            stock_days: 0.0,
            replenishment_state: REPLENISHMENT_NEEDS,
            cooldown_days: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_days: 0,
            stay_failure_days: 0,
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
            true,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Commercial,
            &commercial_asset,
            50.0,
            true,
        ));
        allocator.rebuild_zone_index();

        households.run_household_replenishment(&mut allocator);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_RESERVED
        );
        assert_eq!(allocator.buildings[1].stock, 44.0);
        assert_eq!(households.households[0].budget, 164.0);

        households.run_household_replenishment(&mut allocator);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_PICKUP_PENDING
        );

        households.run_household_replenishment(&mut allocator);
        assert_eq!(
            households.households[0].replenishment_state,
            REPLENISHMENT_FULFILLED
        );
        assert_eq!(households.households[0].stock, 6.0);
        assert_eq!(allocator.buildings[1].revenue, 36.0);
    }

    #[test]
    fn immigrant_household_plans_nearby_work_during_founding() {
        let mut households = HouseholdSystem::new();
        let hid = households.admit_immigrant_household(0, 2);

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
            true,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Industrial,
            &industrial_asset,
            0.0,
            true,
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
        households.plan_agent_work_and_return_trips(&mut agents, &allocator);

        assert_eq!(agents.planned_activity[a0], 1);
        assert_eq!(agents.work_building[a0], 1);
        assert_eq!(agents.planned_target_building[a0], 1);
    }

    fn make_household(
        home_building_id: usize,
        member_count: u16,
        reserve_days: f32,
        stock_days: f32,
    ) -> Household {
        let consumption_rate = 1.0;
        let daily_supply_cost =
            member_count.max(1) as f32 * consumption_rate * HOUSEHOLD_SUPPLY_UNIT_PRICE;
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
            cooldown_days: 0,
            reserved_store_building_id: usize::MAX,
            reserved_amount: 0.0,
            reserved_total_cost: 0.0,
            pickup_eta_days: 0,
            stay_failure_days: 0,
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
            true,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
            true,
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
            true,
        ));
        allocator.buildings.push(make_building(
            20.0,
            ZoneType::Residential,
            &residential_asset,
            0.0,
            true,
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
            true,
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
        assert_eq!(allocator.buildings[0].occupancy, 2);
        assert_eq!(agents.home_building[a0], 0);
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
        let mut home = make_building(0.0, ZoneType::Residential, &residential_asset, 0.0, true);
        home.level = 2;
        home.occupancy = 2;
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
