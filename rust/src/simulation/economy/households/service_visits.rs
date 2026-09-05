// SPDX-License-Identifier: GPL-2.0-only

//! Sampled visible service visits layered over aggregate commercial service demand.

use std::sync::atomic::AtomicU32;

use super::data::{Household, HouseholdSystem};
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, building_operation_factors, economy_profile_for_building,
};
use super::replenishment::{
    REPLENISHMENT_FULFILLED, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
    REPLENISHMENT_STABLE,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    ReachableBucketScanEvent, chunk_for_point,
};
use crate::simulation::economy::agents::tick::building_origin_trip_is_feasible;
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, AgentSystem, TRANSIT_IN_BUILDING,
};
#[cfg(test)]
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const PERSONAL_SERVICE_RESOURCE_ID: &str = "personal_services";
const VISIBLE_PERSONAL_SERVICE_VISIT_SAMPLE_RATE: f32 = 1.0;
const VISIBLE_SERVICE_VISIT_MAX_REQUESTS_PER_HOUR: usize = 64;
const SERVICE_VISIT_SEARCH_CANDIDATES: usize = 8;
const SERVICE_VISIT_ROUTE_SCAN_CANDIDATES: usize = 128;

#[derive(Default)]
struct ServiceVisitDiagnostics {
    attempts: u32,
    successes: u32,
    returns_scheduled: u32,
    failed_no_shopper: u32,
    failed_no_service_candidates: u32,
    rejected_unreachable: u32,
    candidate_count: u32,
}

impl ServiceVisitDiagnostics {
    fn has_signal(&self) -> bool {
        self.attempts > 0 || self.returns_scheduled > 0 || self.failed_no_shopper > 0
    }

    fn merge(&mut self, other: Self) {
        self.attempts += other.attempts;
        self.successes += other.successes;
        self.returns_scheduled += other.returns_scheduled;
        self.failed_no_shopper += other.failed_no_shopper;
        self.failed_no_service_candidates += other.failed_no_service_candidates;
        self.rejected_unreachable += other.rejected_unreachable;
        self.candidate_count += other.candidate_count;
    }
}

#[derive(Clone, Copy)]
struct ServiceVisitRequest {
    household_id: usize,
    visitor_agent_id: usize,
    priority_key: u64,
}

#[derive(Clone, Copy)]
struct ServiceVisitPlan {
    household_id: usize,
    visitor_agent_id: usize,
    service_building_id: usize,
}

struct ServiceVisitEntry {
    building_idx: usize,
    chunk: (i32, i32),
    foot_components: BuildingModeComponents,
    car_components: BuildingModeComponents,
}

struct ServiceVisitIndex {
    entries: Vec<ServiceVisitEntry>,
    foot_buckets: ReachableBucketIndex,
    car_buckets: ReachableBucketIndex,
}

impl ServiceVisitIndex {
    fn build(
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        resource_runtime_id: ResourceRuntimeId,
        graph: &RegionGraph,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
    ) -> Self {
        let mut entries: Vec<_> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, building)| {
                if !service_building_can_host_visible_visit(catalog, building, resource_runtime_id)
                {
                    return None;
                }
                let foot_components =
                    foot_components.building_components(allocator, graph, idx, TransitFlags::FOOT);
                let car_components =
                    car_components.building_components(allocator, graph, idx, TransitFlags::CAR);
                if foot_components.as_slice().is_empty() && car_components.as_slice().is_empty() {
                    return None;
                }
                Some(ServiceVisitEntry {
                    building_idx: idx,
                    chunk: chunk_for_point(building.center_x, building.center_y),
                    foot_components,
                    car_components,
                })
            })
            .collect();
        entries.sort_unstable_by_key(|entry| (entry.chunk, entry.building_idx));

        let mut foot_bucket_entries = Vec::with_capacity(entries.len());
        let mut car_bucket_entries = Vec::with_capacity(entries.len());
        for (entry_idx, entry) in entries.iter().enumerate() {
            index_service_components(
                &mut foot_bucket_entries,
                entry.foot_components,
                entry.chunk,
                entry_idx,
            );
            index_service_components(
                &mut car_bucket_entries,
                entry.car_components,
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

    fn has_any(&self) -> bool {
        !self.entries.is_empty()
    }

    fn fill_route_feasible_candidates(
        &self,
        home_idx: usize,
        has_car: bool,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
        pathfind_count: &AtomicU32,
        candidates: &mut Vec<usize>,
        seen_candidates: &mut Vec<usize>,
        diagnostics: &mut ServiceVisitDiagnostics,
    ) {
        candidates.clear();
        seen_candidates.clear();
        if home_idx >= allocator.buildings.len() {
            return;
        }
        let home = &allocator.buildings[home_idx];
        let home_foot_components =
            foot_components.building_components(allocator, graph, home_idx, TransitFlags::FOOT);
        self.scan_candidate_bucket(
            &self.foot_buckets,
            home_foot_components,
            home_idx,
            home.center_x,
            home.center_y,
            allocator,
            transit_network,
            graph,
            has_car,
            pathfind_count,
            candidates,
            seen_candidates,
            diagnostics,
        );

        if has_car {
            let home_car_components =
                car_components.building_components(allocator, graph, home_idx, TransitFlags::CAR);
            self.scan_candidate_bucket(
                &self.car_buckets,
                home_car_components,
                home_idx,
                home.center_x,
                home.center_y,
                allocator,
                transit_network,
                graph,
                has_car,
                pathfind_count,
                candidates,
                seen_candidates,
                diagnostics,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_candidate_bucket(
        &self,
        buckets: &ReachableBucketIndex,
        components: BuildingModeComponents,
        home_idx: usize,
        origin_x: f32,
        origin_y: f32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        has_car: bool,
        pathfind_count: &AtomicU32,
        candidates: &mut Vec<usize>,
        seen_candidates: &mut Vec<usize>,
        diagnostics: &mut ServiceVisitDiagnostics,
    ) {
        buckets.scan_nearest(components, origin_x, origin_y, |event| match event {
            ReachableBucketScanEvent::Item { item_idx } => {
                if let Some(entry) = self.entries.get(item_idx) {
                    if seen_candidates.contains(&entry.building_idx) {
                        return true;
                    }
                    if seen_candidates.len() == SERVICE_VISIT_ROUTE_SCAN_CANDIDATES {
                        return false;
                    }
                    seen_candidates.push(entry.building_idx);
                    if !service_visit_route_is_feasible(
                        home_idx,
                        entry.building_idx,
                        has_car,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                    ) {
                        diagnostics.rejected_unreachable += 1;
                        return true;
                    }
                    insert_service_candidate(
                        candidates,
                        SERVICE_VISIT_SEARCH_CANDIDATES,
                        entry.building_idx,
                        origin_x,
                        origin_y,
                        allocator,
                    );
                }
                true
            }
            ReachableBucketScanEvent::RingComplete {
                next_min_distance_sq,
            } => {
                if candidates.len() < SERVICE_VISIT_SEARCH_CANDIDATES {
                    return true;
                }
                let worst = candidates
                    .last()
                    .map(|&idx| {
                        squared_building_distance(origin_x, origin_y, &allocator.buildings[idx])
                    })
                    .unwrap_or(f32::MAX);
                next_min_distance_sq <= worst
            }
        });
    }
}

impl HouseholdSystem {
    pub(super) fn process_visible_service_visit_returns(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        absolute_hour: u32,
    ) {
        let Some(personal_services) =
            catalog.resource_runtime_id_for_id(PERSONAL_SERVICE_RESOURCE_ID)
        else {
            return;
        };
        let mut diagnostics = ServiceVisitDiagnostics::default();
        for agent_idx in 0..agents.len() {
            if !agent_can_be_returned_from_visible_service_visit(
                agent_idx,
                agents,
                &self.households,
                allocator,
                catalog,
                personal_services,
            ) {
                continue;
            }
            let home_idx = agents.home_building[agent_idx];
            agents.planned_target_building[agent_idx] = home_idx;
            agents.planned_activity[agent_idx] = ACTIVITY_HOME;
            diagnostics.returns_scheduled += 1;
        }
        if diagnostics.has_signal() {
            debug_log!(
                "economy",
                "visible service visit diagnostics: hour={} attempts=0 success=0 returns={} candidates=0 no_service_candidates=0 no_shopper=0 rejected_unreachable=0",
                absolute_hour,
                diagnostics.returns_scheduled
            );
        }
    }

    pub(super) fn plan_and_apply_visible_service_visits(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        catalog: &RuntimeEconomyCatalog,
    ) {
        let Some(personal_services) =
            catalog.resource_runtime_id_for_id(PERSONAL_SERVICE_RESOURCE_ID)
        else {
            return;
        };
        let visit_rate_per_resident = service_visit_rate_per_resident(catalog, personal_services)
            * VISIBLE_PERSONAL_SERVICE_VISIT_SAMPLE_RATE;
        if visit_rate_per_resident <= 0.0 {
            return;
        }
        if !staffed_visible_service_building_exists(allocator, catalog, personal_services) {
            return;
        }

        let shopper_candidates = self.collect_eligible_shopper_candidates(agents);
        let (mut requests, mut diagnostics) = self
            .households
            .par_iter()
            .enumerate()
            .fold(
                || (Vec::new(), ServiceVisitDiagnostics::default()),
                |(mut requests, mut diagnostics), (hid, household)| {
                    if let Some(request) = visible_service_visit_request(
                        hid,
                        household,
                        shopper_candidates[hid],
                        agents,
                        allocator,
                        absolute_hour,
                        visit_rate_per_resident,
                        &mut diagnostics,
                    ) {
                        requests.push(request);
                    }
                    (requests, diagnostics)
                },
            )
            .reduce(
                || (Vec::new(), ServiceVisitDiagnostics::default()),
                |mut left, right| {
                    left.0.extend(right.0);
                    left.1.merge(right.1);
                    left
                },
            );
        if requests.is_empty() {
            if diagnostics.has_signal() {
                log_service_visit_diagnostics(absolute_hour, &diagnostics);
            }
            return;
        }

        requests.sort_unstable_by_key(|request| (request.priority_key, request.household_id));
        requests.truncate(VISIBLE_SERVICE_VISIT_MAX_REQUESTS_PER_HOUR);
        let foot_components = ModeComponentIndex::build(graph, TransitFlags::FOOT);
        let car_components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        let service_index = ServiceVisitIndex::build(
            allocator,
            catalog,
            personal_services,
            graph,
            &foot_components,
            &car_components,
        );
        if !service_index.has_any() {
            diagnostics.failed_no_service_candidates += requests.len() as u32;
            log_service_visit_diagnostics(absolute_hour, &diagnostics);
            return;
        }
        let (plans, route_diagnostics) = requests
            .par_iter()
            .fold(
                || {
                    (
                        Vec::new(),
                        ServiceVisitDiagnostics::default(),
                        Vec::with_capacity(SERVICE_VISIT_SEARCH_CANDIDATES),
                        Vec::with_capacity(SERVICE_VISIT_ROUTE_SCAN_CANDIDATES),
                    )
                },
                |(mut plans, mut diagnostics, mut candidates, mut seen_candidates), request| {
                    if let Some(plan) = plan_visible_service_visit(
                        *request,
                        agents,
                        allocator,
                        &service_index,
                        transit_network,
                        graph,
                        &foot_components,
                        &car_components,
                        &mut candidates,
                        &mut seen_candidates,
                        &mut diagnostics,
                    ) {
                        plans.push(plan);
                    }
                    (plans, diagnostics, candidates, seen_candidates)
                },
            )
            .map(|(plans, diagnostics, _, _)| (plans, diagnostics))
            .reduce(
                || (Vec::new(), ServiceVisitDiagnostics::default()),
                |mut left, right| {
                    left.0.extend(right.0);
                    left.1.merge(right.1);
                    left
                },
            );
        diagnostics.merge(route_diagnostics);

        let mut plans = plans;
        plans.sort_unstable_by_key(|plan| (plan.household_id, plan.service_building_id));
        for plan in plans {
            if !service_plan_still_valid(plan, &self.households, agents, allocator) {
                continue;
            }
            agents.planned_target_building[plan.visitor_agent_id] = plan.service_building_id;
            agents.planned_activity[plan.visitor_agent_id] = ACTIVITY_SHOPPING;
            diagnostics.successes += 1;
        }

        if diagnostics.has_signal() {
            log_service_visit_diagnostics(absolute_hour, &diagnostics);
        }
    }

    #[cfg(test)]
    pub(super) fn run_visible_service_visits_for_test(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        self.process_visible_service_visit_returns(agents, allocator, &catalog, absolute_hour);
        self.plan_and_apply_visible_service_visits(
            agents,
            allocator,
            transit_network,
            graph,
            absolute_hour,
            &catalog,
        );
    }
}

fn log_service_visit_diagnostics(absolute_hour: u32, diagnostics: &ServiceVisitDiagnostics) {
    debug_log!(
        "economy",
        "visible service visit diagnostics: hour={} attempts={} success={} returns={} candidates={} no_service_candidates={} no_shopper={} rejected_unreachable={}",
        absolute_hour,
        diagnostics.attempts,
        diagnostics.successes,
        diagnostics.returns_scheduled,
        diagnostics.candidate_count,
        diagnostics.failed_no_service_candidates,
        diagnostics.failed_no_shopper,
        diagnostics.rejected_unreachable
    );
}

fn visible_service_visit_request(
    household_id: usize,
    household: &Household,
    visitor_agent_id: usize,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    absolute_hour: u32,
    visit_rate_per_resident: f32,
    diagnostics: &mut ServiceVisitDiagnostics,
) -> Option<ServiceVisitRequest> {
    if household.member_count == 0
        || household.home_building_id >= allocator.buildings.len()
        || household.budget <= 0.0
        || household.stock_days == 0.0
        || !matches!(
            household.replenishment_state,
            REPLENISHMENT_STABLE | REPLENISHMENT_FULFILLED
        )
    {
        return None;
    }
    let priority_key = stable_service_visit_hash(
        household_id as u64,
        household.home_building_id as u64,
        u64::from(absolute_hour),
    );
    let hourly_visit_probability =
        household.member_count as f32 * visit_rate_per_resident / OPERATIONAL_HOURS_PER_DAY;
    if hourly_visit_probability <= 0.0
        || (hourly_visit_probability < 1.0
            && stable_hash_unit_interval(priority_key) >= hourly_visit_probability)
    {
        return None;
    }
    diagnostics.attempts += 1;
    if visitor_agent_id >= agents.len() {
        diagnostics.failed_no_shopper += 1;
        return None;
    }
    Some(ServiceVisitRequest {
        household_id,
        visitor_agent_id,
        priority_key,
    })
}

#[allow(clippy::too_many_arguments)]
fn plan_visible_service_visit(
    request: ServiceVisitRequest,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    service_index: &ServiceVisitIndex,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    candidates: &mut Vec<usize>,
    seen_candidates: &mut Vec<usize>,
    diagnostics: &mut ServiceVisitDiagnostics,
) -> Option<ServiceVisitPlan> {
    let has_car = agents.has_car[request.visitor_agent_id];
    let home_idx = agents.home_building[request.visitor_agent_id];
    service_index.fill_route_feasible_candidates(
        home_idx,
        has_car,
        allocator,
        transit_network,
        graph,
        foot_components,
        car_components,
        &agents.pathfind_count,
        candidates,
        seen_candidates,
        diagnostics,
    );
    diagnostics.candidate_count += candidates.len() as u32;
    let Some(&service_building_id) = candidates.first() else {
        diagnostics.failed_no_service_candidates += 1;
        return None;
    };
    Some(ServiceVisitPlan {
        household_id: request.household_id,
        visitor_agent_id: request.visitor_agent_id,
        service_building_id,
    })
}

fn service_plan_still_valid(
    plan: ServiceVisitPlan,
    households: &[Household],
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
) -> bool {
    let Some(household) = households.get(plan.household_id) else {
        return false;
    };
    plan.visitor_agent_id < agents.len()
        && plan.service_building_id < allocator.buildings.len()
        && agents.household_id[plan.visitor_agent_id] == plan.household_id
        && agents.transit[plan.visitor_agent_id] == TRANSIT_IN_BUILDING
        && agents.activity[plan.visitor_agent_id] == ACTIVITY_HOME
        && agents.current_building[plan.visitor_agent_id] == household.home_building_id
        && agents.home_building[plan.visitor_agent_id] == household.home_building_id
        && agents.planned_target_building[plan.visitor_agent_id] == usize::MAX
        && agents.target_building[plan.visitor_agent_id] == usize::MAX
}

fn agent_can_be_returned_from_visible_service_visit(
    agent_idx: usize,
    agents: &AgentSystem,
    households: &[Household],
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    personal_services: ResourceRuntimeId,
) -> bool {
    if agent_idx >= agents.len()
        || agents.transit[agent_idx] != TRANSIT_IN_BUILDING
        || agents.activity[agent_idx] != ACTIVITY_SHOPPING
        || agents.planned_target_building[agent_idx] != usize::MAX
        || agents.target_building[agent_idx] != usize::MAX
        || agents.home_building[agent_idx] >= allocator.buildings.len()
        || agents.current_building[agent_idx] == agents.home_building[agent_idx]
        || active_household_replenishment_carrier(agent_idx, agents, households)
    {
        return false;
    }
    allocator
        .buildings
        .get(agents.current_building[agent_idx])
        .is_some_and(|building| {
            service_profile_outputs_resource(catalog, building, personal_services)
        })
}

fn active_household_replenishment_carrier(
    agent_idx: usize,
    agents: &AgentSystem,
    households: &[Household],
) -> bool {
    let household_id = agents.household_id[agent_idx];
    let Some(household) = households.get(household_id) else {
        return false;
    };
    household.shopping_agent_id == agent_idx
        && household.shopping_agent_schedule_seed == agents.schedule_seed[agent_idx]
        && matches!(
            household.replenishment_state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        )
}

fn staffed_visible_service_building_exists(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    allocator.buildings.par_iter().any(|building| {
        service_building_can_host_visible_visit(catalog, building, resource_runtime_id)
    })
}

fn service_building_can_host_visible_visit(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.is_under_construction()
        || building.edge_idx == usize::MAX
        || !matches!(building.zone_type, ZoneType::Commercial)
    {
        return false;
    }
    let Some(profile) = economy_profile_for_building(catalog, building) else {
        return false;
    };
    if profile.kind != EconomyProfileRuntimeKind::ServiceStore
        || profile.output_port(resource_runtime_id).is_none()
    {
        return false;
    }
    building_operation_factors(catalog, building, profile).throughput_factor > 0.0
}

fn service_profile_outputs_resource(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    resource_runtime_id: ResourceRuntimeId,
) -> bool {
    catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .is_some_and(|profile| {
            profile.kind == EconomyProfileRuntimeKind::ServiceStore
                && profile.output_port(resource_runtime_id).is_some()
        })
}

fn service_visit_rate_per_resident(
    catalog: &RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
) -> f32 {
    catalog
        .all_profiles()
        .iter()
        .filter(|profile| profile.kind == EconomyProfileRuntimeKind::DemandSink)
        .flat_map(|profile| {
            profile
                .inputs
                .iter()
                .filter(move |input| input.resource_runtime_id == resource_runtime_id)
                .map(move |_| profile.consumption_rate_per_resident.max(0.0))
        })
        .sum()
}

fn service_visit_route_is_feasible(
    home_idx: usize,
    service_idx: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &AtomicU32,
) -> bool {
    building_origin_trip_is_feasible(
        home_idx,
        service_idx,
        ACTIVITY_SHOPPING,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    ) && building_origin_trip_is_feasible(
        service_idx,
        home_idx,
        ACTIVITY_HOME,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    )
}

fn index_service_components(
    target: &mut Vec<ReachableBucketEntry>,
    components: BuildingModeComponents,
    chunk: (i32, i32),
    entry_idx: usize,
) {
    for &component in components.as_slice() {
        target.push(ReachableBucketEntry::new(component, chunk, entry_idx));
    }
}

fn insert_service_candidate(
    candidates: &mut Vec<usize>,
    candidate_limit: usize,
    candidate: usize,
    origin_x: f32,
    origin_y: f32,
    allocator: &BuildingAllocator,
) {
    if candidate >= allocator.buildings.len() || candidates.contains(&candidate) {
        return;
    }
    let candidate_distance =
        squared_building_distance(origin_x, origin_y, &allocator.buildings[candidate]);
    let mut insert_at = 0usize;
    while insert_at < candidates.len() {
        let existing = candidates[insert_at];
        let existing_distance =
            squared_building_distance(origin_x, origin_y, &allocator.buildings[existing]);
        if candidate_distance
            .total_cmp(&existing_distance)
            .then_with(|| candidate.cmp(&existing))
            .is_lt()
        {
            break;
        }
        insert_at += 1;
    }
    if candidates.len() == candidate_limit && insert_at == candidates.len() {
        return;
    }
    candidates.insert(insert_at, candidate);
    if candidates.len() > candidate_limit {
        candidates.pop();
    }
}

fn squared_building_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

fn stable_service_visit_hash(household_id: u64, home_building_id: u64, absolute_hour: u64) -> u64 {
    mix_u64(
        household_id
            ^ home_building_id.rotate_left(21)
            ^ absolute_hour.rotate_left(42)
            ^ 0x54D2_9B62_1BC7_6A93,
    )
}

fn stable_hash_unit_interval(hash: u64) -> f32 {
    let sample = ((hash >> 40) & 0x00FF_FFFF) as u32;
    sample as f32 / 16_777_216.0
}

fn mix_u64(mut value: u64) -> u64 {
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^ (value >> 31)
}
