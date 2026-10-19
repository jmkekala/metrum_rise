// SPDX-License-Identifier: GPL-2.0-only

//! Sampled visible service visits layered over aggregate commercial service demand.
//!
//! A household's hourly phase draws whether it visits that hour and queues the visit; the
//! `visit requests` queue serves them oldest first at `VISIBLE_SERVICE_VISITS_PER_HOUR`, the
//! cap the hourly pass applied, against the hour's destination front (`SIM-02-J`).

use std::sync::atomic::AtomicU32;

use super::candidate_visits::CandidateVisits;
use super::data::{Household, HouseholdSystem};
use super::destinations::{DestinationIndex, service_hosts_visits};
use super::metrics::OPERATIONAL_HOURS_PER_DAY;
use super::replenishment::{
    REPLENISHMENT_FULFILLED, REPLENISHMENT_SHOPPING_RETURNING, REPLENISHMENT_SHOPPING_TO_STORE,
    REPLENISHMENT_STABLE, ShoppingShortlist, first_eligible_shopper, prepare_walking_access,
    proven_shopping_walk, shopping_route_is_feasible, squared_building_distance,
};
use super::requests::QueueStats;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, ReachableBucketScanEvent,
};
use crate::simulation::economy::agents::tick::{BuildingWalkingAccess, PedestrianReachability};
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, AgentSystem, TRANSIT_IN_BUILDING,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, ResourceRuntimeId, RuntimeEconomyCatalog,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use rayon::prelude::*;

const PERSONAL_SERVICE_RESOURCE_ID: &str = "personal_services";
const VISIBLE_PERSONAL_SERVICE_VISIT_SAMPLE_RATE: f32 = 1.0;
/// Visible service visits served an hour, one per 15 slots; also the most that wait.
pub(crate) const VISIBLE_SERVICE_VISITS_PER_HOUR: u32 = 64;
const SERVICE_VISIT_SEARCH_CANDIDATES: usize = 8;
const SERVICE_VISIT_ROUTE_SCAN_CANDIDATES: usize = 128;

#[derive(Default)]
pub(super) struct ServiceVisitDiagnostics {
    attempts: u32,
    successes: u32,
    returns_scheduled: u32,
    failed_no_shopper: u32,
    failed_no_service_candidates: u32,
    rejected_unreachable: u32,
    rejected_outside_shortlist: u32,
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
        self.rejected_outside_shortlist += other.rejected_outside_shortlist;
        self.candidate_count += other.candidate_count;
    }
}

#[derive(Clone, Copy)]
struct ServiceVisitPlan {
    household_id: usize,
    visitor_agent_id: usize,
    service_building_id: usize,
}

// The service stores of the hour's destination front (`households/destinations.rs`), read
// through their handles: a store that left or stopped hosting visits is skipped.
struct ServiceVisitIndex<'a> {
    services: &'a DestinationIndex,
    catalog: &'a RuntimeEconomyCatalog,
    resource_runtime_id: ResourceRuntimeId,
    walking: Option<&'a PedestrianReachability>,
}

impl ServiceVisitIndex<'_> {
    // The live building of entry `position` while it still hosts visits. O(outputs).
    fn live_service(&self, allocator: &BuildingAllocator, position: usize) -> Option<usize> {
        let entry = self.services.entries.get(position)?;
        let building_idx = allocator.building_handles.resolve(entry.building)?;
        service_hosts_visits(
            self.catalog,
            &allocator.buildings[building_idx],
            self.resource_runtime_id,
        )
        .then_some(building_idx)
    }

    fn has_any(&self) -> bool {
        self.services.has_any()
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_route_feasible_candidates(
        &self,
        home_idx: usize,
        home_access: Option<BuildingWalkingAccess>,
        has_car: bool,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
        pathfind_count: &AtomicU32,
        candidates: &mut Vec<usize>,
        diagnostics: &mut ServiceVisitDiagnostics,
    ) {
        candidates.clear();
        if home_idx >= allocator.buildings.len() {
            return;
        }
        let mut shortlist = ShoppingShortlist::<SERVICE_VISIT_SEARCH_CANDIDATES>::default();
        CandidateVisits::with(|visits| {
            visits.begin_query(self.services.entries.len());
            // Live stores inspected, and stores passed over because they left or stopped hosting.
            let mut inspected = [0; 2];
            let home = &allocator.buildings[home_idx];
            let prepared_home = home_access.or_else(|| {
                self.walking.map(|walking| {
                    prepare_walking_access(home_idx, allocator, transit_network, graph, walking)
                })
            });
            let home_foot_components =
                foot_components.building_components(allocator, graph, home_idx, TransitFlags::FOOT);
            self.scan_candidate_bucket(
                &self.services.foot_buckets,
                home_foot_components,
                home_idx,
                prepared_home.as_ref(),
                home.center_x,
                home.center_y,
                allocator,
                transit_network,
                graph,
                has_car,
                pathfind_count,
                &mut shortlist,
                visits,
                &mut inspected,
                diagnostics,
            );

            if has_car {
                let home_car_components = car_components.building_components(
                    allocator,
                    graph,
                    home_idx,
                    TransitFlags::CAR,
                );
                self.scan_candidate_bucket(
                    &self.services.car_buckets,
                    home_car_components,
                    home_idx,
                    prepared_home.as_ref(),
                    home.center_x,
                    home.center_y,
                    allocator,
                    transit_network,
                    graph,
                    has_car,
                    pathfind_count,
                    &mut shortlist,
                    visits,
                    &mut inspected,
                    diagnostics,
                );
            }
        });
        shortlist.write_to(candidates);
    }

    #[allow(clippy::too_many_arguments)]
    fn scan_candidate_bucket(
        &self,
        buckets: &ReachableBucketIndex,
        components: BuildingModeComponents,
        home_idx: usize,
        prepared_home: Option<&BuildingWalkingAccess>,
        origin_x: f32,
        origin_y: f32,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        has_car: bool,
        pathfind_count: &AtomicU32,
        candidates: &mut ShoppingShortlist<SERVICE_VISIT_SEARCH_CANDIDATES>,
        visits: &mut CandidateVisits,
        inspected: &mut [usize; 2],
        diagnostics: &mut ServiceVisitDiagnostics,
    ) {
        buckets.scan_nearest(components, origin_x, origin_y, |event| match event {
            ReachableBucketScanEvent::Item { item_idx } => {
                if visits.contains(item_idx) {
                    return true;
                }
                visits.mark_seen(item_idx);
                // A store that left or stopped hosting is no candidate; such stores have their
                // own cap, so a closed district cannot make the scan visit every entry.
                let Some(service_idx) = self.live_service(allocator, item_idx) else {
                    inspected[1] += 1;
                    return inspected[1] < SERVICE_VISIT_ROUTE_SCAN_CANDIDATES;
                };
                if inspected[0] == SERVICE_VISIT_ROUTE_SCAN_CANDIDATES {
                    return false;
                }
                inspected[0] += 1;
                let distance = squared_building_distance(
                    origin_x,
                    origin_y,
                    &allocator.buildings[service_idx],
                );
                let Some(position) = candidates.insertion(service_idx, distance) else {
                    diagnostics.rejected_outside_shortlist += 1;
                    return true;
                };
                let service_access = self.walking.map(|walking| {
                    prepare_walking_access(service_idx, allocator, transit_network, graph, walking)
                });
                if !proven_shopping_walk(self.walking, prepared_home, service_access.as_ref())
                    && !shopping_route_is_feasible(
                        home_idx,
                        service_idx,
                        has_car,
                        allocator,
                        transit_network,
                        graph,
                        pathfind_count,
                    )
                {
                    diagnostics.rejected_unreachable += 1;
                    return true;
                }
                candidates.insert(position, service_idx, distance);
                true
            }
            ReachableBucketScanEvent::RingComplete {
                next_min_distance_sq,
            } => {
                if candidates.len() < SERVICE_VISIT_SEARCH_CANDIDATES {
                    return true;
                }
                next_min_distance_sq <= candidates.worst_distance()
            }
        });
    }
}

impl HouseholdSystem {
    /// Ends visitor `agent_idx`'s service visit (`SIM-02-E`): one at the service building heads
    /// home. Returns whether the visit goes on, while the visitor is still on its way. O(1).
    pub(crate) fn end_service_visit(
        &self,
        agent_idx: usize,
        agents: &mut AgentSystem,
        allocator: &BuildingAllocator,
    ) -> bool {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let Some(personal_services) =
            catalog.resource_runtime_id_for_id(PERSONAL_SERVICE_RESOURCE_ID)
        else {
            return false;
        };
        if agent_can_be_returned_from_visible_service_visit(
            agent_idx,
            agents,
            &self.households,
            allocator,
            &catalog,
            personal_services,
        ) {
            let home_idx = agents.home_building[agent_idx];
            agents.planned_target_building[agent_idx] = home_idx;
            agents.planned_activity[agent_idx] = ACTIVITY_HOME;
            return false;
        }
        // Still on its way to the service building.
        agents.activity[agent_idx] == ACTIVITY_SHOPPING
            && agents.transit[agent_idx] != TRANSIT_IN_BUILDING
            && agents.current_building[agent_idx] != agents.home_building[agent_idx]
    }

    /// Queues household `household_id`'s visible service visit at its hourly phase in absolute
    /// hour `absolute_hour` (`SIM-02-J`) when the hour's sampled draw picks it, unless an hour
    /// of visits already waits. O(1).
    pub(super) fn request_service_visit(
        &mut self,
        household_id: usize,
        slot: u64,
        building_count: usize,
        visit_rate_per_resident: f32,
    ) {
        let household = &self.households[household_id];
        if self.visit_requests.len() >= VISIBLE_SERVICE_VISITS_PER_HOUR as usize
            || !visit_due(
                household_id,
                household,
                building_count,
                (slot / SLOTS_PER_HOUR) as u32,
                visit_rate_per_resident,
            )
        {
            return;
        }
        self.visit_requests
            .push(slot, self.handles.handle(household_id));
    }

    /// One slot of the `visit requests` queue (`SIM-02-J`): a visit's credit accrues at
    /// `VISIBLE_SERVICE_VISITS_PER_HOUR` an hour, at most one visit's worth while nothing waits,
    /// and each whole credit serves the oldest request. Returns the requests taken. O(1)
    /// without a whole credit or a request; see [`Self::serve_visits`].
    pub(crate) fn serve_visit_requests(
        &mut self,
        slot: u64,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        let whole = SLOTS_PER_HOUR as u32;
        self.visit_credit = (self.visit_credit + VISIBLE_SERVICE_VISITS_PER_HOUR).min(whole);
        if self.visit_credit < whole || self.visit_requests.is_empty() {
            return 0;
        }
        self.visit_credit -= whole;
        self.serve_visits(slot, 1, agents, allocator, transit_network, graph)
    }

    /// Takes up to `count` visit requests and sends each household's first member at home to
    /// the nearest reachable service store of the hour's destination front; the visitor is
    /// held for its departure. A request whose household left is dropped. Returns the requests
    /// taken. Each costs its household's members and at most
    /// `SERVICE_VISIT_ROUTE_SCAN_CANDIDATES` inspected stores and as many passed over because
    /// they closed, after a front built at once if the hour had none.
    pub(super) fn serve_visits(
        &mut self,
        slot: u64,
        count: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> usize {
        let mut taken = 0;
        let mut batch = std::mem::take(&mut self.visit_batch);
        batch.clear();
        while taken < count {
            let Some((_, handle)) = self.visit_requests.pop() else {
                break;
            };
            taken += 1;
            if let Some(household_id) = self.handles.resolve(handle) {
                batch.push(household_id);
            }
        }
        if !batch.is_empty() {
            self.plan_and_hold_visits(slot, &batch, agents, allocator, transit_network, graph);
        }
        self.visit_batch = batch;
        self.visit_queue_stats
            .record_slot(taken, self.visit_requests.len());
        taken
    }

    // Plans the visits of `batch`, households in queue order, in parallel, then holds the
    // visitors whose plans still hold, in that order.
    fn plan_and_hold_visits(
        &mut self,
        slot: u64,
        batch: &[usize],
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let Some(personal_services) = personal_services_resource(&catalog) else {
            return;
        };
        agents.register_appended_household_members();
        let front = self.destination_front(slot, allocator, transit_network, graph, &catalog);
        let service_index = ServiceVisitIndex {
            services: &front.services,
            catalog: &catalog,
            resource_runtime_id: personal_services,
            walking: self.job_funding.customers.walking_proofs(transit_network),
        };
        let mut diagnostics = std::mem::take(&mut self.visit_hour);
        diagnostics.attempts += batch.len() as u32;
        let (foot_components, car_components) = (
            self.destination_topology.foot(),
            self.destination_topology.car(),
        );
        let (households, customers) = (&self.households, &self.job_funding.customers);
        let shared_agents = &*agents;
        let shared_allocator = &*allocator;
        let plans: Vec<_> = batch
            .par_iter()
            .map_init(
                || Vec::with_capacity(SERVICE_VISIT_SEARCH_CANDIDATES),
                |candidates, &household_id| {
                    let mut diagnostics = ServiceVisitDiagnostics::default();
                    let visitor = first_eligible_shopper(
                        shared_agents,
                        household_id,
                        &households[household_id],
                    );
                    let plan = if visitor == usize::MAX {
                        diagnostics.failed_no_shopper += 1;
                        None
                    } else if !service_index.has_any() {
                        diagnostics.failed_no_service_candidates += 1;
                        None
                    } else {
                        plan_visible_service_visit(
                            household_id,
                            visitor,
                            shared_agents,
                            shared_allocator,
                            &service_index,
                            customers,
                            transit_network,
                            graph,
                            foot_components,
                            car_components,
                            candidates,
                            &mut diagnostics,
                        )
                    };
                    (plan, diagnostics)
                },
            )
            .collect();
        for (plan, plan_diagnostics) in plans {
            diagnostics.merge(plan_diagnostics);
            let Some(plan) = plan else {
                continue;
            };
            if !service_plan_still_valid(plan, &self.households, agents, allocator) {
                continue;
            }
            agents.planned_target_building[plan.visitor_agent_id] = plan.service_building_id;
            agents.planned_activity[plan.visitor_agent_id] = ACTIVITY_SHOPPING;
            // The visitor waits at home for its departure (`SIM-02-E`).
            agents.hold_departure(plan.visitor_agent_id);
            self.held_visitors
                .push(agents.agent_handle(plan.visitor_agent_id));
            diagnostics.successes += 1;
        }
        self.visit_hour = diagnostics;
        self.destinations.restore_front(front);
    }

    /// Logs and resets the hour's visit outcomes.
    pub(super) fn close_visit_hour(&mut self) {
        let diagnostics = std::mem::take(&mut self.visit_hour);
        let queue = std::mem::take(&mut self.visit_queue_stats);
        if diagnostics.has_signal() || queue.taken > 0 {
            log_service_visit_diagnostics(&queue, &diagnostics);
        }
    }

    #[cfg(test)]
    pub(super) fn run_visible_service_visits_for_test(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
    ) {
        // One hour: visits end, every drawn household's visitor leaves at once.
        for agent_idx in 0..agents.len() {
            self.end_service_visit(agent_idx, agents, allocator);
        }
        let slot = u64::from(absolute_hour) * SLOTS_PER_HOUR;
        self.queue_every_household_for_test(slot, agents, allocator);
        self.restock_requests.clear();
        self.serve_visits(slot, usize::MAX, agents, allocator, transit_network, graph);
        for visitor in std::mem::take(&mut self.held_visitors) {
            if let Some(agent_idx) = agents.resolve_agent(visitor) {
                agents.release_departure(agent_idx);
            }
        }
    }
}

fn log_service_visit_diagnostics(queue: &QueueStats, diagnostics: &ServiceVisitDiagnostics) {
    debug_log!(
        "economy",
        "visible service visit diagnostics: {} attempts={} success={} returns={} candidates={} no_service_candidates={} no_shopper={} rejected_unreachable={} rejected_outside_shortlist={}",
        queue,
        diagnostics.attempts,
        diagnostics.successes,
        diagnostics.returns_scheduled,
        diagnostics.candidate_count,
        diagnostics.failed_no_service_candidates,
        diagnostics.failed_no_shopper,
        diagnostics.rejected_unreachable,
        diagnostics.rejected_outside_shortlist
    );
}

/// The rate of personal-service visits per resident and day.
pub(super) fn service_visit_rate(catalog: &RuntimeEconomyCatalog) -> f32 {
    personal_services_resource(catalog).map_or(0.0, |resource| {
        service_visit_rate_per_resident(catalog, resource)
            * VISIBLE_PERSONAL_SERVICE_VISIT_SAMPLE_RATE
    })
}

/// The personal-services resource visible visits buy, if the catalog has it.
pub(super) fn personal_services_resource(
    catalog: &RuntimeEconomyCatalog,
) -> Option<ResourceRuntimeId> {
    catalog.resource_runtime_id_for_id(PERSONAL_SERVICE_RESOURCE_ID)
}

// Whether household `household_id` draws a visible service visit in absolute hour
// `absolute_hour`: a housed, stocked household with money and no restock pending, picked with
// its members' hourly visit probability by a stable hash of household, home and hour. O(1).
fn visit_due(
    household_id: usize,
    household: &Household,
    building_count: usize,
    absolute_hour: u32,
    visit_rate_per_resident: f32,
) -> bool {
    if visit_rate_per_resident <= 0.0
        || household.member_count == 0
        || household.home_building_id >= building_count
        || household.budget <= 0.0
        || household.stock_days == 0.0
        || !matches!(
            household.replenishment_state,
            REPLENISHMENT_STABLE | REPLENISHMENT_FULFILLED
        )
    {
        return false;
    }
    let draw = stable_service_visit_hash(
        household_id as u64,
        household.home_building_id as u64,
        u64::from(absolute_hour),
    );
    let hourly_visit_probability =
        household.member_count as f32 * visit_rate_per_resident / OPERATIONAL_HOURS_PER_DAY;
    hourly_visit_probability > 0.0
        && (hourly_visit_probability >= 1.0
            || stable_hash_unit_interval(draw) < hourly_visit_probability)
}

#[allow(clippy::too_many_arguments)]
fn plan_visible_service_visit(
    household_id: usize,
    visitor_agent_id: usize,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    service_index: &ServiceVisitIndex,
    customers: &crate::simulation::economy::customers::ConsumerMarket,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    candidates: &mut Vec<usize>,
    diagnostics: &mut ServiceVisitDiagnostics,
) -> Option<ServiceVisitPlan> {
    let has_car = agents.has_car[visitor_agent_id];
    let home_idx = agents.home_building[visitor_agent_id];
    service_index.fill_route_feasible_candidates(
        home_idx,
        customers.household_walking_access(household_id, home_idx, allocator, transit_network),
        has_car,
        allocator,
        transit_network,
        graph,
        foot_components,
        car_components,
        &agents.pathfind_count,
        candidates,
        diagnostics,
    );
    diagnostics.candidate_count += candidates.len() as u32;
    let Some(&service_building_id) = candidates.first() else {
        diagnostics.failed_no_service_candidates += 1;
        return None;
    };
    Some(ServiceVisitPlan {
        household_id,
        visitor_agent_id,
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
    agents.resolve_agent(household.shopping_agent) == Some(agent_idx)
        && matches!(
            household.replenishment_state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        )
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
