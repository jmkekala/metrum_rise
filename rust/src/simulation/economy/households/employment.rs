//! Workplace assignment, worker counts, and daily wage payment.

use super::data::{Household, HouseholdSystem};
use super::metrics::{
    economy_profile_for_building, household_demand_profile, household_supply_unit_price,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;

const W_INCOME: f32 = 0.35;
const W_STOCK: f32 = 0.35;
const W_JOB: f32 = 0.20;
const W_COMMUTE: f32 = 0.10;
const GO_TO_WORK_THRESHOLD: f32 = 0.45;
const JOB_LOCK_DAYS: u8 = 7;
const JOB_UNPAID_ABANDON_DAYS: u8 = 2;
const JOB_SEARCH_MAX_RING: i32 = 8;
const JOB_SEARCH_CANDIDATES: usize = 24;

impl HouseholdSystem {
    pub(super) fn recount_worker_assignments(
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

    pub(super) fn assign_agent_workplaces(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let profile = household_demand_profile(&catalog);
        let target_days = profile.stock_target_days;

        // Ejection pre-pass: immediately detach workers from newly-bankrupt buildings so they
        // enter the open job market this same day rather than waiting for unpaid-wage accumulation.
        for i in 0..agents.len() {
            let work = agents.work_building[i];
            if work == usize::MAX || work >= allocator.buildings.len() {
                continue;
            }
            if allocator.buildings[work].is_deserted {
                allocator.buildings[work].worker_count =
                    allocator.buildings[work].worker_count.saturating_sub(1);
                agents.assign_work_building(i, usize::MAX, 0);
            }
        }

        let mut candidates = Vec::with_capacity(JOB_SEARCH_CANDIDATES);
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
            allocator.fill_nearby_buildings(
                home.center_x,
                home.center_y,
                JOB_SEARCH_MAX_RING,
                JOB_SEARCH_CANDIDATES,
                &mut candidates,
                |_, building| {
                    if building.is_deserted || building.broken || building.economy_broken {
                        return false;
                    }
                    catalog
                        .profile_by_runtime_id(building.economy_profile_runtime_id)
                        .is_some_and(|profile| building_offers_work(building, profile))
                },
            );
            if agents.work_building[i] != usize::MAX
                && !candidates.contains(&agents.work_building[i])
            {
                candidates.push(agents.work_building[i]);
            }

            let income_pressure = household_income_pressure(&catalog, &tuning, household);
            let stock_pressure = (1.0
                - (household.stock_days / target_days.max(0.1)).clamp(0.0, 1.0))
            .clamp(0.0, 1.0);

            let mut best_job = usize::MAX;
            let mut best_score = 0.0;
            for &candidate in &candidates {
                if candidate >= allocator.buildings.len() {
                    continue;
                }
                let building = &allocator.buildings[candidate];
                if building.is_deserted || building.broken || building.economy_broken {
                    continue;
                }
                let Some(economy_profile) =
                    catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
                else {
                    continue;
                };
                if !building_offers_work(building, economy_profile) {
                    continue;
                }
                if !workplace_is_reachable(allocator, home_idx, candidate, transit_network, graph) {
                    continue;
                }

                // Budget-based hiring constraint: Only allow hiring if the building can afford
                // to pay at least the current staff plus this potential new worker for one day.
                let average_daily_wage = economy_profile.average_daily_wage();

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
                let reserved = allocator.buildings[candidate].worker_count;
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
                    if old_job != usize::MAX && old_job < allocator.buildings.len() {
                        allocator.buildings[old_job].worker_count =
                            allocator.buildings[old_job].worker_count.saturating_sub(1);
                    }
                    allocator.buildings[best_job].worker_count =
                        allocator.buildings[best_job].worker_count.saturating_add(1);
                    agents.assign_work_building(i, best_job, JOB_LOCK_DAYS);
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
                    agents.assign_work_building(i, usize::MAX, 0);
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
}

fn household_income_pressure(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    household: &Household,
) -> f32 {
    let profile = household_demand_profile(catalog);
    let target_days = profile.stock_target_days;

    let daily_consumption = household.member_count.max(1) as f32 * household.consumption_rate;
    let reserve_target = daily_consumption * household_supply_unit_price(catalog) * target_days
        + household.member_count.max(1) as f32
            * tuning.households.utility_cost_per_member_per_day
            * target_days;
    (1.0 - (household.budget / reserve_target.max(1.0)).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn normalized_commute_penalty(home: &Building, work: &Building) -> f32 {
    let dx = home.center_x - work.center_x;
    let dy = home.center_y - work.center_y;
    ((dx * dx + dy * dy).sqrt() / 2000.0).clamp(0.0, 1.0)
}

fn building_offers_work(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) || matches!(
        profile.kind,
        EconomyProfileRuntimeKind::UtilityProducer | EconomyProfileRuntimeKind::UtilityProcessor
    )
}

fn workplace_is_reachable(
    allocator: &BuildingAllocator,
    home_idx: usize,
    work_idx: usize,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
) -> bool {
    if home_idx == work_idx {
        return true;
    }
    if allocator.entrances.len() == allocator.buildings.len() {
        return allocator
            .freight_car_eta_between_buildings(home_idx, work_idx, transit_network, graph)
            .is_some();
    }
    allocator.buildings[home_idx].edge_idx != usize::MAX
        && allocator.buildings[work_idx].edge_idx != usize::MAX
}
