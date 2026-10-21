// SPDX-License-Identifier: GPL-2.0-only

//! Building search indices and vacancy management.

use crate::debug_log;
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::HashMap;

// One building's index entries, derived on the pool by a zone-index rebuild, so its ordered
// fill reads these compact rows instead of the buildings (`SIM-02-K`).
#[derive(Clone, Copy)]
struct IndexedBuilding {
    chunk: (i32, i32),
    // An explicit cell site: no parcel and no zone.
    explicit_site: bool,
    support_height_m: f32,
    // Half-diagonal of the lot in zoning cells.
    lot_radius_cells: f32,
    // Zone list of a completed baseline private building.
    zone: Option<usize>,
    // Vacancy list of a completed home with a free household place.
    vacancy: Option<usize>,
}

/// Reused storage of [`BuildingAllocator::rebuild_zone_index`]; derived, never saved.
#[derive(Clone, Default)]
pub(crate) struct IndexScratch {
    entries: Vec<IndexedBuilding>,
    // The chunk lists during a rebuild, by cell of the dense grid over the chunk bounds;
    // empty between rebuilds, some keeping storage.
    grid_lists: Vec<Vec<usize>>,
}

// The dense grid over chunk bounds `[min x, min z, max x, max z]`, x-major, so cells follow
// chunk order. None for no chunks or a grid of more than four cells a building, which fills
// the map directly.
struct ChunkGrid {
    bounds: [i32; 4],
    depth: usize,
    cells: usize,
}

impl ChunkGrid {
    fn new(bounds: Option<[i32; 4]>, buildings: usize) -> Option<Self> {
        let bounds = bounds?;
        let width = (i64::from(bounds[2]) - i64::from(bounds[0]) + 1) as usize;
        let depth = (i64::from(bounds[3]) - i64::from(bounds[1]) + 1) as usize;
        let cells = width.checked_mul(depth)?;
        (cells <= 4 * buildings + 64).then_some(Self {
            bounds,
            depth,
            cells,
        })
    }

    fn contains(&self, (x, z): (i32, i32)) -> bool {
        (self.bounds[0]..=self.bounds[2]).contains(&x)
            && (self.bounds[1]..=self.bounds[3]).contains(&z)
    }

    // A chunk within the bounds; O(1).
    fn cell(&self, (x, z): (i32, i32)) -> usize {
        (x - self.bounds[0]) as usize * self.depth + (z - self.bounds[1]) as usize
    }

    fn chunk(&self, cell: usize) -> (i32, i32) {
        (
            self.bounds[0] + (cell / self.depth) as i32,
            self.bounds[1] + (cell % self.depth) as i32,
        )
    }
}

fn merge_chunk_bounds(a: Option<[i32; 4]>, b: Option<[i32; 4]>) -> Option<[i32; 4]> {
    match (a, b) {
        (Some(a), Some(b)) => Some([
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]),
        (a, None) => a,
        (None, b) => b,
    }
}

/// The vacancy lists as a save holds them (`SIM-02`): claims and releases leave them out of
/// building order, admission, rehousing and the admission order read positions in them, and a
/// rebuild recovers only building order.
#[derive(Clone, Debug, Default)]
pub(crate) struct SavedVacancyOrder {
    /// The saved index was stale; its next reader rebuilds it, so no order is kept.
    pub(crate) stale: bool,
    /// Each vacancy list's building ids in order, by zone slot.
    pub(crate) lists: [Vec<usize>; 3],
}

impl BuildingAllocator {
    /// Finds the nearest building centre strictly within the inspector's 30 m pick radius.
    /// Ties prefer the lowest building index. Clean queries visit at most four 512 m chunks
    /// without allocating; a stale allocator index is rebuilt before searching.
    pub(crate) fn nearest_building_idx_at(&mut self, world_x: f32, world_z: f32) -> Option<usize> {
        if !world_x.is_finite() || !world_z.is_finite() {
            return None;
        }
        if self.dirty_index {
            self.rebuild_zone_index();
        }
        let radius = 30.0;
        let min_chunk =
            RegionGraph::get_chunk_coords(Vector3::new(world_x - radius, 0.0, world_z - radius));
        let max_chunk =
            RegionGraph::get_chunk_coords(Vector3::new(world_x + radius, 0.0, world_z + radius));
        let mut best = None;
        let mut best_distance_sq = radius * radius;
        for chunk_x in min_chunk.0..=max_chunk.0 {
            for chunk_z in min_chunk.1..=max_chunk.1 {
                let Some(indices) = self.building_chunks.get(&(chunk_x, chunk_z)) else {
                    continue;
                };
                for &idx in indices {
                    let building = &self.buildings[idx];
                    let dx = building.center_x - world_x;
                    let dz = building.center_y - world_z;
                    let distance_sq = dx * dx + dz * dz;
                    if distance_sq < best_distance_sq
                        || (distance_sq == best_distance_sq
                            && best.is_some_and(|previous| idx < previous))
                    {
                        best = Some(idx);
                        best_distance_sq = distance_sq;
                    }
                }
            }
        }
        best
    }

    /// Repopulates the internal zone and vacancy indices (Bug B16/B16a fix).
    ///
    /// Each building's entries are derived on the pool, then filled in building order, the
    /// chunk lists too: each list moves into a dense grid over the chunk bounds for the fill
    /// and back into the map, keeping its storage. The indices equal a serial pass's
    /// (`SIM-02-K`). O(B/p + chunks) plus an O(B) ordered fill.
    pub fn rebuild_zone_index(&mut self) {
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        // Spare rows, so the first building appended after a load does not copy them.
        crate::utils::storage::reserve_rows(&mut self.vacancy_pos, self.buildings.len());
        self.vacancy_pos.resize(self.buildings.len(), usize::MAX);
        self.building_chunk_bounds = None;
        self.max_building_support_m = f32::NEG_INFINITY;
        self.max_lot_radius_cells = 0.0;
        self.explicit_cell_site_count = 0;

        let mut entries = std::mem::take(&mut self.index_scratch.entries);
        let mut grid_lists = std::mem::take(&mut self.index_scratch.grid_lists);
        // Main-thread index repair reaches here outside any pool (CODE-17).
        let bounds = crate::thread_pools::install(|| {
            self.buildings
                .par_iter()
                .enumerate()
                .map(|(idx, b)| self.indexed_building(idx, b))
                .collect_into_vec(&mut entries);
            entries
                .par_iter()
                .map(|entry| Some([entry.chunk.0, entry.chunk.1, entry.chunk.0, entry.chunk.1]))
                .reduce(|| None, merge_chunk_bounds)
        });
        let grid = ChunkGrid::new(bounds, entries.len());
        if let Some(grid) = &grid {
            grid_lists.resize_with(grid.cells, Vec::new);
            for (chunk, mut list) in self.building_chunks.drain() {
                if grid.contains(chunk) {
                    list.clear();
                    grid_lists[grid.cell(chunk)] = list;
                }
            }
        } else {
            for list in self.building_chunks.values_mut() {
                list.clear();
            }
        }
        for (idx, entry) in entries.iter().enumerate() {
            self.explicit_cell_site_count += usize::from(entry.explicit_site);
            include_chunk(&mut self.building_chunk_bounds, entry.chunk);
            self.max_building_support_m = self.max_building_support_m.max(entry.support_height_m);
            self.max_lot_radius_cells = self.max_lot_radius_cells.max(entry.lot_radius_cells);
            if let Some(zi) = entry.zone {
                self.zone_index[zi].push(idx);
            }
            if let Some(zi) = entry.vacancy {
                self.vacancy_pos[idx] = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(idx);
            }
            match &grid {
                Some(grid) => grid_lists[grid.cell(entry.chunk)].push(idx),
                None => self
                    .building_chunks
                    .entry(entry.chunk)
                    .or_default()
                    .push(idx),
            }
        }
        if let Some(grid) = &grid {
            for (cell, list) in grid_lists.iter_mut().enumerate() {
                if !list.is_empty() {
                    self.building_chunks
                        .insert(grid.chunk(cell), std::mem::take(list));
                }
            }
        }
        self.building_chunks.retain(|_, list| !list.is_empty());
        #[cfg(test)]
        if crate::debug::work::city_checks() {
            let mut direct = HashMap::<(i32, i32), Vec<usize>>::new();
            for (idx, entry) in entries.iter().enumerate() {
                direct.entry(entry.chunk).or_default().push(idx);
            }
            assert!(
                direct == self.building_chunks,
                "the chunk lists hold each chunk's buildings in building order"
            );
        }
        self.index_scratch = IndexScratch {
            entries,
            grid_lists,
        };
        self.rebuild_admission_order();
        self.dirty_index = false;
        // The zone lists name the flow fields' sources (`SIM-02-I`): the next frame compares
        // each zone's sources and rebuilds only fields whose sources changed.
        self.dirty_zones = [true; 3];
    }

    // Building `idx`'s index entries. O(1).
    fn indexed_building(&self, idx: usize, b: &super::Building) -> IndexedBuilding {
        let half_width = b.width_cells as f32 * 0.5;
        let half_depth = b.depth_cells as f32 * 0.5;
        let mut entry = IndexedBuilding {
            chunk: RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y)),
            explicit_site: b.parcel_id == 0 && b.zone_type == ZoneType::None,
            support_height_m: b.support_height_m,
            lot_radius_cells: half_width.hypot(half_depth),
            zone: None,
            vacancy: None,
        };
        if !b.is_under_construction() {
            entry.zone = baseline_private_zone_slot(b.zone_type);
            entry.vacancy = self.housing_vacancy_slot(idx).filter(|_| {
                let resident_cap = self.household_capacity(idx);
                resident_cap > 0 && b.occupancy < resident_cap
            });
        }
        entry
    }

    /// Whether the vacancy lists may be stale: a stale index is rebuilt by its next reader.
    pub(crate) fn vacancy_index_stale(&self) -> bool {
        self.dirty_index || self.vacancy_pos.len() != self.buildings.len()
    }

    /// Restores a saved vacancy order over the index a load just rebuilt with the live asset
    /// registry, so the loaded game picks the homes the saved one would. Where a saved list does
    /// not hold exactly the rebuilt vacancies of its slot, the rebuild stands: the asset packs
    /// changed capacities since the save, or a catalog refresh left the lists as they were. A
    /// stale saved index stays rebuilt and is marked stale again, so its next reader rebuilds it
    /// where the saved game's would. O(V log V).
    pub(crate) fn restore_vacancy_order(&mut self, saved: &SavedVacancyOrder) {
        if saved.stale {
            self.dirty_index = true;
            return;
        }
        let mut sorted = Vec::new();
        for (slot, list) in saved.lists.iter().enumerate() {
            sorted.clear();
            sorted.extend_from_slice(list);
            sorted.sort_unstable();
            // A rebuild lists each slot's vacancies once, in building order.
            if sorted != self.vacancy_index[slot] {
                debug_log!(
                    "save",
                    "saved vacancy list {} holds {} homes against {} rebuilt; keeping the rebuild",
                    slot,
                    list.len(),
                    self.vacancy_index[slot].len()
                );
                return;
            }
        }
        for (slot, list) in saved.lists.iter().enumerate() {
            self.vacancy_index[slot].clone_from(list);
            for (position, &building_idx) in list.iter().enumerate() {
                self.vacancy_pos[building_idx] = position;
            }
        }
        self.rebuild_admission_order();
    }

    /// Adds the newest appended building to clean indices without scanning every building.
    pub(crate) fn index_appended_building(&mut self, building_idx: usize) -> bool {
        if building_idx >= self.buildings.len()
            || building_idx + 1 != self.buildings.len()
            || self.vacancy_pos.len() != building_idx
        {
            return false;
        }

        self.vacancy_pos.push(usize::MAX);
        let b = &self.buildings[building_idx];
        self.explicit_cell_site_count +=
            usize::from(b.parcel_id == 0 && b.zone_type == ZoneType::None);
        let chunk = RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y));
        self.building_chunks
            .entry(chunk)
            .or_default()
            .push(building_idx);
        include_chunk(&mut self.building_chunk_bounds, chunk);
        self.max_building_support_m = self.max_building_support_m.max(b.support_height_m);
        let half_width = b.width_cells as f32 * 0.5;
        let half_depth = b.depth_cells as f32 * 0.5;
        self.max_lot_radius_cells = self.max_lot_radius_cells.max(half_width.hypot(half_depth));
        if !b.is_under_construction() {
            if let Some(zi) = baseline_private_zone_slot(b.zone_type) {
                self.zone_index[zi].push(building_idx);
            }
            if let Some(zi) = self.housing_vacancy_slot(building_idx) {
                let resident_cap = self.household_capacity(building_idx);
                if resident_cap > 0 && b.occupancy < resident_cap {
                    let v_idx = self.vacancy_index[zi].len();
                    self.vacancy_index[zi].push(building_idx);
                    self.vacancy_pos[building_idx] = v_idx;
                    self.admission_vacancy_pushed(zi, building_idx);
                }
            }
        }
        self.dirty_index = false;
        true
    }

    /// Adds a building that just finished construction to clean indices: a sorted insert into
    /// its zone list, which a rebuild keeps in dense order, and a vacancy push. Its chunk entry
    /// already exists. Returns false for a stale index. O(log B) plus the insert.
    pub(super) fn index_completed_building(&mut self, building_idx: usize) -> bool {
        if self.dirty_index || self.vacancy_pos.len() != self.buildings.len() {
            return false;
        }
        let zone_type = self.buildings[building_idx].zone_type;
        if let Some(zi) = baseline_private_zone_slot(zone_type) {
            let list = &mut self.zone_index[zi];
            let position = list.partition_point(|&idx| idx < building_idx);
            list.insert(position, building_idx);
        }
        if let Some(zi) = self.housing_vacancy_slot(building_idx) {
            let resident_cap = self.household_capacity(building_idx);
            if resident_cap > 0
                && self.buildings[building_idx].occupancy < resident_cap
                && self.vacancy_pos[building_idx] == usize::MAX
            {
                self.vacancy_pos[building_idx] = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(building_idx);
                self.admission_vacancy_pushed(zi, building_idx);
            }
        }
        true
    }

    /// Increments occupancy for a building and updates vacancy index if it becomes full.
    /// O(log V) for the admission order.
    pub fn claim_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.household_capacity(building_idx);
        if cap == 0 || self.buildings[building_idx].occupancy >= cap {
            return;
        }
        let b = &mut self.buildings[building_idx];
        b.occupancy += 1;

        // If it was in the vacancy list and is now full, remove it
        if b.occupancy >= cap {
            self.remove_housing_vacancy(building_idx);
        } else {
            self.admission_occupancy_changed(building_idx);
        }
    }

    fn remove_housing_vacancy(&mut self, building_idx: usize) {
        let Some(zi) = self.housing_vacancy_slot(building_idx) else {
            return;
        };
        let Some(&position) = self.vacancy_pos.get(building_idx) else {
            return;
        };
        if position == usize::MAX {
            return;
        }
        let list = &mut self.vacancy_index[zi];
        list.swap_remove(position);
        if let Some(&moved) = list.get(position) {
            self.vacancy_pos[moved] = position;
        }
        self.vacancy_pos[building_idx] = usize::MAX;
        self.admission_vacancy_removed(zi, position);
    }

    /// Latches abandonment and immediately withdraws the home from admission and rehousing. O(1).
    pub(crate) fn mark_building_deserted(&mut self, building_idx: usize) {
        if !self.buildings[building_idx].is_deserted {
            self.record_production_change(building_idx);
            crate::debug::work::add(crate::debug::work::Work::Desertions, 1);
        }
        self.buildings[building_idx].is_deserted = true;
        self.building_visual_revision = self.building_visual_revision.wrapping_add(1);
        self.remove_housing_vacancy(building_idx);
        self.dirty = true;
    }

    /// Predicts the vacancy claims made by forced rehousing without mutating allocator indices.
    /// The caller rebuilds a stale index first, as the rehousing it predicts does.
    pub(crate) fn preview_vacancy_claims_except(
        &self,
        excluded_building: usize,
        claim_count: usize,
    ) -> Vec<usize> {
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return Vec::new();
        };
        let source = &self.vacancy_index[residential_slot];
        let mut virtual_len = source.len();
        let mut slot_overrides = HashMap::<usize, usize>::new();
        let mut occupancy_overrides = HashMap::<usize, u32>::new();
        let mut result = Vec::with_capacity(claim_count.min(source.len()));

        for _ in 0..claim_count {
            let candidate = (0..virtual_len).find_map(|slot| {
                let building_idx = slot_overrides.get(&slot).copied().unwrap_or(source[slot]);
                (building_idx != excluded_building && building_idx < self.buildings.len())
                    .then_some((slot, building_idx))
            });
            let Some((slot, building_idx)) = candidate else {
                break;
            };
            result.push(building_idx);
            let occupancy = occupancy_overrides
                .entry(building_idx)
                .or_insert(self.buildings[building_idx].occupancy);
            *occupancy = occupancy.saturating_add(1);
            if *occupancy < self.household_capacity(building_idx) {
                continue;
            }

            virtual_len -= 1;
            if slot < virtual_len {
                let last_building = slot_overrides
                    .get(&virtual_len)
                    .copied()
                    .unwrap_or(source[virtual_len]);
                slot_overrides.insert(slot, last_building);
            }
            slot_overrides.remove(&virtual_len);
        }
        result
    }

    /// Decrements occupancy for a building and updates vacancy index if it gained space.
    /// O(log V) for the admission order.
    pub fn release_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.household_capacity(building_idx);
        let vacancy_slot = self.housing_vacancy_slot(building_idx);
        let b = &mut self.buildings[building_idx];
        b.occupancy = b.occupancy.saturating_sub(1);
        if cap == 0 {
            return;
        }

        // If it was full and now has space, add it back to vacancy index
        if b.occupancy + 1 == cap {
            let Some(zi) = vacancy_slot else {
                return;
            };
            if self.vacancy_pos[building_idx] == usize::MAX {
                let v_idx = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(building_idx);
                self.vacancy_pos[building_idx] = v_idx;
                self.admission_vacancy_pushed(zi, building_idx);
                return;
            }
        }
        self.admission_occupancy_changed(building_idx);
    }

    // Farms share the residential vacancy list without entering residential zoning/growth indices.
    pub(super) fn housing_vacancy_slot(&self, building_idx: usize) -> Option<usize> {
        let building = &self.buildings[building_idx];
        baseline_private_zone_slot(building.zone_type).or_else(|| {
            self.registry
                .is_field_producer_asset(&building.asset_id)
                .then(|| baseline_private_zone_slot(ZoneType::Residential))
                .flatten()
        })
    }

    /// Fills `sources` (cleared) with the `(endpoint_node, building_index)` pairs of every
    /// completed building of `zone` that is a legal destination for the requested transit mode,
    /// in zone-index order: the flow fields' sources
    /// ([`FlowFieldSystem::rebuild_dirty`](crate::simulation::pathing::flow_field::FlowFieldSystem::rebuild_dirty)),
    /// seeding the exact entrance-model multi-source Dijkstra without a legacy endpoint proxy.
    /// The caller rebuilds a stale index and entrance cache first. O(zone buildings); reuses
    /// `sources`' storage.
    pub fn collect_sources_for_zone(
        &self,
        zone: ZoneType,
        graph: &RegionGraph,
        mode_flags: u8,
        sources: &mut Vec<(u32, usize)>,
    ) {
        sources.clear();
        let Some(zone_idx) = baseline_private_zone_slot(zone) else {
            return;
        };
        let want_car = (mode_flags & crate::simulation::network::types::TransitFlags::CAR) != 0;
        let want_foot = (mode_flags & crate::simulation::network::types::TransitFlags::FOOT) != 0;

        for &idx in &self.zone_index[zone_idx] {
            if idx >= self.buildings.len() || idx >= self.entrances.len() {
                continue;
            }
            let building = &self.buildings[idx];
            if building.edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(building.edge_idx);
            if edge.deleted {
                continue;
            }

            let entrance = &self.entrances[idx];
            if want_foot {
                if entrance.foot_lane_fwd != usize::MAX {
                    sources.push((edge.start_node, idx));
                }
                if entrance.foot_lane_bkw != usize::MAX {
                    sources.push((edge.end_node, idx));
                }
            }
            if want_car {
                if entrance.car_lane_fwd != usize::MAX {
                    sources.push((edge.start_node, idx));
                }
                if entrance.car_lane_bkw != usize::MAX {
                    sources.push((edge.end_node, idx));
                }
            }
        }
    }
}

fn include_chunk(bounds: &mut Option<[i32; 4]>, (x, z): (i32, i32)) {
    if let Some(bounds) = bounds {
        bounds[0] = bounds[0].min(x);
        bounds[1] = bounds[1].min(z);
        bounds[2] = bounds[2].max(x);
        bounds[3] = bounds[3].max(z);
    } else {
        *bounds = Some([x, z, x, z]);
    }
}
