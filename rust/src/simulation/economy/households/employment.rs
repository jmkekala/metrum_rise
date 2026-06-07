//! Workplace assignment, worker counts, and daily wage payment.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU32, Ordering};

use super::data::{Household, HouseholdSystem};
use super::metrics::{
    economy_profile_for_building, household_demand_profile, household_supply_unit_price,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::tick::estimate_building_origin_trip_minutes;
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;
use rayon::prelude::*;

const W_INCOME: f32 = 0.35;
const W_STOCK: f32 = 0.35;
const W_JOB: f32 = 0.20;
const W_COMMUTE: f32 = 0.10;
const GO_TO_WORK_THRESHOLD: f32 = 0.45;
const JOB_LOCK_DAYS: u8 = 7;
const JOB_UNPAID_ABANDON_DAYS: u8 = 2;
const JOB_SEARCH_MAX_RING: i32 = 8;
const JOB_SEARCH_CANDIDATES: usize = 24;
const COMMUTE_PENALTY_MAX_SECONDS: f32 = 30.0 * 60.0;
const EMPTY_JOB_CHOICE: JobChoice = JobChoice {
    building_idx: usize::MAX,
    score: 0.0,
};

#[derive(Clone, Copy)]
struct JobChoice {
    building_idx: usize,
    score: f32,
}

struct WagePaymentPlan {
    agent_idx: usize,
    work_building: usize,
    household_id: usize,
    wage: f32,
}

impl HouseholdSystem {
    pub(super) fn recount_worker_assignments(
        &mut self,
        agents: &AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let building_count = allocator.buildings.len();
        self.reset_worker_count_scratch(building_count);
        let worker_count_scratch = &self.worker_count_scratch;
        agents.work_building.par_iter().for_each(|&work| {
            if work < building_count {
                worker_count_scratch[work].fetch_add(1, Ordering::Relaxed);
            }
        });
        allocator
            .buildings
            .par_iter_mut()
            .zip(worker_count_scratch.par_iter())
            .for_each(|(building, count)| {
                building.worker_count = count.load(Ordering::Relaxed);
            });
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

        let mut ejected_agents: Vec<_> = (0..agents.len())
            .into_par_iter()
            .filter(|&i| {
                let work = agents.work_building[i];
                work < allocator.buildings.len() && allocator.buildings[work].is_deserted
            })
            .collect();
        ejected_agents.sort_unstable();
        for i in ejected_agents {
            let work = agents.work_building[i];
            if work < allocator.buildings.len() && allocator.buildings[work].is_deserted {
                allocator.buildings[work].worker_count =
                    allocator.buildings[work].worker_count.saturating_sub(1);
                agents.assign_work_building(i, usize::MAX, 0);
            }
        }

        let job_index = JobCandidateIndex::build(allocator, &catalog);
        let mut plans: Vec<_> = (0..agents.len())
            .into_par_iter()
            .map_init(
                || {
                    (
                        Vec::with_capacity(JOB_SEARCH_CANDIDATES),
                        WorkplaceReachabilityCache::new(allocator),
                    )
                },
                |(candidates, reachability_cache), i| {
                    plan_agent_workplace(
                        i,
                        agents,
                        allocator,
                        transit_network,
                        graph,
                        &catalog,
                        &tuning,
                        target_days,
                        &self.households,
                        &job_index,
                        candidates,
                        reachability_cache,
                    )
                },
            )
            .filter_map(|plan| plan)
            .collect();
        plans.sort_unstable_by_key(|plan| plan.agent_idx);
        for plan in plans {
            apply_workplace_plan(plan, agents, allocator, &catalog);
        }
    }

    /// Pays wages into each employed agent's household budget.
    pub fn pay_daily_wages(&mut self, agents: &mut AgentSystem, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let mut plans: Vec<_> = (0..agents.len())
            .into_par_iter()
            .filter_map(|i| {
                let work = agents.work_building[i];
                let household_id = agents.household_id[i];
                if work >= allocator.buildings.len() || household_id >= self.households.len() {
                    return None;
                }
                let profile = economy_profile_for_building(&catalog, &allocator.buildings[work])?;
                let wage = profile.average_daily_wage();
                if wage <= 0.0 {
                    return None;
                }
                Some(WagePaymentPlan {
                    agent_idx: i,
                    work_building: work,
                    household_id,
                    wage,
                })
            })
            .collect();
        plans.sort_unstable_by_key(|plan| plan.agent_idx);

        for plan in plans {
            if plan.agent_idx >= agents.len()
                || agents.work_building[plan.agent_idx] != plan.work_building
                || agents.household_id[plan.agent_idx] != plan.household_id
                || plan.work_building >= allocator.buildings.len()
                || plan.household_id >= self.households.len()
            {
                continue;
            }
            if allocator.buildings[plan.work_building].operating_budget >= plan.wage {
                allocator.buildings[plan.work_building].operating_budget -= plan.wage;
                self.households[plan.household_id].budget += plan.wage;
                agents.consecutive_unpaid_days[plan.agent_idx] = 0;
            } else {
                agents.consecutive_unpaid_days[plan.agent_idx] =
                    agents.consecutive_unpaid_days[plan.agent_idx].saturating_add(1);

                if agents.consecutive_unpaid_days[plan.agent_idx] >= JOB_UNPAID_ABANDON_DAYS {
                    // Fire self from work.
                    agents.assign_work_building(plan.agent_idx, usize::MAX, 0);
                    debug_log!(
                        "economy",
                        "agent_idx={} fired self from insolvent building={} due to consecutive unpaid days",
                        plan.agent_idx,
                        plan.work_building
                    );
                }
            }
        }
        self.sync_agent_money_from_households(agents);
    }
}

struct JobAssignmentPlan {
    agent_idx: usize,
    old_job: usize,
    choice_count: u8,
    choices: [JobChoice; JOB_SEARCH_CANDIDATES],
    income_pressure: f32,
    stock_pressure: f32,
}

fn plan_agent_workplace(
    i: usize,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    target_days: f32,
    households: &[Household],
    job_index: &JobCandidateIndex,
    candidates: &mut Vec<usize>,
    reachability_cache: &mut WorkplaceReachabilityCache,
) -> Option<JobAssignmentPlan> {
    if agents.transit[i] != TRANSIT_IN_BUILDING {
        return None;
    }

    let home_idx = agents.home_building[i];
    if home_idx == usize::MAX || home_idx >= allocator.buildings.len() {
        return None;
    }

    let hid = agents.household_id[i];
    if hid == usize::MAX || hid >= households.len() {
        return None;
    }

    let household = &households[hid];
    let home = &allocator.buildings[home_idx];
    job_index.fill_nearby_candidates(
        home.center_x,
        home.center_y,
        JOB_SEARCH_MAX_RING,
        JOB_SEARCH_CANDIDATES,
        allocator,
        candidates,
    );
    let old_job = agents.work_building[i];
    if old_job != usize::MAX && !candidates.contains(&old_job) {
        candidates.push(old_job);
    }

    let income_pressure = household_income_pressure(catalog, tuning, household);
    let stock_pressure =
        (1.0 - (household.stock_days / target_days.max(0.1)).clamp(0.0, 1.0)).clamp(0.0, 1.0);

    let mut choices = [EMPTY_JOB_CHOICE; JOB_SEARCH_CANDIDATES];
    let mut choice_count = 0usize;
    let mut current_job_score = None;
    for &candidate in candidates.iter() {
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
        let Some(commute_seconds) = reachability_cache.commute_seconds(
            allocator,
            home_idx,
            candidate,
            agents.has_car[i],
            transit_network,
            graph,
            &agents.pathfind_count,
        ) else {
            continue;
        };

        let average_daily_wage = economy_profile.average_daily_wage();
        let worker_capacity = allocator.worker_capacity(candidate);
        let budget_capacity = if average_daily_wage > 0.1 {
            (building.operating_budget / average_daily_wage).floor() as u32
        } else {
            worker_capacity
        };
        let effective_capacity = worker_capacity.min(budget_capacity);

        if effective_capacity == 0 && old_job != candidate {
            continue;
        }

        let already_assigned = old_job == candidate;
        let reserved = building.worker_count;
        let open_slots = if already_assigned {
            worker_capacity.saturating_sub(reserved.saturating_sub(1))
        } else {
            effective_capacity.saturating_sub(reserved)
        };
        if open_slots == 0 {
            continue;
        }

        let commute_penalty = normalized_commute_penalty_seconds(commute_seconds);
        let score = W_INCOME * income_pressure + W_STOCK * stock_pressure + W_JOB * 1.0
            - W_COMMUTE * commute_penalty;
        if candidate == old_job {
            current_job_score = Some(score);
        } else if score >= GO_TO_WORK_THRESHOLD {
            insert_job_choice(
                &mut choices,
                &mut choice_count,
                JobChoice {
                    building_idx: candidate,
                    score,
                },
            );
        }
    }

    let can_switch = old_job == usize::MAX
        || agents.job_lock_days[i] == 0
        || agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS;
    if choice_count == 0 || !can_switch {
        return None;
    }
    if current_job_score.is_some_and(|score| score >= choices[0].score) {
        return None;
    }

    Some(JobAssignmentPlan {
        agent_idx: i,
        old_job,
        choice_count: choice_count as u8,
        choices,
        income_pressure,
        stock_pressure,
    })
}

fn insert_job_choice(
    choices: &mut [JobChoice; JOB_SEARCH_CANDIDATES],
    choice_count: &mut usize,
    choice: JobChoice,
) {
    let len = (*choice_count).min(JOB_SEARCH_CANDIDATES);
    let mut insert_at = 0;
    while insert_at < len && choices[insert_at].score >= choice.score {
        insert_at += 1;
    }
    if len == JOB_SEARCH_CANDIDATES && insert_at == len {
        return;
    }

    let new_len = if len < JOB_SEARCH_CANDIDATES {
        *choice_count = len + 1;
        len + 1
    } else {
        len
    };
    for idx in (insert_at + 1..new_len).rev() {
        choices[idx] = choices[idx - 1];
    }
    choices[insert_at] = choice;
}

fn apply_workplace_plan(
    plan: JobAssignmentPlan,
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) {
    if plan.agent_idx >= agents.len()
        || agents.transit[plan.agent_idx] != TRANSIT_IN_BUILDING
        || agents.work_building[plan.agent_idx] != plan.old_job
    {
        return;
    }

    let can_switch = plan.old_job == usize::MAX
        || agents.job_lock_days[plan.agent_idx] == 0
        || agents.consecutive_unpaid_days[plan.agent_idx] >= JOB_UNPAID_ABANDON_DAYS;
    if !can_switch {
        return;
    }

    for choice in plan
        .choices
        .iter()
        .take(usize::from(plan.choice_count))
        .copied()
    {
        let job = choice.building_idx;
        if job >= allocator.buildings.len() {
            continue;
        }
        let building = &allocator.buildings[job];
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

        let average_daily_wage = economy_profile.average_daily_wage();
        let worker_capacity = allocator.worker_capacity(job);
        let budget_capacity = if average_daily_wage > 0.1 {
            (building.operating_budget / average_daily_wage).floor() as u32
        } else {
            worker_capacity
        };
        let effective_capacity = worker_capacity.min(budget_capacity);
        if effective_capacity.saturating_sub(building.worker_count) == 0 {
            continue;
        }

        if plan.old_job < allocator.buildings.len() {
            allocator.buildings[plan.old_job].worker_count = allocator.buildings[plan.old_job]
                .worker_count
                .saturating_sub(1);
        }
        allocator.buildings[job].worker_count =
            allocator.buildings[job].worker_count.saturating_add(1);
        agents.assign_work_building(plan.agent_idx, job, JOB_LOCK_DAYS);
        debug_log!(
            "economy",
            "agent_idx={} accepted job building={} zone={:?} score={:.2} income_pressure={:.2} stock_pressure={:.2}",
            plan.agent_idx,
            job,
            allocator.buildings[job].zone_type,
            choice.score,
            plan.income_pressure,
            plan.stock_pressure
        );
        return;
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

fn normalized_commute_penalty_seconds(commute_seconds: u16) -> f32 {
    (commute_seconds as f32 / COMMUTE_PENALTY_MAX_SECONDS).clamp(0.0, 1.0)
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

struct JobCandidateIndex {
    by_chunk: BTreeMap<(i32, i32), Vec<usize>>,
}

impl JobCandidateIndex {
    fn build(allocator: &BuildingAllocator, catalog: &RuntimeEconomyCatalog) -> Self {
        let mut entries: Vec<_> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.is_deserted
                    || building.edge_idx == usize::MAX
                    || allocator.worker_capacity(idx) == 0
                {
                    return None;
                }
                let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
                if !building_offers_work(building, profile) {
                    return None;
                }
                let chunk = RegionGraph::get_chunk_coords(Vector3::new(
                    building.center_x,
                    0.0,
                    building.center_y,
                ));
                Some((chunk, idx))
            })
            .collect();
        entries.sort_unstable();

        let mut by_chunk = BTreeMap::new();
        for (chunk, idx) in entries {
            by_chunk.entry(chunk).or_insert_with(Vec::new).push(idx);
        }
        Self { by_chunk }
    }

    fn fill_nearby_candidates(
        &self,
        origin_x: f32,
        origin_y: f32,
        max_chunk_radius: i32,
        candidate_limit: usize,
        allocator: &BuildingAllocator,
        candidates: &mut Vec<usize>,
    ) {
        candidates.clear();
        if candidate_limit == 0 {
            return;
        }
        let origin_chunk = RegionGraph::get_chunk_coords(Vector3::new(origin_x, 0.0, origin_y));

        for ring in 0..=max_chunk_radius {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let chunk_key = (origin_chunk.0 + dx, origin_chunk.1 + dz);
                    let Some(indices) = self.by_chunk.get(&chunk_key) else {
                        continue;
                    };
                    candidates.extend(indices.iter().copied());
                }
            }
        }

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_building_distance(origin_x, origin_y, &allocator.buildings[a]);
            let db = squared_building_distance(origin_x, origin_y, &allocator.buildings[b]);
            da.total_cmp(&db).then_with(|| a.cmp(&b))
        });
        candidates.truncate(candidate_limit);
    }
}

fn squared_building_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct WorkplaceReachabilityKey {
    home_idx: usize,
    work_idx: usize,
    has_car: bool,
}

struct WorkplaceReachabilityCache {
    exact_entrance_cache_available: bool,
    cache: HashMap<WorkplaceReachabilityKey, Option<u16>>,
}

impl WorkplaceReachabilityCache {
    fn new(allocator: &BuildingAllocator) -> Self {
        Self {
            exact_entrance_cache_available: allocator.entrances.len() == allocator.buildings.len(),
            cache: HashMap::new(),
        }
    }

    fn commute_seconds(
        &mut self,
        allocator: &BuildingAllocator,
        home_idx: usize,
        work_idx: usize,
        has_car: bool,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &AtomicU32,
    ) -> Option<u16> {
        if home_idx == work_idx {
            return Some(1);
        }
        let key = WorkplaceReachabilityKey {
            home_idx,
            work_idx,
            has_car,
        };
        if let Some(result) = self.cache.get(&key) {
            return *result;
        }
        let result = if self.exact_entrance_cache_available {
            estimate_building_origin_trip_minutes(
                home_idx,
                work_idx,
                has_car,
                allocator,
                transit_network,
                graph,
                pathfind_count,
            )
        } else {
            fallback_attached_commute_seconds(allocator, home_idx, work_idx, has_car)
        };
        self.cache.insert(key, result);
        result
    }
}

fn fallback_attached_commute_seconds(
    allocator: &BuildingAllocator,
    home_idx: usize,
    work_idx: usize,
    has_car: bool,
) -> Option<u16> {
    let home = allocator.buildings.get(home_idx)?;
    let work = allocator.buildings.get(work_idx)?;
    if home.edge_idx == usize::MAX || work.edge_idx == usize::MAX {
        return None;
    }
    let dx = home.center_x - work.center_x;
    let dy = home.center_y - work.center_y;
    let speed_mps = if has_car { 13.0 } else { 1.4 };
    Some(
        ((dx * dx + dy * dy).sqrt() / speed_mps)
            .ceil()
            .clamp(1.0, u16::MAX as f32) as u16,
    )
}
