//! Household stock consumption, shopper-carried store trips, and replenishment state.

#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};

use super::data::{DailyHouseholdLedger, Household, HouseholdSystem};
use super::metrics::{
    OPERATIONAL_HOURS_PER_DAY, economy_profile_for_building, household_demand_profile,
    household_supply_resource_runtime_id, household_supply_unit_price, stock_days,
};
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::accessibility::{
    BuildingModeComponents, ModeComponentIndex, ReachableBucketEntry, ReachableBucketIndex,
    ReachableBucketScanEvent, chunk_for_point,
};
use crate::simulation::economy::agents::tick::building_origin_trip_is_feasible;
use crate::simulation::economy::agents::{
    ACTIVITY_HOME, ACTIVITY_SHOPPING, AgentSystem, TRANSIT_IN_BUILDING, age_group_can_shop,
};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;

const GROCERY_SEARCH_CANDIDATES: usize = 24;
const GROCERY_ROUTE_SCAN_CANDIDATES: usize = GROCERY_SEARCH_CANDIDATES * 16;

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
struct ReplenishmentDiagnostics {
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
    rejected_zero_desired: u32,
    rejected_zero_amount: u32,
    reserved_amount: f32,
    reserved_cost: f32,
}

impl ReplenishmentDiagnostics {
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

#[derive(Default)]
struct HouseholdHourProgress {
    any_zero_stock: bool,
    restock_candidate_exists: bool,
    urgent_restock_candidate_exists: bool,
}

impl HouseholdHourProgress {
    fn merge(&mut self, other: Self) {
        self.any_zero_stock |= other.any_zero_stock;
        self.restock_candidate_exists |= other.restock_candidate_exists;
        self.urgent_restock_candidate_exists |= other.urgent_restock_candidate_exists;
    }
}

struct StoreSupplyEntry {
    building_idx: usize,
    chunk: (i32, i32),
    foot_components: BuildingModeComponents,
    car_components: BuildingModeComponents,
}

struct StoreSupplyIndex {
    entries: Vec<StoreSupplyEntry>,
    foot_buckets: ReachableBucketIndex,
    car_buckets: ReachableBucketIndex,
}

impl StoreSupplyIndex {
    fn build(
        allocator: &BuildingAllocator,
        catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
        household_supply_resource: u16,
        graph: &RegionGraph,
        foot_components: &ModeComponentIndex,
        car_components: &ModeComponentIndex,
    ) -> Self {
        let mut entries: Vec<_> = allocator
            .buildings
            .par_iter()
            .enumerate()
            .filter_map(|(idx, store)| {
                if store.broken
                    || store.economy_broken
                    || store.is_deserted
                    || store.edge_idx == usize::MAX
                    || !matches!(store.zone_type, ZoneType::Commercial)
                    || store.inventory_units(household_supply_resource) <= 0.0
                    || !economy_profile_for_building(catalog, store).is_some_and(|profile| {
                        profile.output_port(household_supply_resource).is_some()
                    })
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
                Some(StoreSupplyEntry {
                    building_idx: idx,
                    chunk: chunk_for_point(store.center_x, store.center_y),
                    foot_components,
                    car_components,
                })
            })
            .collect();
        entries.sort_unstable_by_key(|entry| (entry.chunk, entry.building_idx));

        let mut foot_bucket_entries = Vec::with_capacity(entries.len());
        let mut car_bucket_entries = Vec::with_capacity(entries.len());
        for (entry_idx, entry) in entries.iter().enumerate() {
            index_store_components(
                &mut foot_bucket_entries,
                entry.foot_components,
                entry.chunk,
                entry_idx,
            );
            index_store_components(
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
        pathfind_count: &std::sync::atomic::AtomicU32,
        search_cursor: u32,
        candidates: &mut Vec<usize>,
        seen_candidates: &mut Vec<usize>,
        diagnostics: &mut ReplenishmentDiagnostics,
    ) {
        candidates.clear();
        seen_candidates.clear();
        if home_idx >= allocator.buildings.len() {
            return;
        }
        if search_cursor > 0
            && self.fill_cursor_window_candidates(
                home_idx,
                has_car,
                allocator,
                transit_network,
                graph,
                pathfind_count,
                search_cursor,
                candidates,
                diagnostics,
            )
        {
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
        pathfind_count: &std::sync::atomic::AtomicU32,
        candidates: &mut Vec<usize>,
        seen_candidates: &mut Vec<usize>,
        diagnostics: &mut ReplenishmentDiagnostics,
    ) {
        buckets.scan_nearest(components, origin_x, origin_y, |event| match event {
            ReachableBucketScanEvent::Item { item_idx } => {
                if let Some(entry) = self.entries.get(item_idx) {
                    if seen_candidates.contains(&entry.building_idx) {
                        return true;
                    }
                    if seen_candidates.len() == GROCERY_ROUTE_SCAN_CANDIDATES {
                        return false;
                    }
                    seen_candidates.push(entry.building_idx);
                    if !shopping_route_is_feasible(
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
                    insert_store_candidate(
                        candidates,
                        GROCERY_SEARCH_CANDIDATES,
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
                if candidates.len() < GROCERY_SEARCH_CANDIDATES {
                    return true;
                }
                let worst = candidates
                    .last()
                    .map(|&idx| {
                        squared_store_distance(origin_x, origin_y, &allocator.buildings[idx])
                    })
                    .unwrap_or(f32::MAX);
                next_min_distance_sq <= worst
            }
        });
    }

    fn fill_cursor_window_candidates(
        &self,
        home_idx: usize,
        has_car: bool,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        pathfind_count: &std::sync::atomic::AtomicU32,
        search_cursor: u32,
        candidates: &mut Vec<usize>,
        diagnostics: &mut ReplenishmentDiagnostics,
    ) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let total_entries = self.entries.len();
        let start = search_cursor as usize % total_entries;
        let inspected = total_entries.min(GROCERY_ROUTE_SCAN_CANDIDATES);
        for offset in 0..inspected {
            let entry = &self.entries[(start + offset) % total_entries];
            if !has_car && entry.foot_components.as_slice().is_empty() {
                continue;
            }
            if !shopping_route_is_feasible(
                home_idx,
                entry.building_idx,
                has_car,
                allocator,
                transit_network,
                graph,
                pathfind_count,
            ) {
                diagnostics.rejected_unreachable += 1;
                continue;
            }
            insert_store_candidate(
                candidates,
                GROCERY_SEARCH_CANDIDATES,
                entry.building_idx,
                allocator.buildings[home_idx].center_x,
                allocator.buildings[home_idx].center_y,
                allocator,
            );
        }
        inspected > 0
    }
}

fn index_store_components(
    target: &mut Vec<ReachableBucketEntry>,
    components: BuildingModeComponents,
    chunk: (i32, i32),
    entry_idx: usize,
) {
    for &component in components.as_slice() {
        target.push(ReachableBucketEntry::new(component, chunk, entry_idx));
    }
}

fn insert_store_candidate(
    candidates: &mut Vec<usize>,
    candidate_limit: usize,
    candidate: usize,
    origin_x: f32,
    origin_y: f32,
    allocator: &BuildingAllocator,
) {
    if candidate >= allocator.buildings.len() {
        return;
    }
    if candidates.contains(&candidate) {
        return;
    }
    let candidate_distance =
        squared_store_distance(origin_x, origin_y, &allocator.buildings[candidate]);
    let mut insert_at = 0usize;
    while insert_at < candidates.len() {
        let existing = candidates[insert_at];
        let existing_distance =
            squared_store_distance(origin_x, origin_y, &allocator.buildings[existing]);
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

impl HouseholdSystem {
    pub(super) fn run_household_operational_hour(
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
        let shopping_leg_timeout_hours = tuning
            .operational_clock
            .household_shopping_leg_timeout_hours;
        let profile = household_demand_profile(&catalog);
        let trigger_days = profile.reorder_threshold_days;
        let household_supply_unit_price = household_supply_unit_price(&catalog);

        let progress = self.progress_households_for_operational_hour(
            allocator.buildings.len(),
            trigger_days,
            tuning.households.utility_cost_per_member_per_day,
            household_supply_unit_price,
        );
        if progress.any_zero_stock {
            apply_starvation_happiness_loss(&self.households, agents);
        }

        self.process_active_household_shopping(
            agents,
            allocator,
            &catalog,
            household_supply_resource,
            retry_cooldown_hours,
            terminal_failure_count,
            shopping_leg_timeout_hours,
        );

        self.plan_and_apply_household_replenishment(
            agents,
            allocator,
            transit_network,
            graph,
            absolute_hour,
            &catalog,
            &tuning,
            household_supply_resource,
            progress.restock_candidate_exists,
            progress.urgent_restock_candidate_exists,
        );
    }

    fn progress_households_for_operational_hour(
        &mut self,
        building_count: usize,
        trigger_days: f32,
        utility_cost_per_member_per_day: f32,
        household_supply_unit_price: f32,
    ) -> HouseholdHourProgress {
        self.ensure_daily_ledger_len();
        let households = &mut self.households;
        let daily_ledgers = &mut self.daily_ledgers;
        households
            .par_iter_mut()
            .zip(daily_ledgers.par_iter_mut())
            .enumerate()
            .fold(
                HouseholdHourProgress::default,
                |mut progress, (_, (household, ledger))| {
                    if household.member_count == 0 {
                        return progress;
                    }

                    let hourly_consumption = household.member_count as f32
                        * household.consumption_rate
                        / OPERATIONAL_HOURS_PER_DAY;
                    household.stock = (household.stock - hourly_consumption).max(0.0);
                    let hourly_utility_cost = household.member_count as f32
                        * utility_cost_per_member_per_day
                        / OPERATIONAL_HOURS_PER_DAY;
                    household.budget = (household.budget - hourly_utility_cost).max(0.0);
                    ledger.utility_stock_consumption_cost +=
                        hourly_utility_cost + hourly_consumption * household_supply_unit_price;
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
                    } else if household.replenishment_state == REPLENISHMENT_WAITING_FOR_SHOPPER {
                        if household.stock_days >= trigger_days {
                            household.replenishment_failure_count = 0;
                            household.replenishment_search_cursor = 0;
                            household.replenishment_state = REPLENISHMENT_STABLE;
                        }
                    } else if household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL {
                        if household.stock_days >= trigger_days {
                            household.replenishment_failure_count = 0;
                            household.replenishment_search_cursor = 0;
                            household.replenishment_state = REPLENISHMENT_STABLE;
                        }
                    } else if household.cooldown_hours > 0 {
                        household.cooldown_hours -= 1;
                        household.replenishment_state = REPLENISHMENT_COOLDOWN;
                    } else if household.stock_days < trigger_days {
                        household.replenishment_state = REPLENISHMENT_NEEDS;
                    } else {
                        household.replenishment_failure_count = 0;
                        household.replenishment_search_cursor = 0;
                        household.replenishment_state = REPLENISHMENT_STABLE;
                    }

                    if household.stock_days == 0.0 {
                        progress.any_zero_stock = true;
                    }
                    if household.stock_days < trigger_days
                        && household.home_building_id < building_count
                        && !matches!(
                            household.replenishment_state,
                            REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
                        )
                    {
                        progress.restock_candidate_exists = true;
                        if household.stock_days == 0.0 {
                            progress.urgent_restock_candidate_exists = true;
                        }
                    }
                    progress
                },
            )
            .reduce(HouseholdHourProgress::default, |mut left, right| {
                left.merge(right);
                left
            })
    }

    #[cfg(test)]
    pub(super) fn consume_household_stock(&mut self, agents: &mut AgentSystem) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let trigger_days = household_demand_profile(&catalog).reorder_threshold_days;
        let supply_unit_price = household_supply_unit_price(&catalog);
        let any_zero_stock = AtomicBool::new(false);
        self.ensure_daily_ledger_len();
        self.households
            .par_iter_mut()
            .zip(self.daily_ledgers.par_iter_mut())
            .for_each(|(household, ledger)| {
                if household.member_count == 0 {
                    return;
                }
                let hourly_consumption = household.member_count as f32 * household.consumption_rate
                    / OPERATIONAL_HOURS_PER_DAY;
                household.stock = (household.stock - hourly_consumption).max(0.0);
                let hourly_utility_cost = household.member_count as f32
                    * tuning.households.utility_cost_per_member_per_day
                    / OPERATIONAL_HOURS_PER_DAY;
                household.budget = (household.budget - hourly_utility_cost).max(0.0);
                ledger.utility_stock_consumption_cost +=
                    hourly_utility_cost + hourly_consumption * supply_unit_price;
                household.stock_days = stock_days(
                    household.stock,
                    household.member_count,
                    household.consumption_rate,
                );
                if matches!(
                    household.replenishment_state,
                    REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
                ) {
                    return;
                } else if household.replenishment_state == REPLENISHMENT_FULFILLED {
                    if household.cooldown_hours > 0 {
                        household.cooldown_hours -= 1;
                    }
                    household.replenishment_failure_count = 0;
                    household.replenishment_state = REPLENISHMENT_COOLDOWN;
                } else if household.replenishment_state == REPLENISHMENT_WAITING_FOR_SHOPPER {
                    if household.stock_days >= trigger_days {
                        household.replenishment_failure_count = 0;
                        household.replenishment_search_cursor = 0;
                        household.replenishment_state = REPLENISHMENT_STABLE;
                    }
                } else if household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL {
                    if household.stock_days >= trigger_days {
                        household.replenishment_failure_count = 0;
                        household.replenishment_search_cursor = 0;
                        household.replenishment_state = REPLENISHMENT_STABLE;
                    }
                } else if household.cooldown_hours > 0 {
                    household.cooldown_hours -= 1;
                    household.replenishment_state = REPLENISHMENT_COOLDOWN;
                } else if household.stock_days < trigger_days {
                    household.replenishment_state = REPLENISHMENT_NEEDS;
                } else {
                    household.replenishment_failure_count = 0;
                    household.replenishment_search_cursor = 0;
                    household.replenishment_state = REPLENISHMENT_STABLE;
                }

                if household.stock_days == 0.0 {
                    any_zero_stock.store(true, Ordering::Relaxed);
                }
            });

        if any_zero_stock.load(Ordering::Relaxed) {
            apply_starvation_happiness_loss(&self.households, agents);
        }
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
        let shopping_leg_timeout_hours = tuning
            .operational_clock
            .household_shopping_leg_timeout_hours;

        self.process_active_household_shopping(
            agents,
            allocator,
            &catalog,
            household_supply_resource,
            retry_cooldown_hours,
            terminal_failure_count,
            shopping_leg_timeout_hours,
        );

        let profile = household_demand_profile(&catalog);
        let trigger_days = profile.reorder_threshold_days;
        let restock_candidate_exists = self.households.par_iter().any(|household| {
            household.member_count > 0
                && household.stock_days < trigger_days
                && household.home_building_id < allocator.buildings.len()
                && !matches!(
                    household.replenishment_state,
                    REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
                )
        });
        if !restock_candidate_exists {
            return;
        }
        let urgent_restock_candidate_exists = self.households.par_iter().any(|household| {
            household.member_count > 0
                && household.stock_days == 0.0
                && household.home_building_id < allocator.buildings.len()
                && !matches!(
                    household.replenishment_state,
                    REPLENISHMENT_SHOPPING_TO_STORE | REPLENISHMENT_SHOPPING_RETURNING
                )
        });
        self.plan_and_apply_household_replenishment(
            agents,
            allocator,
            transit_network,
            graph,
            absolute_hour,
            &catalog,
            &tuning,
            household_supply_resource,
            restock_candidate_exists,
            urgent_restock_candidate_exists,
        );
    }

    fn plan_and_apply_household_replenishment(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        absolute_hour: u32,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        household_supply_resource: u16,
        restock_candidate_exists: bool,
        urgent_restock_candidate_exists: bool,
    ) {
        if !restock_candidate_exists {
            return;
        }
        let check_interval = u32::from(
            tuning
                .operational_clock
                .household_replenishment_check_interval_hours,
        );
        let profile = household_demand_profile(catalog);
        let target_days = profile.stock_target_days;
        let trigger_days = profile.reorder_threshold_days;
        let foot_components = ModeComponentIndex::build(graph, TransitFlags::FOOT);
        let car_components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        let store_index = StoreSupplyIndex::build(
            allocator,
            catalog,
            household_supply_resource,
            graph,
            &foot_components,
            &car_components,
        );
        let stock_critical_purchase_available =
            urgent_restock_candidate_exists && store_index.has_any();
        let shopper_candidates = self.collect_eligible_shopper_candidates(agents);
        let (mut plans, mut diagnostics) = self
            .households
            .par_iter()
            .enumerate()
            .fold(
                || {
                    (
                        Vec::new(),
                        ReplenishmentDiagnostics::default(),
                        Vec::with_capacity(GROCERY_SEARCH_CANDIDATES),
                        Vec::with_capacity(GROCERY_ROUTE_SCAN_CANDIDATES),
                    )
                },
                |(mut plans, mut diagnostics, mut candidates, mut seen_candidates),
                 (hid, household)| {
                    if let Some(plan) = plan_household_replenishment(
                        hid,
                        household,
                        shopper_candidates[hid],
                        agents,
                        allocator,
                        &store_index,
                        transit_network,
                        graph,
                        &foot_components,
                        &car_components,
                        absolute_hour,
                        check_interval,
                        stock_critical_purchase_available,
                        target_days,
                        trigger_days,
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
                || (Vec::new(), ReplenishmentDiagnostics::default()),
                |mut left, right| {
                    left.0.extend(right.0);
                    left.1.merge(right.1);
                    left
                },
            );
        plans.sort_unstable_by_key(|plan| plan.household_id);
        self.ensure_daily_ledger_len();
        for plan in plans {
            apply_replenishment_plan(
                plan,
                &mut self.households,
                &mut self.daily_ledgers,
                agents,
                allocator,
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
                catalog,
                &mut diagnostics,
            );
        }

        if diagnostics.has_signal() {
            debug_log!(
                "economy",
                "household replenishment diagnostics: hour={} attempts={} success={} failed={} \
                 urgent_cooldown_skips={} candidates={} no_store_candidates={} \
                 no_shopper={} \
                 rejected_empty={} rejected_invalid_store={} rejected_missing_profile={} \
                 rejected_not_output={} rejected_unaffordable={} rejected_unreachable={} rejected_zero_desired={} \
                 rejected_zero_amount={} reserved_amount={:.1} reserved_cost={:.1}",
                absolute_hour,
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
                diagnostics.rejected_zero_desired,
                diagnostics.rejected_zero_amount,
                diagnostics.reserved_amount,
                diagnostics.reserved_cost
            );
        }
    }

    fn collect_eligible_shopper_candidates(&mut self, agents: &AgentSystem) -> Vec<usize> {
        use std::sync::atomic::Ordering;

        self.reset_shopper_candidate_scratch();
        let scratch = &self.shopper_candidate_scratch;
        let households = &self.households;
        (0..agents.len()).into_par_iter().for_each(|agent_idx| {
            let household_id = agents.household_id[agent_idx];
            if household_id >= households.len()
                || agents.transit[agent_idx] != TRANSIT_IN_BUILDING
                || agents.activity[agent_idx] != ACTIVITY_HOME
                || !age_group_can_shop(agents.age_group[agent_idx])
                || agents.planned_target_building[agent_idx] != usize::MAX
                || agents.target_building[agent_idx] != usize::MAX
            {
                return;
            }
            let household = &households[household_id];
            if agents.current_building[agent_idx] != household.home_building_id
                || agents.home_building[agent_idx] != household.home_building_id
            {
                return;
            }
            let slot = &scratch[household_id];
            let mut current = slot.load(Ordering::Relaxed);
            while agent_idx < current {
                match slot.compare_exchange_weak(
                    current,
                    agent_idx,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(next) => current = next,
                }
            }
        });
        scratch
            .iter()
            .map(|slot| slot.load(Ordering::Relaxed))
            .collect()
    }

    fn process_active_household_shopping(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        household_supply_resource: u16,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
        shopping_leg_timeout_hours: u16,
    ) {
        self.ensure_daily_ledger_len();
        for hid in 0..self.households.len() {
            match self.households[hid].replenishment_state {
                REPLENISHMENT_SHOPPING_TO_STORE => self.process_shopping_to_store(
                    hid,
                    agents,
                    allocator,
                    catalog,
                    household_supply_resource,
                    retry_cooldown_hours,
                    terminal_failure_count,
                    shopping_leg_timeout_hours,
                ),
                REPLENISHMENT_SHOPPING_RETURNING => self.process_shopping_returning(
                    hid,
                    agents,
                    retry_cooldown_hours,
                    terminal_failure_count,
                ),
                _ => {}
            }
        }
    }

    fn process_shopping_to_store(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        catalog: &RuntimeEconomyCatalog,
        household_supply_resource: u16,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
        shopping_leg_timeout_hours: u16,
    ) {
        if !shopping_carrier_matches(&self.households[hid], hid, agents) {
            self.cancel_replenishment_before_pickup_with_ledger(
                hid,
                agents,
                allocator,
                household_supply_resource,
                retry_cooldown_hours,
                terminal_failure_count,
            );
            return;
        }
        let store_idx = self.households[hid].reserved_store_building_id;
        if !valid_store_for_pickup(allocator, catalog, household_supply_resource, store_idx) {
            self.cancel_replenishment_before_pickup_with_ledger(
                hid,
                agents,
                allocator,
                household_supply_resource,
                retry_cooldown_hours,
                terminal_failure_count,
            );
            return;
        }
        let agent_idx = self.households[hid].shopping_agent_id;
        if agents.transit[agent_idx] == TRANSIT_IN_BUILDING
            && agents.current_building[agent_idx] == store_idx
        {
            let total_cost = self.households[hid].reserved_total_cost;
            let store = &mut allocator.buildings[store_idx];
            store.revenue += total_cost;
            store.operating_budget += total_cost;
            self.households[hid].replenishment_state = REPLENISHMENT_SHOPPING_RETURNING;
            self.households[hid].shopping_timeout_hours_remaining = shopping_leg_timeout_hours;
            schedule_shopper_home(&self.households[hid], agents);
        } else if shopping_leg_timed_out(&mut self.households[hid]) {
            self.cancel_replenishment_before_pickup_with_ledger(
                hid,
                agents,
                allocator,
                household_supply_resource,
                retry_cooldown_hours,
                terminal_failure_count,
            );
        }
    }

    fn process_shopping_returning(
        &mut self,
        hid: usize,
        agents: &mut AgentSystem,
        retry_cooldown_hours: u16,
        terminal_failure_count: u16,
    ) {
        if !shopping_carrier_matches(&self.households[hid], hid, agents) {
            self.cancel_replenishment_after_pickup_with_ledger(
                hid,
                agents,
                retry_cooldown_hours,
                terminal_failure_count,
            );
            return;
        }
        let agent_idx = self.households[hid].shopping_agent_id;
        let home_idx = self.households[hid].home_building_id;
        if home_idx == usize::MAX {
            self.cancel_replenishment_after_pickup_with_ledger(
                hid,
                agents,
                retry_cooldown_hours,
                terminal_failure_count,
            );
            return;
        }
        if agents.transit[agent_idx] == TRANSIT_IN_BUILDING
            && agents.current_building[agent_idx] == home_idx
        {
            let household = &mut self.households[hid];
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
        } else if shopping_leg_timed_out(&mut self.households[hid]) {
            self.cancel_replenishment_after_pickup_with_ledger(
                hid,
                agents,
                retry_cooldown_hours,
                terminal_failure_count,
            );
        } else {
            schedule_shopper_home(&self.households[hid], agents);
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

fn plan_household_replenishment(
    hid: usize,
    household: &Household,
    shopper_agent_id: usize,
    agents: &AgentSystem,
    allocator: &BuildingAllocator,
    store_index: &StoreSupplyIndex,
    transit_network: &TransitNetwork,
    graph: &RegionGraph,
    foot_components: &ModeComponentIndex,
    car_components: &ModeComponentIndex,
    absolute_hour: u32,
    check_interval: u32,
    stock_critical_purchase_available: bool,
    target_days: f32,
    trigger_days: f32,
    candidates: &mut Vec<usize>,
    seen_candidates: &mut Vec<usize>,
    diagnostics: &mut ReplenishmentDiagnostics,
) -> Option<ReplenishmentCandidatePlan> {
    let check_offset_matches = absolute_hour % check_interval
        == u32::from(household.replenishment_offset_hours % check_interval as u16);
    let stock_critical_urgent = household.stock_days == 0.0 && stock_critical_purchase_available;
    let waiting_for_shopper = household.replenishment_state == REPLENISHMENT_WAITING_FOR_SHOPPER;
    let terminal_retry =
        household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL && check_offset_matches;
    if stock_critical_urgent && household.cooldown_hours > 0 {
        diagnostics.urgent_cooldown_skips += 1;
    }
    if household.member_count == 0
        || household.home_building_id == usize::MAX
        || household.home_building_id >= allocator.buildings.len()
        || household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
        || household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING
        || (household.replenishment_state == REPLENISHMENT_FAILED_TERMINAL && !terminal_retry)
        || household.cooldown_hours > 0
        || household.stock_days >= trigger_days
        || (!waiting_for_shopper
            && !terminal_retry
            && !stock_critical_urgent
            && !check_offset_matches)
    {
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
        &agents.pathfind_count,
        household.replenishment_search_cursor,
        candidates,
        seen_candidates,
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

fn apply_starvation_happiness_loss(households: &[Household], agents: &mut AgentSystem) {
    let household_id = &agents.agents.household_id;
    agents
        .agents
        .happiness
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, happiness)| {
            let hid = household_id[i];
            if hid < households.len() && households[hid].stock_days == 0.0 {
                *happiness = (*happiness - 4.0).clamp(0.0, 100.0);
            }
        });
}

fn apply_replenishment_plan(
    plan: ReplenishmentCandidatePlan,
    households: &mut [Household],
    daily_ledgers: &mut [DailyHouseholdLedger],
    agents: &mut AgentSystem,
    allocator: &mut BuildingAllocator,
    household_supply_resource: u16,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
    shopping_leg_timeout_hours: u16,
    catalog: &crate::simulation::economy::definitions::RuntimeEconomyCatalog,
    diagnostics: &mut ReplenishmentDiagnostics,
) {
    let Some(household) = households.get(plan.household_id) else {
        return;
    };
    if household.replenishment_state == REPLENISHMENT_SHOPPING_TO_STORE
        || household.replenishment_state == REPLENISHMENT_SHOPPING_RETURNING
    {
        return;
    }
    if !eligible_shopper_for_household(agents, plan.shopper_agent_id, plan.household_id, household)
    {
        let household = &mut households[plan.household_id];
        household.replenishment_state = REPLENISHMENT_WAITING_FOR_SHOPPER;
        household.shopping_agent_id = usize::MAX;
        household.shopping_agent_schedule_seed = 0;
        diagnostics.failed_no_shopper += 1;
        return;
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
        {
            if store.broken || store.economy_broken || store.is_deserted {
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
        let max_affordable_amount = if store_profile.unit_price_currency > 0.0 {
            household.budget / store_profile.unit_price_currency
        } else {
            f32::MAX
        };
        let amount = desired_amount
            .min(available_stock)
            .min(max_affordable_amount);
        let total_cost = amount * store_profile.unit_price_currency;
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
        let store = &mut allocator.buildings[store_idx];
        store.remove_inventory_units(household_supply_resource, amount);
        household.budget -= total_cost;
        household.reserved_store_building_id = store_idx;
        household.reserved_amount = amount;
        household.reserved_total_cost = total_cost;
        household.shopping_agent_id = plan.shopper_agent_id;
        household.shopping_agent_schedule_seed = agents.schedule_seed[plan.shopper_agent_id];
        household.shopping_timeout_hours_remaining = shopping_leg_timeout_hours;
        household.replenishment_search_cursor = 0;
        household.replenishment_state = REPLENISHMENT_SHOPPING_TO_STORE;
        if let Some(ledger) = daily_ledgers.get_mut(plan.household_id) {
            ledger.shopping_spend += total_cost;
        }
        agents.planned_target_building[plan.shopper_agent_id] = store_idx;
        agents.planned_activity[plan.shopper_agent_id] = ACTIVITY_SHOPPING;
        diagnostics.successes += 1;
        diagnostics.reserved_amount += amount;
        diagnostics.reserved_cost += total_cost;
    } else {
        register_replenishment_failure(household, retry_cooldown_hours, terminal_failure_count);
        diagnostics.failed_no_sale += 1;
    }
}

pub(super) fn clear_replenishment_request(household: &mut Household) {
    household.replenishment_state = REPLENISHMENT_STABLE;
    household.cooldown_hours = 0;
    household.reserved_store_building_id = usize::MAX;
    household.reserved_amount = 0.0;
    household.reserved_total_cost = 0.0;
    household.shopping_agent_id = usize::MAX;
    household.shopping_agent_schedule_seed = 0;
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
    schedule_shopper_home(household, agents);
    clear_replenishment_request(household);
    register_replenishment_failure(household, retry_cooldown_hours, terminal_failure_count);
}

fn cancel_replenishment_after_pickup(
    household: &mut Household,
    agents: &mut AgentSystem,
    retry_cooldown_hours: u16,
    terminal_failure_count: u16,
) {
    schedule_shopper_home(household, agents);
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

fn shopping_carrier_matches(
    household: &Household,
    household_id: usize,
    agents: &AgentSystem,
) -> bool {
    let agent_idx = household.shopping_agent_id;
    agent_idx < agents.len()
        && agents.household_id[agent_idx] == household_id
        && agents.schedule_seed[agent_idx] == household.shopping_agent_schedule_seed
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
            && matches!(store.zone_type, ZoneType::Commercial)
            && economy_profile_for_building(catalog, store)
                .is_some_and(|profile| profile.output_port(household_supply_resource).is_some())
    })
}

fn shopping_route_is_feasible(
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

fn schedule_shopper_home(household: &Household, agents: &mut AgentSystem) {
    let agent_idx = household.shopping_agent_id;
    if agent_idx >= agents.len()
        || agents.schedule_seed[agent_idx] != household.shopping_agent_schedule_seed
        || household.home_building_id == usize::MAX
    {
        return;
    }
    if agents.transit[agent_idx] == TRANSIT_IN_BUILDING
        && agents.current_building[agent_idx] == household.home_building_id
    {
        agents.planned_target_building[agent_idx] = usize::MAX;
        agents.planned_activity[agent_idx] = ACTIVITY_HOME;
        return;
    }
    agents.planned_target_building[agent_idx] = household.home_building_id;
    agents.planned_activity[agent_idx] = ACTIVITY_HOME;
}

fn squared_store_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

pub(super) fn stable_replenishment_offset_hours(
    home_building_id: usize,
    household_seed: u32,
) -> u16 {
    let mixed = (home_building_id as u64)
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(u64::from(household_seed).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    (mixed % OPERATIONAL_HOURS_PER_DAY as u64) as u16
}
