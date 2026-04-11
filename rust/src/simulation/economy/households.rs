//! Household runtime state and the first-pass building-centric economy loop.
//!
//! The v0.1 foundation keeps households explicit, lightweight, and tied to
//! residential buildings without reviving per-agent grocery trips. This module
//! owns household stock/budget state, simple building-side economic updates,
//! daily replenishment requests, and decision-utility-driven work/home planning.

use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
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
const OFFICE_BASE_RATE: f32 = 120.0;
const STARTUP_OPERATING_FLOAT: f32 = 500.0;

const WAGE_INDUSTRIAL: f32 = 100.0;
const WAGE_COMMERCIAL: f32 = 90.0;
const WAGE_OFFICE: f32 = 95.0;

const UTILITY_COST_COMMERCIAL: f32 = 8.0;
const UTILITY_COST_INDUSTRIAL: f32 = 12.0;
const UTILITY_COST_OFFICE: f32 = 10.0;
const UTILITY_COST_MIXED: f32 = 9.0;

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
        self.run_household_replenishment(allocator);
        self.plan_agent_work_and_return_trips(agents, allocator);
        self.sync_agent_money_from_households(agents);
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
                ZoneType::Commercial | ZoneType::Industrial | ZoneType::Office | ZoneType::Mixed
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
                ZoneType::Office => UTILITY_COST_OFFICE,
                ZoneType::Mixed => UTILITY_COST_MIXED,
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
            let throughput = match zone {
                ZoneType::Commercial => COMMERCIAL_BASE_RATE,
                ZoneType::Industrial => INDUSTRIAL_BASE_RATE,
                ZoneType::Office => OFFICE_BASE_RATE,
                ZoneType::Mixed => COMMERCIAL_BASE_RATE * 0.5,
                _ => 0.0,
            } * staffing_factor
                * utility_factor;

            let building = &mut allocator.buildings[idx];
            match zone {
                ZoneType::Commercial | ZoneType::Mixed => {}
                ZoneType::Industrial => {
                    building.stock += throughput;
                }
                ZoneType::Office => {
                    let service_revenue = throughput * 1.5;
                    building.revenue += service_revenue;
                    building.operating_budget += service_revenue;
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
                &[ZoneType::Commercial, ZoneType::Mixed],
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
                &[
                    ZoneType::Industrial,
                    ZoneType::Commercial,
                    ZoneType::Office,
                    ZoneType::Mixed,
                ],
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
                    ZoneType::Industrial
                        | ZoneType::Commercial
                        | ZoneType::Office
                        | ZoneType::Mixed
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
                ZoneType::Commercial | ZoneType::Mixed => WAGE_COMMERCIAL,
                ZoneType::Office => WAGE_OFFICE,
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
}

fn stock_days(stock: f32, member_count: u16, consumption_rate: f32) -> f32 {
    let daily_consumption = member_count as f32 * consumption_rate;
    if daily_consumption <= 0.0 {
        0.0
    } else {
        stock / daily_consumption
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::TRANSIT_IN_BUILDING;
    use godot::prelude::Vector2;

    fn make_building(center_x: f32, zone_type: ZoneType, stock: f32, utility: bool) -> Building {
        Building {
            center_x,
            center_y: 0.0,
            width_cells: 2,
            depth_cells: 2,
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
            asset_id: String::new(),
            level: 1,
            broken: false,
            stock,
            revenue: 0.0,
            operating_budget: 500.0,
            utility_service_available: utility,
            shipment_cooldown_days: 0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
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
        });

        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(make_building(0.0, ZoneType::Residential, 0.0, true));
        allocator
            .buildings
            .push(make_building(20.0, ZoneType::Commercial, 50.0, true));
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
        allocator
            .buildings
            .push(make_building(0.0, ZoneType::Residential, 0.0, true));
        allocator
            .buildings
            .push(make_building(20.0, ZoneType::Industrial, 0.0, true));
        allocator.rebuild_zone_index();

        let mut agents = AgentSystem::new();
        let a0 = agents.spawn_agent(0, 0, 0.0, 0.0, 0, 0.0, 0.0);
        let a1 = agents.spawn_agent(0, 0, 0.0, 0.0, 0, 0.0, 0.0);
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
}
