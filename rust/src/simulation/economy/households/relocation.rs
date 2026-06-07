//! Household housing resolution, relocation, and eviction.

use std::collections::BTreeMap;

use super::HouseholdSystem;
use super::metrics::{household_is_housed, household_reserve_days, level_tuning_value};
use super::replenishment::clear_replenishment_request;
use crate::debug_log;
use crate::simulation::buildings::allocator::{
    Building, BuildingAllocator, baseline_private_zone_slot,
};
use crate::simulation::economy::agents::AgentSystem;
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;

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
    chunk: (i32, i32),
}

struct VacancyPlanner {
    candidates: Vec<VacancyCandidate>,
    levels_desc: Vec<u8>,
    by_level: BTreeMap<u8, Vec<usize>>,
    by_level_chunk: BTreeMap<u8, BTreeMap<(i32, i32), Vec<usize>>>,
    level_chunk_bounds: BTreeMap<u8, ((i32, i32), (i32, i32))>,
}

impl VacancyPlanner {
    fn new(allocator: &BuildingAllocator) -> Self {
        let mut candidates = Vec::new();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return Self::from_candidates(candidates);
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
                chunk: building_chunk(building),
            });
        }
        candidates.sort_unstable_by(|left, right| {
            right
                .level
                .cmp(&left.level)
                .then_with(|| left.building_idx.cmp(&right.building_idx))
        });
        Self::from_candidates(candidates)
    }

    fn from_candidates(candidates: Vec<VacancyCandidate>) -> Self {
        let mut planner = Self {
            candidates,
            levels_desc: Vec::new(),
            by_level: BTreeMap::new(),
            by_level_chunk: BTreeMap::new(),
            level_chunk_bounds: BTreeMap::new(),
        };
        for candidate_pos in 0..planner.candidates.len() {
            planner.index_candidate(candidate_pos);
        }
        planner.levels_desc.sort_unstable_by(|a, b| b.cmp(a));
        planner.levels_desc.dedup();
        planner
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

        let levels = self.levels_desc.clone();
        for level in levels {
            if minimum_level_exclusive.is_some_and(|minimum_level| level <= minimum_level) {
                break;
            }
            let move_in_threshold =
                level_tuning_value(&config.residential_move_in_min_reserve_days_by_level, level);
            if reserve_days + f32::EPSILON < move_in_threshold {
                continue;
            }
            let candidate_pos = if let Some((origin_x, origin_y)) = current_center {
                self.nearest_candidate_in_level(level, origin_x, origin_y, allocator, current_home)
            } else {
                self.first_candidate_in_level(level, allocator, current_home)
            };
            if let Some(candidate_pos) = candidate_pos {
                self.candidates[candidate_pos].remaining_slots = self.candidates[candidate_pos]
                    .remaining_slots
                    .saturating_sub(1);
                return Some(self.candidates[candidate_pos].building_idx);
            }
        }
        None
    }

    fn release_home(&mut self, allocator: &BuildingAllocator, building_idx: usize) {
        let Some(candidate) = vacancy_candidate_for_building(allocator, building_idx) else {
            return;
        };
        if let Some(existing_pos) = self
            .candidates
            .iter()
            .position(|existing| existing.building_idx == building_idx)
        {
            let needs_reindex = self.candidates[existing_pos].level != candidate.level
                || self.candidates[existing_pos].chunk != candidate.chunk;
            self.candidates[existing_pos] = candidate;
            if needs_reindex {
                self.rebuild_indices();
            }
            return;
        }
        self.candidates.push(candidate);
        self.index_candidate(self.candidates.len() - 1);
    }

    fn rebuild_indices(&mut self) {
        self.levels_desc.clear();
        self.by_level.clear();
        self.by_level_chunk.clear();
        self.level_chunk_bounds.clear();
        for candidate_pos in 0..self.candidates.len() {
            self.index_candidate(candidate_pos);
        }
    }

    fn index_candidate(&mut self, candidate_pos: usize) {
        let candidate = &self.candidates[candidate_pos];
        insert_level_desc(&mut self.levels_desc, candidate.level);
        insert_candidate_pos_by_building(
            self.by_level.entry(candidate.level).or_default(),
            &self.candidates,
            candidate_pos,
        );
        insert_candidate_pos_by_building(
            self.by_level_chunk
                .entry(candidate.level)
                .or_default()
                .entry(candidate.chunk)
                .or_default(),
            &self.candidates,
            candidate_pos,
        );
        update_level_chunk_bounds(
            &mut self.level_chunk_bounds,
            candidate.level,
            candidate.chunk,
        );
    }

    fn first_candidate_in_level(
        &self,
        level: u8,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
    ) -> Option<usize> {
        self.by_level
            .get(&level)?
            .iter()
            .copied()
            .find(|&pos| self.candidate_is_available(pos, allocator, current_home))
    }

    fn nearest_candidate_in_level(
        &self,
        level: u8,
        origin_x: f32,
        origin_y: f32,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
    ) -> Option<usize> {
        let by_chunk = self.by_level_chunk.get(&level)?;
        let origin_chunk = RegionGraph::get_chunk_coords(Vector3::new(origin_x, 0.0, origin_y));
        let max_ring =
            max_chunk_ring_for_level(self.level_chunk_bounds.get(&level).copied()?, origin_chunk);
        let mut best: Option<(usize, f32, usize)> = None;
        for ring in 0..=max_ring {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let chunk_key = (origin_chunk.0 + dx, origin_chunk.1 + dz);
                    let Some(candidate_positions) = by_chunk.get(&chunk_key) else {
                        continue;
                    };
                    for &candidate_pos in candidate_positions {
                        if !self.candidate_is_available(candidate_pos, allocator, current_home) {
                            continue;
                        }
                        let building_idx = self.candidates[candidate_pos].building_idx;
                        let building = &allocator.buildings[building_idx];
                        let distance = squared_distance_to_building(origin_x, origin_y, building);
                        let challenger = (candidate_pos, distance, building_idx);
                        if best.is_none_or(|current| {
                            challenger.1.total_cmp(&current.1).is_lt()
                                || (challenger.1.total_cmp(&current.1).is_eq()
                                    && challenger.2 < current.2)
                        }) {
                            best = Some(challenger);
                        }
                    }
                }
            }
            if let Some((_, best_distance, _)) = best {
                if ring == max_ring
                    || best_distance
                        <= min_possible_ring_distance_sq(origin_x, origin_y, origin_chunk, ring + 1)
                {
                    break;
                }
            }
        }
        best.map(|(candidate_pos, _, _)| candidate_pos)
    }

    fn candidate_is_available(
        &self,
        candidate_pos: usize,
        allocator: &BuildingAllocator,
        current_home: Option<usize>,
    ) -> bool {
        let Some(candidate) = self.candidates.get(candidate_pos) else {
            return false;
        };
        if candidate.remaining_slots == 0 || Some(candidate.building_idx) == current_home {
            return false;
        }
        allocator
            .buildings
            .get(candidate.building_idx)
            .is_some_and(|building| {
                !building.broken
                    && !building.economy_broken
                    && !building.is_deserted
                    && !building.pending_redevelopment
                    && matches!(building.zone_type, ZoneType::Residential)
            })
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
        chunk: building_chunk(building),
    })
}

fn insert_level_desc(levels: &mut Vec<u8>, level: u8) {
    match levels.binary_search_by(|existing| existing.cmp(&level).reverse()) {
        Ok(_) => {}
        Err(pos) => levels.insert(pos, level),
    }
}

fn insert_candidate_pos_by_building(
    positions: &mut Vec<usize>,
    candidates: &[VacancyCandidate],
    candidate_pos: usize,
) {
    let building_idx = candidates[candidate_pos].building_idx;
    let insert_pos = positions
        .binary_search_by(|&existing_pos| candidates[existing_pos].building_idx.cmp(&building_idx))
        .unwrap_or_else(|pos| pos);
    positions.insert(insert_pos, candidate_pos);
}

fn update_level_chunk_bounds(
    bounds_by_level: &mut BTreeMap<u8, ((i32, i32), (i32, i32))>,
    level: u8,
    chunk: (i32, i32),
) {
    bounds_by_level
        .entry(level)
        .and_modify(|(min_chunk, max_chunk)| {
            min_chunk.0 = min_chunk.0.min(chunk.0);
            min_chunk.1 = min_chunk.1.min(chunk.1);
            max_chunk.0 = max_chunk.0.max(chunk.0);
            max_chunk.1 = max_chunk.1.max(chunk.1);
        })
        .or_insert((chunk, chunk));
}

fn max_chunk_ring_for_level(bounds: ((i32, i32), (i32, i32)), origin_chunk: (i32, i32)) -> i32 {
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

fn min_possible_ring_distance_sq(
    origin_x: f32,
    origin_y: f32,
    origin_chunk: (i32, i32),
    ring: i32,
) -> f32 {
    let mut best = f32::INFINITY;
    for dx in -ring..=ring {
        for dz in -ring..=ring {
            if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                continue;
            }
            let chunk = (origin_chunk.0 + dx, origin_chunk.1 + dz);
            best = best.min(squared_distance_to_chunk(origin_x, origin_y, chunk));
        }
    }
    best
}

fn squared_distance_to_chunk(origin_x: f32, origin_y: f32, chunk: (i32, i32)) -> f32 {
    let min_x = chunk.0 as f32 * RegionGraph::CHUNK_SIZE;
    let max_x = min_x + RegionGraph::CHUNK_SIZE;
    let min_y = chunk.1 as f32 * RegionGraph::CHUNK_SIZE;
    let max_y = min_y + RegionGraph::CHUNK_SIZE;
    let dx = if origin_x < min_x {
        min_x - origin_x
    } else if origin_x > max_x {
        origin_x - max_x
    } else {
        0.0
    };
    let dy = if origin_y < min_y {
        min_y - origin_y
    } else if origin_y > max_y {
        origin_y - max_y
    } else {
        0.0
    };
    dx * dx + dy * dy
}

fn squared_distance_to_building(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}

fn building_chunk(building: &Building) -> (i32, i32) {
    RegionGraph::get_chunk_coords(Vector3::new(building.center_x, 0.0, building.center_y))
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
