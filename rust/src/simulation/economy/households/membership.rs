// SPDX-License-Identifier: GPL-2.0-only

//! Agent-to-household membership rebuilds and money synchronization.

use std::sync::atomic::Ordering;

use super::data::HouseholdSystem;
use super::metrics::stock_days;
use super::replenishment::clear_replenishment_request;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, AgentSystem, age_group_can_work,
};
use rayon::prelude::*;

impl HouseholdSystem {
    pub(super) fn debug_validate_agent_household_refs(&self, _agents: &AgentSystem) {
        #[cfg(debug_assertions)]
        {
            let agents = _agents;
            let household_count = self.households.len();
            let households = &self.households;
            let pending_household_size = &agents.agents.pending_household_size;
            let home_building = &agents.agents.home_building;
            agents
                .agents
                .household_id
                .par_iter()
                .enumerate()
                .for_each(|(i, &household_id)| {
                    if pending_household_size[i] > 0 || household_id == usize::MAX {
                        return;
                    }
                    assert!(
                        household_id < household_count,
                        "agent {i} references missing household {household_id}"
                    );
                    let home = home_building[i];
                    assert_eq!(
                        households[household_id].home_building_id, home,
                        "agent {i} household/home mismatch"
                    );
                });
        }
    }

    #[cfg(test)]
    pub(super) fn ensure_agent_households(&mut self, agents: &mut AgentSystem) {
        let household_count = self.households.len();
        let households = &self.households;
        let pending_household_size = &agents.agents.pending_household_size;
        let home_building = &agents.agents.home_building;
        agents
            .agents
            .household_id
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, household_id)| {
                if pending_household_size[i] > 0 || *household_id == usize::MAX {
                    return;
                }
                let home = home_building[i];
                let hid = *household_id;
                if hid >= household_count {
                    *household_id = usize::MAX;
                    return;
                }
                if home == usize::MAX {
                    if households[hid].home_building_id != usize::MAX {
                        *household_id = usize::MAX;
                    }
                    return;
                }
                if households[hid].home_building_id != home {
                    *household_id = usize::MAX;
                }
            });
    }

    pub(super) fn rebuild_household_and_worker_counts(
        &mut self,
        agents: &AgentSystem,
        allocator: &mut BuildingAllocator,
    ) {
        let household_count = self.households.len();
        let building_count = allocator.buildings.len();
        self.reset_member_count_scratch();
        self.reset_child_count_scratch();
        self.reset_adult_count_scratch();
        self.reset_elder_count_scratch();
        self.reset_worker_count_scratch(building_count);

        let member_count_scratch = &self.member_count_scratch;
        let child_count_scratch = &self.child_count_scratch;
        let adult_count_scratch = &self.adult_count_scratch;
        let elder_count_scratch = &self.elder_count_scratch;
        let worker_count_scratch = &self.worker_count_scratch;
        agents
            .household_id
            .par_iter()
            .zip(agents.work_building.par_iter())
            .zip(agents.age_group.par_iter())
            .for_each(|((&household_id, &work_building), &age_group)| {
                if household_id < household_count {
                    member_count_scratch[household_id].fetch_add(1, Ordering::Relaxed);
                    match age_group {
                        AGE_CHILD => {
                            child_count_scratch[household_id].fetch_add(1, Ordering::Relaxed);
                        }
                        AGE_ADULT => {
                            adult_count_scratch[household_id].fetch_add(1, Ordering::Relaxed);
                        }
                        AGE_ELDER => {
                            elder_count_scratch[household_id].fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
                if age_group_can_work(age_group) && work_building < building_count {
                    worker_count_scratch[work_building].fetch_add(1, Ordering::Relaxed);
                }
            });

        self.households
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, household)| {
                household.member_count = member_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.child_count = child_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.adult_count = adult_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.elder_count = elder_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.stock_days = stock_days(
                    household.stock,
                    household.member_count,
                    household.consumption_rate,
                );
                if household.member_count == 0 {
                    clear_replenishment_request(household);
                }
            });

        allocator
            .buildings
            .par_iter_mut()
            .zip(worker_count_scratch.par_iter())
            .for_each(|(building, count)| {
                building.worker_count = count.load(Ordering::Relaxed);
            });
    }

    #[cfg(test)]
    pub(super) fn rebuild_household_membership(&mut self, agents: &AgentSystem) {
        let household_count = self.households.len();
        self.reset_member_count_scratch();
        self.reset_child_count_scratch();
        self.reset_adult_count_scratch();
        self.reset_elder_count_scratch();
        let member_count_scratch = &self.member_count_scratch;
        let child_count_scratch = &self.child_count_scratch;
        let adult_count_scratch = &self.adult_count_scratch;
        let elder_count_scratch = &self.elder_count_scratch;
        agents
            .household_id
            .par_iter()
            .zip(agents.age_group.par_iter())
            .for_each(|(&hid, &age_group)| {
                if hid < household_count {
                    member_count_scratch[hid].fetch_add(1, Ordering::Relaxed);
                    match age_group {
                        AGE_CHILD => {
                            child_count_scratch[hid].fetch_add(1, Ordering::Relaxed);
                        }
                        AGE_ADULT => {
                            adult_count_scratch[hid].fetch_add(1, Ordering::Relaxed);
                        }
                        AGE_ELDER => {
                            elder_count_scratch[hid].fetch_add(1, Ordering::Relaxed);
                        }
                        _ => {}
                    }
                }
            });

        self.households
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, household)| {
                household.member_count = member_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.child_count = child_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.adult_count = adult_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.elder_count = elder_count_scratch[idx]
                    .load(Ordering::Relaxed)
                    .min(u16::MAX as u32) as u16;
                household.stock_days = stock_days(
                    household.stock,
                    household.member_count,
                    household.consumption_rate,
                );
                if household.member_count == 0 {
                    clear_replenishment_request(household);
                }
            });
    }

    pub(super) fn sync_agent_money_from_households(&mut self, agents: &mut AgentSystem) {
        let households = &self.households;
        let household_id = &agents.agents.household_id;
        agents
            .agents
            .money
            .par_iter_mut()
            .enumerate()
            .for_each(|(i, money)| {
                let hid = household_id[i];
                if hid == usize::MAX || hid >= households.len() {
                    return;
                }
                let household = &households[hid];
                let per_member = if household.member_count > 0 {
                    household.budget / household.member_count as f32
                } else {
                    0.0
                };
                *money = per_member.max(0.0);
            });
    }
}
