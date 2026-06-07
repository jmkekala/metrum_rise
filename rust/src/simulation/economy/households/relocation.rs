//! Household housing resolution, relocation, and eviction.

use super::HouseholdSystem;
use super::metrics::{household_is_housed, household_reserve_days, level_tuning_value};
use super::replenishment::clear_replenishment_request;
use crate::debug_log;
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::zoning::ZoneType;

#[derive(Default)]
struct HousingResolutionDiagnostics {
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

/// Explicit household runtime record anchored to a residential building.

impl HouseholdSystem {
    pub(super) fn resolve_household_housing(
        &mut self,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let config = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));

        let mut diagnostics = HousingResolutionDiagnostics::default();
        for household_id in 0..self.households.len() {
            let household = &self.households[household_id];
            if household.member_count == 0 {
                continue;
            }
            diagnostics.checked_households = diagnostics.checked_households.saturating_add(1);

            let reserve_days = household_reserve_days(&catalog, &config, household);
            let current_home = household.home_building_id;
            let is_housed = household_is_housed(household, allocator);

            if !is_housed {
                diagnostics.unhoused_start_households =
                    diagnostics.unhoused_start_households.saturating_add(1);
                self.households[household_id].stay_failure_days = 0;
                if let Some(target_home) = self.find_affordable_home_for_household(
                    household_id,
                    reserve_days,
                    allocator,
                    None,
                    &config.households,
                ) {
                    self.relocate_household(
                        household_id,
                        usize::MAX,
                        target_home,
                        agents,
                        allocator,
                    );
                    diagnostics.relocated_unhoused =
                        diagnostics.relocated_unhoused.saturating_add(1);
                } else {
                    self.households[household_id].unhoused_days_elapsed = self.households
                        [household_id]
                        .unhoused_days_elapsed
                        .saturating_add(1);
                    diagnostics.still_unhoused = diagnostics.still_unhoused.saturating_add(1);
                }
                continue;
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
                if let Some(target_home) = self.find_affordable_home_for_household(
                    household_id,
                    reserve_days,
                    allocator,
                    Some(current_home),
                    &config.households,
                ) && allocator.buildings[target_home].level > current_level
                {
                    self.relocate_household(
                        household_id,
                        current_home,
                        target_home,
                        agents,
                        allocator,
                    );
                    diagnostics.relocated_upgrade = diagnostics.relocated_upgrade.saturating_add(1);
                }
                continue;
            }

            diagnostics.stay_failed = diagnostics.stay_failed.saturating_add(1);
            self.households[household_id].stay_failure_days = self.households[household_id]
                .stay_failure_days
                .saturating_add(1);
            if self.households[household_id].stay_failure_days
                < config.households.stay_failure_days_before_eviction
            {
                diagnostics.waiting_for_eviction =
                    diagnostics.waiting_for_eviction.saturating_add(1);
                continue;
            }

            if let Some(target_home) = self.find_affordable_home_for_household(
                household_id,
                reserve_days,
                allocator,
                Some(current_home),
                &config.households,
            ) {
                self.relocate_household(household_id, current_home, target_home, agents, allocator);
                diagnostics.relocated_failed_stay =
                    diagnostics.relocated_failed_stay.saturating_add(1);
            } else {
                self.evict_household(household_id, current_home, agents, allocator);
                diagnostics.evicted = diagnostics.evicted.saturating_add(1);
            }
        }

        debug_log!(
            "economy",
            "household housing resolution: checked={} housed={} unhoused_start={} stay_ok={} \
             stay_failed={} waiting={} relocated_unhoused={} relocated_failed_stay={} \
             relocated_upgrade={} evicted={} still_unhoused={}",
            diagnostics.checked_households,
            diagnostics.housed_households,
            diagnostics.unhoused_start_households,
            diagnostics.stay_passed,
            diagnostics.stay_failed,
            diagnostics.waiting_for_eviction,
            diagnostics.relocated_unhoused,
            diagnostics.relocated_failed_stay,
            diagnostics.relocated_upgrade,
            diagnostics.evicted,
            diagnostics.still_unhoused,
        );
    }

    fn find_affordable_home_for_household(
        &self,
        household_id: usize,
        reserve_days: f32,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
        config: &crate::simulation::economy::definitions::HouseholdRuntimeTuning,
    ) -> Option<usize> {
        let _household = &self.households[household_id];
        let current_center = current_home.and_then(|building_idx| {
            allocator
                .buildings
                .get(building_idx)
                .map(|building| (building.center_x, building.center_y))
        });

        let mut candidates = Vec::new();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };
        for &building_idx in &allocator.vacancy_index[residential_slot] {
            if Some(building_idx) == current_home || building_idx >= allocator.buildings.len() {
                continue;
            }
            let building = &allocator.buildings[building_idx];
            if building.broken || building.economy_broken || building.pending_redevelopment {
                continue;
            }

            let free_slots = allocator
                .household_capacity(building_idx)
                .saturating_sub(building.occupancy);
            if free_slots == 0 {
                continue;
            }

            let move_in_threshold = level_tuning_value(
                &config.residential_move_in_min_reserve_days_by_level,
                building.level,
            );
            if reserve_days + f32::EPSILON < move_in_threshold {
                continue;
            }

            let distance = current_center.map_or(0.0, |(origin_x, origin_y)| {
                let dx = building.center_x - origin_x;
                let dy = building.center_y - origin_y;
                dx * dx + dy * dy
            });
            candidates.push((building_idx, building.level, distance));
        }

        candidates.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| {
                    left.2
                        .partial_cmp(&right.2)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| left.0.cmp(&right.0))
        });
        candidates.first().map(|candidate| candidate.0)
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
        allocator.claim_vacancy(new_home);

        self.households[household_id].home_building_id = new_home;
        self.households[household_id].stay_failure_days = 0;
        self.households[household_id].unhoused_days_elapsed = 0;

        for agent_idx in 0..agents.len() {
            if agents.household_id[agent_idx] != household_id {
                continue;
            }
            agents.relocate_household_member_home(
                agent_idx,
                old_home,
                new_home,
                old_home < allocator.buildings.len(),
            );
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

        let household = &mut self.households[household_id];
        household.home_building_id = usize::MAX;
        household.stay_failure_days = 0;
        household.unhoused_days_elapsed = 0;
        clear_replenishment_request(household);

        for agent_idx in 0..agents.len() {
            if agents.household_id[agent_idx] != household_id {
                continue;
            }
            agents.evict_household_member_home(agent_idx, old_home);
        }
    }
}
