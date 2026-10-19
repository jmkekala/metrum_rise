// SPDX-License-Identifier: GPL-2.0-only

//! Household housing resolution, relocation, and eviction.
//!
//! The `housing requests` queue serves them a few per slot (`SIM-02-J`): a household's hourly
//! phase queues its rehousing while it has no usable home, and its daily phase queues its
//! housing review. Both read the allocator's live vacancy lists, rehousing by level and lowest
//! building ID, reviews by level and nearest home, so each request costs a bounded search
//! instead of a planner built over every vacancy.

use super::HouseholdSystem;
use super::metrics::{
    household_has_independent_member, household_is_housed, household_reserve_days,
    level_tuning_value,
};
use super::requests::QueueStats;
use crate::debug_log;
use crate::simulation::buildings::allocator::{Building, BuildingAllocator, ChunkBounds};
use crate::simulation::economy::accessibility::{
    chunk_for_point, min_possible_ring_distance_sq, scan_ring_chunks,
};
use crate::simulation::economy::agents::{AgentSystem, NO_MEMBER};
use crate::simulation::economy::definitions::{
    HouseholdRuntimeTuning, RuntimeEconomyCatalog, RuntimeEconomyTuning,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};

/// Outcomes of housing requests, counted over one hour of the queue or one whole pass.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct HousingResolutionDiagnostics {
    queue: QueueStats,
    rehoused: u32,
    checked_households: u32,
    housed_households: u32,
    unhoused_start_households: u32,
    stay_passed: u32,
    stay_failed: u32,
    waiting_for_eviction: u32,
    relocated_unhoused: u32,
    relocated_failed_stay: u32,
    relocated_upgrade: u32,
    evicted: u32,
    still_unhoused: u32,
}

impl HousingResolutionDiagnostics {
    fn log(&self, scope: &str) {
        debug_log!(
            "economy",
            "household housing resolution: scope={} {} rehoused={} checked={} housed={} \
             unhoused_start={} stay_ok={} stay_failed={} waiting={} relocated_unhoused={} \
             relocated_failed_stay={} relocated_upgrade={} evicted={} still_unhoused={}",
            scope,
            self.queue,
            self.rehoused,
            self.checked_households,
            self.housed_households,
            self.unhoused_start_households,
            self.stay_passed,
            self.stay_failed,
            self.waiting_for_eviction,
            self.relocated_unhoused,
            self.relocated_failed_stay,
            self.relocated_upgrade,
            self.evicted,
            self.still_unhoused,
        );
    }
}

fn max_chunk_ring(bounds: ChunkBounds, origin_chunk: (i32, i32)) -> i32 {
    let (min_chunk, max_chunk) = bounds;
    [
        (origin_chunk.0 - min_chunk.0).abs(),
        (origin_chunk.1 - min_chunk.1).abs(),
        (origin_chunk.0 - max_chunk.0).abs(),
        (origin_chunk.1 - max_chunk.1).abs(),
    ]
    .into_iter()
    .max()
    .unwrap_or(0)
}

fn squared_distance_to_building(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

// Whether a household living in `current_home` may move into listed home `building_idx`: a
// place left in a different, operational home not about to be redeveloped. O(1).
fn home_is_available(
    allocator: &BuildingAllocator,
    building_idx: usize,
    current_home: Option<usize>,
) -> bool {
    Some(building_idx) != current_home
        && allocator.buildings.get(building_idx).is_some_and(|home| {
            !home.broken
                && !home.economy_broken
                && !home.is_deserted
                && !home.is_under_construction()
                && !home.pending_redevelopment
                && allocator.household_capacity(building_idx) > home.occupancy
        })
}

// The available listed home of `level` nearest `(origin_x, origin_y)`, ties to the lower
// building ID. Scans 512 m chunk rings outward and stops once no unscanned ring can hold a
// closer home. O(scanned chunks + their homes).
fn nearest_home_in_level(
    allocator: &BuildingAllocator,
    level: u8,
    origin_x: f32,
    origin_y: f32,
    current_home: Option<usize>,
) -> Option<usize> {
    let (by_chunk, bounds) = allocator.rehousing_homes_by_chunk(level)?;
    let origin_chunk = chunk_for_point(origin_x, origin_y);
    let max_ring = max_chunk_ring(bounds, origin_chunk);
    let mut best: Option<(f32, usize)> = None;
    for ring in 0..=max_ring {
        scan_ring_chunks(origin_chunk, ring, |chunk| {
            let Some(homes) = by_chunk.get(&chunk) else {
                return;
            };
            for &building_idx in homes {
                if !home_is_available(allocator, building_idx, current_home) {
                    continue;
                }
                let distance = squared_distance_to_building(
                    origin_x,
                    origin_y,
                    &allocator.buildings[building_idx],
                );
                if best.is_none_or(|(best_distance, best_building)| {
                    distance
                        .total_cmp(&best_distance)
                        .then(building_idx.cmp(&best_building))
                        .is_lt()
                }) {
                    best = Some((distance, building_idx));
                }
            }
        });
        if let Some((best_distance, _)) = best
            && (ring == max_ring
                || best_distance
                    < min_possible_ring_distance_sq(origin_x, origin_y, origin_chunk, ring + 1))
        {
            break;
        }
    }
    best.map(|(_, building_idx)| building_idx)
}

// The home a household with `reserve_days` of reserves moves to: the highest level whose
// move-in reserve it meets, above `minimum_level_exclusive` when given; in that level the
// available home nearest `current_home` or, without a home, the lowest building ID. Reads the
// live vacancy lists, which the move itself then updates. O(levels × (scanned chunks + their
// homes)).
fn affordable_home(
    reserve_days: f32,
    allocator: &BuildingAllocator,
    current_home: Option<usize>,
    minimum_level_exclusive: Option<u8>,
    config: &HouseholdRuntimeTuning,
) -> Option<usize> {
    let current_center = current_home
        .and_then(|building_idx| allocator.buildings.get(building_idx))
        .map(|building| (building.center_x, building.center_y));
    for level in allocator.rehousing_levels_desc() {
        if minimum_level_exclusive.is_some_and(|minimum_level| level <= minimum_level) {
            break;
        }
        let move_in_threshold =
            level_tuning_value(&config.residential_move_in_min_reserve_days_by_level, level);
        if reserve_days + f32::EPSILON < move_in_threshold {
            continue;
        }
        let home = match current_center {
            Some((origin_x, origin_y)) => {
                nearest_home_in_level(allocator, level, origin_x, origin_y, current_home)
            }
            None => allocator
                .rehousing_homes(level)
                .find(|&building_idx| home_is_available(allocator, building_idx, current_home)),
        };
        if home.is_some() {
            return home;
        }
    }
    None
}

impl HouseholdSystem {
    /// One slot of the `housing requests` queue (`SIM-02-J`): up to `quota` requests from the
    /// queued rehousings and housing reviews, oldest first by queue slot and household. A
    /// request whose household left is dropped. Returns the requests taken. O(1) with nothing queued; each
    /// request costs a vacancy search, O(levels × (scanned chunks + their homes)), and moves its
    /// members, after a stale vacancy index's rebuild.
    pub(crate) fn serve_housing_requests(
        &mut self,
        quota: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> usize {
        if self.rehousing.is_empty() && self.housing_reviews.is_empty() {
            return 0;
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        // A stale index is rebuilt by its first reader, wherever that is (`SIM-02`).
        if allocator.dirty_index {
            allocator.rebuild_zone_index();
        }
        // Agents a loader or tool appended directly join their lists before housing walks them.
        agents.register_appended_household_members();
        let mut diagnostics = self.housing_diagnostics;
        let mut served = 0;
        while served < quota {
            // Both queues oldest first, merged by queue slot and household; a rehousing goes
            // before a review queued in the same slot by the same household.
            let rehousing_first = match (self.rehousing.front(), self.housing_reviews.front()) {
                (Some(rehousing), Some(review)) => rehousing <= review,
                (rehousing, _) => rehousing.is_some(),
            };
            if rehousing_first && let Some((_, handle)) = self.rehousing.pop() {
                if let Some(household_id) = self.handles.resolve(handle)
                    && self.rehouse_household(household_id, &catalog, &tuning, agents, allocator)
                {
                    diagnostics.rehoused = diagnostics.rehoused.saturating_add(1);
                }
            } else if let Some((_, handle)) = self.housing_reviews.pop() {
                if let Some(household_id) = self.handles.resolve(handle) {
                    self.review_household_housing(
                        household_id,
                        &catalog,
                        &tuning,
                        agents,
                        allocator,
                        &mut diagnostics,
                    );
                }
            } else {
                break;
            }
            served += 1;
        }
        diagnostics
            .queue
            .record_slot(served, self.rehousing.len() + self.housing_reviews.len());
        self.housing_diagnostics = diagnostics;
        served
    }

    /// Logs and resets the hour's housing outcomes, at the hour's last slot.
    pub(super) fn close_housing_hour(&mut self) {
        let diagnostics = std::mem::take(&mut self.housing_diagnostics);
        if diagnostics.queue.taken > 0 {
            diagnostics.log("hour");
        }
    }

    // Moves unhoused household `household_id` into the highest-level vacancy its reserves
    // afford, lowest building ID first. Returns whether it moved. O(levels + log V + members).
    fn rehouse_household(
        &mut self,
        household_id: usize,
        catalog: &RuntimeEconomyCatalog,
        tuning: &RuntimeEconomyTuning,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) -> bool {
        let household = &self.households[household_id];
        if household.member_count == 0
            || !household_has_independent_member(household)
            || household_is_housed(household, allocator)
        {
            return false;
        }
        let reserves = household_reserve_days(catalog, tuning, household);
        let old_home = household.home_building_id;
        let Some(home) = allocator.first_rehousing_vacancy(|level| {
            reserves + f32::EPSILON
                >= level_tuning_value(
                    &tuning
                        .households
                        .residential_move_in_min_reserve_days_by_level,
                    level,
                )
        }) else {
            return false;
        };
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
        }
        self.occupy_household_home(household_id, old_home, home, agents, allocator);
        true
    }

    /// Rehouses every unhoused household at once, in household order, as the hourly pass did
    /// before the `housing requests` queue. The common fully housed case moves nobody.
    #[cfg(test)]
    pub(super) fn rehouse_unhoused_before_growth(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if allocator.dirty_index {
            allocator.rebuild_zone_index();
        }
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        for household_id in 0..self.households.len() {
            self.rehouse_household(household_id, &catalog, &tuning, agents, allocator);
        }
    }

    /// Housing affordability, relocation and eviction of every household at once, as their
    /// daily phases would queue them.
    #[cfg(test)]
    pub(super) fn resolve_household_housing(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let config = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        if allocator.dirty_index {
            allocator.rebuild_zone_index();
        }
        let mut diagnostics = HousingResolutionDiagnostics::default();
        for household_id in 0..self.households.len() {
            self.review_household_housing(
                household_id,
                &catalog,
                &config,
                agents,
                allocator,
                &mut diagnostics,
            );
        }
        diagnostics.log("pass");
    }

    // Household `household_id`'s housing review: an unhoused household takes the best home it
    // can afford; a housed one stays, upgrades to a higher level nearby, or after failing its
    // stay reserve long enough relocates nearby or is evicted. The vacancy index is current.
    fn review_household_housing(
        &mut self,
        household_id: usize,
        catalog: &RuntimeEconomyCatalog,
        config: &RuntimeEconomyTuning,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        diagnostics: &mut HousingResolutionDiagnostics,
    ) {
        let household = &self.households[household_id];
        if household.member_count == 0 {
            return;
        }
        diagnostics.checked_households = diagnostics.checked_households.saturating_add(1);

        if !household_has_independent_member(household) {
            let current_home = household.home_building_id;
            diagnostics.unhoused_start_households =
                diagnostics.unhoused_start_households.saturating_add(1);
            if current_home < allocator.buildings.len() {
                self.evict_household(household_id, current_home, agents, allocator);
                diagnostics.evicted = diagnostics.evicted.saturating_add(1);
            }
            self.households[household_id].unhoused_days_elapsed = self.households[household_id]
                .unhoused_days_elapsed
                .saturating_add(1);
            diagnostics.still_unhoused = diagnostics.still_unhoused.saturating_add(1);
            return;
        }

        let reserve_days = household_reserve_days(catalog, config, household);
        let current_home = household.home_building_id;
        let is_housed = household_is_housed(household, allocator);

        if !is_housed {
            diagnostics.unhoused_start_households =
                diagnostics.unhoused_start_households.saturating_add(1);
            self.households[household_id].stay_failure_days = 0;
            if let Some(target_home) =
                affordable_home(reserve_days, allocator, None, None, &config.households)
            {
                self.relocate_household(household_id, current_home, target_home, agents, allocator);
                diagnostics.relocated_unhoused = diagnostics.relocated_unhoused.saturating_add(1);
            } else {
                self.households[household_id].unhoused_days_elapsed = self.households[household_id]
                    .unhoused_days_elapsed
                    .saturating_add(1);
                diagnostics.still_unhoused = diagnostics.still_unhoused.saturating_add(1);
            }
            return;
        }
        diagnostics.housed_households = diagnostics.housed_households.saturating_add(1);
        self.households[household_id].unhoused_days_elapsed = 0;

        let current_level = allocator.buildings[current_home].level;
        let stay_threshold = level_tuning_value(
            &config.households.residential_stay_min_reserve_days_by_level,
            current_level,
        );

        if reserve_days >= stay_threshold {
            diagnostics.stay_passed = diagnostics.stay_passed.saturating_add(1);
            self.households[household_id].stay_failure_days = 0;
            if let Some(target_home) = affordable_home(
                reserve_days,
                allocator,
                Some(current_home),
                Some(current_level),
                &config.households,
            ) {
                self.relocate_household(household_id, current_home, target_home, agents, allocator);
                diagnostics.relocated_upgrade = diagnostics.relocated_upgrade.saturating_add(1);
            }
            return;
        }

        diagnostics.stay_failed = diagnostics.stay_failed.saturating_add(1);
        self.households[household_id].stay_failure_days = self.households[household_id]
            .stay_failure_days
            .saturating_add(1);
        if self.households[household_id].stay_failure_days
            < config.households.stay_failure_days_before_eviction
        {
            diagnostics.waiting_for_eviction = diagnostics.waiting_for_eviction.saturating_add(1);
            return;
        }

        if let Some(target_home) = affordable_home(
            reserve_days,
            allocator,
            Some(current_home),
            None,
            &config.households,
        ) {
            self.relocate_household(household_id, current_home, target_home, agents, allocator);
            diagnostics.relocated_failed_stay = diagnostics.relocated_failed_stay.saturating_add(1);
        } else {
            self.evict_household(household_id, current_home, agents, allocator);
            diagnostics.evicted = diagnostics.evicted.saturating_add(1);
        }
    }

    fn relocate_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        new_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if household_id >= self.households.len() || new_home >= allocator.buildings.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
        }
        self.occupy_household_home(household_id, old_home, new_home, agents, allocator);
    }

    fn occupy_household_home(
        &mut self,
        household_id: usize,
        old_home: usize,
        new_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        // A reserved purchase whose shopper has not left the old home would lose its trip with
        // the move, so it is cancelled and refunded first (`SIM-02-J`).
        if self.shopper_not_left(household_id, old_home, agents) {
            self.cancel_household_replenishment(household_id, agents, allocator);
        }
        allocator.claim_vacancy(new_home);

        self.households[household_id].home_building_id = new_home;
        self.households[household_id].stay_failure_days = 0;
        self.households[household_id].unhoused_days_elapsed = 0;

        let mut agent_idx = agents.first_household_member(household_id);
        while agent_idx != NO_MEMBER {
            agents.relocate_household_member_home(
                agent_idx,
                old_home,
                new_home,
                old_home < allocator.buildings.len(),
            );
            agent_idx = agents.next_household_member(agent_idx);
        }
    }

    fn evict_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        if household_id >= self.households.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
        }

        self.cancel_household_replenishment(household_id, agents, allocator);
        let household = &mut self.households[household_id];
        household.home_building_id = usize::MAX;
        household.stay_failure_days = 0;
        household.unhoused_days_elapsed = 0;

        let mut agent_idx = agents.first_household_member(household_id);
        while agent_idx != NO_MEMBER {
            agents.evict_household_member_home(agent_idx, old_home);
            agent_idx = agents.next_household_member(agent_idx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "manual matched release timing for housing chunk bounds"]
    fn benchmark_housing_chunk_bounds() {
        use std::{hint::black_box, time::Instant};
        for ring in [8, 32, 80, 128] {
            let mut samples = Vec::new();
            for _ in 0..11 {
                let start = Instant::now();
                for _ in 0..1_000 {
                    black_box(min_possible_ring_distance_sq(
                        black_box(150.0),
                        black_box(-50.0),
                        black_box((0, -1)),
                        black_box(ring),
                    ));
                }
                samples.push(start.elapsed().as_secs_f64() * 1e6 / 1_000.0);
            }
            samples.sort_by(f64::total_cmp);
            println!(
                "housing_chunk_bounds ring={ring} median_us={:.3}",
                samples[5]
            );
        }
    }
}
