//! Agent references to building allocator indices.

use super::TRANSIT_ACCESS_INGRESS;
use super::data::AgentSystem;
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::zoning::ZoneType;
use rand::Rng;

impl AgentSystem {
    /// Clears schedule-derived building cache fields for one agent.
    pub(crate) fn clear_schedule_building_cache(&mut self, agent_idx: usize) {
        self.agents.cached_commute_minutes[agent_idx] = 0;
        self.agents.next_commute_refresh_time[agent_idx] = 0.0;
        self.agents.next_departure_day[agent_idx] = u32::MAX;
        self.agents.next_departure_minute[agent_idx] = 0;
        self.agents.next_departure_origin_building[agent_idx] = usize::MAX;
        self.agents.next_departure_target_building[agent_idx] = usize::MAX;
        self.agents.next_departure_activity[agent_idx] = 0;
        self.agents.cached_schedule_work_building[agent_idx] = usize::MAX;
        self.agents.cached_work_profile_index[agent_idx] = u16::MAX;
    }

    /// Forcefully removes all agents from a building that has been deleted.
    pub fn evict_building(&mut self, building_id: usize) {
        for i in 0..self.agents.len() {
            let cache_touches_building = self.agents.next_departure_origin_building[i]
                == building_id
                || self.agents.next_departure_target_building[i] == building_id
                || self.agents.cached_schedule_work_building[i] == building_id;
            let mut clear_schedule_cache = cache_touches_building;

            if self.agents.work_building[i] == building_id {
                self.agents.work_building[i] = usize::MAX;
                clear_schedule_cache = true;
            }
            if self.agents.home_building[i] == building_id {
                self.agents.home_building[i] = usize::MAX;
                self.agents.household_id[i] = usize::MAX;
                self.agents.pending_household_size[i] = 0;
                clear_schedule_cache = true;
            }
            if self.agents.planned_target_building[i] == building_id {
                self.agents.planned_target_building[i] = usize::MAX;
                clear_schedule_cache = true;
            }

            if self.agents.current_building[i] == building_id {
                self.agents.current_building[i] = usize::MAX;
                self.agents.target_building[i] = usize::MAX;
                self.agents.transit[i] = TRANSIT_ACCESS_INGRESS;
                clear_schedule_cache = true;
            } else if self.agents.target_building[i] == building_id {
                if self.agents.home_building[i] != usize::MAX {
                    self.agents.target_building[i] = self.agents.home_building[i];
                    self.agents.planned_target_building[i] = self.agents.home_building[i];
                    self.agents.activity[i] = 0;
                } else {
                    self.agents.target_building[i] = usize::MAX;
                    self.agents.pending_household_size[i] = 0;
                    self.agents.transit[i] = TRANSIT_ACCESS_INGRESS;
                }
                clear_schedule_cache = true;
            }

            if clear_schedule_cache {
                self.clear_schedule_building_cache(i);
            }
        }
        self.invalidate_lane_bucket_snapshot();
    }

    /// Finds a residential building with available vacancy.
    /// Uses the allocator's `vacancy_index` for O(1) random selection.
    pub fn find_available_home(&mut self, allocator: &mut BuildingAllocator) -> Option<usize> {
        let mut rng = rand::thread_rng();
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };

        let total_vacant = allocator.vacancy_index[residential_slot].len();
        if total_vacant == 0 {
            return None;
        }

        let pick = rng.gen_range(0..total_vacant);
        let list = &allocator.vacancy_index[residential_slot];
        let building_idx = list.get(pick).copied().unwrap_or(usize::MAX);
        if building_idx != usize::MAX {
            allocator.claim_vacancy(building_idx);
            return Some(building_idx);
        }
        None
    }

    /// Re-calculates building occupancy and vacancy index from scratch.
    pub fn recalculate_occupancy(&mut self, allocator: &mut BuildingAllocator) {
        for b in &mut allocator.buildings {
            b.occupancy = 0;
        }
        for i in 0..self.agents.len() {
            let h = self.agents.home_building[i];
            if h != usize::MAX && h < allocator.buildings.len() {
                allocator.buildings[h].occupancy += 1;
            }
        }
        allocator.rebuild_zone_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evict_building_clears_schedule_cache_references() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.work_building[agent_idx] = 2;
        sys.agents.target_building[agent_idx] = 2;
        sys.agents.planned_target_building[agent_idx] = 2;
        sys.agents.cached_commute_minutes[agent_idx] = 18;
        sys.agents.next_commute_refresh_time[agent_idx] = 12.0;
        sys.agents.next_departure_day[agent_idx] = 3;
        sys.agents.next_departure_minute[agent_idx] = 540;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.next_departure_activity[agent_idx] = 1;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;
        sys.agents.cached_work_profile_index[agent_idx] = 4;

        sys.evict_building(2);

        assert_eq!(sys.agents.work_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.target_building[agent_idx], 1);
        assert_eq!(sys.agents.planned_target_building[agent_idx], 1);
        assert_eq!(sys.agents.cached_commute_minutes[agent_idx], 0);
        assert_eq!(sys.agents.next_commute_refresh_time[agent_idx], 0.0);
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.next_departure_origin_building[agent_idx],
            usize::MAX
        );
        assert_eq!(
            sys.agents.next_departure_target_building[agent_idx],
            usize::MAX
        );
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
        assert_eq!(sys.agents.cached_work_profile_index[agent_idx], u16::MAX);
    }
}
