// SPDX-License-Identifier: GPL-2.0-only

//! Persistent household member lists (`SIM-02-I`, `docs/simulation.md#snapshots`).
//!
//! Each household's members form a singly linked list threaded through the agents in
//! ascending dense agent index. The order is canonical, so a list rebuilt at load visits the
//! members in the same order as one maintained through every join, removal and remap; float
//! sums over members, such as expected wages, depend on it. The lists are derived and never
//! saved. Agents appended to the store directly are linked by the next mutation, as
//! `HandleTable::register_appended` registers them; a load or a wholesale restore rebuilds
//! the lists in O(agents).

use std::collections::HashMap;

use super::data::AgentSystem;

/// End of a household's member list.
pub(crate) const NO_MEMBER: usize = usize::MAX;

/// Member lists of every household in ascending agent index. A join or leave costs
/// O(household size) and allocates nothing once the columns have grown.
#[derive(Debug, Default)]
pub(crate) struct HouseholdMembers {
    // First member by household dense index; households past the end have none.
    heads: Vec<usize>,
    // Next member of the same household by agent dense index, for every registered agent.
    next: Vec<usize>,
}

impl HouseholdMembers {
    fn head(&self, household_id: usize) -> usize {
        self.heads.get(household_id).copied().unwrap_or(NO_MEMBER)
    }

    // Inserts `agent` at its sorted position in `household_id`'s list. O(household size).
    fn link(&mut self, agent: usize, household_id: usize) {
        if household_id >= self.heads.len() {
            self.heads.resize(household_id + 1, NO_MEMBER);
        }
        let head = self.heads[household_id];
        if head == NO_MEMBER || agent < head {
            self.next[agent] = head;
            self.heads[household_id] = agent;
            return;
        }
        let mut previous = head;
        while self.next[previous] != NO_MEMBER && self.next[previous] < agent {
            previous = self.next[previous];
        }
        self.next[agent] = self.next[previous];
        self.next[previous] = agent;
    }

    // Removes `agent` from `household_id`'s list. O(household size).
    fn unlink(&mut self, agent: usize, household_id: usize) {
        let following = std::mem::replace(&mut self.next[agent], NO_MEMBER);
        if self.heads[household_id] == agent {
            self.heads[household_id] = following;
            return;
        }
        let mut previous = self.heads[household_id];
        while self.next[previous] != agent {
            previous = self.next[previous];
        }
        self.next[previous] = following;
    }

    // Registers agents appended to the store directly, linking each at the end of its
    // household's list. A store truncated directly is rebuilt.
    fn register_appended(&mut self, household_id: &[usize]) {
        if self.next.len() > household_id.len() {
            self.rebuild(household_id);
            return;
        }
        while self.next.len() < household_id.len() {
            let agent = self.next.len();
            self.next.push(NO_MEMBER);
            if household_id[agent] != usize::MAX {
                self.link(agent, household_id[agent]);
            }
        }
    }

    // Links every agent from `household_id` in descending index, so each list ascends.
    fn rebuild(&mut self, household_id: &[usize]) {
        self.heads.clear();
        self.next.clear();
        self.next.resize(household_id.len(), NO_MEMBER);
        for (agent, &household) in household_id.iter().enumerate().rev() {
            if household == usize::MAX {
                continue;
            }
            if household >= self.heads.len() {
                self.heads.resize(household + 1, NO_MEMBER);
            }
            self.next[agent] = self.heads[household];
            self.heads[household] = agent;
        }
    }

    // Moves the lists of households renumbered by a household swap-remove, with the
    // simultaneous semantics of `mapping` (original id to current id). O(mapping).
    fn remap_households(&mut self, mapping: &HashMap<usize, usize>) {
        let moved: Vec<(usize, usize)> = mapping
            .iter()
            .map(|(&original, &current)| (current, self.head(original)))
            .collect();
        for &original in mapping.keys() {
            if let Some(head) = self.heads.get_mut(original) {
                *head = NO_MEMBER;
            }
        }
        for (current, head) in moved {
            if current >= self.heads.len() {
                self.heads.resize(current + 1, NO_MEMBER);
            }
            self.heads[current] = head;
        }
    }
}

/// Members of one household in ascending agent index.
pub(crate) struct HouseholdMemberIter<'a> {
    next: &'a [usize],
    member: usize,
}

impl Iterator for HouseholdMemberIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<usize> {
        let member = self.member;
        if member == NO_MEMBER {
            return None;
        }
        self.member = self.next[member];
        Some(member)
    }
}

impl AgentSystem {
    /// Lowest-index member of household `household_id`, or [`NO_MEMBER`]. O(1).
    pub(crate) fn first_household_member(&self, household_id: usize) -> usize {
        self.household_members.head(household_id)
    }

    /// Member after `agent_idx` in its household, in ascending agent index, or [`NO_MEMBER`].
    /// O(1).
    pub(crate) fn next_household_member(&self, agent_idx: usize) -> usize {
        self.household_members.next[agent_idx]
    }

    /// Members of household `household_id` in ascending agent index. O(1) per member.
    pub(crate) fn household_members(&self, household_id: usize) -> HouseholdMemberIter<'_> {
        HouseholdMemberIter {
            next: &self.household_members.next,
            member: self.household_members.head(household_id),
        }
    }

    /// Links agents appended to the store directly since the last mutation. O(1) when none
    /// were, else O(household size) per appended agent.
    pub(crate) fn register_appended_household_members(&mut self) {
        self.household_members
            .register_appended(&self.agents.household_id);
    }

    /// Rebuilds every household's member list from the agents' `household_id`, after a load
    /// or a restore that rewrote agent records. O(agents).
    pub(crate) fn rebuild_household_members(&mut self) {
        self.household_members.rebuild(&self.agents.household_id);
    }

    // Moves `agent_idx` to `household_id` (`usize::MAX` leaves its household) and keeps both
    // lists sorted. O(household sizes).
    pub(super) fn set_household_membership(&mut self, agent_idx: usize, household_id: usize) {
        self.register_appended_household_members();
        let old = self.agents.household_id[agent_idx];
        if old == household_id {
            return;
        }
        if old != usize::MAX {
            self.household_members.unlink(agent_idx, old);
        }
        self.agents.household_id[agent_idx] = household_id;
        if household_id != usize::MAX {
            self.household_members.link(agent_idx, household_id);
        }
    }

    // Unlinks the agent at `index` and the store's last agent before `swap_remove(index)`;
    // returns the moved agent's household for `relink_after_swap_remove`.
    pub(super) fn unlink_for_swap_remove(&mut self, index: usize) -> usize {
        self.register_appended_household_members();
        let last = self.agents.len() - 1;
        let removed = self.agents.household_id[index];
        if removed != usize::MAX {
            self.household_members.unlink(index, removed);
        }
        if index == last {
            return usize::MAX;
        }
        let moved = self.agents.household_id[last];
        if moved != usize::MAX {
            self.household_members.unlink(last, moved);
        }
        moved
    }

    // Drops the removed slot and links the agent swap-moved to `index` into `moved_household`.
    pub(super) fn relink_after_swap_remove(&mut self, index: usize, moved_household: usize) {
        self.household_members.next.pop();
        if moved_household != usize::MAX {
            self.household_members.link(index, moved_household);
        }
    }

    // Follows a household swap-remove remap of `household_id`. O(mapping).
    pub(super) fn remap_household_members(&mut self, mapping: &HashMap<usize, usize>) {
        self.household_members.remap_households(mapping);
    }

    /// Asserts that the maintained member lists equal a fresh rebuild from `household_id`.
    #[cfg(test)]
    pub(crate) fn assert_household_members_match_rebuild(&self) {
        let mut fresh = HouseholdMembers::default();
        fresh.rebuild(&self.agents.household_id);
        let maintained = &self.household_members;
        assert_eq!(
            maintained.next, fresh.next,
            "maintained member links differ from a rebuild"
        );
        for household in 0..maintained.heads.len().max(fresh.heads.len()) {
            assert_eq!(
                maintained.head(household),
                fresh.head(household),
                "household {household} first member differs from a rebuild"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::BuildingAllocator;

    fn members(agents: &AgentSystem, household: usize) -> Vec<usize> {
        agents.household_members(household).collect()
    }

    #[test]
    fn member_lists_ascend_and_follow_joins_and_swap_removes() {
        let mut agents = AgentSystem::new();
        let mut allocator = BuildingAllocator::new();
        for household in [2, 0, 2, 1, 0, 2] {
            let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.assign_household_id(agent, household);
        }
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 0), [1, 4]);
        assert_eq!(members(&agents, 2), [0, 2, 5]);
        // The last agent (household 2) moves into slot 1 and joins its list in order.
        agents.kill_agent(1, &mut allocator);
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 0), [4]);
        assert_eq!(members(&agents, 2), [0, 1, 2]);
        agents.assign_household_id(2, 0);
        agents.assign_household_id(3, usize::MAX);
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 0), [2, 4]);
        assert_eq!(members(&agents, 1), [] as [usize; 0]);
        // Removing the last agent moves nobody; a moved agent without a household joins none.
        agents.kill_agent(4, &mut allocator);
        agents.kill_agent(0, &mut allocator);
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 0), [2]);
        assert_eq!(members(&agents, 2), [1]);
    }

    #[test]
    fn member_lists_follow_carrier_materialization_and_household_remaps() {
        let mut agents = AgentSystem::new();
        for household in [0, 1, 2] {
            let agent = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
            agents.assign_household_id(agent, household);
        }
        let carrier = agents.spawn_household_arrival_carrier(
            usize::MAX,
            3,
            crate::simulation::economy::agents::household_age_composition(usize::MAX, 0, 3),
            0,
            0.0,
            0.0,
        );
        agents.materialize_household_carrier(carrier, 3, 0, None);
        let member = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.assign_household_id(member, 3);
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 3), [carrier, member]);
        // Households 0 and 1 left; 3 moved into 0 and 2 into 1, both at once.
        let mut allocator = BuildingAllocator::new();
        agents.kill_agent(1, &mut allocator);
        agents.kill_agent(0, &mut allocator);
        agents.remap_household_indices(&HashMap::from([(3, 0), (2, 1)]));
        agents.assert_household_members_match_rebuild();
        assert_eq!(agents.household_id, [0, 0, 1]);
        assert_eq!(members(&agents, 0), [0, 1]);
        assert_eq!(members(&agents, 1), [2]);
        assert_eq!(members(&agents, 3), [] as [usize; 0]);
    }

    #[test]
    fn agents_appended_directly_join_their_lists_on_the_next_mutation() {
        let mut agents = AgentSystem::new();
        let first = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        agents.assign_household_id(first, 0);
        let mut appended = agents.agents.get(first).unwrap().to_owned();
        appended.household_id = 0;
        agents.agents.push(appended);
        agents.register_appended_household_members();
        agents.assert_household_members_match_rebuild();
        assert_eq!(members(&agents, 0), [0, 1]);
    }
}
