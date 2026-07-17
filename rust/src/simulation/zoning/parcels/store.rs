//! Stable parcel storage and chunk-local parcel lookup.

use super::geometry::{
    chunks_for_aabb, point_inside_parcel, rectangles_overlap_geometry, segment_touches_parcel,
};
use super::types::{ParcelGeometry, ParcelId, ZoningParcel};
use godot::prelude::Vector2;
use std::collections::{HashMap, HashSet};

/// Stable parcel collection plus its coarse chunk index.
#[derive(Clone, Debug)]
pub struct ParcelStore {
    parcels: Vec<ZoningParcel>,
    id_to_index: HashMap<ParcelId, usize>,
    chunk_index: HashMap<(i32, i32), Vec<ParcelId>>,
    next_id: u64,
}

impl Default for ParcelStore {
    fn default() -> Self {
        Self {
            parcels: Vec::new(),
            id_to_index: HashMap::new(),
            chunk_index: HashMap::new(),
            next_id: 1,
        }
    }
}

impl ParcelStore {
    /// Returns every parcel in stable storage order.
    pub fn parcels(&self) -> &[ZoningParcel] {
        &self.parcels
    }

    /// Returns the parcel for one stable id.
    pub fn get(&self, id: ParcelId) -> Option<&ZoningParcel> {
        let index = *self.id_to_index.get(&id)?;
        self.parcels.get(index)
    }

    /// Returns a mutable parcel reference for one stable id.
    pub fn get_mut(&mut self, id: ParcelId) -> Option<&mut ZoningParcel> {
        let index = *self.id_to_index.get(&id)?;
        self.parcels.get_mut(index)
    }

    /// Removes all parcels and resets id allocation.
    pub fn clear(&mut self) {
        self.parcels.clear();
        self.id_to_index.clear();
        self.chunk_index.clear();
        self.next_id = 1;
    }

    pub(crate) fn insert_new(
        &mut self,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) -> ParcelId {
        let id = ParcelId::from_raw(self.next_id);
        self.next_id = self.next_id.saturating_add(1).max(1);
        self.insert_with_id(id, geometry, zone_profile_runtime_id);
        id
    }

    pub(crate) fn insert_loaded(
        &mut self,
        id: ParcelId,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) {
        self.next_id = self.next_id.max(id.raw().saturating_add(1)).max(1);
        self.insert_with_id(id, geometry, zone_profile_runtime_id);
    }

    fn insert_with_id(
        &mut self,
        id: ParcelId,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) {
        let parcel = ZoningParcel::new(id, geometry, zone_profile_runtime_id);
        let index = self.parcels.len();
        self.parcels.push(parcel);
        self.id_to_index.insert(id, index);
        self.index_parcel(index);
    }

    pub(crate) fn remove_edges_not_in_mapping(&mut self, mapping: &HashMap<usize, usize>) -> bool {
        let mut changed = false;
        for parcel in &mut self.parcels {
            if let Some(&new_idx) = mapping.get(&parcel.edge_idx()) {
                if parcel.edge_idx() != new_idx {
                    parcel.set_edge_idx(new_idx);
                    changed = true;
                }
            } else {
                parcel.set_edge_idx(usize::MAX);
                changed = true;
            }
        }
        let before = self.parcels.len();
        self.parcels
            .retain(|parcel| parcel.edge_idx() != usize::MAX);
        changed |= self.parcels.len() != before;
        if changed {
            self.rebuild_indices();
        }
        changed
    }

    pub(crate) fn capture_attached_to_edge(&self, edge_idx: usize) -> Vec<(usize, ZoningParcel)> {
        self.parcels
            .iter()
            .enumerate()
            .filter(|(_, parcel)| parcel.edge_idx() == edge_idx)
            .map(|(index, parcel)| (index, parcel.clone()))
            .collect()
    }

    pub(crate) fn remove_attached_to_edge(&mut self, edge_idx: usize) -> usize {
        let before = self.parcels.len();
        self.parcels.retain(|parcel| parcel.edge_idx() != edge_idx);
        let removed = before - self.parcels.len();
        if removed > 0 {
            self.rebuild_indices();
        }
        removed
    }

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

    pub(crate) fn restore_removed(
        &mut self,
        original_count: usize,
        removed: Vec<(usize, ZoningParcel)>,
    ) {
        debug_assert!(self.can_restore_removed(original_count, &removed));
        let mut retained = std::mem::take(&mut self.parcels).into_iter();
        let mut removed = removed.into_iter().peekable();
        let mut restored = Vec::with_capacity(original_count);

        for index in 0..original_count {
            if removed
                .peek()
                .is_some_and(|(removed_index, _)| *removed_index == index)
            {
                restored.push(removed.next().expect("peeked zoning undo parcel").1);
            } else {
                restored.push(
                    retained
                        .next()
                        .expect("prevalidated retained zoning parcel"),
                );
            }
        }
        debug_assert!(removed.next().is_none());
        debug_assert!(retained.next().is_none());

        self.parcels = restored;
        self.rebuild_indices();
    }

    pub(crate) fn find_at_point(&self, point: Vector2) -> Option<ParcelId> {
        let chunk = super::geometry::chunk_key(point);
        let ids = self.chunk_index.get(&chunk)?;
        ids.iter().copied().find(|&id| {
            self.get(id)
                .map(|parcel| point_inside_parcel(point, parcel))
                .unwrap_or(false)
        })
    }

    pub(crate) fn find_touching_segment(&self, start: Vector2, end: Vector2) -> Vec<ParcelId> {
        if start.distance_squared_to(end) <= super::OVERLAP_EPSILON_M * super::OVERLAP_EPSILON_M {
            return self.find_at_point(start).into_iter().collect();
        }

        let min = Vector2::new(start.x.min(end.x), start.y.min(end.y));
        let max = Vector2::new(start.x.max(end.x), start.y.max(end.y));
        let mut visited = HashSet::new();
        let mut touched = Vec::new();
        for chunk in chunks_for_aabb(min, max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                if !visited.insert(id) {
                    continue;
                }
                let Some(parcel) = self.get(id) else {
                    continue;
                };
                if segment_touches_parcel(start, end, parcel) {
                    touched.push(id);
                }
            }
        }
        touched.sort_unstable();
        touched
    }

    pub(crate) fn overlaps_existing(&self, geometry: &ParcelGeometry) -> bool {
        let mut visited = HashSet::new();
        self.overlaps_existing_with_scratch(geometry, &mut visited)
    }

    pub(crate) fn overlaps_existing_with_scratch(
        &self,
        geometry: &ParcelGeometry,
        visited: &mut HashSet<ParcelId>,
    ) -> bool {
        for chunk in chunks_for_aabb(geometry.aabb_min, geometry.aabb_max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                if !visited.insert(id) {
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

    pub(crate) fn overlaps_existing_except(
        &self,
        geometry: &ParcelGeometry,
        ignored_id: ParcelId,
    ) -> bool {
        let mut visited = HashSet::new();
        for chunk in chunks_for_aabb(geometry.aabb_min, geometry.aabb_max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                if id == ignored_id || !visited.insert(id) {
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

    pub(crate) fn set_zone_profile_runtime_id(&mut self, id: ParcelId, runtime_id: u16) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        if parcel.zone_profile_runtime_id() == runtime_id {
            return false;
        }
        parcel.set_zone_profile_runtime_id(runtime_id);
        true
    }

    pub(crate) fn set_occupied_building(&mut self, id: ParcelId, building_idx: usize) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        if parcel.occupied_building().is_some() {
            return false;
        }
        parcel.set_occupied_building(Some(building_idx));
        true
    }

    pub(crate) fn clear_occupied_building(&mut self, id: ParcelId) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        if parcel.occupied_building().is_none() {
            return false;
        }
        parcel.set_occupied_building(None);
        true
    }

    pub(crate) fn remap_occupied_building(&mut self, old_idx: usize, new_idx: usize) -> bool {
        let mut changed = false;
        for parcel in &mut self.parcels {
            if parcel.occupied_building() == Some(old_idx) {
                parcel.set_occupied_building(Some(new_idx));
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn clear_all_occupancy(&mut self) -> bool {
        let mut changed = false;
        for parcel in &mut self.parcels {
            if parcel.occupied_building().is_some() {
                parcel.set_occupied_building(None);
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn replace_geometry(&mut self, id: ParcelId, geometry: ParcelGeometry) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        parcel.replace_geometry(geometry);
        self.rebuild_chunk_index();
        true
    }

    fn index_parcel(&mut self, index: usize) {
        let parcel = &self.parcels[index];
        for chunk in chunks_for_aabb(parcel.aabb_min(), parcel.aabb_max()) {
            self.chunk_index.entry(chunk).or_default().push(parcel.id());
        }
    }

    fn rebuild_chunk_index(&mut self) {
        self.chunk_index.clear();
        for idx in 0..self.parcels.len() {
            self.index_parcel(idx);
        }
    }

    fn rebuild_indices(&mut self) {
        self.id_to_index.clear();
        for (idx, parcel) in self.parcels.iter().enumerate() {
            self.id_to_index.insert(parcel.id(), idx);
        }
        self.rebuild_chunk_index();
    }
}
