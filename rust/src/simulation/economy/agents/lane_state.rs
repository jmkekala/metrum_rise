// SPDX-License-Identifier: GPL-2.0-only

//! Shared lane occupancy contract for movement, road edits, and snapshot restoration.

use super::{AgentVec, MODE_WALK, TRANSIT_INTERSECTION, TRANSIT_NETWORK};
use crate::simulation::network::lanes::{Lane, LaneSystem, LaneType};

/// Resolves a lane only when its transport type agrees with the agent's mode.
/// Constant time and allocation-free; callers retain route and access-policy ownership.
pub(crate) fn lane_for_mode(
    lanes: &LaneSystem,
    id: usize,
    mode: u8,
) -> Result<&Lane, &'static str> {
    let lane = lanes.lanes.get(id).ok_or("lane ID is out of bounds")?;
    let desired = if mode == MODE_WALK {
        LaneType::Foot
    } else {
        LaneType::Vehicle
    };
    if lane.lane_type != desired {
        return Err("lane type does not match travel mode");
    }
    Ok(lane)
}

/// Checks persisted ownership without changing or repairing the saved state.
pub(crate) fn validate_lane_owner(
    lane: &Lane,
    edge: usize,
    node: u32,
    transit: u8,
) -> Result<(), &'static str> {
    if lane.edge_id == usize::MAX {
        if transit != TRANSIT_INTERSECTION || lane.node_id != node as usize {
            return Err("connector does not match transit state or junction owner");
        }
    } else if lane.edge_id != edge {
        return Err("lane does not match road owner");
    }
    Ok(())
}

/// Disjoint occupancy fields; safe references let serial and parallel callers share one writer.
pub(super) struct LaneStateMut<'a> {
    pub(super) agent: usize,
    pub(super) lane: &'a mut usize,
    pub(super) edge: &'a mut usize,
    pub(super) node: &'a mut u32,
    pub(super) distance: &'a mut f32,
    pub(super) transit: &'a mut u8,
}

impl LaneStateMut<'_> {
    /// Validates before publishing all lane ownership fields together. A violation is a
    /// simulation bug: fail at the caller rather than silently changing the trip's mode.
    #[inline]
    #[track_caller]
    pub(super) fn enter(self, lanes: &LaneSystem, mode: u8, id: usize, distance: f32, node: u32) {
        let lane = match lane_for_mode(lanes, id, mode) {
            Ok(lane) => lane,
            Err(reason) => panic!(
                "lane transition rejected: agent={} mode={mode} from_lane={} from_edge={} from_node={} from_transit={} to_lane={id} node={node} revision={}: {reason}",
                self.agent,
                *self.lane,
                *self.edge,
                *self.node,
                *self.transit,
                lanes.revision()
            ),
        };
        assert!(
            distance.is_finite() && distance >= 0.0,
            "lane transition rejected: agent={} lane={id} invalid distance={distance}",
            self.agent
        );
        let transit = if lane.edge_id == usize::MAX {
            TRANSIT_INTERSECTION
        } else {
            TRANSIT_NETWORK
        };
        assert!(
            validate_lane_owner(lane, lane.edge_id, node, transit).is_ok(),
            "lane transition rejected: agent={} lane={id} node={node} lane_node={}",
            self.agent,
            lane.node_id
        );
        *self.lane = id;
        *self.edge = lane.edge_id;
        *self.node = node;
        *self.distance = distance;
        *self.transit = transit;
    }
}

impl AgentVec {
    /// Commits a serial lane transition through the same contract as parallel movement.
    #[track_caller]
    pub(crate) fn enter_lane(
        &mut self,
        i: usize,
        lanes: &LaneSystem,
        id: usize,
        distance: f32,
        node: u32,
    ) {
        LaneStateMut {
            agent: i,
            lane: &mut self.current_lane_id[i],
            edge: &mut self.current_edge[i],
            node: &mut self.current_node[i],
            distance: &mut self.lane_distance[i],
            transit: &mut self.transit[i],
        }
        .enter(lanes, self.transit_mode[i], id, distance, node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::agents::{AgentSystem, MODE_CAR};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    #[test]
    fn rejected_transitions_preserve_previous_occupancy() {
        let mut agents = AgentSystem::new();
        let i = agents.spawn_housed_agent(usize::MAX, 0.0, 0.0);
        let mut lanes = LaneSystem::new();
        lanes.lanes.push(Lane {
            edge_id: 7,
            lane_type: LaneType::Foot,
            ..Default::default()
        });
        lanes.lanes.push(Lane {
            edge_id: 8,
            lane_type: LaneType::Vehicle,
            ..Default::default()
        });
        lanes.lanes.push(Lane {
            node_id: 9,
            lane_type: LaneType::Foot,
            ..Default::default()
        });
        agents.transit_mode[i] = MODE_WALK;
        agents.agents.enter_lane(i, &lanes, 0, 3.0, 4);
        for (id, distance, node) in [
            (1, 0.0, 4),
            (99, 0.0, 4),
            (2, 0.0, 4),
            (0, f32::NAN, 4),
            (0, -1.0, 4),
        ] {
            let result = catch_unwind(AssertUnwindSafe(|| {
                agents.agents.enter_lane(i, &lanes, id, distance, node);
            }));
            assert!(result.is_err());
            assert_eq!(
                (
                    agents.current_lane_id[i],
                    agents.current_edge[i],
                    agents.current_node[i],
                    agents.lane_distance[i],
                    agents.transit[i]
                ),
                (0, 7, 4, 3.0, TRANSIT_NETWORK)
            );
        }
        agents.agents.enter_lane(i, &lanes, 2, 0.0, 9);
        assert_eq!(
            (
                agents.current_edge[i],
                agents.current_node[i],
                agents.transit[i]
            ),
            (usize::MAX, 9, TRANSIT_INTERSECTION)
        );
        agents.transit_mode[i] = MODE_CAR;
        assert!(
            catch_unwind(AssertUnwindSafe(|| agents
                .agents
                .enter_lane(i, &lanes, 0, 0.0, 4)))
            .is_err()
        );
        agents.agents.enter_lane(i, &lanes, 1, 0.0, 4);
        assert_eq!(
            (
                agents.current_lane_id[i],
                agents.current_edge[i],
                agents.transit[i]
            ),
            (1, 8, TRANSIT_NETWORK)
        );
    }
}
