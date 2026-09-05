// SPDX-License-Identifier: GPL-2.0-only

//! Per-tick repair of stale agent building references.

use super::super::TRANSIT_ACCESS_INGRESS;
use super::runtime::dispatch_agents;
use super::slices::RawSlice;
use crate::simulation::economy::agents::data::AgentSystem;

impl AgentSystem {
    /// Clears or redirects building references that no longer point at live buildings.
    pub(super) fn scrub_invalid_building_refs(&mut self, bldg_count: usize, n: usize) {
        let s_home = RawSlice::new(&mut self.agents.home_building);
        let s_work = RawSlice::new(&mut self.agents.work_building);
        let s_cur_b = RawSlice::new(&mut self.agents.current_building);
        let s_tgt_b = RawSlice::new(&mut self.agents.target_building);
        let s_plan_b = RawSlice::new(&mut self.agents.planned_target_building);
        let s_transit = RawSlice::new(&mut self.agents.transit);
        let s_cached_commute = RawSlice::new(&mut self.agents.cached_commute_minutes);
        let s_next_commute_refresh = RawSlice::new(&mut self.agents.next_commute_refresh_time);
        let s_next_departure_day = RawSlice::new(&mut self.agents.next_departure_day);
        let s_next_departure_minute = RawSlice::new(&mut self.agents.next_departure_minute);
        let s_next_departure_origin =
            RawSlice::new(&mut self.agents.next_departure_origin_building);
        let s_next_departure_target =
            RawSlice::new(&mut self.agents.next_departure_target_building);
        let s_next_departure_activity = RawSlice::new(&mut self.agents.next_departure_activity);
        let s_cached_schedule_work = RawSlice::new(&mut self.agents.cached_schedule_work_building);
        let s_cached_work_profile = RawSlice::new(&mut self.agents.cached_work_profile_index);

        dispatch_agents(n, |i| unsafe {
            let mut clear_schedule_cache = false;
            if *s_home.get(i) != usize::MAX && *s_home.get(i) >= bldg_count {
                *s_home.get_mut(i) = usize::MAX;
                clear_schedule_cache = true;
            }
            if *s_work.get(i) != usize::MAX && *s_work.get(i) >= bldg_count {
                *s_work.get_mut(i) = usize::MAX;
                clear_schedule_cache = true;
            }
            if *s_cur_b.get(i) != usize::MAX && *s_cur_b.get(i) >= bldg_count {
                *s_cur_b.get_mut(i) = usize::MAX;
                *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                clear_schedule_cache = true;
            }
            let tgt = *s_tgt_b.get(i);
            if tgt != usize::MAX && tgt >= bldg_count {
                let home = *s_home.get(i);
                if home != usize::MAX {
                    *s_tgt_b.get_mut(i) = home;
                } else {
                    *s_tgt_b.get_mut(i) = usize::MAX;
                    *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
                }
                clear_schedule_cache = true;
            }
            let planned = *s_plan_b.get(i);
            if planned != usize::MAX && planned >= bldg_count {
                *s_plan_b.get_mut(i) = usize::MAX;
                clear_schedule_cache = true;
            }
            if *s_next_departure_origin.get(i) != usize::MAX
                && *s_next_departure_origin.get(i) >= bldg_count
            {
                clear_schedule_cache = true;
            }
            if *s_next_departure_target.get(i) != usize::MAX
                && *s_next_departure_target.get(i) >= bldg_count
            {
                clear_schedule_cache = true;
            }
            if *s_cached_schedule_work.get(i) != usize::MAX
                && *s_cached_schedule_work.get(i) >= bldg_count
            {
                clear_schedule_cache = true;
            }
            if clear_schedule_cache {
                *s_cached_commute.get_mut(i) = 0;
                *s_next_commute_refresh.get_mut(i) = 0.0;
                *s_next_departure_day.get_mut(i) = u32::MAX;
                *s_next_departure_minute.get_mut(i) = 0;
                *s_next_departure_origin.get_mut(i) = usize::MAX;
                *s_next_departure_target.get_mut(i) = usize::MAX;
                *s_next_departure_activity.get_mut(i) = 0;
                *s_cached_schedule_work.get_mut(i) = usize::MAX;
                *s_cached_work_profile.get_mut(i) = u16::MAX;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use crate::simulation::economy::agents::data::AgentSystem;

    #[test]
    fn scrub_invalid_building_refs_clears_schedule_cache_references() {
        let mut sys = AgentSystem::new();
        let agent_idx = sys.spawn_housed_agent(0, 0.0, 0.0);
        sys.agents.cached_commute_minutes[agent_idx] = 22;
        sys.agents.next_commute_refresh_time[agent_idx] = 99.0;
        sys.agents.next_departure_day[agent_idx] = 7;
        sys.agents.next_departure_minute[agent_idx] = 480;
        sys.agents.next_departure_origin_building[agent_idx] = 0;
        sys.agents.next_departure_target_building[agent_idx] = 9;
        sys.agents.next_departure_activity[agent_idx] = 1;
        sys.agents.cached_schedule_work_building[agent_idx] = 9;
        sys.agents.cached_work_profile_index[agent_idx] = 2;

        sys.scrub_invalid_building_refs(1, sys.agents.len());

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
        assert_eq!(sys.agents.next_departure_activity[agent_idx], 0);
        assert_eq!(
            sys.agents.cached_schedule_work_building[agent_idx],
            usize::MAX
        );
        assert_eq!(sys.agents.cached_work_profile_index[agent_idx], u16::MAX);
    }
}
