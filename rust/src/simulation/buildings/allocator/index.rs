//! Building search indices and vacancy management.

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector3;

impl BuildingAllocator {
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

        for (idx, b) in self.buildings.iter().enumerate() {
            let zi = b.zone_type as usize;
            if zi < 6 {
                self.zone_index[zi].push(idx);
                let chunk =
                    RegionGraph::get_chunk_coords(Vector3::new(b.center_x, 0.0, b.center_y));
                self.building_chunks.entry(chunk).or_default().push(idx);

                let resident_cap = self.resident_capacity(idx);
                if resident_cap > 0 && b.occupancy < resident_cap {
                    let v_idx = self.vacancy_index[zi].len();
                    self.vacancy_index[zi].push(idx);
                    self.vacancy_pos[idx] = v_idx;
                }
            }
        }
        self.dirty_index = false;
    }

    /// Increments occupancy for a building and updates vacancy index if it becomes full. O(1).
    pub fn claim_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.resident_capacity(building_idx);
        let b = &mut self.buildings[building_idx];
        b.occupancy += 1;

        // If it was in the vacancy list and is now full, remove it
        if b.occupancy >= cap {
            let zi = b.zone_type as usize;
            let v_pos = self.vacancy_pos[building_idx];
            if v_pos != usize::MAX {
                let list = &mut self.vacancy_index[zi];
                let last_b_idx = *list.last().unwrap();
                list.swap_remove(v_pos);
                self.vacancy_pos[last_b_idx] = v_pos;
                self.vacancy_pos[building_idx] = usize::MAX;
            }
        }
    }

    /// Decrements occupancy for a building and updates vacancy index if it gained space. O(1).
    pub fn release_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.resident_capacity(building_idx);
        let b = &mut self.buildings[building_idx];
        b.occupancy = b.occupancy.saturating_sub(1);

        // If it was full and now has space, add it back to vacancy index
        if b.occupancy + 1 == cap {
            let zi = b.zone_type as usize;
            if self.vacancy_pos[building_idx] == usize::MAX {
                let v_idx = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(building_idx);
                self.vacancy_pos[building_idx] = v_idx;
            }
        }
    }

    /// Pick a random building from a specific zone type. O(1).
    pub fn get_random_building_by_zone(
        &self,
        zone: ZoneType,
        rng: &mut impl rand::Rng,
    ) -> Option<usize> {
        let list = &self.zone_index[zone as usize];
        if list.is_empty() {
            return None;
        }
        Some(list[rng.gen_range(0..list.len())])
    }

    /// Returns `(endpoint_node, building_index)` pairs for all buildings of `zone`
    /// that are legal destinations for the requested transit mode.
    ///
    /// Used by [`FlowFieldSystem::rebuild_dirty`] to seed the exact entrance-model
    /// multi-source Dijkstra without falling back to a legacy endpoint proxy.
    pub fn get_sources_for_zone(
        &self,
        zone: ZoneType,
        graph: &RegionGraph,
        mode_flags: u8,
    ) -> Vec<(u32, usize)> {
        let mut sources = Vec::new();
        let want_car = (mode_flags & crate::simulation::network::types::TransitFlags::CAR) != 0;
        let want_foot = (mode_flags & crate::simulation::network::types::TransitFlags::FOOT) != 0;

        for &idx in &self.zone_index[zone as usize] {
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

    /// Pick a random building from any of the specified zone types. O(1).
    pub fn get_random_building_by_zones(
        &self,
        zones: &[ZoneType],
        rng: &mut impl rand::Rng,
    ) -> Option<usize> {
        // We sum the counts and pick based on weighted probability of lengths
        let mut total = 0;
        for &zone in zones {
            total += self.zone_index[zone as usize].len();
        }

        if total == 0 {
            return None;
        }

        let mut pick = rng.gen_range(0..total);
        for &zone in zones {
            let list = &self.zone_index[zone as usize];
            if pick < list.len() {
                return Some(list[pick]);
            }
            pick -= list.len();
        }
        None
    }
}
