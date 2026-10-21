// SPDX-License-Identifier: GPL-2.0-only

//! Residential admission order (`ALLOC-03`): vacancy positions grouped by the starter household
//! size admission offers there, so admission finds the home a scan of the whole residential
//! vacancy list would pick, in O(log V) instead of O(V). Rehousing and housing reviews read the
//! same vacancies by level, and by level and 512 m chunk for the home nearest a household.

use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::economy::accessibility::chunk_for_point;
#[cfg(test)]
use crate::simulation::economy::agents::household_age_composition;
use crate::simulation::economy::households::candidate_immigrant_household_size_for_vacancy;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;
use std::collections::{BTreeMap, BTreeSet};

/// Inclusive `(min, max)` 512 m chunk coordinates.
pub(crate) type ChunkBounds = ((i32, i32), (i32, i32));

/// Listed homes by 512 m chunk, each set in building order.
pub(crate) type HomesByChunk = BTreeMap<(i32, i32), BTreeSet<usize>>;

// A listed home rehousing may claim: level, 512 m chunk (`chunk_for_point`) and building.
type RehousingHome = (u8, (i32, i32), usize);

/// Residential `vacancy_index` positions grouped by offered household size, each group ordered by
/// position. Kept in step with every residential vacancy push, swap-remove and occupancy change.
#[derive(Clone, Default)]
pub(crate) struct AdmissionOrder {
    // Offered size per vacancy position; 0 where admission skips the entry.
    sizes: Vec<u16>,
    // Positions by offered size; group 0 stays empty.
    by_size: Vec<BTreeSet<u32>>,
    // Rehousing uses the same vacancy lifecycle, ordered by level then stable building ID.
    homes: Vec<Option<RehousingHome>>,
    by_level: Vec<BTreeSet<usize>>,
    // The same homes per level and chunk, each set in building order, for housing reviews'
    // nearest-home search (`SIM-02-J`). An emptied chunk leaves its map.
    by_level_chunk: Vec<HomesByChunk>,
    // Chunk bounds per level covering its homes. They only grow while the level has homes, so
    // a search may scan rings that have since emptied.
    level_bounds: Vec<Option<ChunkBounds>>,
}

impl AdmissionOrder {
    /// Whether `other` orders the same vacancies the same way; groups emptied by incremental
    /// upkeep count as absent.
    #[cfg(test)]
    pub(crate) fn same_as(&self, other: &Self) -> bool {
        fn groups<T: Ord>(groups: &[BTreeSet<T>]) -> &[BTreeSet<T>] {
            let used = groups
                .iter()
                .rposition(|group| !group.is_empty())
                .map_or(0, |i| i + 1);
            &groups[..used]
        }
        let used_chunks = |maps: &[HomesByChunk]| {
            let used = maps
                .iter()
                .rposition(|map| !map.is_empty())
                .map_or(0, |i| i + 1);
            maps[..used].to_vec()
        };
        self.sizes == other.sizes
            && self.homes == other.homes
            && groups(&self.by_size) == groups(&other.by_size)
            && groups(&self.by_level) == groups(&other.by_level)
            && used_chunks(&self.by_level_chunk) == used_chunks(&other.by_level_chunk)
    }

    /// Whether this order describes a vacancy list of `vacancies` entries.
    pub(super) fn covers(&self, vacancies: usize) -> bool {
        self.sizes.len() == vacancies
    }

    fn push(&mut self, size: u16, home: Option<RehousingHome>) {
        self.sizes.push(0);
        self.homes.push(None);
        self.set(self.sizes.len() - 1, size);
        self.set_home(self.homes.len() - 1, home);
    }

    fn set_home(&mut self, position: usize, home: Option<RehousingHome>) {
        let previous = std::mem::replace(&mut self.homes[position], home);
        if previous == home {
            return;
        }
        if let Some((level, chunk, building)) = previous {
            let level = usize::from(level);
            self.by_level[level].remove(&building);
            let chunks = &mut self.by_level_chunk[level];
            if let Some(homes) = chunks.get_mut(&chunk) {
                homes.remove(&building);
                if homes.is_empty() {
                    chunks.remove(&chunk);
                }
            }
            if chunks.is_empty() {
                self.level_bounds[level] = None;
            }
        }
        if let Some((level, chunk, building)) = home {
            let level = usize::from(level);
            let levels = self.by_level.len().max(level + 1);
            self.by_level.resize_with(levels, BTreeSet::new);
            self.by_level_chunk.resize_with(levels, BTreeMap::new);
            self.level_bounds.resize(levels, None);
            self.by_level[level].insert(building);
            self.by_level_chunk[level]
                .entry(chunk)
                .or_default()
                .insert(building);
            let bounds = self.level_bounds[level].get_or_insert((chunk, chunk));
            bounds.0 = (bounds.0.0.min(chunk.0), bounds.0.1.min(chunk.1));
            bounds.1 = (bounds.1.0.max(chunk.0), bounds.1.1.max(chunk.1));
        }
    }

    fn set(&mut self, position: usize, size: u16) {
        let Some(slot) = self.sizes.get_mut(position) else {
            return;
        };
        let old = std::mem::replace(slot, size);
        if old == size {
            return;
        }
        let key = position as u32;
        if old != 0 {
            self.by_size[usize::from(old)].remove(&key);
        }
        if size != 0 {
            let group = usize::from(size);
            if self.by_size.len() <= group {
                self.by_size.resize_with(group + 1, BTreeSet::new);
            }
            self.by_size[group].insert(key);
        }
    }

    // Mirrors `Vec::swap_remove` on the vacancy list.
    fn swap_remove(&mut self, position: usize) {
        if position >= self.sizes.len() {
            return;
        }
        let last = self.sizes.len() - 1;
        let moved = self.sizes[last];
        let moved_home = self.homes[last];
        self.set(last, 0);
        self.set_home(last, None);
        self.sizes.pop();
        self.homes.pop();
        if position != last {
            self.set(position, moved);
            self.set_home(position, moved_home);
        }
    }

    /// Returns `(vacancy position, offered size)` for the smallest `(worker rank, size,
    /// position)`, the order a full scan of the vacancy list applies.
    ///
    /// Without the worker preference this is the first entry of the smallest group, O(log V).
    /// With it, worker rank 0 needs an adult. Every household of two or more has one, so each
    /// such group's first position is taken; a single is a lone elder one time in five, so the
    /// size-1 walk checks 1.25 positions on average and k positions with probability 5^-(k-1).
    #[cfg(test)]
    pub(super) fn pick(
        &self,
        vacancies: &[usize],
        applicant_seed: impl Fn(usize) -> usize,
        next_household_id: usize,
        prefer_worker_capable: bool,
    ) -> Option<(usize, u16)> {
        let groups = self
            .by_size
            .iter()
            .enumerate()
            .map(|(size, group)| (size as u16, group));
        if prefer_worker_capable {
            for (size, group) in groups.clone() {
                if let Some(&position) = group.iter().find(|&&position| {
                    let building_idx = vacancies[position as usize];
                    household_age_composition(applicant_seed(building_idx), next_household_id, size)
                        .adult_count
                        > 0
                }) {
                    return Some((position as usize, size));
                }
            }
        }
        groups
            .filter_map(|(size, group)| group.first().map(|&position| (position as usize, size)))
            .next()
    }
}

impl BuildingAllocator {
    fn rehousing_key(&self, building_idx: usize) -> Option<RehousingHome> {
        let building = self.buildings.get(building_idx)?;
        (!building.pending_redevelopment
            && self.household_capacity(building_idx) > building.occupancy)
            .then(|| {
                (
                    building.level,
                    chunk_for_point(building.center_x, building.center_y),
                    building_idx,
                )
            })
    }

    /// Finds the highest eligible residential level and then the lowest building ID.
    /// Uses the existing vacancy lifecycle; O(levels + log V), without scanning vacant homes.
    /// The caller rebuilds a stale index first.
    pub(crate) fn first_rehousing_vacancy(&self, eligible: impl Fn(u8) -> bool) -> Option<usize> {
        self.admission_order
            .by_level
            .iter()
            .enumerate()
            .rev()
            .find_map(|(level, homes)| {
                eligible(level as u8)
                    .then(|| homes.first().copied())
                    .flatten()
            })
    }

    /// Levels with a listed rehousing home, highest first. O(levels). The caller rebuilds a
    /// stale index first, as for every reader below.
    pub(crate) fn rehousing_levels_desc(&self) -> impl Iterator<Item = u8> + '_ {
        self.admission_order
            .by_level
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, homes)| !homes.is_empty())
            .map(|(level, _)| level as u8)
    }

    /// The listed rehousing homes of `level`, in building order.
    pub(crate) fn rehousing_homes(&self, level: u8) -> impl Iterator<Item = usize> + '_ {
        self.admission_order
            .by_level
            .get(usize::from(level))
            .into_iter()
            .flatten()
            .copied()
    }

    /// The listed rehousing homes of `level` by 512 m chunk (`chunk_for_point`), each set in
    /// building order, and chunk bounds that cover them; `None` for a level without homes.
    pub(crate) fn rehousing_homes_by_chunk(
        &self,
        level: u8,
    ) -> Option<(&HomesByChunk, ChunkBounds)> {
        let level = usize::from(level);
        let bounds = (*self.admission_order.level_bounds.get(level)?)?;
        Some((&self.admission_order.by_level_chunk[level], bounds))
    }

    /// The starter household size admission offers at `building_idx`, or `None` where it skips
    /// the building: missing, full, or a flat too small for any household.
    pub(super) fn admission_size(&self, building_idx: usize) -> Option<u16> {
        let building = self.buildings.get(building_idx)?;
        if self.household_capacity(building_idx) <= building.occupancy {
            return None;
        }
        candidate_immigrant_household_size_for_vacancy(
            self.flat_size_m2(building_idx),
            self.applicant_seed(building_idx),
            building.occupancy,
        )
    }

    /// Recomputes the admission order from the residential vacancy list. O(V log V).
    pub(crate) fn rebuild_admission_order(&mut self) {
        let Some(slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return;
        };
        // Main-thread index repair reaches here outside any pool (CODE-17).
        let sizes: Vec<u16> = crate::thread_pools::install(|| {
            self.vacancy_index[slot]
                .par_iter()
                .map(|&building_idx| self.admission_size(building_idx).unwrap_or(0))
                .collect()
        });
        let mut by_size: Vec<Vec<u32>> = Vec::new();
        for (position, &size) in sizes.iter().enumerate() {
            if size == 0 {
                continue;
            }
            let group = usize::from(size);
            if by_size.len() <= group {
                by_size.resize_with(group + 1, Vec::new);
            }
            by_size[group].push(position as u32);
        }
        // The rehousing homes, keyed on the pool and sorted by level, chunk and building, so
        // every level's sets are bulk-built as `set_home` would fill them one by one from an
        // empty order (`SIM-02-K`). O(V log V / p) plus the bulk builds.
        let (homes, keyed): (Vec<_>, Vec<RehousingHome>) = crate::thread_pools::install(|| {
            let homes: Vec<Option<RehousingHome>> = self.vacancy_index[slot]
                .par_iter()
                .map(|&building_idx| self.rehousing_key(building_idx))
                .collect();
            let mut keyed: Vec<RehousingHome> = homes.par_iter().filter_map(|home| *home).collect();
            keyed.par_sort_unstable();
            (homes, keyed)
        });
        let levels = keyed.last().map_or(0, |home| usize::from(home.0) + 1);
        let mut by_level = vec![BTreeSet::new(); levels];
        let mut by_level_chunk = vec![HomesByChunk::new(); levels];
        let mut level_bounds = vec![None; levels];
        for run in keyed.chunk_by(|a, b| a.0 == b.0) {
            let level = usize::from(run[0].0);
            by_level[level] = run.iter().map(|home| home.2).collect();
            by_level_chunk[level] = run
                .chunk_by(|a, b| a.1 == b.1)
                .map(|homes| (homes[0].1, homes.iter().map(|home| home.2).collect()))
                .collect();
            level_bounds[level] = run.iter().fold(None, |bounds: Option<ChunkBounds>, home| {
                let chunk = home.1;
                Some(bounds.map_or((chunk, chunk), |(low, high)| {
                    (
                        (low.0.min(chunk.0), low.1.min(chunk.1)),
                        (high.0.max(chunk.0), high.1.max(chunk.1)),
                    )
                }))
            });
        }
        // Positions arrive ascending, so each set is bulk-built without re-sorting work.
        self.admission_order = AdmissionOrder {
            sizes,
            by_size: by_size.into_iter().map(BTreeSet::from_iter).collect(),
            homes,
            by_level,
            by_level_chunk,
            level_bounds,
        };
    }

    pub(super) fn clear_admission_order(&mut self) {
        self.admission_order = AdmissionOrder::default();
    }

    /// Records a building just pushed onto vacancy list `slot`. O(log V).
    pub(super) fn admission_vacancy_pushed(&mut self, slot: usize, building_idx: usize) {
        if !self.is_residential_slot(slot) {
            return;
        }
        if self
            .admission_order
            .covers(self.vacancy_index[slot].len() - 1)
        {
            let size = self.admission_size(building_idx).unwrap_or(0);
            let home = self.rehousing_key(building_idx);
            self.admission_order.push(size, home);
        }
    }

    /// Mirrors a swap-remove at `position` that already shrank vacancy list `slot`. O(log V).
    pub(super) fn admission_vacancy_removed(&mut self, slot: usize, position: usize) {
        if self.is_residential_slot(slot)
            && self
                .admission_order
                .covers(self.vacancy_index[slot].len() + 1)
        {
            self.admission_order.swap_remove(position);
        }
    }

    /// Re-keys a listed building after its occupancy changed: the offered size hashes it.
    /// O(log V).
    pub(super) fn admission_occupancy_changed(&mut self, building_idx: usize) {
        let Some(slot) = self.housing_vacancy_slot(building_idx) else {
            return;
        };
        let Some(&position) = self.vacancy_pos.get(building_idx) else {
            return;
        };
        if position == usize::MAX
            || !self.is_residential_slot(slot)
            || !self.admission_order.covers(self.vacancy_index[slot].len())
        {
            return;
        }
        let size = self.admission_size(building_idx).unwrap_or(0);
        self.admission_order.set(position, size);
        let home = self.rehousing_key(building_idx);
        self.admission_order.set_home(position, home);
    }

    fn is_residential_slot(&self, slot: usize) -> bool {
        baseline_private_zone_slot(ZoneType::Residential) == Some(slot)
    }
}
