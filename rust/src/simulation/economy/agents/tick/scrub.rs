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

        dispatch_agents(n, |i| unsafe {
            if *s_home.get(i) != usize::MAX && *s_home.get(i) >= bldg_count {
                *s_home.get_mut(i) = usize::MAX;
            }
            if *s_work.get(i) != usize::MAX && *s_work.get(i) >= bldg_count {
                *s_work.get_mut(i) = usize::MAX;
            }
            if *s_cur_b.get(i) != usize::MAX && *s_cur_b.get(i) >= bldg_count {
                *s_cur_b.get_mut(i) = usize::MAX;
                *s_transit.get_mut(i) = TRANSIT_ACCESS_INGRESS;
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
            }
            let planned = *s_plan_b.get(i);
            if planned != usize::MAX && planned >= bldg_count {
                *s_plan_b.get_mut(i) = usize::MAX;
            }
        });
    }
}
