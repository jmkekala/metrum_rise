// SPDX-License-Identifier: GPL-2.0-only

//! Workplace assignment, worker counts, and daily wage payment.

use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicU32;

use super::building_economy::liquidate_outputs_until_budget;
use super::data::{Household, HouseholdSystem};
use super::metrics::{
    UTILITY_SERVICE_POWER, active_worker_capacity_for_profile,
    demand_sink_cash_cost_per_resident_excluding_resource, household_demand_profile,
    household_supply_resource_runtime_id, household_supply_unit_price,
    refresh_commercial_activity_floor, service_funded_worker_capacity,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, ReachableBucketScanEvent,
    chunk_for_point, lower_bound_travel_seconds, max_speed_for_modes,
};
use crate::simulation::economy::agents::tick::estimate_building_origin_trip_seconds;
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING, age_group_can_work};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::tax_amount;
use crate::simulation::economy::logistics::ShipmentSystem;
#[cfg(test)]
use crate::simulation::economy::logistics::has_connected_border_node;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::work_area::profile_kind_uses_explicit_work_area;
use crate::simulation::zoning::ZoneType;
use crate::{debug, debug_log};
use rayon::prelude::*;
use std::time::Instant;

const W_INCOME: f32 = 0.35;
const W_STOCK: f32 = 0.35;
const W_JOB: f32 = 0.20;
const W_COMMUTE: f32 = 0.10;
const GO_TO_WORK_THRESHOLD: f32 = 0.45;
const JOB_LOCK_DAYS: u8 = 7;
const JOB_UNPAID_ABANDON_DAYS: u8 = 2;
const JOB_SEARCH_CANDIDATES: usize = 24;
const JOB_ROUTE_SCAN_CANDIDATES: usize = JOB_SEARCH_CANDIDATES * 4;
const COMMUTE_PENALTY_MAX_SECONDS: f32 = 30.0 * 60.0;
const EMPTY_JOB_CHOICE: JobChoice = JobChoice {
    building_idx: usize::MAX,
    score: 0.0,
};
const EMPTY_HOME_JOB_OPTION: HomeJobOption = HomeJobOption {
    building_idx: usize::MAX,
    commute_penalty: 1.0,
    average_daily_wage: 0.0,
    effective_capacity: 0,
    open_slots: 0,
};
const EMPTY_WORKPLACE_ROUTE_ENTRY: WorkplaceRouteCacheEntry =
    ((usize::MAX, usize::MAX, false), None);

type WorkplaceRouteCacheKey = (usize, usize, bool);
type WorkplaceRouteCacheEntry = (WorkplaceRouteCacheKey, Option<u16>);
type CurrentJobOptionKey = (HomeJobOptionsKey, usize);

#[derive(Clone, Copy)]
struct JobChoice {
    building_idx: usize,
    score: f32,
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
struct HomeJobOptionsKey {
    home_idx: usize,
    has_car: bool,
}

#[derive(Clone, Copy)]
struct HomeJobOption {
    building_idx: usize,
    commute_penalty: f32,
    average_daily_wage: f32,
    effective_capacity: u32,
    open_slots: u32,
}

struct HomeJobOptions {
    option_count: u8,
    options: [HomeJobOption; JOB_SEARCH_CANDIDATES],
}

struct HomeJobOptionsBuild {
    key: HomeJobOptionsKey,
    options: HomeJobOptions,
    route_entry_count: u8,
    route_entries: [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
}

struct CurrentJobOptionBuild {
    key: CurrentJobOptionKey,
    option: Option<HomeJobOption>,
    route_entry_count: u8,
    route_entries: [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
}

struct JobSupplyEntry {
    building_idx: usize,
    open_slots: u32,
    average_daily_wage: f32,
    effective_capacity: u32,
    chunk: (i32, i32),
    foot_components: BuildingModeComponents,
    car_components: BuildingModeComponents,
}

struct JobSupplySnapshot {
    entries: Vec<JobSupplyEntry>,
    foot_buckets: ReachableBucketIndex,
    car_buckets: ReachableBucketIndex,
}

struct WagePaymentPlan {
    agent_idx: usize,
    work_building: usize,
    household_id: usize,
    wage: f32,
}

struct HomeJobBuildScratch {
    seen_entries: Vec<u32>,
    seen_epoch: u32,
}

impl HomeJobBuildScratch {
    fn new() -> Self {
        Self {
            seen_entries: Vec::new(),
            seen_epoch: 0,
        }
    }

    fn begin_query(&mut self, entry_count: usize) {
        if self.seen_entries.len() < entry_count {
            self.seen_entries.resize(entry_count, 0);
        }
        self.seen_epoch = self.seen_epoch.wrapping_add(1);
        if self.seen_epoch == 0 {
            self.seen_entries.fill(0);
            self.seen_epoch = 1;
        }
    }

    fn mark_seen(&mut self, entry_idx: usize) -> bool {
        if entry_idx >= self.seen_entries.len() {
            return false;
        }
        if self.seen_entries[entry_idx] == self.seen_epoch {
            return false;
        }
        self.seen_entries[entry_idx] = self.seen_epoch;
        true
    }
}

impl HouseholdSystem {
    #[cfg(test)]
    pub(super) fn assign_agent_workplaces(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.assign_agent_workplaces_with_service_funding(
            agents,
            allocator,
            transit_network,
            graph,
            &[],
            has_connected_border_node(graph),
        );
    }

    pub(crate) fn assign_agent_workplaces_with_service_funding(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        service_funding_by_building: &[f32],
        owa_exports_available: bool,
    ) {
        let timing_enabled = debug::category_enabled("economy");
        let total_start = Instant::now();
        let mut phase_start = total_start;
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let load_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        refresh_commercial_activity_floor(
            &catalog,
            &self.households,
            allocator,
            owa_exports_available,
        );
        let profile = household_demand_profile(&catalog);
        let target_days = profile.stock_target_days;

        eject_inactive_work_assignments(agents, allocator, &catalog);
        let prep_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();

        let (home_option_keys, current_job_keys) =
            collect_home_job_option_keys(agents, allocator.buildings.len(), self.households.len());
        let home_key_count = home_option_keys.len();
        let current_key_count = current_job_keys.len();
        let collect_keys_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if home_option_keys.is_empty() && current_job_keys.is_empty() {
            if timing_enabled {
                debug_log!(
                    "economy",
                    "workplace_assignment_detail skipped_reason=no_candidate_agents agents={} buildings={} households={} job_supply=0 home_keys=0 current_keys=0 home_options=0 current_options=0 route_cache_entries={} new_route_entries=0 plans=0 load_ms={:.3} prep_ms={:.3} route_cache_ms=0.000 component_ms=0.000 supply_ms=0.000 collect_keys_ms={:.3} speed_bounds_ms=0.000 option_build_ms=0.000 insert_routes_ms=0.000 plan_ms=0.000 apply_ms=0.000 total_ms={:.3}",
                    agents.len(),
                    allocator.buildings.len(),
                    self.households.len(),
                    self.workplace_route_cache.len(),
                    load_ms,
                    prep_ms,
                    collect_keys_ms,
                    total_start.elapsed().as_secs_f64() * 1000.0,
                );
            }
            return;
        }
        phase_start = Instant::now();

        let has_job_supply =
            has_potential_job_supply(allocator, &catalog, service_funding_by_building);
        let supply_prefilter_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if !has_job_supply {
            if timing_enabled {
                debug_log!(
                    "economy",
                    "workplace_assignment_detail skipped_reason=no_open_jobs agents={} buildings={} households={} job_supply=0 home_keys={} current_keys={} home_options=0 current_options=0 route_cache_entries={} new_route_entries=0 plans=0 load_ms={:.3} prep_ms={:.3} route_cache_ms=0.000 component_ms=0.000 supply_ms={:.3} collect_keys_ms={:.3} speed_bounds_ms=0.000 option_build_ms=0.000 insert_routes_ms=0.000 plan_ms=0.000 apply_ms=0.000 total_ms={:.3}",
                    agents.len(),
                    allocator.buildings.len(),
                    self.households.len(),
                    home_key_count,
                    current_key_count,
                    self.workplace_route_cache.len(),
                    load_ms,
                    prep_ms,
                    supply_prefilter_ms,
                    collect_keys_ms,
                    total_start.elapsed().as_secs_f64() * 1000.0,
                );
            }
            return;
        }
        phase_start = Instant::now();

        self.refresh_workplace_route_cache(allocator, transit_network);
        let route_cache_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let foot_components = ModeComponentIndex::build(graph, TransitFlags::FOOT);
        let car_components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        let component_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let job_supply = JobSupplySnapshot::build(
            allocator,
            graph,
            &catalog,
            &foot_components,
            &car_components,
            service_funding_by_building,
        );
        let job_supply_count = job_supply.entries.len();
        let supply_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if job_supply.entries.is_empty() {
            if timing_enabled {
                debug_log!(
                    "economy",
                    "workplace_assignment_detail skipped_reason=no_reachable_open_jobs agents={} buildings={} households={} job_supply=0 home_keys={} current_keys={} home_options=0 current_options=0 route_cache_entries={} new_route_entries=0 plans=0 load_ms={:.3} prep_ms={:.3} route_cache_ms={:.3} component_ms={:.3} supply_ms={:.3} collect_keys_ms={:.3} speed_bounds_ms=0.000 option_build_ms=0.000 insert_routes_ms=0.000 plan_ms=0.000 apply_ms=0.000 total_ms={:.3}",
                    agents.len(),
                    allocator.buildings.len(),
                    self.households.len(),
                    home_key_count,
                    current_key_count,
                    self.workplace_route_cache.len(),
                    load_ms,
                    prep_ms,
                    route_cache_ms,
                    component_ms,
                    supply_ms,
                    collect_keys_ms,
                    total_start.elapsed().as_secs_f64() * 1000.0,
                );
            }
            return;
        }
        phase_start = Instant::now();
        let max_walk_commute_speed = max_speed_for_modes(graph, TransitFlags::FOOT).max(1.0);
        let max_car_commute_speed =
            max_speed_for_modes(graph, TransitFlags::FOOT | TransitFlags::CAR).max(1.0);
        let speed_bounds_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let (home_job_options, current_job_options, new_route_entries) = build_home_job_options(
            &home_option_keys,
            &current_job_keys,
            &job_supply,
            &foot_components,
            &car_components,
            allocator,
            transit_network,
            graph,
            &catalog,
            &self.workplace_route_cache,
            &agents.pathfind_count,
            max_walk_commute_speed,
            max_car_commute_speed,
            service_funding_by_building,
        );
        let home_option_count = home_job_options.len();
        let current_option_count = current_job_options.len();
        let new_route_entry_count = new_route_entries.len();
        let option_build_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        for (key, result) in new_route_entries {
            self.workplace_route_cache.insert(key, result);
        }
        let insert_routes_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();

        let mut plans: Vec<_> = (0..agents.len())
            .into_par_iter()
            .filter_map(|i| {
                plan_agent_workplace(
                    i,
                    agents,
                    allocator.buildings.len(),
                    &catalog,
                    &tuning,
                    target_days,
                    &self.households,
                    &home_job_options,
                    &current_job_options,
                )
            })
            .collect();
        // Resolve resident workers first when their best available job is at home.
        plans.sort_unstable_by_key(|plan| {
            (
                plan.choices[0].building_idx != agents.home_building[plan.agent_idx],
                plan.agent_idx,
            )
        });
        let plan_count = plans.len();
        let plan_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        for plan in plans {
            apply_workplace_plan(
                plan,
                agents,
                allocator,
                &catalog,
                service_funding_by_building,
            );
        }
        let apply_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                "economy",
                "workplace_assignment_detail agents={} buildings={} households={} job_supply={} home_keys={} current_keys={} home_options={} current_options={} route_cache_entries={} new_route_entries={} plans={} load_ms={:.3} prep_ms={:.3} route_cache_ms={:.3} component_ms={:.3} supply_ms={:.3} collect_keys_ms={:.3} speed_bounds_ms={:.3} option_build_ms={:.3} insert_routes_ms={:.3} plan_ms={:.3} apply_ms={:.3} total_ms={:.3}",
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                job_supply_count,
                home_key_count,
                current_key_count,
                home_option_count,
                current_option_count,
                self.workplace_route_cache.len(),
                new_route_entry_count,
                plan_count,
                load_ms,
                prep_ms,
                route_cache_ms,
                component_ms,
                supply_ms,
                collect_keys_ms,
                speed_bounds_ms,
                option_build_ms,
                insert_routes_ms,
                plan_ms,
                apply_ms,
                total_start.elapsed().as_secs_f64() * 1000.0,
            );
        }
    }

    pub(crate) fn enforce_service_funding_staffing(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        service_funding_by_building: &[f32],
        owa_exports_available: bool,
    ) {
        if service_funding_by_building.is_empty() {
            return;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        refresh_commercial_activity_floor(
            &catalog,
            &self.households,
            allocator,
            owa_exports_available,
        );
        let funded_capacity_by_building: Vec<Option<u32>> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .map(|(building_idx, building)| {
                let profile = active_work_profile(&catalog, building)?;
                if profile.utility_service.as_deref() != Some(UTILITY_SERVICE_POWER) {
                    return None;
                }
                let capacity = active_worker_capacity_for_profile(&catalog, building, profile);
                Some(service_funded_worker_capacity(
                    capacity,
                    profile,
                    building_idx,
                    service_funding_by_building,
                ))
            })
            .collect();
        enforce_worker_capacities(agents, allocator, &funded_capacity_by_building);
    }

    /// Pays wages into each employed agent's household budget.
    #[cfg(test)]
    pub(super) fn pay_daily_wages(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        income_tax_rate: f32,
        treasury_balance: &mut f64,
    ) -> f32 {
        let empty_logistics = ShipmentSystem::new();
        let tax = self.pay_daily_wages_with_service_funding(
            agents,
            allocator,
            income_tax_rate,
            treasury_balance,
            &[],
            &empty_logistics,
            true,
        );
        self.sync_agent_money_from_households(agents);
        tax
    }

    pub(crate) fn pay_daily_wages_with_service_funding(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        income_tax_rate: f32,
        treasury_balance: &mut f64,
        service_funding_by_building: &[f32],
        logistics: &ShipmentSystem,
        owa_exports_available: bool,
    ) -> f32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let reserved_outbound = logistics.reserved_outbound_view(catalog.resource_count());
        refresh_commercial_activity_floor(
            &catalog,
            &self.households,
            allocator,
            owa_exports_available,
        );
        eject_inactive_work_assignments(agents, allocator, &catalog);
        shed_overstaffed_active_capacity_workers(agents, allocator, &catalog);
        self.last_city_service_wages = super::data::CityServiceWages::default();
        let city_funded_by_building: Vec<bool> = allocator
            .buildings
            .iter()
            .map(|building| allocator.is_city_service_building(building))
            .collect();
        let active_worker_capacity_by_building: Vec<u32> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .map(|(building_idx, building)| {
                active_work_profile(&catalog, building)
                    .map(|profile| {
                        let capacity =
                            active_worker_capacity_for_profile(&catalog, building, profile);
                        service_funded_worker_capacity(
                            capacity,
                            profile,
                            building_idx,
                            service_funding_by_building,
                        )
                    })
                    .unwrap_or(0)
            })
            .collect();
        let plans: Vec<_> = (0..agents.len())
            .into_par_iter()
            .filter_map(|i| {
                let work = agents.work_building[i];
                let household_id = agents.household_id[i];
                if !age_group_can_work(agents.age_group[i]) {
                    return None;
                }
                if work >= allocator.buildings.len() || household_id >= self.households.len() {
                    return None;
                }
                let profile = active_work_profile(&catalog, &allocator.buildings[work])?;
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
        self.ensure_daily_ledger_len();

        let mut income_tax_collected = 0.0;
        let mut paid_workers_by_building = vec![0u32; allocator.buildings.len()];
        for plan in plans {
            let within_active_capacity = paid_workers_by_building[plan.work_building]
                < active_worker_capacity_by_building[plan.work_building];
            let city_funded = city_funded_by_building[plan.work_building];
            if within_active_capacity
                && !city_funded
                && allocator.buildings[plan.work_building].operating_budget < plan.wage
            {
                liquidate_outputs_until_budget(
                    plan.work_building,
                    &mut allocator.buildings[plan.work_building],
                    &catalog,
                    &reserved_outbound,
                    catalog.resource_count(),
                    tuning.owa_distress_liquidation_multiplier,
                    plan.wage,
                );
            }
            if within_active_capacity
                && (city_funded
                    || allocator.buildings[plan.work_building].operating_budget >= plan.wage)
            {
                let income_tax = tax_amount(plan.wage, income_tax_rate);
                let net_wage = plan.wage - income_tax;
                if city_funded {
                    *treasury_balance -= plan.wage as f64;
                    self.last_city_service_wages.total += plan.wage;
                    if catalog
                        .profile_by_runtime_id(
                            allocator.buildings[plan.work_building].economy_profile_runtime_id,
                        )
                        .is_some_and(|profile| {
                            profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER)
                        })
                    {
                        self.last_city_service_wages.power += plan.wage;
                    }
                } else {
                    allocator.buildings[plan.work_building].operating_budget -= plan.wage;
                }
                self.households[plan.household_id].budget += net_wage;
                self.daily_ledgers[plan.household_id].wage_income += net_wage;
                income_tax_collected += income_tax;
                paid_workers_by_building[plan.work_building] =
                    paid_workers_by_building[plan.work_building].saturating_add(1);
                agents.consecutive_unpaid_days[plan.agent_idx] = 0;
            } else {
                agents.consecutive_unpaid_days[plan.agent_idx] =
                    agents.consecutive_unpaid_days[plan.agent_idx].saturating_add(1);

                if agents.consecutive_unpaid_days[plan.agent_idx] >= JOB_UNPAID_ABANDON_DAYS {
                    allocator.buildings[plan.work_building].worker_count = allocator.buildings
                        [plan.work_building]
                        .worker_count
                        .saturating_sub(1);
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
        income_tax_collected
    }

    fn refresh_workplace_route_cache(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
    ) {
        let building_revision = allocator.building_ref_revision();
        let entrance_revision = allocator.entrance_ref_revision();
        let cch_generation = transit_network.cch_graph.build_generation;
        if self.workplace_route_cache_building_revision != building_revision
            || self.workplace_route_cache_entrance_revision != entrance_revision
            || self.workplace_route_cache_cch_generation != cch_generation
        {
            self.workplace_route_cache.clear();
            self.workplace_route_cache_building_revision = building_revision;
            self.workplace_route_cache_entrance_revision = entrance_revision;
            self.workplace_route_cache_cch_generation = cch_generation;
        }
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
    building_count: usize,
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    target_days: f32,
    households: &[Household],
    home_job_options: &BTreeMap<HomeJobOptionsKey, HomeJobOptions>,
    current_job_options: &BTreeMap<CurrentJobOptionKey, HomeJobOption>,
) -> Option<JobAssignmentPlan> {
    if agents.transit[i] != TRANSIT_IN_BUILDING {
        return None;
    }
    if !age_group_can_work(agents.age_group[i]) {
        return None;
    }

    let home_idx = agents.home_building[i];
    if home_idx == usize::MAX || home_idx >= building_count {
        return None;
    }

    let hid = agents.household_id[i];
    if hid == usize::MAX || hid >= households.len() {
        return None;
    }

    let household = &households[hid];
    let old_job = agents.work_building[i];
    let can_switch = old_job == usize::MAX
        || old_job >= building_count
        || agents.job_lock_days[i] == 0
        || agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS;
    if !can_switch {
        return None;
    }

    let home_key = HomeJobOptionsKey {
        home_idx,
        has_car: agents.has_car[i],
    };
    let options = home_job_options.get(&home_key)?;

    let income_pressure = household_income_pressure(catalog, tuning, household);
    let stock_pressure =
        (1.0 - (household.stock_days / target_days.max(0.1)).clamp(0.0, 1.0)).clamp(0.0, 1.0);

    let mut choices = [EMPTY_JOB_CHOICE; JOB_SEARCH_CANDIDATES];
    let mut choice_count = 0usize;
    let mut current_job_score = if old_job < building_count
        && agents.consecutive_unpaid_days[i] < JOB_UNPAID_ABANDON_DAYS
    {
        current_job_options.get(&(home_key, old_job)).map(|option| {
            W_INCOME * income_pressure + W_STOCK * stock_pressure + W_JOB * 1.0
                - W_COMMUTE * option.commute_penalty
        })
    } else {
        None
    };
    for option in options
        .options
        .iter()
        .take(usize::from(options.option_count))
        .copied()
    {
        let candidate = option.building_idx;
        let score = W_INCOME * income_pressure + W_STOCK * stock_pressure + W_JOB * 1.0
            - W_COMMUTE * option.commute_penalty;
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

    if choice_count == 0 {
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
    service_funding_by_building: &[f32],
) {
    if plan.agent_idx >= agents.len()
        || agents.transit[plan.agent_idx] != TRANSIT_IN_BUILDING
        || !age_group_can_work(agents.age_group[plan.agent_idx])
        || agents.work_building[plan.agent_idx] != plan.old_job
    {
        return;
    }

    let can_switch = plan.old_job == usize::MAX
        || plan.old_job >= allocator.buildings.len()
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
        if building.is_deserted
            || building.broken
            || building.economy_broken
            || building.is_under_construction()
        {
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

        let effective_capacity = effective_job_capacity(
            allocator,
            catalog,
            job,
            economy_profile,
            service_funding_by_building,
        );
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

fn eject_inactive_work_assignments(
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) {
    let ejected_agents: Vec<_> = (0..agents.len())
        .into_par_iter()
        .filter(|&i| {
            let work = agents.work_building[i];
            if work >= allocator.buildings.len() {
                return false;
            }
            if !age_group_can_work(agents.age_group[i]) {
                return true;
            }
            active_work_profile(catalog, &allocator.buildings[work]).is_none()
        })
        .collect();
    for i in ejected_agents {
        let work = agents.work_building[i];
        allocator.buildings[work].worker_count =
            allocator.buildings[work].worker_count.saturating_sub(1);
        agents.assign_work_building(i, usize::MAX, 0);
    }
}

fn shed_overstaffed_active_capacity_workers(
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
) {
    let active_capacity_by_building: Vec<Option<u32>> = allocator
        .buildings
        .par_iter()
        .map(|building| {
            let profile = active_work_profile(catalog, building)?;
            if profile.kind == EconomyProfileRuntimeKind::ServiceStore
                || profile_kind_uses_explicit_work_area(profile.kind)
            {
                Some(active_worker_capacity_for_profile(
                    catalog, building, profile,
                ))
            } else {
                None
            }
        })
        .collect();
    enforce_worker_capacities(agents, allocator, &active_capacity_by_building);
}

fn enforce_worker_capacities(
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    active_capacity_by_building: &[Option<u32>],
) {
    if active_capacity_by_building
        .iter()
        .all(|capacity| capacity.is_none())
    {
        return;
    }

    let mut kept_workers_by_building = vec![0u32; allocator.buildings.len()];
    // Keep lower agent IDs first when workers compete for a limited capacity.
    for agent_idx in 0..agents.len() {
        let work = agents.work_building[agent_idx];
        let Some(active_capacity) = active_capacity_by_building.get(work).copied().flatten() else {
            continue;
        };
        if !age_group_can_work(agents.age_group[agent_idx]) {
            continue;
        }
        let kept_workers = &mut kept_workers_by_building[work];
        if *kept_workers < active_capacity {
            *kept_workers = kept_workers.saturating_add(1);
            continue;
        }
        agents.assign_work_building(agent_idx, usize::MAX, 0);
        agents.consecutive_unpaid_days[agent_idx] = 0;
    }

    // Publishing one count per building is cheaper than another Rayon dispatch after the
    // ordered retention pass (matched staffing benchmark); capacity evaluation stays parallel.
    for ((building, capacity), kept_workers) in allocator
        .buildings
        .iter_mut()
        .zip(active_capacity_by_building)
        .zip(kept_workers_by_building)
    {
        if capacity.is_some() {
            building.worker_count = kept_workers;
        }
    }
}

fn active_work_profile<'a>(
    catalog: &'a RuntimeEconomyCatalog,
    building: &Building,
) -> Option<&'a EconomyProfileRuntime> {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
    {
        return None;
    }
    let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
    if profile.worker_capacity == 0 || !building_offers_work(building, profile) {
        return None;
    }
    Some(profile)
}

fn household_income_pressure(
    catalog: &RuntimeEconomyCatalog,
    tuning: &RuntimeEconomyTuning,
    household: &Household,
) -> f32 {
    let profile = household_demand_profile(catalog);
    let target_days = profile.stock_target_days;

    let members = household.member_count.max(1) as f32;
    let household_supply_resource = household_supply_resource_runtime_id(catalog);
    let daily_supply_cost =
        members * household.consumption_rate.max(0.0) * household_supply_unit_price(catalog);
    let daily_service_cost = members
        * demand_sink_cash_cost_per_resident_excluding_resource(catalog, household_supply_resource);
    let daily_utility_cost = members * tuning.households.utility_cost_per_member_per_day;
    let reserve_target =
        (daily_supply_cost + daily_service_cost + daily_utility_cost) * target_days;
    (1.0 - (household.budget / reserve_target.max(1.0)).clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn normalized_commute_penalty_seconds(commute_seconds: f32) -> f32 {
    (commute_seconds / COMMUTE_PENALTY_MAX_SECONDS).clamp(0.0, 1.0)
}

fn building_offers_work(building: &Building, profile: &EconomyProfileRuntime) -> bool {
    matches!(
        building.zone_type,
        ZoneType::Commercial | ZoneType::Industrial
    ) || matches!(
        profile.kind,
        EconomyProfileRuntimeKind::FieldProducer
            | EconomyProfileRuntimeKind::Extractor
            | EconomyProfileRuntimeKind::UtilityProducer
            | EconomyProfileRuntimeKind::UtilityProcessor
    )
}

fn effective_job_capacity(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    building_idx: usize,
    profile: &EconomyProfileRuntime,
    service_funding_by_building: &[f32],
) -> u32 {
    let building = &allocator.buildings[building_idx];
    let active_capacity = active_worker_capacity_for_profile(catalog, building, profile);
    let funded_capacity = service_funded_worker_capacity(
        active_capacity,
        profile,
        building_idx,
        service_funding_by_building,
    );
    let average_daily_wage = profile.average_daily_wage();
    if allocator.is_city_service_building(building) || average_daily_wage <= 0.1 {
        funded_capacity
    } else {
        let budget_capacity =
            (building.operating_budget.max(0.0) / average_daily_wage).floor() as u32;
        funded_capacity.min(budget_capacity)
    }
}

fn has_potential_job_supply(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    service_funding_by_building: &[f32],
) -> bool {
    allocator
        .buildings
        .par_iter()
        .enumerate()
        .any(|(idx, building)| {
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.is_under_construction()
                || building.edge_idx == usize::MAX
            {
                return false;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                return false;
            };
            if !building_offers_work(building, profile) {
                return false;
            }
            effective_job_capacity(
                allocator,
                catalog,
                idx,
                profile,
                service_funding_by_building,
            )
            .saturating_sub(building.worker_count)
                > 0
        })
}

impl JobSupplySnapshot {
    fn build(
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
        service_funding_by_building: &[f32],
    ) -> Self {
        let entries: Vec<_> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if building.broken
                    || building.economy_broken
                    || building.is_deserted
                    || building.is_under_construction()
                    || building.edge_idx == usize::MAX
                {
                    return None;
                }
                let profile = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)?;
                if !building_offers_work(building, profile) {
                    return None;
                }
                let average_daily_wage = profile.average_daily_wage();
                let effective_capacity = effective_job_capacity(
                    allocator,
                    catalog,
                    idx,
                    profile,
                    service_funding_by_building,
                );
                let open_slots = effective_capacity.saturating_sub(building.worker_count);
                if open_slots == 0 {
                    return None;
                }
                let foot_components =
                    foot_components.building_components(allocator, graph, idx, TransitFlags::FOOT);
                let car_components =
                    car_components.building_components(allocator, graph, idx, TransitFlags::CAR);
                if foot_components.as_slice().is_empty()
                    && car_components.as_slice().is_empty()
                    && allocator.household_capacity(idx) == 0
                {
                    return None;
                }
                Some(JobSupplyEntry {
                    building_idx: idx,
                    open_slots,
                    average_daily_wage,
                    effective_capacity,
                    chunk: chunk_for_point(building.center_x, building.center_y),
                    foot_components,
                    car_components,
                })
            })
            .collect();

        let mut foot_bucket_entries = Vec::with_capacity(entries.len());
        let mut car_bucket_entries = Vec::with_capacity(entries.len());
        for (entry_idx, entry) in entries.iter().enumerate() {
            entry.foot_components.append_bucket_entries(
                &mut foot_bucket_entries,
                entry.chunk,
                entry_idx,
            );
            entry.car_components.append_bucket_entries(
                &mut car_bucket_entries,
                entry.chunk,
                entry_idx,
            );
        }

        Self {
            entries,
            foot_buckets: ReachableBucketIndex::from_entries(foot_bucket_entries),
            car_buckets: ReachableBucketIndex::from_entries(car_bucket_entries),
        }
    }
}

fn collect_home_job_option_keys(
    agents: &AgentSystem,
    building_count: usize,
    household_count: usize,
) -> (Vec<HomeJobOptionsKey>, Vec<CurrentJobOptionKey>) {
    let requests: Vec<_> = (0..agents.len())
        .into_par_iter()
        .filter_map(|i| {
            if agents.transit[i] != TRANSIT_IN_BUILDING {
                return None;
            }
            if !age_group_can_work(agents.age_group[i]) {
                return None;
            }
            let home_idx = agents.home_building[i];
            if home_idx >= building_count || agents.household_id[i] >= household_count {
                return None;
            }
            let old_job = agents.work_building[i];
            let can_switch = old_job == usize::MAX
                || old_job >= building_count
                || agents.job_lock_days[i] == 0
                || agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS;
            if !can_switch {
                return None;
            }
            let key = HomeJobOptionsKey {
                home_idx,
                has_car: agents.has_car[i],
            };
            let current_job_key = (old_job < building_count).then_some((key, old_job));
            Some((key, current_job_key))
        })
        .collect();
    let mut keys: Vec<_> = requests.iter().map(|(key, _)| *key).collect();
    keys.sort_unstable();
    keys.dedup();
    let mut current_job_keys: Vec<_> = requests
        .into_iter()
        .filter_map(|(_, current_job_key)| current_job_key)
        .collect();
    current_job_keys.sort_unstable();
    current_job_keys.dedup();
    (keys, current_job_keys)
}

#[allow(clippy::too_many_arguments)]
fn build_home_job_options(
    keys: &[HomeJobOptionsKey],
    current_job_keys: &[CurrentJobOptionKey],
    job_supply: &JobSupplySnapshot,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    catalog: &RuntimeEconomyCatalog,
    route_cache: &HashMap<WorkplaceRouteCacheKey, Option<u16>>,
    pathfind_count: &AtomicU32,
    max_walk_commute_speed: f32,
    max_car_commute_speed: f32,
    service_funding_by_building: &[f32],
) -> (
    BTreeMap<HomeJobOptionsKey, HomeJobOptions>,
    BTreeMap<CurrentJobOptionKey, HomeJobOption>,
    Vec<WorkplaceRouteCacheEntry>,
) {
    let exact_entrance_cache_available = allocator.entrances.len() == allocator.buildings.len();
    let builds: Vec<_> = keys
        .par_iter()
        .map_init(HomeJobBuildScratch::new, |scratch, &key| {
            let mut route_entries = [EMPTY_WORKPLACE_ROUTE_ENTRY; JOB_ROUTE_SCAN_CANDIDATES];
            let mut route_entry_count = 0usize;
            let options = build_home_job_options_for_key(
                key,
                job_supply,
                foot_components,
                car_components,
                allocator,
                transit_network,
                graph,
                route_cache,
                pathfind_count,
                exact_entrance_cache_available,
                if key.has_car {
                    max_car_commute_speed
                } else {
                    max_walk_commute_speed
                },
                scratch,
                &mut route_entries,
                &mut route_entry_count,
            );
            HomeJobOptionsBuild {
                key,
                options,
                route_entry_count: route_entry_count as u8,
                route_entries,
            }
        })
        .collect();

    let mut home_options = BTreeMap::new();
    let mut route_entries = Vec::new();
    for build in builds {
        if build.options.option_count > 0 {
            home_options.insert(build.key, build.options);
        }
        route_entries.extend(
            build
                .route_entries
                .iter()
                .take(usize::from(build.route_entry_count))
                .copied(),
        );
    }

    let current_builds: Vec<_> = current_job_keys
        .par_iter()
        .map(|&key| {
            let mut route_entries = [EMPTY_WORKPLACE_ROUTE_ENTRY; JOB_ROUTE_SCAN_CANDIDATES];
            let mut route_entry_count = 0usize;
            let option = build_current_job_option_for_key(
                key,
                allocator,
                transit_network,
                graph,
                catalog,
                route_cache,
                pathfind_count,
                exact_entrance_cache_available,
                &mut route_entries,
                &mut route_entry_count,
                service_funding_by_building,
            );
            CurrentJobOptionBuild {
                key,
                option,
                route_entry_count: route_entry_count as u8,
                route_entries,
            }
        })
        .collect();

    let mut current_job_options = BTreeMap::new();
    for build in current_builds {
        if let Some(option) = build.option {
            current_job_options.insert(build.key, option);
        }
        route_entries.extend(
            build
                .route_entries
                .iter()
                .take(usize::from(build.route_entry_count))
                .copied(),
        );
    }

    (home_options, current_job_options, route_entries)
}

#[allow(clippy::too_many_arguments)]
fn build_home_job_options_for_key(
    key: HomeJobOptionsKey,
    job_supply: &JobSupplySnapshot,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &HashMap<WorkplaceRouteCacheKey, Option<u16>>,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    max_commute_speed: f32,
    scratch: &mut HomeJobBuildScratch,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
) -> HomeJobOptions {
    let Some(home) = allocator.buildings.get(key.home_idx) else {
        return empty_home_job_options();
    };
    if home.broken || home.economy_broken || home.is_deserted || home.is_under_construction() {
        return empty_home_job_options();
    }

    let mut options = empty_home_job_options();
    let mut option_count = 0usize;
    scratch.begin_query(job_supply.entries.len());

    // The sorted supply snapshot also answers on-site jobs without a road-component query.
    if allocator.household_capacity(key.home_idx) > 0
        && let Ok(entry_idx) = job_supply
            .entries
            .binary_search_by_key(&key.home_idx, |entry| entry.building_idx)
    {
        let entry = &job_supply.entries[entry_idx];
        scratch.mark_seen(entry_idx);
        insert_home_job_option(
            &mut options.options,
            &mut option_count,
            HomeJobOption {
                building_idx: key.home_idx,
                commute_penalty: 0.0,
                average_daily_wage: entry.average_daily_wage,
                effective_capacity: entry.effective_capacity,
                open_slots: entry.open_slots,
            },
        );
    }

    let home_foot_components =
        foot_components.building_components(allocator, graph, key.home_idx, TransitFlags::FOOT);
    scan_home_job_bucket(
        &job_supply.foot_buckets,
        home_foot_components,
        key,
        job_supply,
        allocator,
        transit_network,
        graph,
        route_cache,
        pathfind_count,
        exact_entrance_cache_available,
        home.center_x,
        home.center_y,
        max_commute_speed,
        scratch,
        &mut options,
        &mut option_count,
        new_route_entries,
        new_route_entry_count,
    );

    if key.has_car {
        let home_car_components =
            car_components.building_components(allocator, graph, key.home_idx, TransitFlags::CAR);
        scan_home_job_bucket(
            &job_supply.car_buckets,
            home_car_components,
            key,
            job_supply,
            allocator,
            transit_network,
            graph,
            route_cache,
            pathfind_count,
            exact_entrance_cache_available,
            home.center_x,
            home.center_y,
            max_commute_speed,
            scratch,
            &mut options,
            &mut option_count,
            new_route_entries,
            new_route_entry_count,
        );
    }

    options.option_count = option_count as u8;
    options
}

#[allow(clippy::too_many_arguments)]
fn scan_home_job_bucket(
    bucket_index: &ReachableBucketIndex,
    components: BuildingModeComponents,
    key: HomeJobOptionsKey,
    job_supply: &JobSupplySnapshot,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &HashMap<WorkplaceRouteCacheKey, Option<u16>>,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    origin_x: f32,
    origin_y: f32,
    max_commute_speed: f32,
    scratch: &mut HomeJobBuildScratch,
    options: &mut HomeJobOptions,
    option_count: &mut usize,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
) {
    bucket_index.scan_nearest(components, origin_x, origin_y, |event| match event {
        ReachableBucketScanEvent::Item { item_idx } => {
            if !scratch.mark_seen(item_idx) {
                return true;
            }
            let Some(entry) = job_supply.entries.get(item_idx) else {
                return true;
            };
            let Some(commute_seconds) = cached_commute_seconds(
                key.home_idx,
                entry.building_idx,
                key.has_car,
                allocator,
                transit_network,
                graph,
                route_cache,
                pathfind_count,
                exact_entrance_cache_available,
                new_route_entries,
                new_route_entry_count,
            ) else {
                return true;
            };
            insert_home_job_option(
                &mut options.options,
                option_count,
                HomeJobOption {
                    building_idx: entry.building_idx,
                    commute_penalty: normalized_commute_penalty_seconds(commute_seconds as f32),
                    average_daily_wage: entry.average_daily_wage,
                    effective_capacity: entry.effective_capacity,
                    open_slots: entry.open_slots,
                },
            );
            true
        }
        ReachableBucketScanEvent::RingComplete {
            next_min_distance_sq,
        } => !home_job_search_can_stop(
            options,
            *option_count,
            next_min_distance_sq,
            max_commute_speed,
        ),
    });
}

fn home_job_search_can_stop(
    options: &HomeJobOptions,
    option_count: usize,
    next_min_distance_sq: f32,
    max_commute_speed: f32,
) -> bool {
    if option_count < JOB_SEARCH_CANDIDATES {
        return false;
    }
    let lower_bound = lower_bound_travel_seconds(next_min_distance_sq, max_commute_speed);
    // Equal capped penalties can still be beaten by wage, capacity, or building ID.
    normalized_commute_penalty_seconds(lower_bound)
        > options.options[option_count - 1].commute_penalty
}

fn empty_home_job_options() -> HomeJobOptions {
    HomeJobOptions {
        option_count: 0,
        options: [EMPTY_HOME_JOB_OPTION; JOB_SEARCH_CANDIDATES],
    }
}

#[allow(clippy::too_many_arguments)]
fn build_current_job_option_for_key(
    key: CurrentJobOptionKey,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    catalog: &RuntimeEconomyCatalog,
    route_cache: &HashMap<WorkplaceRouteCacheKey, Option<u16>>,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
    service_funding_by_building: &[f32],
) -> Option<HomeJobOption> {
    let (home_key, work_idx) = key;
    let work = allocator.buildings.get(work_idx)?;
    let profile = active_work_profile(catalog, work)?;
    let commute_seconds = cached_commute_seconds(
        home_key.home_idx,
        work_idx,
        home_key.has_car,
        allocator,
        transit_network,
        graph,
        route_cache,
        pathfind_count,
        exact_entrance_cache_available,
        new_route_entries,
        new_route_entry_count,
    )?;
    let average_daily_wage = profile.average_daily_wage();
    let effective_capacity = effective_job_capacity(
        allocator,
        catalog,
        work_idx,
        profile,
        service_funding_by_building,
    );
    Some(HomeJobOption {
        building_idx: work_idx,
        commute_penalty: normalized_commute_penalty_seconds(commute_seconds as f32),
        average_daily_wage,
        effective_capacity,
        open_slots: effective_capacity.saturating_sub(work.worker_count),
    })
}

fn insert_home_job_option(
    options: &mut [HomeJobOption; JOB_SEARCH_CANDIDATES],
    option_count: &mut usize,
    option: HomeJobOption,
) {
    let len = (*option_count).min(JOB_SEARCH_CANDIDATES);
    let mut insert_at = 0;
    while insert_at < len && home_job_option_precedes(options[insert_at], option) {
        insert_at += 1;
    }
    if len == JOB_SEARCH_CANDIDATES && insert_at == len {
        return;
    }

    let new_len = if len < JOB_SEARCH_CANDIDATES {
        *option_count = len + 1;
        len + 1
    } else {
        len
    };
    for idx in (insert_at + 1..new_len).rev() {
        options[idx] = options[idx - 1];
    }
    options[insert_at] = option;
}

fn home_job_option_precedes(left: HomeJobOption, right: HomeJobOption) -> bool {
    home_job_option_order(left, right) != CmpOrdering::Greater
}

fn home_job_option_order(left: HomeJobOption, right: HomeJobOption) -> CmpOrdering {
    left.commute_penalty
        .total_cmp(&right.commute_penalty)
        .then_with(|| right.average_daily_wage.total_cmp(&left.average_daily_wage))
        .then_with(|| right.effective_capacity.cmp(&left.effective_capacity))
        .then_with(|| right.open_slots.cmp(&left.open_slots))
        .then_with(|| left.building_idx.cmp(&right.building_idx))
}

#[allow(clippy::too_many_arguments)]
fn cached_commute_seconds(
    home_idx: usize,
    work_idx: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &HashMap<WorkplaceRouteCacheKey, Option<u16>>,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
) -> Option<u16> {
    if home_idx == work_idx {
        return Some(0);
    }
    let key = (home_idx, work_idx, has_car);
    if let Some(result) = route_cache.get(&key) {
        return *result;
    }
    if !exact_entrance_cache_available {
        return None;
    }
    let result = estimate_building_origin_trip_seconds(
        home_idx,
        work_idx,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    );
    if *new_route_entry_count < JOB_ROUTE_SCAN_CANDIDATES {
        new_route_entries[*new_route_entry_count] = (key, result);
        *new_route_entry_count += 1;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_search_keeps_farther_candidates_that_can_win_a_capped_commute_tie() {
        let mut options = empty_home_job_options();
        let mut count = 0;
        for building_idx in 0..JOB_SEARCH_CANDIDATES {
            insert_home_job_option(
                &mut options.options,
                &mut count,
                HomeJobOption {
                    building_idx,
                    commute_penalty: 1.0,
                    average_daily_wage: 100.0,
                    effective_capacity: 4,
                    open_slots: 4,
                },
            );
        }
        let farther = HomeJobOption {
            building_idx: 100,
            commute_penalty: 1.0,
            average_daily_wage: 200.0,
            effective_capacity: 4,
            open_slots: 4,
        };
        assert!(home_job_option_order(farther, options.options[count - 1]).is_lt());
        assert!(!home_job_search_can_stop(
            &options,
            count,
            2_500.0_f32.powi(2),
            1.0
        ));
    }
}
