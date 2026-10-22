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
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, BuildingEntranceKey};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, ReachableBucketScanEvent,
    chunk_for_point, lower_bound_travel_seconds,
};
use crate::simulation::economy::agents::tick::{
    BuildingTripEnds, SharedNetworkQuotes, TripEstimate, TripQueryScratch,
    estimate_building_origin_trip_seconds_with_scratch, estimate_trip_seconds_with_scratch,
};
use crate::simulation::economy::agents::{
    AgentHandle, AgentSystem, TRANSIT_IN_BUILDING, age_group_can_work,
};
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, EconomyProfileRuntimeKind, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
#[cfg(test)]
use crate::simulation::economy::fiscal::CityFiscalPolicy;
use crate::simulation::economy::fiscal::tax_amount;
#[cfg(test)]
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
    building: Handle<Building>,
    // The door the arrival ends were derived from.
    entrance: Option<BuildingEntranceKey>,
    chunk: (i32, i32),
    foot_components: BuildingModeComponents,
    car_components: BuildingModeComponents,
    // Arrival ends shared by every home's commute estimate to this workplace.
    trip_ends: BuildingTripEnds,
}

// The road-network side of a job board: each member's buckets and arrival ends, the
// targets prepared for them and the top commute speeds. A pure function of the members, the
// road topology and network and the members' doors, so it is rebuilt when its key or one of
// its members' doors changes and never saved.
pub(super) struct JobSupplySnapshot {
    key: JobSupplyKey,
    entries: Vec<JobSupplyEntry>,
    foot_buckets: ReachableBucketIndex,
    car_buckets: ReachableBucketIndex,
    // Farthest network node any workplace's trips use, measured from its bucketed centre.
    node_slack_m: f32,
    max_walk_commute_speed: f32,
    max_car_commute_speed: f32,
}

// Board hour, component generation, CCH build and metric, and lane count and revision that a
// supply snapshot was built for.
type JobSupplyKey = (u64, Option<u32>, u32, u64, usize, u64);

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

    /// The funding census (`ECON-17`) of the live city with `service_funding_by_building`,
    /// published, then job search against it: a fixture for hiring tests on a hand-built city,
    /// which run the census on the live stores as the census tests do, not as the slots
    /// copy, step and publish it (`SIM-02-I`).
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
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.census_run.abandon();
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
        self.job_funding
            .set_published(Some(self.census_key(transit_network)));
        self.publish_funded_job_capacities(allocator);
        self.assign_agent_workplaces_from_census(
            agents,
            allocator,
            transit_network,
            graph,
            minute_of_day,
        );
    }

    /// Job search of every agent who may look now, at once, in agent order, as their hourly
    /// phases would queue it, on a board built now from the latest funding census.
    #[cfg(test)]
    pub(crate) fn assign_agent_workplaces_from_census(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        self.expire_workplace_route_cache();
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        eject_inactive_work_assignments(&mut self.households, agents, allocator, &catalog);
        agents.handles.register_appended(agents.len());
        self.job_requests.clear();
        for agent_idx in 0..agents.len() {
            if job_search_due(
                agents,
                agent_idx,
                allocator.buildings.len(),
                self.households.len(),
            ) {
                self.job_requests.push(0, agents.agent_handle(agent_idx));
            }
        }
        self.job_board = Default::default();
        self.job_supply = None;
        self.serve_job_requests(
            0,
            usize::MAX,
            agents,
            allocator,
            transit_network,
            graph,
            minute_of_day,
        );
    }

    /// Queues the job searches of the agents in hourly phase `slot` (`SIM-02-J`): first an
    /// assignment to a workplace that stopped operating, or of an agent who cannot work, ends;
    /// then an agent in a building who may look for work queues its search, once its job lock
    /// has passed, while unemployed or after unpaid days. O(phase agents).
    pub(crate) fn queue_job_requests(
        &mut self,
        phase_agents: &[AgentHandle],
        slot: u64,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if phase_agents.is_empty() {
            return;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        for &handle in phase_agents {
            let Some(agent_idx) = agents.resolve_agent(handle) else {
                continue;
            };
            let work = agents.work_building[agent_idx];
            if work < allocator.buildings.len()
                && (!age_group_can_work(agents.age_group[agent_idx])
                    || active_work_profile(&catalog, &allocator.buildings[work]).is_none())
            {
                release_worker(&mut self.households, agents, allocator, agent_idx, work);
            }
            if job_search_due(
                agents,
                agent_idx,
                allocator.buildings.len(),
                self.households.len(),
            ) {
                self.job_requests.push(slot, handle);
            }
        }
    }

    /// One slot of the `job requests` queue (`SIM-02-J`): takes up to `quota` searches and
    /// runs them against the hour's job board. A request whose agent left is dropped. Returns
    /// the requests taken. O(1) with nothing queued; see [`Self::search_jobs`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn serve_job_requests(
        &mut self,
        slot: u64,
        quota: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) -> usize {
        if self.job_requests.is_empty() {
            return 0;
        }
        let mut batch = std::mem::take(&mut self.job_batch);
        batch.clear();
        let mut taken = 0;
        while taken < quota {
            let Some((_, handle)) = self.job_requests.pop() else {
                break;
            };
            taken += 1;
            if let Some(agent_idx) = agents.resolve_agent(handle) {
                batch.push(agent_idx);
            }
        }
        if !batch.is_empty() {
            self.search_jobs(
                slot,
                &batch,
                agents,
                allocator,
                transit_network,
                graph,
                minute_of_day,
            );
        }
        self.job_batch = batch;
        self.job_queue_stats
            .record_slot(taken, self.job_requests.len());
        taken
    }

    /// Logs and resets the hour's job search work, at the hour's last slot.
    pub(super) fn close_job_hour(&mut self) {
        let queue = std::mem::take(&mut self.job_queue_stats);
        let (keys, plans, ms) = std::mem::take(&mut self.job_hour);
        if queue.taken > 0 {
            debug_log!(
                debug::Channel::EconomyPhases,
                "job_requests_hour {} home_keys={} plans={} search_ms={:.3} board={} \
                 route_cache_entries={} network_quote_entries={}",
                queue,
                keys,
                plans,
                ms,
                self.job_board.members().len(),
                self.workplace_route_cache.len(),
                self.workplace_network_quotes.len()
            );
        }
    }

    /// Prepares what the first job search after loading would for the network's current
    /// revision (`SIM-02-K`): the network-leg storage and one route scratch per worker of the
    /// current pool, as [`Self::serve_job_requests`] sizes them. Neither changes a result:
    /// legs are exact under one revision and the scratch only memoizes storage. O(lanes) per
    /// scratch on a new revision, O(1) otherwise.
    pub(crate) fn prepare_job_search_scratch(
        &mut self,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.workplace_network_quotes.sync(transit_network);
        self.workplace_query_shards
            .resize_with(rayon::current_num_threads(), WorkplaceQueryShard::default);
        self.workplace_query_shards
            .par_iter_mut()
            .for_each(|shard| shard.query.prepare(transit_network, graph));
    }

    /// Whether [`Self::prepare_job_search_scratch`] left its storage current for `network`.
    #[cfg(test)]
    pub(crate) fn job_search_scratch_prepared_for_test(&self, network: &TransitNetwork) -> bool {
        self.workplace_network_quotes.synced_for_test(network)
            && !self.workplace_query_shards.is_empty()
            && self
                .workplace_query_shards
                .iter()
                .all(|shard| shard.query.prepared_for_test(network, false))
    }

    /// Drops the job search's scratch and network-leg storage, as before the first search.
    #[cfg(test)]
    pub(crate) fn discard_job_search_scratch_for_test(&mut self) {
        self.workplace_network_quotes = Default::default();
        self.workplace_query_shards.clear();
    }

    // The searches of `batch`, agents in queue order: each eligible agent's best options from
    // its home, ranked as before, against the hour's job board with live funded jobs and
    // workers; homes shared within the batch are searched once. Plans apply resident workers
    // with a job at home first, then in agent order, against live slot counts; a searcher who
    // keeps its job is locked again. O(batch homes × scanned workplaces) plus a board built at
    // the hour's first search, O(B) on Rayon, and its network side after a road change.
    #[allow(clippy::too_many_arguments)]
    fn search_jobs(
        &mut self,
        slot: u64,
        batch: &[usize],
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        let start = Instant::now();
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let target_days = household_demand_profile(&catalog).stock_target_days;
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        let hour = slot / SLOTS_PER_HOUR;
        if self
            .job_board
            .members_for(hour, allocator, &catalog)
            .is_empty()
        {
            // No open job: every employed searcher keeps its job under a new lock.
            relock_settled_workers(
                agents,
                batch,
                allocator.buildings.len(),
                self.households.len(),
                &[],
            );
            return;
        }
        let (home_option_keys, current_job_keys) = collect_home_job_option_keys(
            agents,
            batch,
            allocator.buildings.len(),
            self.households.len(),
        );
        if home_option_keys.is_empty() && current_job_keys.is_empty() {
            return;
        }
        self.refresh_workplace_route_cache(allocator, transit_network);
        self.workplace_network_quotes.sync(transit_network);
        self.destination_topology.refresh(transit_network, graph);
        let key = job_supply_key(
            hour,
            self.destination_topology.generation(),
            transit_network,
        );
        if self
            .job_supply
            .as_ref()
            .is_none_or(|supply| supply.key != key || !supply.doors_current(allocator))
        {
            let speeds = self
                .destination_topology
                .max_commute_speeds(transit_network, graph);
            let supply = JobSupplySnapshot::build(
                key,
                self.job_board.members(),
                allocator,
                transit_network,
                graph,
                &self.destination_topology,
                speeds,
            );
            let target_searches =
                supply.prepare_targets(&mut self.workplace_network_quotes, transit_network);
            agents
                .pathfind_count
                .fetch_add(target_searches as u32, std::sync::atomic::Ordering::Relaxed);
            self.job_supply = Some(supply);
        }
        let job_supply = self.job_supply.take().unwrap();
        if job_supply.entries.is_empty() {
            self.job_supply = Some(job_supply);
            relock_settled_workers(
                agents,
                batch,
                allocator.buildings.len(),
                self.households.len(),
                &[],
            );
            return;
        }
        let (home_job_options, current_job_options, new_route_entries) = build_home_job_options(
            &home_option_keys,
            &current_job_keys,
            &job_supply,
            self.destination_topology.foot(),
            self.destination_topology.car(),
            allocator,
            transit_network,
            graph,
            &catalog,
            &self.workplace_route_cache,
            &mut self.workplace_network_quotes,
            &mut self.workplace_query_shards,
            &agents.pathfind_count,
        );
        self.job_supply = Some(job_supply);
        // Reserve the completed batch before ordered insertion; no per-quote growth.
        self.workplace_route_cache.reserve(new_route_entries.len());
        for (key, result) in new_route_entries {
            if self.workplace_route_cache.insert(key, result).is_none() {
                self.workplace_route_cache_order
                    .push_back((self.workplace_route_cache_pass, key));
            }
        }
        let mut plans: Vec<_> = batch
            .par_iter()
            .filter_map(|&agent_idx| {
                plan_agent_workplace(
                    agent_idx,
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
        let mut planned_agents: Vec<usize> = plans.iter().map(|plan| plan.agent_idx).collect();
        planned_agents.sort_unstable();
        let plan_count = plans.len();
        for plan in plans {
            apply_workplace_plan(plan, &mut self.households, agents, allocator, &catalog);
        }
        relock_settled_workers(
            agents,
            batch,
            allocator.buildings.len(),
            self.households.len(),
            &planned_agents,
        );
        let (keys, plan_total, ms) = &mut self.job_hour;
        *keys += home_option_keys.len() as u32;
        *plan_total += plan_count as u32;
        *ms += start.elapsed().as_secs_f64() * 1000.0;
        debug_log!(
            debug::Channel::EconomyWorkplace,
            "workplace_assignment_detail minute={} batch={} home_keys={} current_keys={} plans={} total_ms={:.3}",
            minute_of_day,
            batch.len(),
            home_option_keys.len(),
            current_job_keys.len(),
            plan_count,
            start.elapsed().as_secs_f64() * 1000.0,
        );
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
        // Main-thread funding changes reach here outside any pool (CODE-17).
        let funded_capacity_by_building: Vec<Option<u32>> = crate::thread_pools::install(|| {
            allocator
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
                .collect()
        });
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

// Employed workers of `batch` who searched and had no better job keep it under a new lock, so
// they do not repeat the same search every hour. `planned_agents` (sorted) found a better job;
// if it was full they stay unlocked and search again at their next phase. Workers leaving an
// unpaid job are never locked. Same eligibility as `job_search_due`. O(batch log batch).
fn relock_settled_workers(
    agents: &mut AgentSystem,
    batch: &[usize],
    building_count: usize,
    household_count: usize,
    planned_agents: &[usize],
) {
    let today = agents.settlement_days;
    for &i in batch {
        let work = agents.work_building[i];
        if agents.job_lock_until_day[i] <= today
            && work < building_count
            && agents.transit[i] == TRANSIT_IN_BUILDING
            && age_group_can_work(agents.age_group[i])
            && agents.home_building[i] < building_count
            && agents.household_id[i] < household_count
            && agents.consecutive_unpaid_days[i] < JOB_UNPAID_ABANDON_DAYS
            && planned_agents.binary_search(&i).is_err()
        {
            agents.job_lock_until_day[i] = today + u32::from(job_lock_days(i, work));
        }
    }
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

        // Funded jobs from the workplace's own row, which moves with it (`SIM-02-J`).
        if building
            .funded_job_capacity
            .saturating_sub(building.worker_count)
            == 0
        {
            continue;
        }

        if plan.old_job < allocator.buildings.len() {
            allocator.set_worker_count(
                plan.old_job,
                allocator.buildings[plan.old_job]
                    .worker_count
                    .saturating_sub(1),
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

#[cfg(test)]
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
        allocator.set_worker_count(
            work,
            allocator.buildings[work].worker_count.saturating_sub(1),
        );
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

// A workplace's open funded jobs now, from its own row: the funded jobs the last census
// copied onto it less its workers, and its profile's wage.
#[derive(Clone, Copy)]
pub(super) struct OpenJob {
    average_daily_wage: f32,
    effective_capacity: u32,
    open_slots: u32,
}

// Building `idx`'s open job, if it is an operational workplace with funded jobs left. Reads only
// the building's own row, so a board member's live state is checked by handle. O(1).
pub(super) fn open_job(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    idx: usize,
) -> Option<OpenJob> {
    let building = &allocator.buildings[idx];
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
    let effective_capacity = building.funded_job_capacity;
    let open_slots = effective_capacity.saturating_sub(building.worker_count);
    (open_slots > 0).then(|| OpenJob {
        average_daily_wage: profile.average_daily_wage(),
        effective_capacity,
        open_slots,
    })
}

fn job_supply_key(hour: u64, components: Option<u32>, network: &TransitNetwork) -> JobSupplyKey {
    (
        hour,
        components,
        network.cch_graph.build_generation,
        network.cch_graph.metric_revision,
        network.lane_system.lanes.len(),
        network.lane_system.revision(),
    )
}

// Whether agent `i` may look for work now: in a building, of working age, housed in a
// household, and unemployed, past its job lock or unpaid for too long. O(1).
fn job_search_due(
    agents: &AgentSystem,
    i: usize,
    building_count: usize,
    household_count: usize,
) -> bool {
    if agents.transit[i] != TRANSIT_IN_BUILDING || !age_group_can_work(agents.age_group[i]) {
        return false;
    }
    if agents.home_building[i] >= building_count || agents.household_id[i] >= household_count {
        return false;
    }
    let old_job = agents.work_building[i];
    old_job == usize::MAX
        || old_job >= building_count
        || agents.job_lock_until_day[i] <= agents.settlement_days
        || agents.consecutive_unpaid_days[i] >= JOB_UNPAID_ABANDON_DAYS
}

impl JobSupplySnapshot {
    // The network side of board `members` for `key`: each live member's door, components,
    // chunk and arrival ends, in handle order, and its buckets, with the top walking and car
    // commute speeds `speeds`. O(members) on Rayon.
    fn build(
        key: JobSupplyKey,
        members: &[Handle<Building>],
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        topology: &super::destinations::HouseholdTopology,
        [max_walk_commute_speed, max_car_commute_speed]: [f32; 2],
    ) -> Self {
        let entries: Vec<_> = members
            .par_iter()
            .filter_map(|&building| {
                let idx = allocator.building_handles.resolve(building)?;
                let entrance = allocator.entrances.get(idx).map(BuildingEntranceKey::new);
                let foot_components =
                    topology
                        .foot()
                        .building_components(allocator, graph, idx, TransitFlags::FOOT);
                let car_components =
                    topology
                        .car()
                        .building_components(allocator, graph, idx, TransitFlags::CAR);
                if foot_components.as_slice().is_empty()
                    && car_components.as_slice().is_empty()
                    && allocator.household_capacity(idx) == 0
                {
                    return None;
                }
                let center = &allocator.buildings[idx];
                Some(JobSupplyEntry {
                    building,
                    entrance,
                    chunk: chunk_for_point(center.center_x, center.center_y),
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
            .filter_map(|entry| {
                let building =
                    &allocator.buildings[allocator.building_handles.resolve(entry.building)?];
                Some(
                    entry
                        .trip_ends
                        .max_node_distance(building.center_x, building.center_y),
                )
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
            key,
            entries,
            foot_buckets: ReachableBucketIndex::from_entries(foot_bucket_entries),
            node_slack_m,
            car_buckets: ReachableBucketIndex::from_entries(car_bucket_entries),
            max_walk_commute_speed,
            max_car_commute_speed,
        }
    }

    // Whether every live entry's door is still the one its arrival ends were derived from:
    // another building's placement leaves the snapshot valid. O(entries).
    fn doors_current(&self, allocator: &BuildingAllocator) -> bool {
        self.entries.iter().all(|entry| {
            allocator
                .building_handles
                .resolve(entry.building)
                .is_none_or(|idx| {
                    allocator.entrances.get(idx).map(BuildingEntranceKey::new) == entry.entrance
                })
        })
    }

    // Prepares the members' arrival targets in the shared network quotes; returns the backward
    // target searches it ran.
    fn prepare_targets(&self, quotes: &mut SharedNetworkQuotes, network: &TransitNetwork) -> usize {
        let targets = std::array::from_fn(|mode| {
            self.entries
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
        quotes.prepare_targets(targets, network)
    }

    // Entry `entry`'s live building and open job; `None` once it left or filled. O(1).
    fn open_job(
        &self,
        entry: &JobSupplyEntry,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
    ) -> Option<(usize, OpenJob)> {
        let idx = allocator.building_handles.resolve(entry.building)?;
        open_job(allocator, catalog, idx).map(|job| (idx, job))
    }
}

// The distinct homes, and current jobs from them, of the searchers in `batch` who may still
// look for work, each list sorted. O(batch log batch).
fn collect_home_job_option_keys(
    agents: &AgentSystem,
    batch: &[usize],
    building_count: usize,
    household_count: usize,
) -> (Vec<HomeJobOptionsKey>, Vec<CurrentJobOptionKey>) {
    let requests: Vec<_> = batch
        .par_iter()
        .filter_map(|&i| {
            if !job_search_due(agents, i, building_count, household_count) {
                return None;
            }
            let home_idx = agents.home_building[i];
            let old_job = agents.work_building[i];
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
        shard
            .query
            .prepare_deferred_quotes(transit_network, graph, network_quotes);
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
                    let mut route_entries =
                        [EMPTY_WORKPLACE_ROUTE_ENTRY; JOB_ROUTE_SCAN_CANDIDATES];
                    let mut route_entry_count = 0usize;
                    let options = build_home_job_options_for_key(
                        key,
                        job_supply,
                        foot_components,
                        car_components,
                        allocator,
                        transit_network,
                        graph,
                        catalog,
                        route_cache,
                        shared,
                        pathfind_count,
                        exact_entrance_cache_available,
                        if key.has_car {
                            job_supply.max_car_commute_speed
                        } else {
                            job_supply.max_walk_commute_speed
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
    catalog: &RuntimeEconomyCatalog,
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

    // The handle-sorted supply snapshot also answers on-site jobs without a road-component
    // query.
    let home_handle = allocator.building_handles.handle(key.home_idx);
    if allocator.household_capacity(key.home_idx) > 0
        && let Ok(entry_idx) = job_supply
            .entries
            .binary_search_by_key(&home_handle, |entry| entry.building)
    {
        scratch.mark_seen(entry_idx);
        if let Some(job) = open_job(allocator, catalog, key.home_idx) {
            insert_home_job_option(
                &mut options.options,
                &mut option_count,
                HomeJobOption {
                    building_idx: key.home_idx,
                    commute_penalty: 0.0,
                    average_daily_wage: job.average_daily_wage,
                    effective_capacity: job.effective_capacity,
                    open_slots: job.open_slots,
                },
            );
        }
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
        allocator,
        catalog,
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
            allocator,
            catalog,
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
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
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
            // A workplace that left or filled since the board was built is no option.
            let Some((work_idx, job)) = job_supply.open_job(entry, allocator, catalog) else {
                return true;
            };
            let Some(commute_seconds) = cached_commute_seconds_within(
                key.home_idx,
                work_idx,
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
                    building_idx: work_idx,
                    commute_penalty: normalized_commute_penalty_seconds(commute_seconds as f32),
                    average_daily_wage: job.average_daily_wage,
                    effective_capacity: job.effective_capacity,
                    open_slots: job.open_slots,
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
    // Funded jobs from the workplace's own row, which moves with it (`SIM-02-J`).
    let effective_capacity = work.funded_job_capacity;
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
