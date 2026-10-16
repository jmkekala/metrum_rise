// SPDX-License-Identifier: GPL-2.0-only

//! The one agent the player inspects (`DEBUG-05`).
//!
//! Presentation state: never saved and never read by the tick. The selection keeps the agent's
//! row next to the render ID that names it to Godot, and the single swap-remove site repairs the
//! row in O(1), so finding the inspected agent never scans the population. A world load replaces
//! the agent system and with it the selection.

use super::data::AgentSystem;

/// The inspected agent's row and render ID.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InspectedAgent {
    pub(crate) index: usize,
    pub(crate) render_id: u64,
}

/// An inspection the simulation ended by removing its agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EndedInspection {
    pub(crate) render_id: u64,
    pub(crate) reason: &'static str,
}

impl AgentSystem {
    /// Inspects the agent at `index` instead of any other. Returns its render ID.
    pub(crate) fn inspect_agent(&mut self, index: usize) -> Option<u64> {
        let render_id = *self.agents.render_id.get(index)?;
        self.inspected = Some(InspectedAgent { index, render_id });
        self.inspection_ended = None;
        self.inspected_failure = None;
        Some(render_id)
    }

    /// Ends the inspection.
    pub(crate) fn clear_inspected_agent(&mut self) {
        self.inspected = None;
        self.inspection_ended = None;
        self.inspected_failure = None;
    }

    /// The inspected agent's row, when `render_id` is the inspected agent.
    pub(crate) fn inspected_agent_index(&self, render_id: u64) -> Option<usize> {
        self.inspected
            .filter(|inspected| inspected.render_id == render_id)
            .map(|inspected| inspected.index)
    }

    /// The `events` line of the inspected agent's latest trip failure, when `render_id` is the
    /// inspected agent and it failed since it was picked.
    pub(crate) fn inspected_agent_failure(&self, render_id: u64) -> Option<&str> {
        self.inspected
            .filter(|inspected| inspected.render_id == render_id)
            .and(self.inspected_failure.as_deref())
    }

    /// Why the simulation ended `render_id`'s inspection, if it did.
    pub(crate) fn inspection_end_reason(&self, render_id: u64) -> Option<&'static str> {
        self.inspection_ended
            .filter(|ended| ended.render_id == render_id)
            .map(|ended| ended.reason)
    }

    // Row `index` was swap-removed and row `last` moved into it.
    pub(super) fn repair_inspection_after_remove(
        &mut self,
        index: usize,
        last: usize,
        reason: &'static str,
    ) {
        let Some(inspected) = self.inspected.as_mut() else {
            return;
        };
        if inspected.index == index {
            self.inspection_ended = Some(EndedInspection {
                render_id: inspected.render_id,
                reason,
            });
            self.inspected = None;
            self.inspected_failure = None;
        } else if inspected.index == last {
            inspected.index = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::economy::agents::VEHICLE_FREIGHT_DELIVERY;

    fn agents(count: usize) -> AgentSystem {
        let mut agents = AgentSystem::new();
        for _ in 0..count {
            agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        }
        agents
    }

    #[test]
    fn inspection_follows_the_agent_moved_by_a_swap_remove() {
        let mut agents = agents(4);
        let mut allocator = BuildingAllocator::new();
        let render_id = agents.inspect_agent(3).unwrap();
        agents.kill_agent(1, &mut allocator);
        assert_eq!(agents.inspected_agent_index(render_id), Some(1));
        assert_eq!(agents.render_id[1], render_id);
        // Removing the last row moves nobody.
        agents.kill_agent(2, &mut allocator);
        assert_eq!(agents.inspected_agent_index(render_id), Some(1));
        assert_eq!(agents.inspected_agent_index(render_id + 1), None);
        agents.kill_agent(1, &mut allocator);
        assert_eq!(
            agents.inspection_end_reason(render_id),
            Some("left the city with its household")
        );
    }

    #[test]
    fn removing_the_inspected_agent_ends_the_inspection_with_a_reason() {
        let mut agents = agents(3);
        agents.vehicle_type[0] = VEHICLE_FREIGHT_DELIVERY;
        agents.freight_shipment_id[0] = 7;
        let render_id = agents.inspect_agent(0).unwrap();
        agents.remove_freight_carrier(0, 7);
        assert_eq!(agents.inspected_agent_index(render_id), None);
        assert_eq!(
            agents.inspection_end_reason(render_id),
            Some("finished its freight trip")
        );
        // The agent moved into row 0 is not the inspected one.
        assert_ne!(agents.render_id[0], render_id);
        agents.inspect_agent(0);
        assert_eq!(agents.inspection_end_reason(render_id), None);
    }
}
