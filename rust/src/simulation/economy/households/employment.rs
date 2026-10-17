// SPDX-License-Identifier: GPL-2.0-only

//! Workplace assignment, worker counts, and daily wage payment.

use std::cmp::Ordering as CmpOrdering;
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::AtomicU32;

use super::building_economy::liquidate_outputs_until_budget;
use super::candidate_visits::CandidateVisits;
use super::data::{Household, HouseholdSystem};
use super::metrics::{
    UTILITY_SERVICE_POWER, demand_sink_cash_cost_per_resident_excluding_resource,
    household_demand_profile, household_supply_resource_runtime_id, household_supply_unit_price,
    physical_worker_capacity_for_profile, service_funded_worker_capacity,
};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, ReachableBucketScanEvent,
    chunk_for_point, lower_bound_travel_seconds, max_speed_for_modes,
};
use crate::simulation::economy::agents::tick::{
    BuildingTripEnds, SharedNetworkQuotes, TripEstimate, TripQueryScratch,
    estimate_building_origin_trip_seconds_with_scratch, estimate_trip_seconds_with_scratch,
};
use crate::simulation::economy::agents::{AgentSystem, TRANSIT_IN_BUILDING, age_group_can_work};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{CityFiscalPolicy, tax_amount};
use crate::simulation::economy::logistics::ShipmentSystem;
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
// A new job, or a search that keeps the current one, locks the worker for 5–9 days (mean 7),
// spread per agent so workers hired together do not all reconsider on the same day.
const JOB_LOCK_MIN_DAYS: u8 = 5;
const JOB_LOCK_MAX_DAYS: u8 = 9;
const JOB_UNPAID_ABANDON_DAYS: u8 = 2;
/// Workplace passes a complete commute quote stays valid: one per operational hour, a day.
pub(crate) const WORKPLACE_QUOTE_PASSES: u32 = 24;
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

pub(super) type WorkplaceRouteCacheKey = (usize, usize, bool);
type WorkplaceRouteCacheEntry = (WorkplaceRouteCacheKey, Option<u16>);
/// Exact home/workplace/car quotations. Consumers only perform keyed lookups, so the
/// existing fixed fast hasher changes neither choice order nor daily price invalidation.
pub(super) type WorkplaceRouteCache =
    HashMap<WorkplaceRouteCacheKey, Option<u16>, foldhash::fast::FixedState>;
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

/// Stable workplace-query ownership with reusable result buffers, independent of worker scheduling.
#[derive(Default)]
pub(super) struct WorkplaceQueryShard {
    query: TripQueryScratch,
    home: Vec<HomeJobOptionsBuild>,
    current: Vec<CurrentJobOptionBuild>,
}

struct JobSupplyEntry {
    building_idx: usize,
    open_slots: u32,
    average_daily_wage: f32,
    effective_capacity: u32,
    chunk: (i32, i32),
    foot_components: BuildingModeComponents,
    car_components: BuildingModeComponents,
    // Arrival ends shared by every home's commute estimate to this workplace.
    trip_ends: BuildingTripEnds,
}

struct JobSupplySnapshot {
    entries: Vec<JobSupplyEntry>,
    foot_buckets: ReachableBucketIndex,
    car_buckets: ReachableBucketIndex,
    // Farthest network node any workplace's trips use, measured from its bucketed centre.
    node_slack_m: f32,
}

/// One day's wage paid at a worker's daily phase (`SIM-02-G`).
#[derive(Clone, Copy, Debug)]
pub(crate) struct WagePayment {
    /// The employer.
    pub(crate) building: usize,
    /// Gross wage, before income tax.
    pub(crate) gross: f32,
    /// Income tax withheld for the treasury.
    pub(crate) income_tax: f32,
    /// Whether the treasury paid it.
    pub(crate) city_funded: bool,
}

// Ends `agent_idx`'s assignment to `work` and updates the workplace's count. O(1).
fn release_worker(
    households: &mut [Household],
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    agent_idx: usize,
    work: usize,
) {
    allocator.set_worker_count(
        work,
        allocator.buildings[work].worker_count.saturating_sub(1),
    );
    reassign_worker(households, agents, agent_idx, usize::MAX, 0);
}

// Assigns `agent_idx` to `work_building` (`usize::MAX` ends its job) and keeps its household's
// employed adults with it, as callers keep the workplaces' counts: demand and the census read
// the count between the household's hourly phases, whose recount also covers assignments that
// bypass this (`SIM-02-I`). O(1).
fn reassign_worker(
    households: &mut [Household],
    agents: &mut AgentSystem,
    agent_idx: usize,
    work_building: usize,
    job_lock_days: u8,
) {
    let employed = |agents: &AgentSystem| {
        age_group_can_work(agents.age_group[agent_idx])
            && agents.work_building[agent_idx] != usize::MAX
    };
    let was_employed = employed(agents);
    agents.assign_work_building(agent_idx, work_building, job_lock_days);
    let is_employed = employed(agents);
    if was_employed != is_employed
        && let Some(household) = households.get_mut(agents.household_id[agent_idx])
    {
        household.employed_adults = if is_employed {
            household.employed_adults.saturating_add(1)
        } else {
            household.employed_adults.saturating_sub(1)
        };
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
            &mut ShipmentSystem::new(),
            &CityFiscalPolicy::from_runtime_tuning(&load_runtime_economy_tuning().unwrap()),
            0.0,
            420,
        );
    }

    /// Funding census, then job search against it.
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn assign_agent_workplaces_with_service_funding(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        service_funding_by_building: &[f32],
        logistics: &mut ShipmentSystem,
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        self.refresh_funding_census(
            allocator,
            logistics,
            transit_network,
            graph,
            service_funding_by_building,
            fiscal_policy,
            treasury_balance,
            minute_of_day,
        );
        self.assign_agent_workplaces_from_census(
            agents,
            allocator,
            transit_network,
            graph,
            minute_of_day,
        );
    }

    /// Recomputes the funding census (`ECON-17`) from the current state. The hour runs it first,
    /// so every reader that hour sees a census of the state the previous hour's phases left, and a
    /// census rebuilt on load never feeds the simulation (`SIM-02-F`).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn refresh_funding_census(
        &mut self,
        allocator: &mut BuildingAllocator,
        logistics: &mut ShipmentSystem,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        service_funding_by_building: &[f32],
        fiscal_policy: &CityFiscalPolicy,
        treasury_balance: f64,
        minute_of_day: u16,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        allocator.prepare_production_changes();
        crate::debug::work::add(crate::debug::work::Work::LiveCensuses, 1);
        self.job_funding.refresh(
            allocator,
            &self.households,
            logistics,
            transit_network,
            graph,
            &catalog,
            &tuning,
            fiscal_policy,
            service_funding_by_building,
            treasury_balance,
            minute_of_day,
        );
        self.publish_funded_job_capacities(allocator);
    }

    /// Job search against the latest funding census.
    pub(crate) fn assign_agent_workplaces_from_census(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        self.expire_workplace_route_cache();
        let timing_category = if debug::Channel::EconomyWorkplace.enabled() {
            debug::Channel::EconomyWorkplace
        } else if debug::Channel::EconomyPhases.enabled() {
            debug::Channel::EconomyPhases
        } else {
            debug::Channel::Economy
        };
        let timing_enabled = timing_category.enabled();
        let queries_before = if timing_enabled {
            agents
                .pathfind_count
                .load(std::sync::atomic::Ordering::Relaxed)
        } else {
            0
        };
        let total_start = Instant::now();
        let mut phase_start = total_start;
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let load_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let profile = household_demand_profile(&catalog);
        let target_days = profile.stock_target_days;

        eject_inactive_work_assignments(&mut self.households, agents, allocator, &catalog);
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
                    timing_category,
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
            has_potential_job_supply(allocator, &catalog, &self.job_funding.capacities);
        let supply_prefilter_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if !has_job_supply {
            if timing_enabled {
                debug_log!(
                    timing_category,
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
        self.workplace_network_quotes.sync(transit_network);
        let route_cache_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let foot_components = ModeComponentIndex::build(graph, TransitFlags::FOOT);
        let car_components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        let component_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        let job_supply = JobSupplySnapshot::build(
            allocator,
            transit_network,
            graph,
            &catalog,
            &foot_components,
            &car_components,
            &self.job_funding.capacities,
        );
        let job_supply_count = job_supply.entries.len();
        let supply_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if job_supply.entries.is_empty() {
            if timing_enabled {
                debug_log!(
                    timing_category,
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
        let targets = std::array::from_fn(|mode| {
            job_supply
                .entries
                .par_iter()
                .flat_map_iter(|entry| {
                    entry
                        .trip_ends
                        .network_targets(mode as u8)
                        .into_iter()
                        .flatten()
                })
                .collect()
        });
        let target_searches = self
            .workplace_network_quotes
            .prepare_targets(targets, transit_network);
        agents
            .pathfind_count
            .fetch_add(target_searches as u32, std::sync::atomic::Ordering::Relaxed);
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
            &mut self.workplace_network_quotes,
            &mut self.workplace_query_shards,
            &agents.pathfind_count,
            max_walk_commute_speed,
            max_car_commute_speed,
            &self.job_funding.capacities,
        );
        let home_option_count = home_job_options.len();
        let current_option_count = current_job_options.len();
        let new_route_entry_count = new_route_entries.len();
        let option_build_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        // Reserve the completed batch before ordered insertion; no per-quote growth.
        self.workplace_route_cache.reserve(new_route_entry_count);
        for (key, result) in new_route_entries {
            if self.workplace_route_cache.insert(key, result).is_none() {
                self.workplace_route_cache_order
                    .push_back((self.workplace_route_cache_pass, key));
            }
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
        let mut planned_agents: Vec<usize> = plans.iter().map(|plan| plan.agent_idx).collect();
        planned_agents.sort_unstable();
        let plan_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        phase_start = Instant::now();
        for plan in plans {
            apply_workplace_plan(
                plan,
                &mut self.households,
                agents,
                allocator,
                &catalog,
                &self.job_funding.capacities,
            );
        }
        relock_settled_workers(
            agents,
            allocator.buildings.len(),
            self.households.len(),
            &planned_agents,
        );
        let apply_ms = phase_start.elapsed().as_secs_f64() * 1000.0;
        if timing_enabled {
            debug_log!(
                timing_category,
                "workplace_query_work minute={} cch_searches={} target_searches={}",
                minute_of_day,
                agents
                    .pathfind_count
                    .load(std::sync::atomic::Ordering::Relaxed)
                    .wrapping_sub(queries_before),
                target_searches
            );

            debug_log!(
                timing_category,
                "workplace_assignment_detail agents={} buildings={} households={} job_supply={} home_keys={} current_keys={} home_options={} current_options={} route_cache_entries={} network_quote_entries={} new_route_entries={} plans={} load_ms={:.3} prep_ms={:.3} route_cache_ms={:.3} component_ms={:.3} supply_ms={:.3} collect_keys_ms={:.3} speed_bounds_ms={:.3} option_build_ms={:.3} insert_routes_ms={:.3} plan_ms={:.3} apply_ms={:.3} total_ms={:.3}",
                agents.len(),
                allocator.buildings.len(),
                self.households.len(),
                job_supply_count,
                home_key_count,
                current_key_count,
                home_option_count,
                current_option_count,
                self.workplace_route_cache.len(),
                self.workplace_network_quotes.len(),
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
    ) {
        if service_funding_by_building.is_empty() {
            return;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        // The activity floor stays as the hour pass set it: production at each building's phase
        // reads it until the next hour, and a load restores it.
        let funded_capacity_by_building: Vec<Option<u32>> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .map(|(building_idx, building)| {
                let profile = active_work_profile(&catalog, building)?;
                if profile.utility_service.as_deref() != Some(UTILITY_SERVICE_POWER) {
                    return None;
                }
                // The same fiscal ceiling the funding census advertises to hiring (`ECON-17`).
                let capacity = physical_worker_capacity_for_profile(building, profile);
                Some(service_funded_worker_capacity(
                    capacity,
                    profile,
                    building_idx,
                    service_funding_by_building,
                ))
            })
            .collect();
        enforce_worker_capacities(
            &mut self.households,
            agents,
            allocator,
            &funded_capacity_by_building,
        );
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
        self.pay_funded_daily_wages(
            agents,
            allocator,
            income_tax_rate,
            treasury_balance,
            &empty_logistics,
        )
    }

    /// Every worker's daily wage at once, in agent order, as their daily phases would pay it
    /// within one fresh window of each workplace.
    #[cfg(test)]
    pub(crate) fn pay_funded_daily_wages(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        income_tax_rate: f32,
        treasury_balance: &mut f64,
        _logistics: &ShipmentSystem,
    ) -> f32 {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.publish_funded_job_capacities(allocator);
        for building in &mut allocator.buildings {
            building.paid_workers_in_window = 0;
        }
        self.ensure_daily_ledger_len();
        self.day_totals.city_service_wages = Default::default();
        let mut income_tax = 0.0;
        for agent_idx in 0..agents.len() {
            if let Some(payment) = self.pay_agent_daily_wage(
                agent_idx,
                agents,
                allocator,
                &catalog,
                tuning.owa_distress_liquidation_multiplier,
                income_tax_rate,
                treasury_balance,
            ) {
                income_tax += payment.income_tax;
            }
        }
        self.last_day_totals.city_service_wages = self.day_totals.city_service_wages;
        income_tax
    }

    /// Copies the latest census's funded jobs and activity onto the buildings, where wages at
    /// each worker's daily phase and the activity floor at each building's hourly phase read
    /// them until the next census (`SIM-02-G`, `SIM-02-I`). O(buildings).
    pub(super) fn publish_funded_job_capacities(&self, allocator: &mut BuildingAllocator) {
        let capacities = &self.job_funding.capacities;
        let activities = self.job_funding.activities();
        allocator
            .buildings
            .par_iter_mut()
            .enumerate()
            .for_each(|(building_idx, building)| {
                building.funded_job_capacity = capacities.get(building_idx).copied().unwrap_or(0);
                building.funded_activity = activities.get(building_idx).copied().unwrap_or(0.0);
            });
    }

    /// Settles agent `agent_idx`'s workday at its daily phase (`SIM-02-G`, `ECON-17`). An
    /// assignment to a workplace that no longer operates, or of an agent who cannot work, ends.
    /// A store or work area above its funded staff releases a worker once its funded jobs have
    /// been paid in the window. Otherwise the worker is
    /// paid one day's wage while the workplace has paid fewer than its funded jobs in its own
    /// 24-hour window: a short private employer first sells output at distress price, and a
    /// worker left unpaid on `JOB_UNPAID_ABANDON_DAYS` days in a row quits. Wages never push a
    /// private budget negative. Returns the wage paid. O(output ports).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pay_agent_daily_wage(
        &mut self,
        agent_idx: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        liquidation_multiplier: f32,
        income_tax_rate: f32,
        treasury_balance: &mut f64,
    ) -> Option<WagePayment> {
        let work = agents.work_building[agent_idx];
        if work >= allocator.buildings.len() {
            return None;
        }
        let profile = if age_group_can_work(agents.age_group[agent_idx]) {
            active_work_profile(catalog, &allocator.buildings[work])
        } else {
            None
        };
        let Some(profile) = profile else {
            release_worker(&mut self.households, agents, allocator, agent_idx, work);
            return None;
        };
        let building = &allocator.buildings[work];
        let capacity = building
            .funded_job_capacity
            .min(physical_worker_capacity_for_profile(building, profile));
        // Like the midnight shedding, the first workers keep the funded jobs: once the window
        // has paid them, an overstaffed store or work area releases the rest.
        if (profile.kind == EconomyProfileRuntimeKind::ServiceStore
            || profile_kind_uses_explicit_work_area(profile.kind))
            && building.worker_count > capacity
            && building.paid_workers_in_window >= capacity
        {
            release_worker(&mut self.households, agents, allocator, agent_idx, work);
            agents.consecutive_unpaid_days[agent_idx] = 0;
            return None;
        }
        let household_id = agents.household_id[agent_idx];
        let wage = profile.average_daily_wage();
        if household_id >= self.households.len() || wage <= 0.0 {
            return None;
        }
        self.ensure_daily_ledger_len();
        let power_plant = profile.utility_service.as_deref() == Some(UTILITY_SERVICE_POWER);
        let city_funded = allocator.is_city_service_building(building);
        let within_paid_capacity = building.paid_workers_in_window < capacity;
        let building = &mut allocator.buildings[work];
        if within_paid_capacity && !city_funded && building.operating_budget < wage {
            // Outside the logistics hour no shipment holds unloaded source stock.
            liquidate_outputs_until_budget(
                work,
                building,
                catalog,
                &[],
                catalog.resource_count(),
                liquidation_multiplier,
                wage,
            );
        }
        if !within_paid_capacity || (!city_funded && building.operating_budget < wage) {
            if within_paid_capacity {
                building.unpaid_cash_wages_in_window =
                    building.unpaid_cash_wages_in_window.saturating_add(1);
            }
            let unpaid = &mut agents.consecutive_unpaid_days[agent_idx];
            *unpaid = unpaid.saturating_add(1);
            if *unpaid >= JOB_UNPAID_ABANDON_DAYS {
                release_worker(&mut self.households, agents, allocator, agent_idx, work);
                debug_log!(
                    "economy",
                    "agent_idx={} fired self from insolvent building={} due to consecutive unpaid days",
                    agent_idx,
                    work
                );
            }
            return None;
        }
        let income_tax = tax_amount(wage, income_tax_rate);
        let net_wage = wage - income_tax;
        if city_funded {
            *treasury_balance -= f64::from(wage);
            self.day_totals.city_service_wages.total += wage;
            if power_plant {
                self.day_totals.city_service_wages.power += wage;
            }
        } else {
            building.operating_budget -= wage;
            self.day_totals.private_payroll += f64::from(wage);
        }
        building.paid_workers_in_window = building.paid_workers_in_window.saturating_add(1);
        self.households[household_id].budget += net_wage;
        let ledger = &mut self.daily_ledgers[household_id];
        ledger.wage_income += net_wage;
        ledger.paid_workers = ledger.paid_workers.saturating_add(1);
        agents.consecutive_unpaid_days[agent_idx] = 0;
        Some(WagePayment {
            building: work,
            gross: wage,
            income_tax,
            city_funded,
        })
    }

    // Job searches repeat for a worker only after a multi-day lock, so a commute priced a day
    // ago is rarely asked again. Expiring each quote a day after it was added bounds the cache
    // to one day's searches (it grew to 7.4M entries at 30k by day 15 without a limit) and
    // keeps the daily pricing cadence without a midnight clear (`SIM-02-G`). O(expired).
    pub(super) fn expire_workplace_route_cache(&mut self) {
        self.workplace_route_cache_pass = self.workplace_route_cache_pass.wrapping_add(1);
        while let Some(&(pass, key)) = self.workplace_route_cache_order.front() {
            if self.workplace_route_cache_pass.wrapping_sub(pass) < WORKPLACE_QUOTE_PASSES {
                break;
            }
            self.workplace_route_cache_order.pop_front();
            self.workplace_route_cache.remove(&key);
        }
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
            self.workplace_route_cache_order.clear();
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

#[allow(clippy::too_many_arguments)]
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
        || agents.job_lock_until_day[i] <= agents.settlement_days
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
        } else if old_job >= building_count || score >= GO_TO_WORK_THRESHOLD {
            // Demand counts unemployed adults as the first available labor supply.
            // Savings must not make those adults refuse every funded, reachable offer;
            // income/stock pressure still controls voluntary switches from an existing job.
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

// Lock length for `agent_idx` at `work_building`: a deterministic spread over
// `JOB_LOCK_MIN_DAYS..=JOB_LOCK_MAX_DAYS`, not a random draw.
fn job_lock_days(agent_idx: usize, work_building: usize) -> u8 {
    let mut hash = (agent_idx as u64) ^ (work_building as u64).rotate_left(32);
    hash = (hash ^ (hash >> 33)).wrapping_mul(0xff51_afd7_ed55_8ccd);
    hash ^= hash >> 33;
    let span = u64::from(JOB_LOCK_MAX_DAYS - JOB_LOCK_MIN_DAYS + 1);
    JOB_LOCK_MIN_DAYS + (hash % span) as u8
}

// Employed workers who were searched this pass and had no better job keep it under a new lock,
// so they do not repeat the same search every hour. `planned_agents` (sorted) found a better
// job; if it was full they stay unlocked and retry next hour. Workers leaving an unpaid job are
// never locked. Same eligibility as `collect_home_job_option_keys`. O(agents), parallel.
fn relock_settled_workers(
    agents: &mut AgentSystem,
    building_count: usize,
    household_count: usize,
    planned_agents: &[usize],
) {
    let today = agents.settlement_days;
    let soa = &mut agents.agents;
    let (transit, age_group, home, household, work, unpaid) = (
        &soa.transit,
        &soa.age_group,
        &soa.home_building,
        &soa.household_id,
        &soa.work_building,
        &soa.consecutive_unpaid_days,
    );
    soa.job_lock_until_day
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, lock_until)| {
            if *lock_until <= today
                && work[i] < building_count
                && transit[i] == TRANSIT_IN_BUILDING
                && age_group_can_work(age_group[i])
                && home[i] < building_count
                && household[i] < household_count
                && unpaid[i] < JOB_UNPAID_ABANDON_DAYS
                && planned_agents.binary_search(&i).is_err()
            {
                *lock_until = today + u32::from(job_lock_days(i, work[i]));
            }
        });
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
    households: &mut [Household],
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    job_capacities: &[u32],
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
        || agents.job_lock_until_day[plan.agent_idx] <= agents.settlement_days
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

        let effective_capacity = job_capacities.get(job).copied().unwrap_or(0);
        if effective_capacity.saturating_sub(building.worker_count) == 0 {
            continue;
        }

        if plan.old_job < allocator.buildings.len() {
            allocator.set_worker_count(
                plan.old_job,
                allocator.buildings[plan.old_job].worker_count.saturating_sub(1),
            );
        }
        allocator.set_worker_count(job, allocator.buildings[job].worker_count.saturating_add(1));
        let lock_days = job_lock_days(plan.agent_idx, job);
        reassign_worker(households, agents, plan.agent_idx, job, lock_days);
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
    households: &mut [Household],
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
        allocator.set_worker_count(work, allocator.buildings[work].worker_count.saturating_sub(1));
        reassign_worker(households, agents, i, usize::MAX, 0);
    }
}

fn enforce_worker_capacities(
    households: &mut [Household],
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
        reassign_worker(households, agents, agent_idx, usize::MAX, 0);
        agents.consecutive_unpaid_days[agent_idx] = 0;
    }

    // Publishing one count per building is cheaper than another Rayon dispatch after the
    // ordered retention pass (matched staffing benchmark); capacity evaluation stays parallel.
    for (index, (capacity, kept_workers)) in active_capacity_by_building
        .iter()
        .zip(kept_workers_by_building)
        .enumerate()
    {
        if capacity.is_some() {
            allocator.set_worker_count(index, kept_workers);
        }
    }
}

/// Returns the recipe of a live workplace eligible to pay its assigned employees.
pub(super) fn active_work_profile<'a>(
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
    let daily_utility_cost =
        super::metrics::household_daily_utility_allowance(catalog, tuning, members);
    let reserve_target =
        (daily_supply_cost + daily_service_cost + daily_utility_cost) * target_days;
    (1.0 - ((household.budget - household.power_arrears.total()) / reserve_target.max(1.0))
        .clamp(0.0, 1.0))
    .clamp(0.0, 1.0)
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

fn has_potential_job_supply(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    job_capacities: &[u32],
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
            job_capacities
                .get(idx)
                .copied()
                .unwrap_or(0)
                .saturating_sub(building.worker_count)
                > 0
        })
}

impl JobSupplySnapshot {
    fn build(
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        catalog: &RuntimeEconomyCatalog,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
        job_capacities: &[u32],
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
                let effective_capacity = job_capacities.get(idx).copied().unwrap_or(0);
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
                    trip_ends: BuildingTripEnds::destination(
                        idx,
                        allocator,
                        transit_network,
                        graph,
                    ),
                })
            })
            .collect();
        let node_slack_m = entries
            .iter()
            .map(|entry| {
                let building = &allocator.buildings[entry.building_idx];
                entry
                    .trip_ends
                    .max_node_distance(building.center_x, building.center_y)
            })
            .fold(0.0, f32::max);

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
            node_slack_m,
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
                || agents.job_lock_until_day[i] <= agents.settlement_days
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
    route_cache: &WorkplaceRouteCache,
    network_quotes: &mut SharedNetworkQuotes,
    shards: &mut Vec<WorkplaceQueryShard>,
    pathfind_count: &AtomicU32,
    max_walk_commute_speed: f32,
    max_car_commute_speed: f32,
    job_capacities: &[u32],
) -> (
    BTreeMap<HomeJobOptionsKey, HomeJobOptions>,
    BTreeMap<CurrentJobOptionKey, HomeJobOption>,
    Vec<WorkplaceRouteCacheEntry>,
) {
    #[cfg(test)]
    let _contract_phase = crate::debug::work::phases::enter("workplace_queries");
    let exact_entrance_cache_available = allocator.entrances.len() == allocator.buildings.len();
    // Preparation is outside the per-key loops. Storage is bounded by worker count and
    // graph size; result buffers grow only when the corresponding sorted key slice grows.
    shards.resize_with(rayon::current_num_threads(), WorkplaceQueryShard::default);
    let home_chunk = keys.len().div_ceil(shards.len()).max(1);
    let current_chunk = current_job_keys.len().div_ceil(shards.len()).max(1);
    shards.par_iter_mut().for_each(|shard| {
        shard.query.prepare_deferred_quotes(transit_network, graph, network_quotes);
        shard.home.clear();
        shard.home.reserve(home_chunk);
        shard.current.clear();
        shard.current.reserve(current_chunk);
    });
    let shared = &*network_quotes;
    shards
        .par_iter_mut()
        .zip(keys.par_chunks(home_chunk))
        .for_each(|(shard, keys)| {
            CandidateVisits::with(|scratch| {
                scratch.begin_query(job_supply.entries.len());
                for &key in keys {
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
                        shared,
                        pathfind_count,
                        exact_entrance_cache_available,
                        if key.has_car {
                            max_car_commute_speed
                        } else {
                            max_walk_commute_speed
                        },
                        scratch,
                        &mut shard.query,
                        &mut route_entries,
                        &mut route_entry_count,
                    );
                    shard.home.push(HomeJobOptionsBuild {
                        key,
                        options,
                        route_entry_count: route_entry_count as u8,
                        route_entries,
                    });
                }
            });
        });
    for shard in shards.iter_mut() {
        shard.query.publish_quotes(network_quotes);
    }

    let mut home_options = BTreeMap::new();
    let mut route_entries = Vec::new();
    for build in shards.iter_mut().flat_map(|shard| shard.home.drain(..)) {
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

    let shared = &*network_quotes;
    shards
        .par_iter_mut()
        .zip(current_job_keys.par_chunks(current_chunk))
        .for_each(|(shard, keys)| {
            for &key in keys {
                let mut route_entries = [EMPTY_WORKPLACE_ROUTE_ENTRY; JOB_ROUTE_SCAN_CANDIDATES];
                let mut route_entry_count = 0usize;
                let option = build_current_job_option_for_key(
                    key,
                    allocator,
                    transit_network,
                    graph,
                    catalog,
                    route_cache,
                    shared,
                    pathfind_count,
                    exact_entrance_cache_available,
                    &mut shard.query,
                    &mut route_entries,
                    &mut route_entry_count,
                    job_capacities,
                );
                shard.current.push(CurrentJobOptionBuild {
                    key,
                    option,
                    route_entry_count: route_entry_count as u8,
                    route_entries,
                });
            }
        });
    for shard in shards.iter_mut() {
        shard.query.publish_quotes(network_quotes);
    }

    let mut current_job_options = BTreeMap::new();
    for build in shards.iter_mut().flat_map(|shard| shard.current.drain(..)) {
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
    route_cache: &WorkplaceRouteCache,
    network_quotes: &SharedNetworkQuotes,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    max_commute_speed: f32,
    scratch: &mut CandidateVisits,
    query: &mut TripQueryScratch,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
) -> HomeJobOptions {
    crate::debug::work::add(crate::debug::work::Work::JobSearchKeys, 1);
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

    let home_trip_ends = BuildingTripEnds::origin(key.home_idx, allocator, transit_network, graph);
    let home_foot_components =
        foot_components.building_components(allocator, graph, key.home_idx, TransitFlags::FOOT);
    scan_home_job_bucket(
        &job_supply.foot_buckets,
        home_foot_components,
        key,
        &home_trip_ends,
        job_supply,
        transit_network,
        graph,
        route_cache,
        network_quotes,
        pathfind_count,
        exact_entrance_cache_available,
        home.center_x,
        home.center_y,
        max_commute_speed,
        scratch,
        query,
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
            &home_trip_ends,
            job_supply,
            transit_network,
            graph,
            route_cache,
            network_quotes,
            pathfind_count,
            exact_entrance_cache_available,
            home.center_x,
            home.center_y,
            max_commute_speed,
            scratch,
            query,
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
    home_trip_ends: &BuildingTripEnds,
    job_supply: &JobSupplySnapshot,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &WorkplaceRouteCache,
    network_quotes: &SharedNetworkQuotes,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    origin_x: f32,
    origin_y: f32,
    max_commute_speed: f32,
    scratch: &mut CandidateVisits,
    query: &mut TripQueryScratch,
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
            let Some(commute_seconds) = cached_commute_seconds_within(
                key.home_idx,
                entry.building_idx,
                key.has_car,
                home_trip_ends,
                &entry.trip_ends,
                transit_network,
                graph,
                route_cache,
                network_quotes,
                pathfind_count,
                exact_entrance_cache_available,
                max_commute_speed,
                commute_budget_seconds(options, *option_count),
                query,
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
        } => !home_job_search_can_stop_after(
            options,
            *option_count,
            home_trip_ends
                .lower_bound_seconds_to_distance(
                    key.has_car,
                    origin_x,
                    origin_y,
                    next_min_distance_sq.sqrt(),
                    job_supply.node_slack_m,
                    max_commute_speed,
                )
                .max(lower_bound_travel_seconds(
                    next_min_distance_sq,
                    max_commute_speed,
                )),
        ),
    });
}

#[cfg(test)]
fn home_job_search_can_stop(
    options: &HomeJobOptions,
    option_count: usize,
    next_min_distance_sq: f32,
    max_commute_speed: f32,
) -> bool {
    home_job_search_can_stop_after(
        options,
        option_count,
        lower_bound_travel_seconds(next_min_distance_sq, max_commute_speed),
    )
}

// Stops once every unscanned job takes at least `lower_bound_s` and so cannot displace the ranked
// options.
fn home_job_search_can_stop_after(
    options: &HomeJobOptions,
    option_count: usize,
    lower_bound_s: f32,
) -> bool {
    if option_count < JOB_SEARCH_CANDIDATES {
        return false;
    }
    // Equal capped penalties can still be beaten by wage, capacity, or building ID.
    normalized_commute_penalty_seconds(lower_bound_s)
        > options.options[option_count - 1].commute_penalty
}

// Longest commute that could still displace the last ranked option, with one second of slack for
// rounding; unbounded until the options are full or once their penalty is capped.
fn commute_budget_seconds(options: &HomeJobOptions, option_count: usize) -> f32 {
    if option_count < JOB_SEARCH_CANDIDATES {
        return f32::INFINITY;
    }
    let last_penalty = options.options[option_count - 1].commute_penalty;
    if last_penalty >= 1.0 {
        return f32::INFINITY;
    }
    last_penalty * COMMUTE_PENALTY_MAX_SECONDS + 1.0
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
    route_cache: &WorkplaceRouteCache,
    network_quotes: &SharedNetworkQuotes,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    query: &mut TripQueryScratch,
    new_route_entries: &mut [WorkplaceRouteCacheEntry; JOB_ROUTE_SCAN_CANDIDATES],
    new_route_entry_count: &mut usize,
    job_capacities: &[u32],
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
        network_quotes,
        pathfind_count,
        exact_entrance_cache_available,
        query,
        new_route_entries,
        new_route_entry_count,
    )?;
    let average_daily_wage = profile.average_daily_wage();
    let effective_capacity = job_capacities.get(work_idx).copied().unwrap_or(0);
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

// A commute over `budget_s` cannot enter the ranked options; it is neither returned nor cached.
#[allow(clippy::too_many_arguments)]
fn cached_commute_seconds_within(
    home_idx: usize,
    work_idx: usize,
    has_car: bool,
    home_trip_ends: &BuildingTripEnds,
    work_trip_ends: &BuildingTripEnds,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &WorkplaceRouteCache,
    network_quotes: &SharedNetworkQuotes,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    network_speed_bound_ms: f32,
    budget_s: f32,
    query: &mut TripQueryScratch,
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
    let TripEstimate::Seconds(result) = estimate_trip_seconds_with_scratch(
        home_trip_ends,
        work_trip_ends,
        has_car,
        transit_network,
        graph,
        pathfind_count,
        network_speed_bound_ms,
        budget_s,
        Some(network_quotes),
        query,
    ) else {
        return None;
    };
    if *new_route_entry_count < JOB_ROUTE_SCAN_CANDIDATES {
        new_route_entries[*new_route_entry_count] = (key, result);
        *new_route_entry_count += 1;
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn cached_commute_seconds(
    home_idx: usize,
    work_idx: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    route_cache: &WorkplaceRouteCache,
    network_quotes: &SharedNetworkQuotes,
    pathfind_count: &AtomicU32,
    exact_entrance_cache_available: bool,
    query: &mut TripQueryScratch,
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
    let result = estimate_building_origin_trip_seconds_with_scratch(
        home_idx,
        work_idx,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
        Some(network_quotes),
        query,
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
    use crate::simulation::economy::agents::tick::estimate_trip_seconds_between;

    #[test]
    fn clearing_households_discards_workplace_query_shards() {
        let mut households = HouseholdSystem::new();
        households.workplace_query_shards.push(Default::default());
        households.clear();
        assert!(households.workplace_query_shards.is_empty());
    }

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

    #[test]
    fn bucket_commutes_preserve_all_twenty_four_ranked_job_options() {
        use crate::simulation::buildings::allocator::BuildingEntrance;
        use crate::simulation::network::graph::Edge;
        use crate::simulation::network::lanes::LaneType;
        use crate::simulation::network::types::NodeType;
        use crate::simulation::pathing::cch::CchGraph;
        use godot::prelude::{Vector2, Vector3};
        use std::sync::atomic::Ordering;

        let mut graph = RegionGraph::new();
        for node in 0..45 {
            graph.add_node(
                Vector3::new(node as f32 * 100.0, 0.0, 0.0),
                NodeType::Junction,
            );
        }
        for edge in 0..44 {
            let geometry = vec![graph.node(edge).pos, graph.node(edge + 1).pos];
            graph.add_edge(Edge {
                start_node: edge,
                end_node: edge + 1,
                fwd_lanes: 1,
                bkw_lanes: 1,
                allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
                width: 10.0,
                speed_limit: 15.0,
                base_cost: 300.0 + (edge % 3) as f32 / 7.0,
                physical_length: 100.0,
                physical_geometry: geometry.clone(),
                geometry,
                ..Default::default()
            });
        }
        graph.rebuild_adjacency_list();
        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = CchGraph::build(&graph);
        let entrances: Vec<_> = (0..44)
            .map(|edge| {
                let lane = |kind, forward| {
                    network.lane_system.edge_lanes[&edge]
                        .iter()
                        .copied()
                        .find(|&id| {
                            let lane = &network.lane_system.lanes[id];
                            lane.lane_type == kind && lane.is_fwd == forward
                        })
                        .unwrap()
                };
                BuildingEntrance {
                    edge_idx: edge,
                    entrance_s_m: 50.0,
                    side: 1,
                    door_pos: Vector2::new(edge as f32 * 100.0 + 50.0, 12.0),
                    curb_pos: Vector2::new(edge as f32 * 100.0 + 50.0, 5.0),
                    foot_lane_fwd: lane(LaneType::Foot, true),
                    foot_lane_bkw: lane(LaneType::Foot, false),
                    car_lane_fwd: lane(LaneType::Vehicle, true),
                    car_lane_bkw: lane(LaneType::Vehicle, false),
                    ..Default::default()
                }
            })
            .collect();
        for has_car in [false, true] {
            let origin = BuildingTripEnds::at_entrance(&entrances[0], true, &network, &graph);
            let destinations: Vec<_> = entrances[1..]
                .iter()
                .map(|entrance| BuildingTripEnds::at_entrance(entrance, false, &network, &graph))
                .collect();
            let mut expected = Vec::new();
            let mut rankings = Vec::new();
            let mut counters = Vec::new();
            for batched in [false, true] {
                // Identical recustomization changes the quote revision, ensuring the batch
                // pass cannot accidentally reuse the point pass's worker-local leg cache.
                network.cch_graph.customize(&graph);
                let mut shared = SharedNetworkQuotes::default();
                if batched {
                    shared.prepare_targets(
                        std::array::from_fn(|mode| {
                            destinations
                                .iter()
                                .flat_map(|end| {
                                    end.network_targets(mode as u8).into_iter().flatten()
                                })
                                .collect()
                        }),
                        &network,
                    );
                }
                let counter = AtomicU32::new(0);
                let mut options = empty_home_job_options();
                let mut count = 0;
                for index in 0..43 {
                    let job = index * 17 % 43;
                    let estimate = estimate_trip_seconds_between(
                        &origin,
                        &destinations[job],
                        has_car,
                        &network,
                        &graph,
                        &counter,
                        15.0,
                        commute_budget_seconds(&options, count),
                        batched.then_some(&shared),
                    );
                    if batched {
                        assert_eq!(estimate, expected[index], "has_car={has_car}, job={job}");
                    } else {
                        expected.push(estimate);
                    }
                    if let TripEstimate::Seconds(Some(seconds)) = estimate {
                        insert_home_job_option(
                            &mut options.options,
                            &mut count,
                            HomeJobOption {
                                building_idx: job,
                                commute_penalty: normalized_commute_penalty_seconds(seconds as f32),
                                average_daily_wage: 100.0 + (job % 3) as f32 * 10.0,
                                effective_capacity: 2 + job as u32 % 5,
                                open_slots: 1 + job as u32 % 4,
                            },
                        );
                    }
                }
                assert_eq!(count, JOB_SEARCH_CANDIDATES);
                rankings.push(options.options.map(|option| {
                    (
                        option.building_idx,
                        option.commute_penalty.to_bits(),
                        option.average_daily_wage.to_bits(),
                        option.effective_capacity,
                        option.open_slots,
                    )
                }));
                counters.push(counter.load(Ordering::Relaxed));
            }
            assert_eq!(rankings[0], rankings[1]);
            assert!(
                counters[0] > 4,
                "point pass must perform distinct network searches"
            );
            assert!(
                counters[1] <= 4,
                "each of the two ranks/modes needs at most one forward search: {counters:?}"
            );
        }
    }
}
