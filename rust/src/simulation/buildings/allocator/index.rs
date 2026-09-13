// SPDX-License-Identifier: GPL-2.0-only

//! Building search indices and vacancy management.

use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector3;
use std::collections::HashMap;

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
    pub fn rebuild_zone_index(&mut self) {
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.vacancy_pos.resize(self.buildings.len(), usize::MAX);
        self.building_chunks.clear();
        self.max_lot_radius_cells = 0.0;

        for (idx, b) in self.buildings.iter().enumerate() {
            if b.edge_idx != usize::MAX {
                let chunk =
                    RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y));
                self.building_chunks.entry(chunk).or_default().push(idx);
                let half_width = b.width_cells as f32 * 0.5;
                let half_depth = b.depth_cells as f32 * 0.5;
                self.max_lot_radius_cells =
                    self.max_lot_radius_cells.max(half_width.hypot(half_depth));
            }
            if b.is_under_construction() {
                continue;
            }
            if let Some(zi) = baseline_private_zone_slot(b.zone_type) {
                self.zone_index[zi].push(idx);
            }
            if let Some(zi) = self.housing_vacancy_slot(idx) {
                let resident_cap = self.household_capacity(idx);
                if resident_cap > 0 && b.occupancy < resident_cap {
                    let v_idx = self.vacancy_index[zi].len();
                    self.vacancy_index[zi].push(idx);
                    self.vacancy_pos[idx] = v_idx;
                }
            }
        }
        self.dirty_index = false;
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
        if b.edge_idx != usize::MAX {
            let chunk = RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y));
            self.building_chunks
                .entry(chunk)
                .or_default()
                .push(building_idx);
            let half_width = b.width_cells as f32 * 0.5;
            let half_depth = b.depth_cells as f32 * 0.5;
            self.max_lot_radius_cells = self.max_lot_radius_cells.max(half_width.hypot(half_depth));
        }
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
                }
            }
        }
        self.dirty_index = false;
        true
    }

    /// Increments occupancy for a building and updates vacancy index if it becomes full. O(1).
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
    }

    /// Latches abandonment and immediately withdraws the home from admission and rehousing. O(1).
    pub(crate) fn mark_building_deserted(&mut self, building_idx: usize) {
        self.buildings[building_idx].is_deserted = true;
        self.remove_housing_vacancy(building_idx);
        self.dirty = true;
    }

    /// Predicts the vacancy claims made by forced rehousing without mutating allocator indices.
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

    /// Decrements occupancy for a building and updates vacancy index if it gained space. O(1).
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
            }
        }
    }

    // Farms share the residential vacancy list without entering residential zoning/growth indices.
    fn housing_vacancy_slot(&self, building_idx: usize) -> Option<usize> {
        let building = &self.buildings[building_idx];
        baseline_private_zone_slot(building.zone_type).or_else(|| {
            self.registry
                .is_field_producer_asset(&building.asset_id)
                .then(|| baseline_private_zone_slot(ZoneType::Residential))
                .flatten()
        })
    }

    /// Returns `(endpoint_node, building_index)` pairs for all buildings of `zone`
    /// that are legal destinations for the requested transit mode.
    ///
    /// Used by [`FlowFieldSystem::rebuild_dirty`](crate::simulation::pathing::flow_field::FlowFieldSystem::rebuild_dirty)
    /// to seed the exact entrance-model
    /// multi-source Dijkstra without falling back to a legacy endpoint proxy.
    pub fn get_sources_for_zone(
        &self,
        zone: ZoneType,
        graph: &RegionGraph,
        mode_flags: u8,
    ) -> Vec<(u32, usize)> {
        let Some(zone_idx) = baseline_private_zone_slot(zone) else {
            return Vec::new();
        };
        let mut sources = Vec::new();
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

        sources
    }
}
