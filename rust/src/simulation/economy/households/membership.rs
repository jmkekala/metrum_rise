// SPDX-License-Identifier: GPL-2.0-only

//! Agent-to-household membership rebuilds and money synchronization.

use std::sync::atomic::Ordering;

use super::data::HouseholdSystem;
use super::metrics::stock_days;
use super::replenishment::clear_replenishment_request;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, AgentSystem, age_group_can_shop, age_group_can_work,
};
use rayon::prelude::*;

impl HouseholdSystem {
    /// Rebuilds derived shopping mode eligibility after restoring resident agents.
    /// O(agents + households), parallel and allocation-free after scratch is sized.
    /// Normal ticks collect these flags inside the existing membership census instead.
    pub(crate) fn rebuild_household_shopping_access(&mut self, agents: &AgentSystem) {
        self.reset_shopping_car_scratch();
        let flags = &self.shopping_car_scratch;
        agents
            .household_id
            .par_iter()
            .zip(agents.age_group.par_iter())
            .zip(agents.has_car.par_iter())
            .for_each(|((&household, &age), &has_car)| {
                if household < flags.len() && has_car && age_group_can_shop(age) {
                    flags[household].store(1, Ordering::Relaxed);
                }
            });
        self.households
            .par_iter_mut()
            .zip(flags)
            .for_each(|(household, flag)| {
                household.shopping_has_car = flag.load(Ordering::Relaxed) != 0;
            });
    }

    /// Repairs the moved resident's active shopper reference in O(1) after an agent swap-remove.
    pub(crate) fn remap_shopper_agent_index(
        &mut self,
        old_idx: usize,
        new_idx: usize,
        agents: &AgentSystem,
    ) {
        let household_id = agents.household_id[new_idx];
        if let Some(household) = self.households.get_mut(household_id)
            && household.shopping_agent_id == old_idx
            && household.shopping_agent_schedule_seed == agents.schedule_seed[new_idx]
        {
            household.shopping_agent_id = new_idx;
        }
    }

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
        self.reset_shopping_car_scratch();
        self.reset_worker_count_scratch(building_count);

        let member_count_scratch = &self.member_count_scratch;
        let child_count_scratch = &self.child_count_scratch;
        let adult_count_scratch = &self.adult_count_scratch;
        let elder_count_scratch = &self.elder_count_scratch;
        let shopping_car_scratch = &self.shopping_car_scratch;
        let worker_count_scratch = &self.worker_count_scratch;
        agents
            .household_id
            .par_iter()
            .zip(agents.work_building.par_iter())
            .zip(agents.age_group.par_iter())
            .zip(agents.has_car.par_iter())
            .for_each(
                |(((&household_id, &work_building), &age_group), &has_car)| {
                    if household_id < household_count {
                        member_count_scratch[household_id].fetch_add(1, Ordering::Relaxed);
                        if has_car && age_group_can_shop(age_group) {
                            shopping_car_scratch[household_id].store(1, Ordering::Relaxed);
                        }
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
                },
            );

        self.households
            .par_iter_mut()
            .enumerate()
            .for_each(|(idx, household)| {
                household.shopping_has_car = shopping_car_scratch[idx].load(Ordering::Relaxed) != 0;
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

        let workers_changed = allocator
            .buildings
            .par_iter_mut()
            .zip(worker_count_scratch.par_iter())
            .map(|(building, count)| {
                let count = count.load(Ordering::Relaxed);
                let changed = building.worker_count != count;
                building.worker_count = count;
                changed
            })
            .reduce(|| false, |a, b| a || b);
        // A recount can repair assignments that bypassed the incremental mutators. Keep
        // its existing parallel scan and conservatively invalidate incomplete history.
        if workers_changed {
            allocator.invalidate_production_changes();
        }
        self.refresh_household_wage_snapshot(agents, allocator);
    }

    /// Rebuilds expected income from active assignments in stable member order.
    /// O(agents + households), with reused membership storage and parallel household sums.
    pub(crate) fn refresh_household_wage_snapshot(
        &mut self,
        agents: &AgentSystem,
        allocator: &BuildingAllocator,
    ) {
        let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let (heads, next) = super::relocation::build_household_member_index(
            agents,
            self.households.len(),
            std::mem::take(&mut self.household_member_heads_scratch),
            std::mem::take(&mut self.household_member_next_scratch),
        );
        self.has_unhoused_households = self
            .households
            .par_iter_mut()
            .enumerate()
            .map(|(hid, household)| {
                let mut member = heads[hid];
                let mut wages = 0.0_f64;
                let mut employed = 0_u16;
                while member != usize::MAX {
                    if age_group_can_work(agents.age_group[member]) {
                        // Transfers count an assignment until employment actually ends,
                        // even if the employer has stopped operating or paying wages.
                        if agents.work_building[member] != usize::MAX {
                            employed = employed.saturating_add(1);
                        }
                        if let Some(building) =
                            allocator.buildings.get(agents.work_building[member])
                        {
                            if let Some(profile) =
                                super::employment::active_work_profile(&catalog, building)
                            {
                                if agents.consecutive_unpaid_days[member] == 0 {
                                    wages += f64::from(profile.average_daily_wage());
                                }
                            }
                        }
                    }
                    member = next[member];
                }
                household.employed_adults = employed;
                household.gross_daily_wages = wages as f32;
                household.member_count > 0
                    && !super::metrics::household_is_housed(household, allocator)
            })
            .reduce(|| false, |left, right| left || right);
        self.household_member_heads_scratch = heads;
        self.household_member_next_scratch = next;
    }

    pub(super) fn sync_agent_money_from_households(&mut self, agents: &mut AgentSystem) {
        // Job allocation ran after the income census and freight may have remapped agents.
        // Reuse the census counters in this existing agent pass; member links are stale here.
        self.reset_adult_count_scratch();
        let employed = &self.adult_count_scratch;
        let households = &self.households;
        let household_id = &agents.agents.household_id;
        let work_building = &agents.agents.work_building;
        let age_group = &agents.agents.age_group;
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
                if age_group_can_work(age_group[i]) && work_building[i] != usize::MAX {
                    employed[hid].fetch_add(1, Ordering::Relaxed);
                }
                let per_member = if household.member_count > 0 {
                    household.budget / household.member_count as f32
                } else {
                    0.0
                };
                *money = per_member.max(0.0);
            });
        self.households
            .par_iter_mut()
            .enumerate()
            .for_each(|(hid, household)| {
                household.employed_adults = employed[hid]
                    .load(Ordering::Relaxed)
                    .min(u32::from(u16::MAX)) as u16;
            });
    }
}
