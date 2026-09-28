// SPDX-License-Identifier: GPL-2.0-only

//! Road-local parcel removal and exact dense-storage inverse journals.

use super::{ParcelStore, ZoningParcel, removed_with_road};
use rayon::prelude::*;

impl ParcelStore {
    /// Captures removable attachments in original storage order without a city-wide scan.
    pub(crate) fn capture_attached_to_edge(&self, edge_idx: usize) -> Vec<(usize, ZoningParcel)> {
        let Some(ids) = self.edge_index.get(&edge_idx) else {
            return Vec::new();
        };
        let mut captured: Vec<_> = ids
            .par_iter()
            .with_min_len(256)
            .filter_map(|id| {
                let index = self.id_to_index[id];
                let parcel = &self.parcels[index];
                removed_with_road(parcel, edge_idx).then(|| (index, parcel.clone()))
            })
            .collect();
        captured.sort_unstable_by_key(|(index, _)| *index);
        captured
    }

    /// Removes attachments in reverse storage order, retaining occupied cell-derived lots.
    pub(crate) fn remove_attached_to_edge(
        &mut self,
        edge_idx: usize,
        mut removed: impl FnMut(&ZoningParcel),
    ) -> usize {
        let Some(ids) = self.edge_index.get(&edge_idx) else {
            return 0;
        };
        let mut indices: Vec<_> = ids
            .par_iter()
            .with_min_len(256)
            .filter_map(|id| {
                let index = self.id_to_index[id];
                removed_with_road(&self.parcels[index], edge_idx).then_some(index)
            })
            .collect();
        indices.sort_unstable_by(|a, b| b.cmp(a));
        // Descending removals keep each yet-unvisited original index valid. The inverse
        // replays these swaps backwards; mutation and callback publication are sequential.
        for &index in &indices {
            let id = self.parcels[index].id();
            let parcel = self.remove_local(id).expect("indexed road parcel");
            removed(&parcel);
        }
        indices.len()
    }

    /// Checks the length and ordered, absent identities required by a removal inverse.
    pub(crate) fn can_restore_removed(
        &self,
        original_count: usize,
        removed: &[(usize, ZoningParcel)],
    ) -> bool {
        if self.parcels.len().saturating_add(removed.len()) != original_count {
            return false;
        }
        let mut previous_index = None;
        for (index, parcel) in removed {
            if *index >= original_count
                || previous_index.is_some_and(|previous| previous >= *index)
                || self.id_to_index.contains_key(&parcel.id())
            {
                return false;
            }
            previous_index = Some(*index);
        }
        true
    }

    /// Reverses descending removals, repairing only restored/moved records and their chunks.
    pub(crate) fn restore_removed(
        &mut self,
        original_count: usize,
        removed: Vec<(usize, ZoningParcel)>,
    ) {
        debug_assert!(self.can_restore_removed(original_count, &removed));
        for (index, parcel) in removed {
            let end = self.parcels.len();
            let restored = self.restore_local(parcel);
            debug_assert!(restored);
            if index != end {
                self.parcels.swap(index, end);
                self.edge_slots.swap(index, end);
                self.id_to_index.insert(self.parcels[index].id(), index);
                self.id_to_index.insert(self.parcels[end].id(), end);
                self.reorder_chunk_entries(&[index, end]);
            }
        }
    }
}
