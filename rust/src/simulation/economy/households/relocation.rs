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

const NO_MEMBER: usize = usize::MAX;

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

struct VacancyCandidate {
    building_idx: usize,
    level: u8,
    remaining_slots: u32,
}

struct VacancyPlanner {
    candidates: Vec<VacancyCandidate>,
}

impl VacancyPlanner {
    fn new(allocator: &BuildingAllocator) -> Self {
        let mut candidates = Vec::new();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return Self { candidates };
        };
        for &building_idx in &allocator.vacancy_index[residential_slot] {
            if building_idx >= allocator.buildings.len() {
                continue;
            }
            let building = &allocator.buildings[building_idx];
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.pending_redevelopment
            {
                continue;
            }
            let remaining_slots = allocator
                .household_capacity(building_idx)
                .saturating_sub(building.occupancy);
            if remaining_slots == 0 {
                continue;
            }
            candidates.push(VacancyCandidate {
                building_idx,
                level: building.level,
                remaining_slots,
            });
        }
        candidates.sort_unstable_by(|left, right| {
            right
                .level
                .cmp(&left.level)
                .then_with(|| left.building_idx.cmp(&right.building_idx))
        });
        Self { candidates }
    }

    fn claim_affordable_home(
        &mut self,
        reserve_days: f32,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
        minimum_level_exclusive: Option<u8>,
        config: &crate::simulation::economy::definitions::HouseholdRuntimeTuning,
    ) -> Option<usize> {
        let current_center = current_home.and_then(|building_idx| {
            allocator
                .buildings
                .get(building_idx)
                .map(|building| (building.center_x, building.center_y))
        });

        let mut best: Option<(usize, u8, f32, usize)> = None;
        for (candidate_pos, candidate) in self.candidates.iter().enumerate() {
            if best.is_some_and(|(_, best_level, _, _)| candidate.level < best_level) {
                break;
            }
            if minimum_level_exclusive.is_some_and(|level| candidate.level <= level) {
                break;
            }
            if candidate.remaining_slots == 0 || Some(candidate.building_idx) == current_home {
                continue;
            }
            let building = &allocator.buildings[candidate.building_idx];
            if building.broken
                || building.economy_broken
                || building.is_deserted
                || building.pending_redevelopment
            {
                continue;
            }

            let move_in_threshold = level_tuning_value(
                &config.residential_move_in_min_reserve_days_by_level,
                candidate.level,
            );
            if reserve_days + f32::EPSILON < move_in_threshold {
                continue;
            }

            let distance = current_center.map_or(0.0, |(origin_x, origin_y)| {
                let dx = building.center_x - origin_x;
                let dy = building.center_y - origin_y;
                dx * dx + dy * dy
            });
            let challenger = (
                candidate_pos,
                candidate.level,
                distance,
                candidate.building_idx,
            );
            if best.is_none_or(|current| {
                challenger.1 > current.1
                    || (challenger.1 == current.1
                        && (challenger.2.total_cmp(&current.2).is_lt()
                            || (challenger.2.total_cmp(&current.2).is_eq()
                                && challenger.3 < current.3)))
            }) {
                best = Some(challenger);
            }
        }

        let (candidate_pos, _, _, building_idx) = best?;
        self.candidates[candidate_pos].remaining_slots = self.candidates[candidate_pos]
            .remaining_slots
            .saturating_sub(1);
        Some(building_idx)
    }

    fn release_home(&mut self, allocator: &BuildingAllocator, building_idx: usize) {
        let Some(candidate) = vacancy_candidate_for_building(allocator, building_idx) else {
            return;
        };
        if let Some(existing) = self
            .candidates
            .iter_mut()
            .find(|existing| existing.building_idx == building_idx)
        {
            existing.level = candidate.level;
            existing.remaining_slots = candidate.remaining_slots;
            return;
        }
        self.candidates.push(candidate);
        self.sort_candidates();
    }

    fn sort_candidates(&mut self) {
        self.candidates.sort_unstable_by(|left, right| {
            right
                .level
                .cmp(&left.level)
                .then_with(|| left.building_idx.cmp(&right.building_idx))
        });
    }
}

fn vacancy_candidate_for_building(
    allocator: &BuildingAllocator,
    building_idx: usize,
) -> Option<VacancyCandidate> {
    let building = allocator.buildings.get(building_idx)?;
    if building.broken
        || building.economy_broken
        || building.is_deserted
        || building.pending_redevelopment
        || !matches!(building.zone_type, ZoneType::Residential)
    {
        return None;
    }
    let remaining_slots = allocator
        .household_capacity(building_idx)
        .saturating_sub(building.occupancy);
    if remaining_slots == 0 {
        return None;
    }
    Some(VacancyCandidate {
        building_idx,
        level: building.level,
        remaining_slots,
    })
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
        let member_heads_scratch = std::mem::take(&mut self.household_member_heads_scratch);
        let member_next_scratch = std::mem::take(&mut self.household_member_next_scratch);
        let (household_member_heads, household_member_next) = build_household_member_index(
            agents,
            self.households.len(),
            member_heads_scratch,
            member_next_scratch,
        );
        let mut vacancy_planner = VacancyPlanner::new(allocator);

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
                if let Some(target_home) = vacancy_planner.claim_affordable_home(
                    reserve_days,
                    allocator,
                    None,
                    None,
                    &config.households,
                ) {
                    self.relocate_household(
                        household_id,
                        usize::MAX,
                        target_home,
                        agents,
                        allocator,
                        &mut vacancy_planner,
                        &household_member_heads,
                        &household_member_next,
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
                if let Some(target_home) = vacancy_planner.claim_affordable_home(
                    reserve_days,
                    allocator,
                    Some(current_home),
                    Some(current_level),
                    &config.households,
                ) {
                    self.relocate_household(
                        household_id,
                        current_home,
                        target_home,
                        agents,
                        allocator,
                        &mut vacancy_planner,
                        &household_member_heads,
                        &household_member_next,
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

            if let Some(target_home) = vacancy_planner.claim_affordable_home(
                reserve_days,
                allocator,
                Some(current_home),
                None,
                &config.households,
            ) {
                self.relocate_household(
                    household_id,
                    current_home,
                    target_home,
                    agents,
                    allocator,
                    &mut vacancy_planner,
                    &household_member_heads,
                    &household_member_next,
                );
                diagnostics.relocated_failed_stay =
                    diagnostics.relocated_failed_stay.saturating_add(1);
            } else {
                self.evict_household(
                    household_id,
                    current_home,
                    agents,
                    allocator,
                    &mut vacancy_planner,
                    &household_member_heads,
                    &household_member_next,
                );
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

        self.household_member_heads_scratch = household_member_heads;
        self.household_member_next_scratch = household_member_next;
    }

    fn relocate_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        new_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        vacancy_planner: &mut VacancyPlanner,
        household_member_heads: &[usize],
        household_member_next: &[usize],
    ) {
        if household_id >= self.households.len() || new_home >= allocator.buildings.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
            vacancy_planner.release_home(allocator, old_home);
        }
        allocator.claim_vacancy(new_home);

        self.households[household_id].home_building_id = new_home;
        self.households[household_id].stay_failure_days = 0;
        self.households[household_id].unhoused_days_elapsed = 0;

        let mut agent_idx = household_member_heads
            .get(household_id)
            .copied()
            .unwrap_or(NO_MEMBER);
        while agent_idx != NO_MEMBER {
            agents.relocate_household_member_home(
                agent_idx,
                old_home,
                new_home,
                old_home < allocator.buildings.len(),
            );
            agent_idx = household_member_next
                .get(agent_idx)
                .copied()
                .unwrap_or(NO_MEMBER);
        }
    }

    fn evict_household(
        &mut self,
        household_id: usize,
        old_home: usize,
        agents: &mut AgentSystem,
        allocator: &mut BuildingAllocator,
        vacancy_planner: &mut VacancyPlanner,
        household_member_heads: &[usize],
        household_member_next: &[usize],
    ) {
        if household_id >= self.households.len() {
            return;
        }
        if old_home < allocator.buildings.len() {
            allocator.release_vacancy(old_home);
            vacancy_planner.release_home(allocator, old_home);
        }

        let household = &mut self.households[household_id];
        household.home_building_id = usize::MAX;
        household.stay_failure_days = 0;
        household.unhoused_days_elapsed = 0;
        clear_replenishment_request(household);

        let mut agent_idx = household_member_heads
            .get(household_id)
            .copied()
            .unwrap_or(NO_MEMBER);
        while agent_idx != NO_MEMBER {
            agents.evict_household_member_home(agent_idx, old_home);
            agent_idx = household_member_next
                .get(agent_idx)
                .copied()
                .unwrap_or(NO_MEMBER);
        }
    }
}

fn build_household_member_index(
    agents: &AgentSystem,
    household_count: usize,
    mut household_member_heads: Vec<usize>,
    mut household_member_next: Vec<usize>,
) -> (Vec<usize>, Vec<usize>) {
    household_member_heads.clear();
    household_member_heads.resize(household_count, NO_MEMBER);
    household_member_next.clear();
    household_member_next.resize(agents.len(), NO_MEMBER);
    for agent_idx in (0..agents.len()).rev() {
        let household_id = agents.household_id[agent_idx];
        if household_id >= household_count {
            continue;
        }
        household_member_next[agent_idx] = household_member_heads[household_id];
        household_member_heads[household_id] = agent_idx;
    }
    (household_member_heads, household_member_next)
}
