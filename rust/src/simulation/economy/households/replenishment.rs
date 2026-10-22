// SPDX-License-Identifier: GPL-2.0-only

//! Household stock consumption, shopper-carried store trips, and replenishment state.

use super::candidate_visits::CandidateVisits;
use super::data::{DailyHouseholdLedger, Household, HouseholdSystem};
use super::destinations::{DestinationIndex, store_sells_supplies};
use super::electricity::ElectricityProfiles;
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, economy_profile_for_building, household_demand_profile,
    household_supply_resource_runtime_id, household_supply_unit_price, stable_hash64, stock_days,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::core::handles::Handle;
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketIndex, ReachableBucketScanEvent,
};
use crate::simulation::economy::agents::tick::{
    BuildingWalkingAccess, PedestrianReachability, building_origin_trip_is_feasible,
    estimate_building_origin_trip_seconds,
};
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, AgentHandle, AgentSystem, NO_MEMBER, TRANSIT_IN_BUILDING,
    age_group_can_shop,
};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::economy::fiscal::{split_gross_tax, tax_amount};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const GROCERY_SEARCH_CANDIDATES: usize = 24;
const GROCERY_ROUTE_SCAN_CANDIDATES: usize = GROCERY_SEARCH_CANDIDATES * 16;
const HOUSEHOLD_UTILITY_SERVICE_COUNT: f32 = 2.0;

/// Household stock is healthy and no replenishment is pending.
pub const REPLENISHMENT_STABLE: u8 = 0;
/// Household stock fell below the trigger and needs a restock attempt.
pub const REPLENISHMENT_NEEDS: u8 = 1;
/// Household needs restock but no eligible member is currently at home to carry it.
pub const REPLENISHMENT_WAITING_FOR_SHOPPER: u8 = 2;
/// A selected household member is travelling from home to the supplying store.
pub const REPLENISHMENT_SHOPPING_TO_STORE: u8 = 3;
/// A selected household member has picked up supplies and is returning home.
pub const REPLENISHMENT_SHOPPING_RETURNING: u8 = 4;
/// Household stock was replenished on the latest economy pass.
pub const REPLENISHMENT_FULFILLED: u8 = 5;
/// Household is waiting before retrying another replenishment attempt.
pub const REPLENISHMENT_COOLDOWN: u8 = 6;
/// Household failed repeatedly and is exposed as an unresolved shortage.
pub const REPLENISHMENT_FAILED_TERMINAL: u8 = 7;

#[derive(Default)]
pub(super) struct ReplenishmentDiagnostics {
    attempts: u32,
    successes: u32,
    failed_no_store_candidates: u32,
    failed_no_shopper: u32,
    failed_no_sale: u32,
    urgent_cooldown_skips: u32,
    candidate_count: u32,
    rejected_empty: u32,
    rejected_invalid_store: u32,
    rejected_missing_profile: u32,
    rejected_not_output: u32,
    rejected_unaffordable: u32,
    rejected_unreachable: u32,
    rejected_outside_shortlist: u32,
    rejected_zero_desired: u32,
    rejected_zero_amount: u32,
    reserved_amount: f32,
    reserved_cost: f32,
}

impl ReplenishmentDiagnostics {
    /// Counts a household out of stock that waits out its cooldown before shopping again.
    pub(super) fn count_urgent_cooldown_skip(&mut self) {
        self.urgent_cooldown_skips += 1;
    }

    fn has_signal(&self) -> bool {
        self.attempts > 0 || self.failed_no_shopper > 0 || self.urgent_cooldown_skips > 0
    }

    fn merge(&mut self, other: Self) {
        self.attempts += other.attempts;
        self.successes += other.successes;
        self.failed_no_store_candidates += other.failed_no_store_candidates;
        self.failed_no_shopper += other.failed_no_shopper;
        self.failed_no_sale += other.failed_no_sale;
        self.urgent_cooldown_skips += other.urgent_cooldown_skips;
        self.candidate_count += other.candidate_count;
        self.rejected_empty += other.rejected_empty;
        self.rejected_invalid_store += other.rejected_invalid_store;
        self.rejected_missing_profile += other.rejected_missing_profile;
        self.rejected_not_output += other.rejected_not_output;
        self.rejected_unaffordable += other.rejected_unaffordable;
        self.rejected_unreachable += other.rejected_unreachable;
        self.rejected_outside_shortlist += other.rejected_outside_shortlist;
        self.rejected_zero_desired += other.rejected_zero_desired;
        self.rejected_zero_amount += other.rejected_zero_amount;
        self.reserved_amount += other.reserved_amount;
        self.reserved_cost += other.reserved_cost;
    }
}

struct ReplenishmentCandidatePlan {
    household_id: usize,
    shopper_agent_id: usize,
    desired_amount: f32,
    candidate_count: u8,
    candidates: [usize; GROCERY_SEARCH_CANDIDATES],
}

/// Tuning of one household hour, loaded once per phase batch.
struct HouseholdHourTuning {
    trigger_days: f32,
    water_sewage_cost_per_member_per_day: f32,
    household_supply_unit_price: f32,
}

impl HouseholdHourTuning {
    fn load(catalog: &RuntimeEconomyCatalog, tuning: &RuntimeEconomyTuning) -> Self {
        Self {
            trigger_days: household_demand_profile(catalog).reorder_threshold_days,
            water_sewage_cost_per_member_per_day: tuning
                .households
                .water_sewage_cost_per_member_per_day,
            household_supply_unit_price: household_supply_unit_price(catalog),
        }
    }
}

/// A cached cyclic pedestrian component proves both walking legs in O(1); the ordinary
/// planner, which always allows walking, then succeeds too. False means only unproven.
pub(super) fn proven_shopping_walk(
    walking: Option<&PedestrianReachability>,
    home: Option<&BuildingWalkingAccess>,
    store: Option<&BuildingWalkingAccess>,
) -> bool {
    let (Some(_), Some(home), Some(store)) = (walking, home, store) else {
        return false;
    };
    home.proves_to(*store) && store.proves_to(*home)
}

/// Prepares compact walking proofs; the ordinary fallback still prepares complete trip ends.
pub(super) fn prepare_walking_access(
    building: usize,
    allocator: &BuildingAllocator,
    network: &TransitNetwork,
    graph: &RegionGraph,
    walking: &PedestrianReachability,
) -> BuildingWalkingAccess {
    if building >= allocator.buildings.len() {
        return BuildingWalkingAccess::default();
    }
    allocator
        .entrances
        .get(building)
        .map_or_else(BuildingWalkingAccess::default, |entrance| {
            BuildingWalkingAccess::at_entrance(entrance, network, graph, walking)
        })
}

// One household's shopping round-trip test, shared by its candidate scans.
struct ShoppingRoutes<'a> {
    home_idx: usize,
    has_car: bool,
    walking: Option<&'a PedestrianReachability>,
    home: Option<BuildingWalkingAccess>,
    allocator: &'a BuildingAllocator,
    transit_network: &'a TransitNetwork,
    graph: &'a RegionGraph,
    pathfind_count: &'a std::sync::atomic::AtomicU32,
}

impl ShoppingRoutes<'_> {
    // Exact: the same answer as `shopping_route_is_feasible`, without a path search
    // when the walking proof holds. O(1) then; otherwise two ordinary trip plans.
    fn feasible(&self, store_idx: usize) -> bool {
        let store_access = self.walking.map(|walking| {
            prepare_walking_access(
                store_idx,
                self.allocator,
                self.transit_network,
                self.graph,
                walking,
            )
        });
        proven_shopping_walk(self.walking, self.home.as_ref(), store_access.as_ref())
            || shopping_route_is_feasible(
                self.home_idx,
                store_idx,
                self.has_car,
                self.allocator,
                self.transit_network,
                self.graph,
                self.pathfind_count,
            )
    }
}

// The stores of the hour's destination front (`households/destinations.rs`), read through
// their handles: a store left or sold out since its chunk folded is skipped.
struct StoreSupplyIndex<'a> {
    stores: &'a DestinationIndex,
    catalog: &'a RuntimeEconomyCatalog,
    household_supply_resource: u16,
}

impl StoreSupplyIndex<'_> {
    fn has_any(&self) -> bool {
        self.stores.has_any()
    }

    // The live building of entry `position` while it still sells household supplies. O(1)
    // plus the profile lookup.
    fn live_store(&self, allocator: &BuildingAllocator, position: usize) -> Option<usize> {
        let entry = self.stores.entries.get(position)?;
        let store_idx = allocator.building_handles.resolve(entry.building)?;
        store_sells_supplies(
            self.catalog,
            &allocator.buildings[store_idx],
            self.household_supply_resource,
        )
        .then_some(store_idx)
    }

    #[allow(clippy::too_many_arguments)]
    fn fill_route_feasible_candidates(
        &self,
        home_idx: usize,
        has_car: bool,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
        walking: Option<&PedestrianReachability>,
        home_access: Option<BuildingWalkingAccess>,
        pathfind_count: &std::sync::atomic::AtomicU32,
        search_cursor: u32,
        candidates: &mut Vec<usize>,
        diagnostics: &mut ReplenishmentDiagnostics,
    ) {
        candidates.clear();
        if home_idx >= allocator.buildings.len() {
            return;
        }
        let mut shortlist = ShoppingShortlist::<GROCERY_SEARCH_CANDIDATES>::default();
        let routes = ShoppingRoutes {
            home_idx,
            has_car,
            walking,
            home: home_access.or_else(|| {
                walking.map(|walking| {
                    prepare_walking_access(home_idx, allocator, transit_network, graph, walking)
                })
            }),
            allocator,
            transit_network,
            graph,
            pathfind_count,
        };
        if search_cursor > 0
            && self.fill_cursor_window_candidates(
                &routes,
                search_cursor,
                &mut shortlist,
                diagnostics,
            )
        {
            shortlist.write_to(candidates);
            return;
        }

        CandidateVisits::with(|visits| {
            visits.begin_query(self.stores.entries.len());
            // Live stores inspected, and stores passed over because they left or sold out.
            let mut inspected = [0; 2];
            let home = &allocator.buildings[home_idx];
            let home_foot_components =
                foot_components.building_components(allocator, graph, home_idx, TransitFlags::FOOT);
            self.scan_candidate_bucket(
                &self.stores.foot_buckets,
                home_foot_components,
                home.center_x,
                home.center_y,
                &routes,
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
                    &self.stores.car_buckets,
                    home_car_components,
                    home.center_x,
                    home.center_y,
                    &routes,
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
        origin_x: f32,
        origin_y: f32,
        routes: &ShoppingRoutes,
        candidates: &mut ShoppingShortlist<GROCERY_SEARCH_CANDIDATES>,
        visits: &mut CandidateVisits,
        inspected: &mut [usize; 2],
        diagnostics: &mut ReplenishmentDiagnostics,
    ) {
        let allocator = routes.allocator;
        buckets.scan_nearest(components, origin_x, origin_y, |event| match event {
            ReachableBucketScanEvent::Item { item_idx } => {
                if visits.contains(item_idx) {
                    return true;
                }
                visits.mark_seen(item_idx);
                // A store that left or sold out is no candidate; such stores have their own
                // cap, so a shortage cannot make the scan visit every entry.
                let Some(store_idx) = self.live_store(allocator, item_idx) else {
                    inspected[1] += 1;
                    return inspected[1] < GROCERY_ROUTE_SCAN_CANDIDATES;
                };
                if inspected[0] == GROCERY_ROUTE_SCAN_CANDIDATES {
                    return false;
                }
                inspected[0] += 1;
                let distance =
                    squared_building_distance(origin_x, origin_y, &allocator.buildings[store_idx]);
                let Some(position) = candidates.insertion(store_idx, distance) else {
                    diagnostics.rejected_outside_shortlist += 1;
                    return true;
                };
                if !routes.feasible(store_idx) {
                    diagnostics.rejected_unreachable += 1;
                    return true;
                }
                candidates.insert(position, store_idx, distance);
                true
            }
            ReachableBucketScanEvent::RingComplete {
                next_min_distance_sq,
            } => {
                if candidates.len() < GROCERY_SEARCH_CANDIDATES {
                    return true;
                }
                next_min_distance_sq <= candidates.worst_distance()
            }
        });
    }

    fn fill_cursor_window_candidates(
        &self,
        routes: &ShoppingRoutes,
        search_cursor: u32,
        candidates: &mut ShoppingShortlist<GROCERY_SEARCH_CANDIDATES>,
        diagnostics: &mut ReplenishmentDiagnostics,
    ) -> bool {
        let (allocator, home_idx) = (routes.allocator, routes.home_idx);
        if self.stores.entries.is_empty() {
            return false;
        }
        let total_entries = self.stores.entries.len();
        let start = search_cursor as usize % total_entries;
        let inspected = total_entries.min(GROCERY_ROUTE_SCAN_CANDIDATES);
        for offset in 0..inspected {
            let position = (start + offset) % total_entries;
            if !routes.has_car && self.stores.entries[position].foot.as_slice().is_empty() {
                continue;
            }
            let Some(store_idx) = self.live_store(allocator, position) else {
                continue;
            };
            let distance = squared_building_distance(
                allocator.buildings[home_idx].center_x,
                allocator.buildings[home_idx].center_y,
                &allocator.buildings[store_idx],
            );
            let Some(position) = candidates.insertion(store_idx, distance) else {
                diagnostics.rejected_outside_shortlist += 1;
                continue;
            };
            if !routes.feasible(store_idx) {
                diagnostics.rejected_unreachable += 1;
                continue;
            }
            candidates.insert(position, store_idx, distance);
        }
        inspected > 0
    }
}

/// Bounded exact distance/ID ordering with distances retained beside candidate IDs.
/// All query storage is on the stack; each insertion costs O(log N) comparisons and O(N)
/// small entry moves, without rereading previously considered building records.
pub(super) struct ShoppingShortlist<const N: usize> {
    entries: [(usize, f32); N],
    len: usize,
}

impl<const N: usize> Default for ShoppingShortlist<N> {
    fn default() -> Self {
        Self {
            entries: [(usize::MAX, 0.0); N],
            len: 0,
        }
    }
}

impl<const N: usize> ShoppingShortlist<N> {
    /// Number of retained candidates, including unfilled shortlists.
    pub(super) fn len(&self) -> usize {
        self.len
    }

    /// Existing ring termination uses the exact squared distance of the worst candidate.
    pub(super) fn worst_distance(&self) -> f32 {
        self.entries[..self.len]
            .last()
            .map_or(f32::MAX, |entry| entry.1)
    }

    /// Finds a unique candidate's insertion position before its route proof. The position
    /// stays valid until the next insertion; rejected route proofs do not mutate the list.
    pub(super) fn insertion(&self, candidate: usize, distance: f32) -> Option<usize> {
        if N == 0 {
            return None;
        }
        let compare = |entry: &(usize, f32)| {
            entry
                .1
                .total_cmp(&distance)
                .then_with(|| entry.0.cmp(&candidate))
        };
        if self.len == N && !compare(&self.entries[N - 1]).is_gt() {
            return None;
        }
        self.entries[..self.len].binary_search_by(compare).err()
    }

    /// Publishes one route-feasible candidate at its previously proved insertion position.
    pub(super) fn insert(&mut self, position: usize, candidate: usize, distance: f32) {
        self.len = (self.len + 1).min(N);
        self.entries
            .copy_within(position..self.len - 1, position + 1);
        self.entries[position] = (candidate, distance);
    }

    /// Copies the final IDs to the caller's retained, preallocated plan buffer.
    pub(super) fn write_to(&self, candidates: &mut Vec<usize>) {
        candidates.clear();
        candidates.extend(self.entries[..self.len].iter().map(|entry| entry.0));
    }
}

/// Test oracle: whether even a reachable candidate can enter the distance/ID shortlist.
/// Callers still count and mark inspected entries before this exact rejection, preserving
/// their mode order, scan limits and subsequent ring termination.
#[cfg(test)]
pub(super) fn shopping_candidate_can_enter(
    candidates: &[usize],
    limit: usize,
    candidate: usize,
    origin_x: f32,
    origin_y: f32,
    allocator: &BuildingAllocator,
) -> bool {
    if limit == 0 || candidate >= allocator.buildings.len() {
        return false;
    }
    if candidates.len() < limit {
        return true;
    }
    let worst = *candidates.last().unwrap();
    squared_building_distance(origin_x, origin_y, &allocator.buildings[candidate])
        .total_cmp(&squared_building_distance(
            origin_x,
            origin_y,
            &allocator.buildings[worst],
        ))
        .then_with(|| candidate.cmp(&worst))
        .is_lt()
}

/// Keeps the nearest unique shopping destinations, breaking distance ties by building ID.
#[cfg(test)]
pub(super) fn insert_shopping_candidate(
    candidates: &mut Vec<usize>,
    candidate_limit: usize,
    candidate: usize,
    origin_x: f32,
    origin_y: f32,
    allocator: &BuildingAllocator,
) {
    if candidate_limit == 0 || candidate >= allocator.buildings.len() {
        return;
    }
    let candidate_distance =
        squared_building_distance(origin_x, origin_y, &allocator.buildings[candidate]);
    let compare = |existing: &usize| {
        let existing_distance =
            squared_building_distance(origin_x, origin_y, &allocator.buildings[*existing]);
        existing_distance
            .total_cmp(&candidate_distance)
            .then_with(|| existing.cmp(&candidate))
    };
    // Most scanned stores cannot enter a full shortlist. One exact comparison rejects
    // them; retained insertions need O(log K) distance reads and at most O(K) ID moves.
    if candidates.len() == candidate_limit
        && candidates.last().is_some_and(|last| !compare(last).is_gt())
    {
        return;
    }
    let Err(insert_at) = candidates.binary_search_by(compare) else {
        return;
    };
    if candidates.len() == candidate_limit {
        // Drop the worst entry first so a full scratch buffer never needs to grow.
        candidates.pop();
    }
    candidates.insert(insert_at, candidate);
}

impl HouseholdSystem {
    /// One hour of restock and visit planning for every household at once, as their hourly
    /// phases would queue them at absolute hour `absolute_hour`, served without quota. Held
    /// trips stay held.
    #[cfg(test)]
    pub(super) fn run_household_operational_hour(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        household_vat_rate: f32,
    ) {
        let slot = u64::from(absolute_hour) * SLOTS_PER_HOUR;
        self.queue_every_household_for_test(slot, agents, allocator);
        self.serve_restock_requests(
            slot,
            usize::MAX,
            agents,
            allocator,
            transit_network,
            graph,
            household_vat_rate,
        );
        self.serve_visits(slot, usize::MAX, agents, allocator, transit_network, graph);
    }

    /// Runs the hour of every household whose hourly phase falls in this slot (`SIM-02-F`):
    /// the recount of its members, service purchases and metered electricity (`SIM-02-I`),
    /// consumption, utilities, the replenishment state's hourly transitions and the timeout of a
    /// shopping leg still under way. O(members of the households in the phase).
    pub(crate) fn run_household_phase(
        &mut self,
        phase_households: &[Handle<Household>],
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if phase_households.is_empty() {
            return;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let hour = HouseholdHourTuning::load(&catalog, &tuning);
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_failure_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;
        let electricity = ElectricityProfiles::new(&catalog);
        self.ensure_daily_ledger_len();
        agents.register_appended_household_members();
        for &handle in phase_households {
            let Some(hid) = self.handles.resolve(handle) else {
                continue;
            };
            self.recount_household(hid, agents, allocator, &catalog);
            // Services first, as the citywide sale on the hour charged before the phases did.
            self.buy_household_services(hid, allocator);
            self.record_household_electricity(&electricity, hid, allocator);
            progress_household_hour(
                &mut self.households[hid],
                &mut self.daily_ledgers[hid],
                &hour,
            );
            self.time_out_shopping_leg(
                hid,
                agents,
                allocator,
                &catalog,
                household_supply_resource,
                retry_cooldown_hours,
                terminal_failure_count,
            );
        }
    }

    #[cfg(test)]
    pub(super) fn consume_household_stock(&mut self, agents: &mut AgentSystem) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let hour = HouseholdHourTuning::load(&catalog, &tuning);
        self.ensure_daily_ledger_len();
        for (household, ledger) in self.households.iter_mut().zip(&mut self.daily_ledgers) {
            progress_household_hour(household, ledger, &hour);
        }
        let all_agents: Vec<_> = (0..agents.len()).map(|i| agents.agent_handle(i)).collect();
        self.starve_phase_agents(&all_agents, agents);
    }

    #[cfg(test)]
    pub(super) fn run_household_replenishment(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_failure_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;

        // One hour: trips arrive, legs still under way time out, new trips leave at once.
        self.advance_shopping_trips_for_test(
            agents,
            allocator,
            transit_network,
            graph,
            tuning.fiscal.household_vat_rate,
        );
        self.ensure_daily_ledger_len();
        for hid in 0..self.households.len() {
            self.time_out_shopping_leg(
                hid,
                agents,
                allocator,
                &catalog,
                household_supply_resource,
                retry_cooldown_hours,
                terminal_failure_count,
            );
        }

        let slot = u64::from(absolute_hour) * SLOTS_PER_HOUR;
        self.queue_every_household_for_test(slot, agents, allocator);
        self.visit_requests.clear();
        self.serve_restock_requests(
            slot,
            usize::MAX,
            agents,
            allocator,
            transit_network,
            graph,
            tuning.fiscal.household_vat_rate,
        );
        for household in std::mem::take(&mut self.held_shoppers) {
            if let Some(hid) = self.handles.resolve(household) {
                self.release_shopper(hid, agents, allocator);
            }
        }
    }

    /// One slot of the `restock requests` queue (`SIM-02-J`): takes up to `quota` requests,
    /// plans them in parallel against the hour's destination front and reserves them in queue
    /// order against live store stock; a reserved shopper is held for its departure. A request
    /// whose household left is dropped. Returns the requests taken. O(1) with nothing queued;
    /// a request walks its household's members and inspects at most
    /// `GROCERY_ROUTE_SCAN_CANDIDATES` stores and passes over as many that left or sold out,
    /// after a front built at once if the hour had none.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn serve_restock_requests(
        &mut self,
        slot: u64,
        quota: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        household_vat_rate: f32,
    ) -> usize {
        if self.restock_requests.is_empty() {
            return 0;
        }
        let mut batch = std::mem::take(&mut self.restock_batch);
        batch.clear();
        let mut taken = 0;
        while taken < quota {
            let Some((queued_slot, handle)) = self.restock_requests.pop() else {
                break;
            };
            taken += 1;
            if let Some(household_id) = self.handles.resolve(handle) {
                batch.push((queued_slot, household_id));
            }
        }
        if !batch.is_empty() {
            self.plan_and_reserve_restocks(
                slot,
                &batch,
                agents,
                allocator,
                transit_network,
                graph,
                household_vat_rate,
            );
        }
        self.restock_batch = batch;
        self.restock_queue_stats
            .record_slot(taken, self.restock_requests.len());
        taken
    }

    // Plans the restocks of `batch`, `(queue slot, household)` in queue order, each against the
    // hour of its queue slot, then reserves them in that order.
    #[allow(clippy::too_many_arguments)]
    fn plan_and_reserve_restocks(
        &mut self,
        slot: u64,
        batch: &[(u64, usize)],
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        household_vat_rate: f32,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let check_interval = u32::from(
            tuning
                .operational_clock
                .household_replenishment_check_interval_hours,
        );
        let profile = household_demand_profile(&catalog);
        let target_days = profile.stock_target_days;
        let trigger_days = profile.reorder_threshold_days;
        agents.register_appended_household_members();
        let front = self.destination_front(slot, allocator, transit_network, graph, &catalog);
        let store_index = StoreSupplyIndex {
            stores: &front.stores,
            catalog: &catalog,
            household_supply_resource,
        };
        let stores_exist = store_index.has_any();
        // Proofs prepared by the hourly funding census; `None` after a topology change.
        let walking = self.job_funding.customers.walking_proofs(transit_network);
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
                || Vec::with_capacity(GROCERY_SEARCH_CANDIDATES),
                |candidates, &(queued_slot, hid)| {
                    let household = &households[hid];
                    let mut diagnostics = ReplenishmentDiagnostics::default();
                    let plan = plan_household_replenishment(
                        hid,
                        household,
                        first_eligible_shopper(shared_agents, hid, household),
                        shared_agents,
                        shared_allocator,
                        &store_index,
                        customers,
                        transit_network,
                        graph,
                        foot_components,
                        car_components,
                        walking,
                        (queued_slot / SLOTS_PER_HOUR) as u32,
                        check_interval,
                        stores_exist,
                        target_days,
                        trigger_days,
                        candidates,
                        &mut diagnostics,
                    );
                    (plan, diagnostics)
                },
            )
            .collect();
        self.ensure_daily_ledger_len();
        self.handles.register_appended(self.households.len());
        let mut diagnostics = std::mem::take(&mut self.replenishment_hour);
        for (plan, plan_diagnostics) in plans {
            diagnostics.merge(plan_diagnostics);
            let Some(plan) = plan else {
                continue;
            };
            let household_id = plan.household_id;
            let reserved = apply_replenishment_plan(
                plan,
                &mut self.households,
                &mut self.daily_ledgers,
                agents,
                allocator,
                transit_network,
                graph,
                tuning.operational_clock.seconds_per_day,
                household_supply_resource,
                tuning
                    .operational_clock
                    .household_replenishment_retry_cooldown_hours,
                tuning
                    .operational_clock
                    .household_replenishment_terminal_failure_count,
                tuning
                    .operational_clock
                    .household_shopping_leg_timeout_hours,
                &catalog,
                household_vat_rate,
                &mut diagnostics,
            );
            if reserved {
                self.held_shoppers.push(self.handles.handle(household_id));
            }
        }
        self.replenishment_hour = diagnostics;
        self.destinations.restore_front(front);
    }

    /// Leaves household `household_id`'s restock waiting for a shopper: what a planned restock
    /// does when no member is free to carry it. O(1).
    pub(super) fn wait_for_shopper(&mut self, household_id: usize) {
        let household = &mut self.households[household_id];
        household.replenishment_state = REPLENISHMENT_WAITING_FOR_SHOPPER;
        household.shopping_agent = AgentHandle::NONE;
        self.replenishment_hour.failed_no_shopper += 1;
    }

    /// Logs and resets the hour's restock and visit outcomes, at the hour's last slot.
    pub(super) fn close_shopping_hour(&mut self) {
        let diagnostics = std::mem::take(&mut self.replenishment_hour);
        let queue = std::mem::take(&mut self.restock_queue_stats);
        if diagnostics.has_signal() || queue.taken > 0 {
            debug_log!(
                "economy",
                "household replenishment diagnostics: {} attempts={} success={} failed={} \
                 urgent_cooldown_skips={} candidates={} no_store_candidates={} \
                 no_shopper={} \
                 rejected_empty={} rejected_invalid_store={} rejected_missing_profile={} \
                 rejected_not_output={} rejected_unaffordable={} rejected_unreachable={} rejected_outside_shortlist={} rejected_zero_desired={} \
                 rejected_zero_amount={} reserved_amount={:.1} reserved_cost={:.1}",
                queue,
                diagnostics.attempts,
                diagnostics.successes,
                diagnostics.failed_no_sale,
                diagnostics.urgent_cooldown_skips,
                diagnostics.candidate_count,
                diagnostics.failed_no_store_candidates,
                diagnostics.failed_no_shopper,
                diagnostics.rejected_empty,
                diagnostics.rejected_invalid_store,
                diagnostics.rejected_missing_profile,
                diagnostics.rejected_not_output,
                diagnostics.rejected_unaffordable,
                diagnostics.rejected_unreachable,
                diagnostics.rejected_outside_shortlist,
                diagnostics.rejected_zero_desired,
                diagnostics.rejected_zero_amount,
                diagnostics.reserved_amount,
                diagnostics.reserved_cost
            );
        }
        self.close_visit_hour();
    }

    #[allow(clippy::too_many_arguments)]
    // A leg still under way counts one hour down its timeout, at the household's hourly phase.
    // Arrivals are taken by the trip's own checks (`SIM-02-E`), so an arrived shopper is left
    // for its next check.
    #[allow(clippy::too_many_arguments)]
    fn time_out_shopping_leg(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        household_supply_resource: u16,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
    ) {
        let state = self.households[hid].replenishment_state;
        if !matches!(
            state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        ) || self.shopping_leg_arrived(hid, agents)
        {
            return;
        }
        if state == REPLENISHMENT_SHOPPING_TO_STORE {
            let store_idx = self.households[hid].reserved_store_building_id;
            if shopping_carrier(&self.households[hid], hid, agents).is_none()
                || !valid_store_for_pickup(allocator, catalog, household_supply_resource, store_idx)
                || shopping_leg_timed_out(&mut self.households[hid])
            {
                self.cancel_replenishment_before_pickup_with_ledger(
                    hid,
                    agents,
                    allocator,
                    household_supply_resource,
                    retry_cooldown_hours,
                    terminal_failure_count,
                );
            }
        } else if shopping_carrier(&self.households[hid], hid, agents).is_none()
            || self.households[hid].home_building_id == usize::MAX
            || shopping_leg_timed_out(&mut self.households[hid])
        {
            self.cancel_replenishment_after_pickup_with_ledger(
                hid,
                agents,
                retry_cooldown_hours,
                terminal_failure_count,
            );
        } else {
            // Re-issues the trip home if a failed route dropped it.
            schedule_shopper_home(&self.households[hid], agents, ACTIVITY_SHOPPING);
        }
    }

    // Whether household `hid`'s shopper stands where its current leg ends.
    fn shopping_leg_arrived(&self, hid: usize, agents: &AgentSystem) -> bool {
        let household = &self.households[hid];
        let destination = match household.replenishment_state {
            REPLENISHMENT_SHOPPING_TO_STORE => household.reserved_store_building_id,
            REPLENISHMENT_SHOPPING_RETURNING => household.home_building_id,
            _ => return false,
        };
        shopping_carrier(household, hid, agents).is_some_and(|agent_idx| {
            agents.transit[agent_idx] == TRANSIT_IN_BUILDING
                && agents.current_building[agent_idx] == destination
        })
    }

    /// Starts household `hid`'s held shopper towards its reserved store at its paced departure
    /// slot (`SIM-02-E`). True while the trip goes on; a shopper that left is refunded.
    pub(crate) fn release_shopper(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> bool {
        if self.households[hid].replenishment_state != REPLENISHMENT_SHOPPING_TO_STORE {
            return false;
        }
        let Some(agent_idx) = shopping_carrier(&self.households[hid], hid, agents) else {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            let tuning = load_runtime_economy_tuning().unwrap_or_else(|err| {
                panic!("could not load built-in economy runtime tuning: {err}")
            });
            self.ensure_daily_ledger_len();
            self.cancel_replenishment_before_pickup_with_ledger(
                hid,
                agents,
                allocator,
                household_supply_resource_runtime_id(&catalog),
                tuning
                    .operational_clock
                    .household_replenishment_retry_cooldown_hours,
                tuning
                    .operational_clock
                    .household_replenishment_terminal_failure_count,
            );
            return false;
        };
        if agents.release_departure(agent_idx) {
            crate::debug::work::add(crate::debug::work::Work::ShopperDispatches, 1);
        }
        true
    }

    /// One check of household `hid`'s shopping trip (`SIM-02-E`): a shopper at the store picks
    /// up and heads home, one at home delivers. Returns whether the trip goes on and the
    /// household VAT collected at pickup. O(1) plus one route-cost query at pickup.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn check_shopping_trip(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        household_vat_rate: f32,
    ) -> (bool, f32) {
        let state = self.households[hid].replenishment_state;
        if !matches!(
            state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        ) {
            return (false, 0.0);
        }
        if !self.shopping_leg_arrived(hid, agents) {
            return (true, 0.0);
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let household_supply_resource = household_supply_resource_runtime_id(&catalog);
        let retry_cooldown_hours = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_failure_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;
        self.ensure_daily_ledger_len();
        if state == REPLENISHMENT_SHOPPING_TO_STORE {
            let store_idx = self.households[hid].reserved_store_building_id;
            if !valid_store_for_pickup(allocator, &catalog, household_supply_resource, store_idx) {
                self.cancel_replenishment_before_pickup_with_ledger(
                    hid,
                    agents,
                    allocator,
                    household_supply_resource,
                    retry_cooldown_hours,
                    terminal_failure_count,
                );
                return (false, 0.0);
            }
            let vat = self.pick_up_shopping(
                hid,
                agents,
                allocator,
                transit_network,
                graph,
                tuning.operational_clock.seconds_per_day,
                tuning
                    .operational_clock
                    .household_shopping_leg_timeout_hours,
                household_vat_rate,
            );
            return (true, vat);
        }
        self.deliver_shopping(hid, agents);
        (false, 0.0)
    }

    // The shopper reached the reserved store: the store is paid and the shopper heads home.
    #[allow(clippy::too_many_arguments)]
    fn pick_up_shopping(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        seconds_per_day: f64,
        shopping_leg_timeout_hours: u16,
        household_vat_rate: f32,
    ) -> f32 {
        let Some(agent_idx) = shopping_carrier(&self.households[hid], hid, agents) else {
            return 0.0;
        };
        let store_idx = self.households[hid].reserved_store_building_id;
        let gross_total_cost = self.households[hid].reserved_total_cost;
        let (store_revenue, household_vat_collected) =
            split_gross_tax(gross_total_cost, household_vat_rate);
        let store = &mut allocator.buildings[store_idx];
        store.revenue += store_revenue;
        store.operating_budget += store_revenue;
        store.daily_household_sales_value += store_revenue;
        self.households[hid].replenishment_state = REPLENISHMENT_SHOPPING_RETURNING;
        self.households[hid].shopping_timeout_hours_remaining = shopping_leg_budget_hours(
            store_idx,
            self.households[hid].home_building_id,
            agents.has_car[agent_idx],
            allocator,
            transit_network,
            graph,
            &agents.pathfind_count,
            seconds_per_day,
            shopping_leg_timeout_hours,
        );
        schedule_shopper_home(&self.households[hid], agents, ACTIVITY_SHOPPING);
        household_vat_collected
    }

    // The shopper is home with the purchase: stock rises and the task ends.
    fn deliver_shopping(&mut self, hid: usize, agents: &mut AgentSystem) {
        let household = &mut self.households[hid];
        // Release the owned task only after its cargo reaches this household. This also
        // clears a home plan reissued while the carrier was still returning.
        schedule_shopper_home(household, agents, ACTIVITY_HOME);
        household.stock += household.reserved_amount;
        household.stock_days = stock_days(
            household.stock,
            household.member_count,
            household.consumption_rate,
        );
        clear_replenishment_request(household);
        household.replenishment_failure_count = 0;
        household.replenishment_search_cursor = 0;
        household.replenishment_state = REPLENISHMENT_FULFILLED;
        household.cooldown_hours = 1;
        if let Some(ledger) = self.daily_ledgers.get_mut(hid) {
            ledger.shopper_trips_completed = ledger.shopper_trips_completed.saturating_add(1);
        }
    }

    /// Test driver of the shopping trips: releases every held shopper and checks every trip
    /// once, as the paced events would within the hour. Returns the VAT collected.
    #[cfg(test)]
    pub(super) fn advance_shopping_trips_for_test(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        household_vat_rate: f32,
    ) -> f32 {
        let mut vat = 0.0;
        for household in std::mem::take(&mut self.held_shoppers) {
            if let Some(hid) = self.handles.resolve(household) {
                self.release_shopper(hid, agents, allocator);
            }
        }
        for hid in 0..self.households.len() {
            vat += self
                .check_shopping_trip(
                    hid,
                    agents,
                    allocator,
                    transit_network,
                    graph,
                    household_vat_rate,
                )
                .1;
        }
        vat
    }

    /// Whether household `hid` has reserved a purchase whose shopper is still inside
    /// `building`, the home it set out from: its trip has not begun. O(1).
    pub(super) fn shopper_not_left(
        &self,
        hid: usize,
        building: usize,
        agents: &AgentSystem,
    ) -> bool {
        let household = &self.households[hid];
        household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
            && shopping_carrier(household, hid, agents).is_some_and(|agent_idx| {
                agents.transit[agent_idx] == TRANSIT_IN_BUILDING
                    && agents.current_building[agent_idx] == building
            })
    }

    /// Cancels an evicted household's shopping task through the normal refund and ledger path.
    pub(super) fn cancel_household_replenishment(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let state = self.households[hid].replenishment_state;
        if !matches!(
            state,
            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
        ) {
            clear_replenishment_request(&mut self.households[hid]);
            return;
        }
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        self.ensure_daily_ledger_len();
        let cooldown = tuning
            .operational_clock
            .household_replenishment_retry_cooldown_hours;
        let terminal_count = tuning
            .operational_clock
            .household_replenishment_terminal_failure_count;
        if state == REPLENISHMENT_SHOPPING_TO_STORE {
            let catalog = load_runtime_economy_catalog().unwrap_or_else(|err| {
                panic!("could not load built-in runtime economy catalog: {err}")
            });
            self.cancel_replenishment_before_pickup_with_ledger(
                hid,
                agents,
                allocator,
                household_supply_resource_runtime_id(&catalog),
                cooldown,
                terminal_count,
            );
        } else {
            self.cancel_replenishment_after_pickup_with_ledger(
                hid,
                agents,
                cooldown,
                terminal_count,
            );
        }
    }

    fn cancel_replenishment_before_pickup_with_ledger(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        household_supply_resource: u16,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
    ) {
        let refund = self.households[hid].reserved_total_cost;
        cancel_replenishment_before_pickup(
            &mut self.households[hid],
            agents,
            allocator,
            household_supply_resource,
            retry_cooldown_hours,
            terminal_failure_count,
        );
        if let Some(ledger) = self.daily_ledgers.get_mut(hid) {
            ledger.shopping_spend -= refund;
            ledger.shopper_trips_failed = ledger.shopper_trips_failed.saturating_add(1);
        }
    }

    fn cancel_replenishment_after_pickup_with_ledger(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
    ) {
        cancel_replenishment_after_pickup(
            &mut self.households[hid],
            agents,
            retry_cooldown_hours,
            terminal_failure_count,
        );
        if let Some(ledger) = self.daily_ledgers.get_mut(hid) {
            ledger.shopper_trips_failed = ledger.shopper_trips_failed.saturating_add(1);
        }
    }
}

fn record_household_utility_cost(ledger: &mut DailyHouseholdLedger, total_cost: f32) {
    let service_cost = total_cost.max(0.0) / HOUSEHOLD_UTILITY_SERVICE_COUNT;
    ledger.water_consumption_cost += service_cost;
    ledger.sewage_consumption_cost += service_cost;
}

/// Whether `household` tries to restock in absolute hour `absolute_hour`: it has members, a
/// home among `building_count` buildings and no shopping trip under way or cooldown left, its
/// stock is under `trigger_days`, and it waits for a shopper, ran out while some store sells
/// (`stores_exist`), or its check offset comes round every `check_interval` hours (the only
/// time a household that failed for good retries). O(1).
pub(super) fn restock_due(
    household: &Household,
    building_count: usize,
    absolute_hour: u32,
    check_interval: u32,
    stores_exist: bool,
    trigger_days: f32,
) -> bool {
    let check_offset_matches = absolute_hour % check_interval
        == u32::from(household.replenishment_offset_hours % check_interval as u16);
    let stock_critical_urgent = household.stock_days == 0.0 && stores_exist;
    let waiting_for_shopper = household.replenishment_state == REPLENISHMENT_WAITING_FOR_SHOPPER;
    let terminal_retry =
        household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL && check_offset_matches;
    !(household.member_count == 0
        || household.home_building_id == usize::MAX
        || household.home_building_id >= building_count
        || household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
        || household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING
        || (household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL && !terminal_retry)
        || household.cooldown_hours > 0
        || household.stock_days >= trigger_days
        || (!waiting_for_shopper
            && !terminal_retry
            && !stock_critical_urgent
            && !check_offset_matches))
}

/// The household's first member, in agent order, who could carry its shopping now: at home,
/// old enough and without another trip; `usize::MAX` for none. O(members).
pub(super) fn first_eligible_shopper(
    agents: &AgentSystem,
    household_id: usize,
    household: &Household,
) -> usize {
    let mut agent_idx = agents.first_household_member(household_id);
    while agent_idx != NO_MEMBER {
        if eligible_shopper_for_household(agents, agent_idx, household_id, household) {
            return agent_idx;
        }
        agent_idx = agents.next_household_member(agent_idx);
    }
    usize::MAX
}

#[allow(clippy::too_many_arguments)]
fn plan_household_replenishment(
    hid: usize,
    household: &Household,
    shopper_agent_id: usize,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    store_index: &StoreSupplyIndex,
    customers: &crate::simulation::economy::customers::ConsumerMarket,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    walking: Option<&PedestrianReachability>,
    absolute_hour: u32,
    check_interval: u32,
    stock_critical_purchase_available: bool,
    target_days: f32,
    trigger_days: f32,
    candidates: &mut Vec<usize>,
    diagnostics: &mut ReplenishmentDiagnostics,
) -> Option<ReplenishmentCandidatePlan> {
    if !restock_due(
        household,
        allocator.buildings.len(),
        absolute_hour,
        check_interval,
        stock_critical_purchase_available,
        trigger_days,
    ) {
        return None;
    }

    if shopper_agent_id == usize::MAX {
        return Some(ReplenishmentCandidatePlan {
            household_id: hid,
            shopper_agent_id,
            desired_amount: 0.0,
            candidate_count: 0,
            candidates: [usize::MAX; GROCERY_SEARCH_CANDIDATES],
        });
    }

    let has_car = agents.has_car[shopper_agent_id];
    store_index.fill_route_feasible_candidates(
        household.home_building_id,
        has_car,
        allocator,
        transit_network,
        graph,
        foot_components,
        car_components,
        walking,
        customers.household_walking_access(
            hid,
            household.home_building_id,
            allocator,
            transit_network,
        ),
        &agents.pathfind_count,
        household.replenishment_search_cursor,
        candidates,
        diagnostics,
    );
    diagnostics.attempts += 1;
    diagnostics.candidate_count += candidates.len() as u32;
    if candidates.is_empty() {
        diagnostics.failed_no_store_candidates += 1;
    }

    let daily_consumption = household.member_count as f32 * household.consumption_rate;
    let target_stock = target_days * daily_consumption;
    let desired_amount = (target_stock - household.stock).max(0.0);
    let mut candidate_array = [usize::MAX; GROCERY_SEARCH_CANDIDATES];
    let mut route_feasible_count = 0usize;
    for &candidate in candidates.iter() {
        if route_feasible_count == GROCERY_SEARCH_CANDIDATES {
            break;
        }
        let slot = route_feasible_count;
        candidate_array[slot] = candidate;
        route_feasible_count += 1;
    }

    Some(ReplenishmentCandidatePlan {
        household_id: hid,
        shopper_agent_id,
        desired_amount,
        candidate_count: route_feasible_count as u8,
        candidates: candidate_array,
    })
}

// One operational hour of `household`: consumption, utilities and the replenishment state's
// hourly transitions. O(1).
fn progress_household_hour(
    household: &mut Household,
    ledger: &mut DailyHouseholdLedger,
    hour: &HouseholdHourTuning,
) {
    if household.member_count == 0 {
        return;
    }
    let hourly_consumption =
        household.member_count as f32 * household.consumption_rate / OPERATIONAL_HOURS_PER_DAY;
    household.stock = (household.stock - hourly_consumption).max(0.0);
    let hourly_utility_cost = household.member_count as f32
        * hour.water_sewage_cost_per_member_per_day
        / OPERATIONAL_HOURS_PER_DAY;
    let cash_before = household.budget;
    household.budget = (household.budget - hourly_utility_cost).max(0.0);
    // Record the debit at the account's precision. Fractional hourly fees otherwise drift from
    // the ledger as household savings grow.
    let paid_utility_cost = (cash_before - household.budget).max(0.0);
    crate::debug::money::record_change(
        crate::debug::money::MoneyFlow::OutsideUtilities,
        cash_before,
        household.budget,
    );
    ledger.household_supply_consumption_cost +=
        hourly_consumption * hour.household_supply_unit_price;
    record_household_utility_cost(ledger, paid_utility_cost);
    household.stock_days = stock_days(
        household.stock,
        household.member_count,
        household.consumption_rate,
    );

    if matches!(
        household.replenishment_state,
        REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
    ) {
    } else if household.replenishment_state == REPLENISHMENT_FULFILLED {
        if household.cooldown_hours > 0 {
            household.cooldown_hours -= 1;
        }
        household.replenishment_failure_count = 0;
        household.replenishment_state = REPLENISHMENT_COOLDOWN;
    } else if matches!(
        household.replenishment_state,
        REPLENISHMENT_WAITING_FOR_SHOPPER | REPLENISHMENT_FAILED_TERMINAL
    ) {
        if household.stock_days >= hour.trigger_days {
            household.replenishment_failure_count = 0;
            household.replenishment_search_cursor = 0;
            household.replenishment_state = REPLENISHMENT_STABLE;
        }
    } else if household.cooldown_hours > 0 {
        household.cooldown_hours -= 1;
        household.replenishment_state = REPLENISHMENT_COOLDOWN;
    } else if household.stock_days < hour.trigger_days {
        household.replenishment_state = REPLENISHMENT_NEEDS;
    } else {
        household.replenishment_failure_count = 0;
        household.replenishment_search_cursor = 0;
        household.replenishment_state = REPLENISHMENT_STABLE;
    }
}

impl HouseholdSystem {
    /// An hour without supplies costs each member of an empty household some happiness, at the
    /// member's own hourly phase (`SIM-02-F`). O(agents in the phase).
    pub(crate) fn starve_phase_agents(
        &self,
        phase_agents: &[AgentHandle],
        agents: &mut AgentSystem,
    ) {
        for &handle in phase_agents {
            let Some(agent_idx) = agents.resolve_agent(handle) else {
                continue;
            };
            let hid = agents.household_id[agent_idx];
            if hid < self.households.len() && self.households[hid].stock_days == 0.0 {
                let happiness = &mut agents.agents.happiness[agent_idx];
                *happiness = (*happiness - 4.0).clamp(0.0, 100.0);
            }
        }
    }
}

// Reserves the purchase and holds the shopper for its paced departure; true when it did.
#[allow(clippy::too_many_arguments)]
fn apply_replenishment_plan(
    plan: ReplenishmentCandidatePlan,
    households: &mut [Household],
    daily_ledgers: &mut [DailyHouseholdLedger],
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    seconds_per_day: f64,
    household_supply_resource: u16,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
    shopping_leg_timeout_hours: u16,
    catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
    household_vat_rate: f32,
    diagnostics: &mut ReplenishmentDiagnostics,
) -> bool {
    let Some(household) = households.get(plan.household_id) else {
        return false;
    };
    if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
        || household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING
    {
        return false;
    }
    if !eligible_shopper_for_household(agents, plan.shopper_agent_id, plan.household_id, household)
    {
        let household = &mut households[plan.household_id];
        household.replenishment_state = REPLENISHMENT_WAITING_FOR_SHOPPER;
        household.shopping_agent = AgentHandle::NONE;
        diagnostics.failed_no_shopper += 1;
        return false;
    }

    let mut desired_amount = plan.desired_amount;
    let mut found_sale = None;
    for &candidate in plan
        .candidates
        .iter()
        .take(usize::from(plan.candidate_count))
    {
        if candidate >= allocator.buildings.len() {
            continue;
        }
        let store = &allocator.buildings[candidate];
        if store.inventory_units(household_supply_resource) <= 0.0
            || store.broken
            || store.economy_broken
            || store.is_deserted
            || store.is_under_construction()
        {
            if store.broken
                || store.economy_broken
                || store.is_deserted
                || store.is_under_construction()
            {
                diagnostics.rejected_invalid_store += 1;
            } else {
                diagnostics.rejected_empty += 1;
            }
            continue;
        }
        let Some(store_profile) = economy_profile_for_building(catalog, store) else {
            diagnostics.rejected_missing_profile += 1;
            continue;
        };
        if store_profile
            .output_port(household_supply_resource)
            .is_none()
        {
            diagnostics.rejected_not_output += 1;
            continue;
        }
        let available_stock = store.inventory_units(household_supply_resource);
        let gross_unit_price = store_profile.unit_price_currency
            + tax_amount(store_profile.unit_price_currency, household_vat_rate);
        let max_affordable_amount = if gross_unit_price > 0.0 {
            household.budget / gross_unit_price
        } else {
            f32::MAX
        };
        let amount = desired_amount
            .min(available_stock)
            .min(max_affordable_amount);
        let total_cost = amount * gross_unit_price;
        if amount > 0.0 && household.budget >= total_cost {
            found_sale = Some((candidate, amount, total_cost));
            break;
        }
        if desired_amount <= 0.0 {
            diagnostics.rejected_zero_desired += 1;
        } else if max_affordable_amount <= 0.0 || household.budget < total_cost {
            diagnostics.rejected_unaffordable += 1;
        } else {
            diagnostics.rejected_zero_amount += 1;
        }
        desired_amount = desired_amount.min(available_stock);
    }

    let household = &mut households[plan.household_id];
    if let Some((store_idx, amount, total_cost)) = found_sale {
        let leg_budget = shopping_leg_budget_hours(
            household.home_building_id,
            store_idx,
            agents.has_car[plan.shopper_agent_id],
            allocator,
            transit_network,
            graph,
            &agents.pathfind_count,
            seconds_per_day,
            shopping_leg_timeout_hours,
        );
        let store = &mut allocator.buildings[store_idx];
        store.remove_inventory_units(household_supply_resource, amount);
        household.budget -= total_cost;
        household.reserved_store_building_id = store_idx;
        household.reserved_amount = amount;
        household.reserved_total_cost = total_cost;
        household.shopping_agent = agents.agent_handle(plan.shopper_agent_id);
        household.shopping_timeout_hours_remaining = leg_budget;
        household.replenishment_search_cursor = 0;
        household.replenishment_state = REPLENISHMENT_SHOPPING_TO_STORE;
        if let Some(ledger) = daily_ledgers.get_mut(plan.household_id) {
            ledger.shopping_spend += total_cost;
        }
        agents.planned_target_building[plan.shopper_agent_id] = store_idx;
        agents.planned_activity[plan.shopper_agent_id] = ACTIVITY_SHOPPING;
        // The shopper waits at home for its paced departure (`SIM-02-E`).
        agents.hold_departure(plan.shopper_agent_id);
        diagnostics.successes += 1;
        diagnostics.reserved_amount += amount;
        diagnostics.reserved_cost += total_cost;
        true
    } else {
        register_replenishment_failure(household, retry_cooldown_hours, terminal_failure_count);
        diagnostics.failed_no_sale += 1;
        false
    }
}

pub(super) fn clear_replenishment_request(household: &mut Household) {
    household.replenishment_state = REPLENISHMENT_STABLE;
    household.cooldown_hours = 0;
    household.reserved_store_building_id = usize::MAX;
    household.reserved_amount = 0.0;
    household.reserved_total_cost = 0.0;
    household.shopping_agent = AgentHandle::NONE;
    household.shopping_timeout_hours_remaining = 0;
}

fn cancel_replenishment_before_pickup(
    household: &mut Household,
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    household_supply_resource: u16,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
) {
    let store_idx = household.reserved_store_building_id;
    if store_idx < allocator.buildings.len() && household.reserved_amount > 0.0 {
        allocator.buildings[store_idx]
            .add_inventory_units(household_supply_resource, household.reserved_amount);
    }
    household.budget += household.reserved_total_cost;
    // A shopper still held for its departure goes back to its own schedule (`SIM-02-J`).
    if let Some(agent_idx) = agents.resolve_agent(household.shopping_agent) {
        agents.release_departure(agent_idx);
    }
    schedule_shopper_home(household, agents, ACTIVITY_HOME);
    clear_replenishment_request(household);
    register_replenishment_failure(household, retry_cooldown_hours, terminal_failure_count);
}

fn cancel_replenishment_after_pickup(
    household: &mut Household,
    agents: &mut AgentSystem,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
) {
    schedule_shopper_home(household, agents, ACTIVITY_HOME);
    clear_replenishment_request(household);
    register_replenishment_failure(household, retry_cooldown_hours, terminal_failure_count);
}

pub(super) fn register_replenishment_failure(
    household: &mut Household,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
) {
    household.replenishment_failure_count = household.replenishment_failure_count.saturating_add(1);
    advance_replenishment_search_cursor(household);
    household.cooldown_hours = retry_cooldown_hours;
    household.replenishment_state =
        if household.replenishment_failure_count >= terminal_failure_count {
            household.cooldown_hours = 0;
            REPLENISHMENT_FAILED_TERMINAL
        } else {
            REPLENISHMENT_COOLDOWN
        };
}

fn advance_replenishment_search_cursor(household: &mut Household) {
    let advance = if household.replenishment_search_cursor == 0 {
        GROCERY_SEARCH_CANDIDATES
    } else {
        GROCERY_ROUTE_SCAN_CANDIDATES
    } as u32;
    household.replenishment_search_cursor =
        household.replenishment_search_cursor.wrapping_add(advance);
}

fn shopping_leg_timed_out(household: &mut Household) -> bool {
    if household.shopping_timeout_hours_remaining == 0 {
        return true;
    }
    household.shopping_timeout_hours_remaining -= 1;
    household.shopping_timeout_hours_remaining == 0
}

fn eligible_shopper_for_household(
    agents: &AgentSystem,
    agent_idx: usize,
    household_id: usize,
    household: &Household,
) -> bool {
    agent_idx < agents.len()
        && agents.household_id[agent_idx] == household_id
        && agents.home_building[agent_idx] == household.home_building_id
        && agents.current_building[agent_idx] == household.home_building_id
        && agents.transit[agent_idx] == TRANSIT_IN_BUILDING
        && agents.activity[agent_idx] == ACTIVITY_HOME
        && age_group_can_shop(agents.age_group[agent_idx])
        && agents.target_building[agent_idx] == usize::MAX
        && agents.planned_target_building[agent_idx] == usize::MAX
}

// The household's live shopper, or `None` when it left the city or the household.
fn shopping_carrier(
    household: &Household,
    household_id: usize,
    agents: &AgentSystem,
) -> Option<usize> {
    agents
        .resolve_agent(household.shopping_agent)
        .filter(|&agent_idx| agents.household_id[agent_idx] == household_id)
}

fn valid_store_for_pickup(
    allocator: &BuildingAllocator,
    catalog: &RuntimeEconomyCatalog,
    household_supply_resource: u16,
    store_idx: usize,
) -> bool {
    allocator.buildings.get(store_idx).is_some_and(|store| {
        !store.broken
            && !store.economy_broken
            && !store.is_deserted
            && !store.is_under_construction()
            && matches!(store.zone_type, ZoneType::Commercial)
            && economy_profile_for_building(catalog, store)
                .is_some_and(|profile| profile.output_port(household_supply_resource).is_some())
    })
}

// The authored timeout is delay allowance beyond the route's physical travel time.
// One bounded route-cost query per accepted leg; no per-frame queries or stored paths.
#[allow(clippy::too_many_arguments)]
fn shopping_leg_budget_hours(
    origin: usize,
    destination: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &std::sync::atomic::AtomicU32,
    seconds_per_day: f64,
    delay_allowance_hours: u16,
) -> u16 {
    let travel_seconds = estimate_building_origin_trip_seconds(
        origin,
        destination,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
        None,
    )
    .unwrap_or(0);
    let travel_hours = (f64::from(travel_seconds) * 24.0 / seconds_per_day).ceil() as u16;
    delay_allowance_hours.saturating_add(travel_hours)
}

/// Checks both the outward shopping trip and the return home through the live trip planner.
pub(super) fn shopping_route_is_feasible(
    home_idx: usize,
    store_idx: usize,
    has_car: bool,
    allocator: &BuildingAllocator,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    pathfind_count: &std::sync::atomic::AtomicU32,
) -> bool {
    building_origin_trip_is_feasible(
        home_idx,
        store_idx,
        ACTIVITY_SHOPPING,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    ) && building_origin_trip_is_feasible(
        store_idx,
        home_idx,
        ACTIVITY_HOME,
        has_car,
        allocator,
        transit_network,
        graph,
        pathfind_count,
    )
}

fn schedule_shopper_home(household: &Household, agents: &mut AgentSystem, activity: u8) {
    let Some(agent_idx) = agents.resolve_agent(household.shopping_agent) else {
        return;
    };
    if household.home_building_id == usize::MAX {
        return;
    }
    if agents.transit[agent_idx] == TRANSIT_IN_BUILDING
        && agents.current_building[agent_idx] == household.home_building_id
    {
        agents.planned_target_building[agent_idx] = usize::MAX;
        agents.planned_activity[agent_idx] = activity;
        agents.activity[agent_idx] = activity;
        return;
    }
    agents.planned_target_building[agent_idx] = household.home_building_id;
    agents.planned_activity[agent_idx] = activity;
}

/// Returns squared center distance without a square root during candidate ranking.
pub(super) fn squared_building_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

/// Stable hour of day at which a household's routine stock checks fall (`ECON-19`).
///
/// The inputs are mixed through a full finalizer: a sum of odd multiples keeps the parity of
/// `home + seed`, which put almost every sequentially admitted household in alternate hours.
pub(crate) fn stable_replenishment_offset_hours(
    home_building_id: usize,
    household_seed: u32,
) -> u16 {
    let mixed = stable_hash64(home_building_id as u64 ^ u64::from(household_seed).rotate_left(32));
    (mixed % OPERATIONAL_HOURS_PER_DAY as u64) as u16
}
