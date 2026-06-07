//! Agent-to-household membership rebuilds and money synchronization.

use super::data::HouseholdSystem;
use super::metrics::stock_days;
use super::replenishment::clear_replenishment_request;
use crate::simulation::economy::agents::AgentSystem;

impl HouseholdSystem {
    pub(super) fn ensure_agent_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            if agents.pending_household_size[i] > 0 {
                continue;
            }
            let home = agents.home_building[i];
            let hid = agents.household_id[i];
            if hid == usize::MAX {
                continue;
            }
            if hid >= self.households.len() {
                agents.assign_household_id(i, usize::MAX);
                continue;
            }
            if home == usize::MAX {
                if self.households[hid].home_building_id == usize::MAX {
                    continue;
                }
                agents.assign_household_id(i, usize::MAX);
                continue;
            }
            if self.households[hid].home_building_id != home {
                agents.assign_household_id(i, usize::MAX);
            }
        }
    }

    pub(super) fn rebuild_household_membership(&mut self, agents: &AgentSystem) {
        for household in &mut self.households {
            household.member_count = 0;
        }
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid != usize::MAX && hid < self.households.len() {
                let household = &mut self.households[hid];
                household.member_count = household.member_count.saturating_add(1);
                household.home_building_id = agents.home_building[i];
            }
        }
        for household in &mut self.households {
            household.stock_days = stock_days(
                household.stock,
                household.member_count,
                household.consumption_rate,
            );
            if household.member_count == 0 {
                clear_replenishment_request(household);
            }
        }
    }

    pub(super) fn sync_agent_money_from_households(&mut self, agents: &mut AgentSystem) {
        for i in 0..agents.len() {
            let hid = agents.household_id[i];
            if hid == usize::MAX || hid >= self.households.len() {
                continue;
            }
            let household = &self.households[hid];
            let per_member = if household.member_count > 0 {
                household.budget / household.member_count as f32
            } else {
                0.0
            };
            agents.money[i] = per_member.max(0.0);
        }
    }
}
