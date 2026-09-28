// SPDX-License-Identifier: GPL-2.0-only

//! Stable parcel storage and chunk-local parcel lookup.

mod removal;
#[cfg(test)]
mod tests;

use super::geometry::{
    chunk_key, chunks_for_aabb, geometry_for_parcel, geometry_overlaps_road_corridor_segment,
    point_inside_parcel, rectangles_overlap_geometry, segment_touches_parcel,
};
use super::types::{ParcelGeometry, ParcelId, ZoningParcel};
use crate::simulation::agriculture::PolygonFootprint;
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

/// Stable parcel collection with chunk lookup and road-attachment membership.
#[derive(Clone, Debug)]
pub struct ParcelStore {
    parcels: Vec<ZoningParcel>,
    id_to_index: HashMap<ParcelId, usize>,
    chunk_index: HashMap<(i32, i32), Vec<ParcelId>>,
    edge_index: HashMap<usize, Vec<ParcelId>>,
    // Parallel to dense parcels: each record's slot in its edge's unordered membership list.
    edge_slots: Vec<usize>,
    chunk_revisions: HashMap<(i32, i32), u64>,
    overlay_revision: u64,
    next_id: u64,
}

impl Default for ParcelStore {
    fn default() -> Self {
        Self {
            parcels: Vec::new(),
            id_to_index: HashMap::new(),
            chunk_index: HashMap::new(),
            edge_index: HashMap::new(),
            edge_slots: Vec::new(),
            chunk_revisions: HashMap::new(),
            overlay_revision: 0,
            next_id: 1,
        }
    }
}

impl ParcelStore {
    /// Local geometry/designation/occupancy version for the existing 512 m parcel chunk.
    pub(crate) fn chunk_revision(&self, chunk: (i32, i32)) -> u64 {
        self.chunk_revisions.get(&chunk).copied().unwrap_or(0)
    }

    fn mark_overlay_bounds(&mut self, min: Vector2, max: Vector2) {
        mark_chunk_revisions(
            &mut self.chunk_revisions,
            &mut self.overlay_revision,
            min,
            max,
        );
    }

    /// Returns dense storage; callers use parcel ids when order must survive local removal.
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
        self.edge_index.clear();
        self.edge_slots.clear();
        self.chunk_revisions.clear();
        self.overlay_revision = self.overlay_revision.wrapping_add(1);
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
        self.mark_overlay_bounds(geometry.aabb_min, geometry.aabb_max);
    }

    /// Removes one known stable id, updating only its chunks and the moved storage slot.
    pub(crate) fn remove_local(&mut self, id: ParcelId) -> Option<ZoningParcel> {
        let index = self.id_to_index.remove(&id)?;
        let removed = self.parcels.swap_remove(index);
        let edge_slot = self.edge_slots.swap_remove(index);
        if let Some(moved) = self.parcels.get(index) {
            self.id_to_index.insert(moved.id(), index);
        }
        self.unindex_edge(id, removed.edge_idx(), edge_slot);
        for chunk in chunks_for_aabb(removed.aabb_min(), removed.aabb_max()) {
            if let Some(ids) = self.chunk_index.get_mut(&chunk) {
                ids.retain(|candidate| *candidate != id);
                if ids.is_empty() {
                    self.chunk_index.remove(&chunk);
                }
            }
        }
        if index < self.parcels.len() {
            self.reorder_chunk_entries(&[index]);
        }
        self.mark_overlay_bounds(removed.aabb_min(), removed.aabb_max());
        Some(removed)
    }

    /// Restores one detached record, retaining its identity, geometry and redevelopment state.
    pub(crate) fn restore_local(&mut self, parcel: ZoningParcel) -> bool {
        let id = parcel.id();
        if id.is_none() || self.id_to_index.contains_key(&id) {
            return false;
        }
        self.next_id = self.next_id.max(id.raw().saturating_add(1)).max(1);
        let index = self.parcels.len();
        self.parcels.push(parcel);
        self.id_to_index.insert(id, index);
        self.index_parcel(index);
        let parcel = &self.parcels[index];
        self.mark_overlay_bounds(parcel.aabb_min(), parcel.aabb_max());
        true
    }

    pub(crate) fn remove_ids(
        &mut self,
        ids: &HashSet<ParcelId>,
        mut removed: impl FnMut(&ZoningParcel),
    ) -> usize {
        if ids.is_empty() {
            return 0;
        }
        let before = self.parcels.len();
        let revisions = &mut self.chunk_revisions;
        let revision = &mut self.overlay_revision;
        self.parcels.retain(|parcel| {
            if !ids.contains(&parcel.id()) {
                return true;
            }
            removed(parcel);
            mark_chunk_revisions(revisions, revision, parcel.aabb_min(), parcel.aabb_max());
            false
        });
        let removed = before - self.parcels.len();
        if removed > 0 {
            self.rebuild_indices();
        }
        removed
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

    /// Finds attached parcels in a 3D road-station interval through the existing chunk index.
    /// Only chunks beside the clipped interval are visited; results follow station then stable ID.
    pub(crate) fn attached_to_road_span(
        &self,
        edge_idx: usize,
        edge: &crate::simulation::network::graph::Edge,
        range: [f32; 2],
    ) -> Vec<&ZoningParcel> {
        let pad = edge.width * 0.5
            + crate::config::SIDEWALK_WIDTH
            + crate::simulation::zoning::MIN_PARCEL_DEPTH_M * 0.5;
        let mut station = 0.0;
        let mut chunks_seen = HashSet::new();
        let mut parcels_seen = HashSet::new();
        let mut found = Vec::new();
        for points in edge.physical_geometry.windows(2) {
            if station > range[1] {
                break;
            }
            let length = points[0].distance_to(points[1]);
            let start_s = station;
            station += length;
            if length <= 1e-6 || station < range[0] {
                continue;
            }
            let start = points[0].lerp(points[1], ((range[0] - start_s) / length).clamp(0.0, 1.0));
            let end = points[0].lerp(points[1], ((range[1] - start_s) / length).clamp(0.0, 1.0));
            let min = Vector2::new(start.x.min(end.x) - pad, start.z.min(end.z) - pad);
            let max = Vector2::new(start.x.max(end.x) + pad, start.z.max(end.z) + pad);
            for chunk in chunks_for_aabb(min, max) {
                if !chunks_seen.insert(chunk) {
                    continue;
                }
                let Some(ids) = self.chunk_index.get(&chunk) else {
                    continue;
                };
                for &id in ids {
                    if !parcels_seen.insert(id) {
                        continue;
                    }
                    let Some(parcel) = self.get(id) else {
                        continue;
                    };
                    let center_s = parcel.frontage_center_t() * edge.physical_length;
                    if parcel.edge_idx() == edge_idx && center_s >= range[0] && center_s <= range[1]
                    {
                        found.push(parcel);
                    }
                }
            }
        }
        found.sort_unstable_by(|a, b| {
            a.frontage_center_t()
                .total_cmp(&b.frontage_center_t())
                .then_with(|| a.id().cmp(&b.id()))
        });
        found
    }

    pub(crate) fn ids_overlapping_road_corridor(
        &self,
        points: &[Vector3],
        half_width_m: f32,
    ) -> Vec<ParcelId> {
        let mut tested = HashSet::new();
        let mut overlapped = HashSet::new();
        self.collect_ids_overlapping_corridor(points, half_width_m, &mut tested, &mut overlapped);
        let mut ids: Vec<_> = overlapped.into_iter().collect();
        ids.sort_unstable();
        ids
    }

    fn collect_ids_overlapping_corridor(
        &self,
        points: &[Vector3],
        half_width_m: f32,
        tested: &mut HashSet<(ParcelId, usize)>,
        overlapped: &mut HashSet<ParcelId>,
    ) {
        if points.len() < 2 || half_width_m <= 0.0 || !half_width_m.is_finite() {
            return;
        }
        for (segment_idx, window) in points.windows(2).enumerate() {
            let start = Vector2::new(window[0].x, window[0].z);
            let end = Vector2::new(window[1].x, window[1].z);
            if start.distance_squared_to(end) <= super::OVERLAP_EPSILON_M * super::OVERLAP_EPSILON_M
            {
                continue;
            }
            let min = Vector2::new(
                start.x.min(end.x) - half_width_m,
                start.y.min(end.y) - half_width_m,
            );
            let max = Vector2::new(
                start.x.max(end.x) + half_width_m,
                start.y.max(end.y) + half_width_m,
            );
            for chunk in chunks_for_aabb(min, max) {
                let Some(ids) = self.chunk_index.get(&chunk) else {
                    continue;
                };
                for &id in ids {
                    if overlapped.contains(&id) || !tested.insert((id, segment_idx)) {
                        continue;
                    }
                    let Some(parcel) = self.get(id) else {
                        continue;
                    };
                    let geometry = geometry_for_parcel(parcel);
                    if geometry_overlaps_road_corridor_segment(&geometry, start, end, half_width_m)
                    {
                        overlapped.insert(id);
                    }
                }
            }
        }
    }

    pub(crate) fn overlaps_existing(&self, geometry: &ParcelGeometry) -> bool {
        let mut visited = HashSet::new();
        self.overlaps_existing_with_scratch(geometry, &mut visited)
    }

    /// Visits local parcel AABB candidates once without allocating a duplicate-id set.
    pub(crate) fn visit_in_bounds(
        &self,
        min: Vector2,
        max: Vector2,
        mut visit: impl FnMut(&ZoningParcel),
    ) {
        for chunk in chunks_for_aabb(min, max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for &id in ids {
                let Some(parcel) = self.get(id) else {
                    continue;
                };
                let pmin = parcel.aabb_min();
                let pmax = parcel.aabb_max();
                if pmin.x > max.x || pmin.y > max.y || pmax.x < min.x || pmax.y < min.y {
                    continue;
                }
                let first = chunk_key(Vector2::new(min.x.max(pmin.x), min.y.max(pmin.y)));
                if first == chunk {
                    visit(parcel);
                }
            }
        }
    }

    /// Tests an arbitrary field polygon against locally indexed authored parcels.
    pub(crate) fn overlaps_polygon(&self, footprint: &PolygonFootprint) -> bool {
        let mut tested = HashSet::new();
        for chunk in chunks_for_aabb(footprint.min, footprint.max) {
            let Some(ids) = self.chunk_index.get(&chunk) else {
                continue;
            };
            for id in ids {
                if tested.insert(*id)
                    && self.get(*id).is_some_and(|parcel| {
                        footprint.overlaps(&PolygonFootprint::new(&parcel.corners()))
                    })
                {
                    return true;
                }
            }
        }
        false
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
        let bounds = (parcel.aabb_min(), parcel.aabb_max());
        self.mark_overlay_bounds(bounds.0, bounds.1);
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
        let bounds = (parcel.aabb_min(), parcel.aabb_max());
        self.mark_overlay_bounds(bounds.0, bounds.1);
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
        // Demolition only. A world reset clears every claim through `clear_all_occupancy`,
        // which must not advance the counter or loading a save would recolour the city.
        parcel.advance_build_generation();
        let bounds = (parcel.aabb_min(), parcel.aabb_max());
        self.mark_overlay_bounds(bounds.0, bounds.1);
        true
    }

    pub(crate) fn remap_occupied_building(
        &mut self,
        id: ParcelId,
        old_idx: usize,
        new_idx: usize,
    ) -> bool {
        let Some(parcel) = self.get_mut(id) else {
            return false;
        };
        if parcel.occupied_building() != Some(old_idx) {
            return false;
        }
        parcel.set_occupied_building(Some(new_idx));
        true
    }

    pub(crate) fn clear_all_occupancy(&mut self) -> bool {
        let mut changed = false;
        for parcel in &mut self.parcels {
            if parcel.occupied_building().is_some() {
                parcel.set_occupied_building(None);
                mark_chunk_revisions(
                    &mut self.chunk_revisions,
                    &mut self.overlay_revision,
                    parcel.aabb_min(),
                    parcel.aabb_max(),
                );
                changed = true;
            }
        }
        changed
    }

    pub(crate) fn replace_geometry(&mut self, id: ParcelId, geometry: ParcelGeometry) -> bool {
        let Some(&index) = self.id_to_index.get(&id) else {
            return false;
        };
        let parcel = &mut self.parcels[index];
        let old_min = parcel.aabb_min();
        let old_max = parcel.aabb_max();
        let old_edge = parcel.edge_idx();
        let old_chunk_min = chunk_key(old_min);
        let old_chunk_max = chunk_key(old_max);
        let new_chunk_min = chunk_key(geometry.aabb_min);
        let new_chunk_max = chunk_key(geometry.aabb_max);
        parcel.replace_geometry(geometry);
        if old_edge != geometry.edge_idx {
            self.unindex_edge(id, old_edge, self.edge_slots[index]);
            let ids = self.edge_index.entry(geometry.edge_idx).or_default();
            self.edge_slots[index] = ids.len();
            ids.push(id);
        }

        let contains = |key: (i32, i32), min: (i32, i32), max: (i32, i32)| {
            key.0 >= min.0 && key.0 <= max.0 && key.1 >= min.1 && key.1 <= max.1
        };
        // Retain shared chunks untouched. Work follows the old/new footprint and the records
        // in changed chunks; distant chunks and their index entries are never traversed.
        for key in chunks_for_aabb(old_min, old_max) {
            if contains(key, new_chunk_min, new_chunk_max) {
                continue;
            }
            if let Some(ids) = self.chunk_index.get_mut(&key) {
                ids.retain(|&existing| existing != id);
                if ids.is_empty() {
                    self.chunk_index.remove(&key);
                }
            }
        }
        for key in chunks_for_aabb(geometry.aabb_min, geometry.aabb_max) {
            if contains(key, old_chunk_min, old_chunk_max) {
                continue;
            }
            let ids = self.chunk_index.entry(key).or_default();
            // Picks use storage order, which can differ from loaded parcel-ID order.
            let position = ids.partition_point(|existing| self.id_to_index[existing] < index);
            ids.insert(position, id);
        }
        self.mark_overlay_bounds(old_min, old_max);
        self.mark_overlay_bounds(geometry.aabb_min, geometry.aabb_max);
        true
    }

    fn index_parcel(&mut self, index: usize) {
        let parcel = &self.parcels[index];
        let ids = self.edge_index.entry(parcel.edge_idx()).or_default();
        self.edge_slots.push(ids.len());
        ids.push(parcel.id());
        for chunk in chunks_for_aabb(parcel.aabb_min(), parcel.aabb_max()) {
            self.chunk_index.entry(chunk).or_default().push(parcel.id());
        }
    }

    fn unindex_edge(&mut self, id: ParcelId, edge: usize, slot: usize) {
        let ids = self.edge_index.get_mut(&edge).expect("indexed parcel edge");
        let removed = ids.swap_remove(slot);
        debug_assert_eq!(removed, id);
        if let Some(moved) = ids.get(slot) {
            self.edge_slots[self.id_to_index[moved]] = slot;
        }
        if ids.is_empty() {
            self.edge_index.remove(&edge);
        }
    }

    // Dense swaps change pick precedence. Remove both swapped entries before binary insertion
    // so every searched chunk list remains ordered; only their footprints are visited.
    fn reorder_chunk_entries(&mut self, indices: &[usize]) {
        for &index in indices {
            let parcel = &self.parcels[index];
            for chunk in chunks_for_aabb(parcel.aabb_min(), parcel.aabb_max()) {
                if let Some(ids) = self.chunk_index.get_mut(&chunk) {
                    ids.retain(|&id| id != parcel.id());
                }
            }
        }
        for &index in indices {
            let parcel = &self.parcels[index];
            for chunk in chunks_for_aabb(parcel.aabb_min(), parcel.aabb_max()) {
                let ids = self.chunk_index.entry(chunk).or_default();
                let position = ids.partition_point(|existing| self.id_to_index[existing] < index);
                ids.insert(position, parcel.id());
            }
        }
    }

    fn rebuild_indices(&mut self) {
        self.id_to_index.clear();
        self.chunk_index.clear();
        self.edge_index.clear();
        self.edge_slots.clear();
        for (idx, parcel) in self.parcels.iter().enumerate() {
            self.id_to_index.insert(parcel.id(), idx);
        }
        for idx in 0..self.parcels.len() {
            self.index_parcel(idx);
        }
    }
}

// Occupied cell coverage belongs to the building lifecycle, including erased cells. Road
// removal and its inverse must agree on which records are actually detached.
fn removed_with_road(parcel: &ZoningParcel, edge_idx: usize) -> bool {
    parcel.edge_idx() == edge_idx && (parcel.cell_lot().is_none() || parcel.is_available())
}

fn mark_chunk_revisions(
    revisions: &mut HashMap<(i32, i32), u64>,
    revision: &mut u64,
    min: Vector2,
    max: Vector2,
) {
    *revision = revision.wrapping_add(1);
    for chunk in chunks_for_aabb(min, max) {
        revisions.insert(chunk, *revision);
    }
}
