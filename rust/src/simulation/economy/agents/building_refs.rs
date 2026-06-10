//! Agent references to building allocator indices.

use super::data::AgentSystem;
use super::determinism::stable_index;
use super::{MODE_WALK, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING, age_group_can_work};
use crate::simulation::buildings::allocator::{BuildingAllocator, baseline_private_zone_slot};
use crate::simulation::zoning::ZoneType;
use godot::prelude::Vector2;

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

    /// Assigns an agent to a household without touching movement state.
    pub(crate) fn assign_household_id(&mut self, agent_idx: usize, household_id: usize) {
        if agent_idx < self.agents.len() {
            self.agents.household_id[agent_idx] = household_id;
        }
    }

    /// Converts a border household carrier into an ordinary resident inside its home building.
    pub(crate) fn materialize_household_carrier(
        &mut self,
        agent_idx: usize,
        household_id: usize,
        age_group: u8,
        door_pos: Option<Vector2>,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }
        self.agents.household_id[agent_idx] = household_id;
        self.agents.age_group[agent_idx] = age_group;
        self.agents.pending_household_size[agent_idx] = 0;
        self.agents.target_building[agent_idx] = usize::MAX;
        self.agents.planned_target_building[agent_idx] = usize::MAX;
        self.clear_route_and_lane_state(agent_idx);
        self.agents.next_replan_time[agent_idx] = 0.0;
        self.agents.transit_mode[agent_idx] = MODE_WALK;
        self.agents.activity[agent_idx] = 0;
        self.agents.planned_activity[agent_idx] = 0;
        self.agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
        if let Some(door) = door_pos {
            self.agents.pos_x[agent_idx] = door.x;
            self.agents.pos_y[agent_idx] = door.y;
        }
        self.clear_schedule_building_cache(agent_idx);
        self.invalidate_lane_bucket_snapshot();
    }

    /// Updates a household member after their household relocates to a new home.
    pub(crate) fn relocate_household_member_home(
        &mut self,
        agent_idx: usize,
        old_home: usize,
        new_home: usize,
        old_home_live: bool,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }

        if self.agents.home_building[agent_idx] != new_home {
            self.agents.home_building[agent_idx] = new_home;
            self.clear_schedule_building_cache(agent_idx);
        }

        if old_home_live && self.agents.current_building[agent_idx] == old_home {
            self.agents.current_building[agent_idx] = new_home;
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_target_building[agent_idx] = usize::MAX;
            self.agents.transit[agent_idx] = TRANSIT_IN_BUILDING;
            self.agents.activity[agent_idx] = 0;
            self.clear_route_and_lane_state(agent_idx);
            self.invalidate_lane_bucket_snapshot();
        } else {
            let mut needs_replan = false;
            if old_home_live && self.agents.target_building[agent_idx] == old_home {
                self.agents.target_building[agent_idx] = new_home;
                needs_replan = true;
            }
            if old_home_live && self.agents.planned_target_building[agent_idx] == old_home {
                self.agents.planned_target_building[agent_idx] = new_home;
                needs_replan = true;
            }
            if !old_home_live
                && self.agents.current_building[agent_idx] == usize::MAX
                && self.agents.target_building[agent_idx] == usize::MAX
            {
                self.agents.target_building[agent_idx] = new_home;
                self.agents.planned_target_building[agent_idx] = new_home;
                self.agents.activity[agent_idx] = 0;
                needs_replan = true;
            }
            if needs_replan {
                self.clear_access_plan_and_path(agent_idx);
                self.agents.next_replan_time[agent_idx] = 0.0;
                self.clear_schedule_building_cache(agent_idx);
            }
        }
    }

    /// Updates a household member after their household loses its home.
    pub(crate) fn evict_household_member_home(&mut self, agent_idx: usize, old_home: usize) {
        if agent_idx >= self.agents.len() {
            return;
        }

        if self.agents.home_building[agent_idx] != usize::MAX {
            self.agents.home_building[agent_idx] = usize::MAX;
            self.clear_schedule_building_cache(agent_idx);
        }

        if self.agents.current_building[agent_idx] == old_home {
            self.agents.current_building[agent_idx] = usize::MAX;
            self.agents.target_building[agent_idx] = usize::MAX;
            self.agents.planned_target_building[agent_idx] = usize::MAX;
            self.agents.transit[agent_idx] = TRANSIT_ACCESS_INGRESS;
            self.clear_route_and_lane_state(agent_idx);
            self.invalidate_lane_bucket_snapshot();
        } else {
            let mut needs_replan = false;
            if self.agents.target_building[agent_idx] == old_home {
                self.agents.target_building[agent_idx] = usize::MAX;
                needs_replan = true;
            }
            if self.agents.planned_target_building[agent_idx] == old_home {
                self.agents.planned_target_building[agent_idx] = usize::MAX;
                needs_replan = true;
            }
            if needs_replan {
                self.clear_access_plan_and_path(agent_idx);
                self.agents.next_replan_time[agent_idx] = 0.0;
                self.clear_schedule_building_cache(agent_idx);
            }
        }
    }

    /// Assigns or clears an agent's workplace and invalidates derived work-trip caches.
    pub(crate) fn assign_work_building(
        &mut self,
        agent_idx: usize,
        mut work_building: usize,
        mut job_lock_days: u8,
    ) {
        if agent_idx >= self.agents.len() {
            return;
        }
        if work_building != usize::MAX && !age_group_can_work(self.agents.age_group[agent_idx]) {
            work_building = usize::MAX;
            job_lock_days = 0;
        }

        let old_work = self.agents.work_building[agent_idx];
        if old_work != work_building {
            self.agents.work_building[agent_idx] = work_building;
            self.clear_schedule_building_cache(agent_idx);
            if old_work != usize::MAX {
                let mut needs_replan = false;
                if self.agents.target_building[agent_idx] == old_work {
                    self.agents.target_building[agent_idx] = usize::MAX;
                    needs_replan = true;
                }
                if self.agents.planned_target_building[agent_idx] == old_work {
                    self.agents.planned_target_building[agent_idx] = usize::MAX;
                    needs_replan = true;
                }
                if needs_replan {
                    self.clear_access_plan_and_path(agent_idx);
                    self.agents.next_replan_time[agent_idx] = 0.0;
                }
            }
        }
        self.agents.job_lock_days[agent_idx] = job_lock_days;
        self.agents.consecutive_unpaid_days[agent_idx] = 0;
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
    /// Uses the allocator's `vacancy_index` for O(1) deterministic selection.
    pub fn find_available_home(&mut self, allocator: &mut BuildingAllocator) -> Option<usize> {
        let Some(residential_slot) = baseline_private_zone_slot(ZoneType::Residential) else {
            return None;
        };

        let total_vacant = allocator.vacancy_index[residential_slot].len();
        if total_vacant == 0 {
            return None;
        }

        let seed = (self.sim_time.to_bits() as u64)
            ^ ((self.agents.len() as u64) << 32)
            ^ (total_vacant as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let pick = stable_index(seed, total_vacant);
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

    fn clear_route_and_lane_state(&mut self, agent_idx: usize) {
        self.clear_access_plan_and_path(agent_idx);
        self.agents.current_node[agent_idx] = u32::MAX;
        self.agents.current_edge[agent_idx] = usize::MAX;
        self.agents.current_lane_id[agent_idx] = usize::MAX;
        self.agents.lane_distance[agent_idx] = 0.0;
        self.agents.lane_change_from_lane_id[agent_idx] = u32::MAX;
        self.agents.lane_change_start_d[agent_idx] = 0.0;
        self.agents.lane_change_length_m[agent_idx] = 0.0;
        self.agents.overtake_blocked_time_s[agent_idx] = 0.0;
        self.agents.overtake_cooldown_s[agent_idx] = 0.0;
        self.agents.speed[agent_idx] = 0.0;
    }

    fn clear_access_plan_and_path(&mut self, agent_idx: usize) {
        self.agents.planned_attach_node[agent_idx] = u32::MAX;
        self.agents.planned_detach_node[agent_idx] = u32::MAX;
        self.agents.planned_attach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_detach_lane_id[agent_idx] = u32::MAX;
        self.agents.planned_attach_lane_d[agent_idx] = 0.0;
        self.agents.planned_detach_lane_d[agent_idx] = 0.0;
        self.agents.access_flags[agent_idx] = 0;
        self.agents.current_path[agent_idx].clear();
        self.agents.current_path_index[agent_idx] = 0;
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

    #[test]
    fn assigning_work_clears_schedule_cache_and_stale_work_trip() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.work_building[agent_idx] = 2;
        sys.agents.target_building[agent_idx] = 2;
        sys.agents.planned_target_building[agent_idx] = 2;
        sys.agents.access_flags[agent_idx] = crate::simulation::economy::agents::ACCESS_PLAN_VALID;
        sys.agents.current_path[agent_idx] = vec![10, 20];
        sys.agents.next_departure_day[agent_idx] = 1;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;

        sys.assign_work_building(agent_idx, 3, 7);

        assert_eq!(sys.agents.work_building[agent_idx], 3);
        assert_eq!(sys.agents.job_lock_days[agent_idx], 7);
        assert_eq!(sys.agents.target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.planned_target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.access_flags[agent_idx], 0);
        assert!(sys.agents.current_path[agent_idx].is_empty());
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
    }

    #[test]
    fn relocating_home_clears_schedule_cache_and_stale_home_trip() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(1, 0.0, 0.0);
        sys.agents.target_building[agent_idx] = 1;
        sys.agents.planned_target_building[agent_idx] = 1;
        sys.agents.access_flags[agent_idx] = crate::simulation::economy::agents::ACCESS_PLAN_VALID;
        sys.agents.current_path[agent_idx] = vec![10, 20];
        sys.agents.next_departure_day[agent_idx] = 1;
        sys.agents.next_departure_origin_building[agent_idx] = 1;
        sys.agents.next_departure_target_building[agent_idx] = 2;
        sys.agents.cached_schedule_work_building[agent_idx] = 2;

        sys.relocate_household_member_home(agent_idx, 1, 4, true);

        assert_eq!(sys.agents.home_building[agent_idx], 4);
        assert_eq!(sys.agents.current_building[agent_idx], 4);
        assert_eq!(sys.agents.target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.planned_target_building[agent_idx], usize::MAX);
        assert_eq!(sys.agents.access_flags[agent_idx], 0);
        assert!(sys.agents.current_path[agent_idx].is_empty());
        assert_eq!(sys.agents.next_departure_day[agent_idx], u32::MAX);
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
    }
}
