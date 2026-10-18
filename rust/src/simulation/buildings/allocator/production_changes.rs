// SPDX-License-Identifier: GPL-2.0-only

//! Allocation-free production-input mutations and conservative census change tracking.

use super::BuildingAllocator;

impl BuildingAllocator {
    /// Prepares bounded history before staffing work; individual mutations never allocate.
    pub(crate) fn prepare_production_changes(&mut self) {
        self.production_changes
            .prepare_recording(self.buildings.len());
    }

    /// Changes authoritative employment and records the affected owner in O(1).
    /// Repeating the current count leaves the revision and history unchanged.
    pub(crate) fn set_worker_count(&mut self, index: usize, count: u32) {
        if self.buildings[index].worker_count == count {
            return;
        }
        self.buildings[index].worker_count = count;
        self.record_production_change(index);
    }

    /// Updates sanitized productive area and records changed owners without allocating.
    /// Missing owners are ignored, matching restoration of detached field/extractor sites.
    pub(crate) fn set_work_area_scale(&mut self, index: usize, scale: f32) {
        let Some(building) = self.buildings.get_mut(index) else {
            return;
        };
        let before = building.work_area_scale.to_bits();
        building.set_work_area_scale(scale);
        if before != building.work_area_scale.to_bits() {
            self.record_production_change(index);
        }
    }

    /// Records a known production-input change; callers must already have updated the owner.
    pub(super) fn record_production_change(&mut self, index: usize) {
        self.production_revision = self.production_revision.wrapping_add(1);
        self.production_changes
            .record_prepared(self.production_revision, index);
    }

    /// Invalidates history after bulk reconciliation or index changes with unknown owners.
    pub(crate) fn invalidate_production_changes(&mut self) {
        self.production_revision = self.production_revision.wrapping_add(1);
        self.production_changes.clear(self.production_revision);
    }

    /// Cursor for production inputs: workers, productive area and operational eligibility.
    pub(crate) fn production_revision(&self) -> u64 {
        self.production_revision
    }

    /// Returns sorted changed owners, or false when a full production-input rebuild is required.
    /// Consumers also validate catalog and road/entrance revisions before reusing derived data.
    pub(crate) fn production_changes_since(&self, since: u64, out: &mut Vec<usize>) -> bool {
        self.production_changes
            .changes_since(since, self.production_revision, out)
    }

    /// Gives a copy of a store (the census input copy, `SIM-02-I`) one production step from
    /// `since` to `revision`: `owners` changed, or `None` when they are unknown, so a consumer at
    /// `since` updates those owners and any other consumer rebuilds. O(owners).
    pub(crate) fn set_copied_production_step(
        &mut self,
        since: u64,
        revision: u64,
        owners: Option<&[usize]>,
    ) {
        self.production_revision = revision;
        match owners {
            Some(owners) => {
                self.production_changes.clear(since);
                self.production_changes
                    .record(revision, owners.iter().copied());
            }
            None => self.production_changes.clear(revision),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::zoning::ZoneType;

    #[test]
    fn changes_are_sparse_and_noops_keep_the_census_cursor() {
        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(super::super::indexed_test_building(
                String::new(),
                ZoneType::Industrial,
                0,
            ));
        allocator.prepare_production_changes();
        let revision = allocator.production_revision();
        allocator.set_worker_count(0, 0);
        assert_eq!(allocator.production_revision(), revision);
        allocator.set_worker_count(0, 3);
        let mut changed = Vec::new();
        assert!(allocator.production_changes_since(revision, &mut changed));
        assert_eq!(changed, [0]);
        let revision = allocator.production_revision();
        allocator.set_worker_count(0, 3);
        assert!(allocator.production_changes_since(revision, &mut changed));
        assert!(changed.is_empty());
        allocator.bump_building_ref_revision();
        assert!(!allocator.production_changes_since(revision, &mut changed));
    }

    #[test]
    fn area_abandonment_and_construction_eligibility_are_tracked() {
        let mut allocator = BuildingAllocator::new();
        allocator
            .buildings
            .push(super::super::indexed_test_building(
                String::new(),
                ZoneType::Industrial,
                0,
            ));
        allocator.prepare_production_changes();
        let mut changed = Vec::new();
        let revision = allocator.production_revision();
        allocator.set_work_area_scale(0, 2.0);
        assert!(allocator.production_changes_since(revision, &mut changed));
        assert_eq!(changed, [0]);
        let revision = allocator.production_revision();
        allocator.set_work_area_scale(0, 2.0);
        allocator.set_work_area_scale(usize::MAX, 2.0);
        assert_eq!(allocator.production_revision(), revision);
        allocator.mark_building_deserted(0);
        assert!(allocator.production_changes_since(revision, &mut changed));
        assert_eq!(changed, [0]);
        let revision = allocator.production_revision();
        allocator.mark_building_deserted(0);
        assert_eq!(allocator.production_revision(), revision);

        allocator.buildings[0].set_construction_hours_left(2);
        allocator.complete_construction(0);
        assert!(allocator.production_changes_since(revision, &mut changed));
        assert_eq!(changed, [0]);
    }
}
