// SPDX-License-Identifier: GPL-2.0-only

//! Residential admission order (`ALLOC-03`): vacancy positions grouped by the starter household
//! size admission offers there, so admission finds the home a scan of the whole residential
//! vacancy list would pick, in O(log V) instead of O(V).

use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::economy::agents::household_age_composition;
use crate::simulation::economy::households::candidate_immigrant_household_size_for_vacancy;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;
use std::collections::BTreeSet;

/// Residential `vacancy_index` positions grouped by offered household size, each group ordered by
/// position. Kept in step with every residential vacancy push, swap-remove and occupancy change.
#[derive(Clone, Default)]
pub(crate) struct AdmissionOrder {
    // Offered size per vacancy position; 0 where admission skips the entry.
    sizes: Vec<u16>,
    // Positions by offered size; group 0 stays empty.
    by_size: Vec<BTreeSet<u32>>,
}

impl AdmissionOrder {
    /// Whether this order describes a vacancy list of `vacancies` entries.
    pub(super) fn covers(&self, vacancies: usize) -> bool {
        self.sizes.len() == vacancies
    }

    fn push(&mut self, size: u16) {
        self.sizes.push(0);
        self.set(self.sizes.len() - 1, size);
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
        self.set(last, 0);
        self.sizes.pop();
        if position != last {
            self.set(position, moved);
        }
    }

    /// Returns `(vacancy position, offered size)` for the smallest `(worker rank, size,
    /// position)`, the order a full scan of the vacancy list applies.
    ///
    /// Without the worker preference this is the first entry of the smallest group, O(log V).
    /// With it, worker rank 0 needs an adult. Every household of two or more has one, so each
    /// such group's first position is taken; a single is a lone elder one time in five, so the
    /// size-1 walk checks 1.25 positions on average and k positions with probability 5^-(k-1).
    pub(super) fn pick(
        &self,
        vacancies: &[usize],
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
                    household_age_composition(building_idx, next_household_id, size).adult_count > 0
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
    /// The starter household size admission offers at `building_idx`, or `None` where it skips
    /// the building: missing, full, or a flat too small for any household.
    pub(super) fn admission_size(&self, building_idx: usize) -> Option<u16> {
        let building = self.buildings.get(building_idx)?;
        if self.household_capacity(building_idx) <= building.occupancy {
            return None;
        }
        candidate_immigrant_household_size_for_vacancy(
            self.flat_size_m2(building_idx),
            building_idx,
            building.occupancy,
        )
    }

    /// Recomputes the admission order from the residential vacancy list. O(V log V).
    pub(crate) fn rebuild_admission_order(&mut self) {
        let Some(slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return;
        };
        let sizes: Vec<u16> = self.vacancy_index[slot]
            .par_iter()
            .map(|&building_idx| self.admission_size(building_idx).unwrap_or(0))
            .collect();
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
        // Positions arrive ascending, so each set is bulk-built without re-sorting work.
        self.admission_order = AdmissionOrder {
            sizes,
            by_size: by_size.into_iter().map(BTreeSet::from_iter).collect(),
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
            self.admission_order.push(size);
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
    }

    fn is_residential_slot(&self, slot: usize) -> bool {
        baseline_private_zone_slot(ZoneType::Residential) == Some(slot)
    }
}
