// SPDX-License-Identifier: GPL-2.0-only

//! The parcel store as a cell-lot plan sees it (`ROAD-68` open question 21 (b)): the store the
//! plan started from, unchanged, plus the plan's own retirements, reattachments and new lots.
//! Every change is recorded as the store call it stands for, so the installer makes exactly
//! the same calls, in the same order, on the store the plan started from.

use super::super::geometry::{chunks_for_aabb, rectangles_overlap_geometry};
use super::super::types::{ParcelGeometry, ParcelId, ZoningParcel};
use super::ParcelStore;
use crate::simulation::zoning::cells::CellLot;
use godot::prelude::Vector2;
use std::collections::{BTreeMap, HashSet};

/// One parcel-store call a cell-lot plan made, replayed by [`ParcelStore::apply_planned`].
#[derive(Clone, Debug)]
pub(crate) enum PlannedParcelChange {
    /// [`ParcelStore::insert_new`], which gave the plan this id.
    Insert {
        /// The id the plan allocated; the replay allocates the same one.
        id: ParcelId,
        /// The new parcel's geometry.
        geometry: ParcelGeometry,
        /// The new parcel's zone profile.
        profile: u16,
    },
    /// [`ParcelStore::set_cell_lot`].
    SetCellLot(ParcelId, CellLot),
    /// [`ParcelStore::remove_local`].
    Remove(ParcelId),
    /// [`ParcelStore::replace_geometry`].
    ReplaceGeometry(ParcelId, ParcelGeometry),
}

/// A read-only parcel store with one plan's changes on top. Reads answer as the store would
/// after the recorded calls; nothing is written to the store itself. `O(log changes)` per
/// read above the store's own query, and the changes are bounded by the plan's regions.
pub(crate) struct PlannedParcels<'a> {
    base: &'a ParcelStore,
    // Parcels the plan changed, by id: retired (`None`), reattached or created.
    changed: BTreeMap<ParcelId, Option<ZoningParcel>>,
    next_id: u64,
    changes: Vec<PlannedParcelChange>,
}

impl<'a> PlannedParcels<'a> {
    /// Starts a plan on `base`, with no changes.
    pub(crate) fn new(base: &'a ParcelStore) -> Self {
        Self {
            base,
            changed: BTreeMap::new(),
            next_id: base.next_id,
            changes: Vec::new(),
        }
    }

    /// The store the plan started from.
    pub(crate) fn base(&self) -> &'a ParcelStore {
        self.base
    }

    /// The recorded calls, in the order the plan made them.
    pub(crate) fn into_changes(self) -> Vec<PlannedParcelChange> {
        self.changes
    }

    /// [`ParcelStore::get`] after the plan's changes.
    pub(crate) fn get(&self, id: ParcelId) -> Option<&ZoningParcel> {
        match self.changed.get(&id) {
            Some(changed) => changed.as_ref(),
            None => self.base.get(id),
        }
    }

    /// [`ParcelStore::visit_in_bounds`] after the plan's changes: each parcel whose bounds meet
    /// the query once, the store's unchanged ones first, then the changed ones by id.
    pub(crate) fn visit_in_bounds(
        &self,
        min: Vector2,
        max: Vector2,
        mut visit: impl FnMut(&ZoningParcel),
    ) {
        self.base.visit_in_bounds(min, max, |parcel| {
            if !self.changed.contains_key(&parcel.id()) {
                visit(parcel);
            }
        });
        for parcel in self.changed.values().flatten() {
            let (pmin, pmax) = (parcel.aabb_min(), parcel.aabb_max());
            if pmin.x <= max.x && pmin.y <= max.y && pmax.x >= min.x && pmax.y >= min.y {
                visit(parcel);
            }
        }
    }

    /// [`ParcelStore::overlaps_existing`] after the plan's changes.
    pub(crate) fn overlaps_existing(&self, geometry: &ParcelGeometry) -> bool {
        self.base
            .overlaps_existing_unless(geometry, |id| self.changed.contains_key(&id))
            || self
                .changed
                .values()
                .flatten()
                .any(|parcel| rectangles_overlap_geometry(geometry, parcel))
    }

    /// [`ParcelStore::insert_new`] on the plan.
    pub(crate) fn insert_new(&mut self, geometry: ParcelGeometry, profile: u16) -> ParcelId {
        let id = ParcelId::from_raw(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.changed
            .insert(id, Some(ZoningParcel::new(id, geometry, profile)));
        self.changes.push(PlannedParcelChange::Insert {
            id,
            geometry,
            profile,
        });
        id
    }

    /// [`ParcelStore::set_cell_lot`] on the plan; a missing parcel is left alone, as there.
    pub(crate) fn set_cell_lot(&mut self, id: ParcelId, lot: CellLot) {
        if let Some(mut parcel) = self.get(id).cloned() {
            parcel.set_cell_lot(lot);
            self.changed.insert(id, Some(parcel));
            self.changes.push(PlannedParcelChange::SetCellLot(id, lot));
        }
    }

    /// [`ParcelStore::remove_local`] on the plan; a missing parcel is left alone, as there.
    pub(crate) fn remove_local(&mut self, id: ParcelId) {
        if self.get(id).is_some() {
            self.changed.insert(id, None);
            self.changes.push(PlannedParcelChange::Remove(id));
        }
    }

    /// [`ParcelStore::replace_geometry`] on the plan; a missing parcel is left alone, as there.
    pub(crate) fn replace_geometry(&mut self, id: ParcelId, geometry: ParcelGeometry) {
        if let Some(mut parcel) = self.get(id).cloned() {
            parcel.replace_geometry(geometry);
            self.changed.insert(id, Some(parcel));
            self.changes
                .push(PlannedParcelChange::ReplaceGeometry(id, geometry));
        }
    }
}

impl ParcelStore {
    /// Makes a plan's recorded calls on this store, in order. The plan must have started from
    /// this store's present state; the replayed inserts then allocate the plan's ids.
    /// O(changes × the calls' own cost).
    pub(crate) fn apply_planned(&mut self, changes: Vec<PlannedParcelChange>) {
        for change in changes {
            match change {
                PlannedParcelChange::Insert {
                    id,
                    geometry,
                    profile,
                } => {
                    let inserted = self.insert_new(geometry, profile);
                    debug_assert_eq!(inserted, id, "a plan is installed on its own base");
                }
                PlannedParcelChange::SetCellLot(id, lot) => self.set_cell_lot(id, lot),
                PlannedParcelChange::Remove(id) => {
                    self.remove_local(id);
                }
                PlannedParcelChange::ReplaceGeometry(id, geometry) => {
                    self.replace_geometry(id, geometry);
                }
            }
        }
    }

    /// [`Self::overlaps_existing`] that ignores the parcels `skip` names.
    pub(crate) fn overlaps_existing_unless(
        &self,
        geometry: &ParcelGeometry,
        skip: impl Fn(ParcelId) -> bool,
    ) -> bool {
        let mut visited = HashSet::new();
        for chunk in chunks_for_aabb(geometry.aabb_min, geometry.aabb_max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                if skip(id) || !visited.insert(id) {
                    continue;
                }
                let Some(parcel) = self.get(id) else {
                    continue;
                };
                if rectangles_overlap_geometry(geometry, parcel) {
                    return true;
                }
            }
        }
        false
    }
}
