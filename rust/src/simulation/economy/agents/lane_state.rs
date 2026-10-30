// SPDX-License-Identifier: GPL-2.0-only

//! Shared lane occupancy contract for movement, road edits, and snapshot restoration.

use super::{
    ACCESS_LANE_ATTACH_PENDING, AgentVec, MODE_WALK, TRANSIT_INTERSECTION, TRANSIT_NETWORK,
};
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

/// Disjoint fields of a lane-attach hold, shared by serial callers and the movement pass.
pub(super) struct LaneHoldMut<'a> {
    pub(super) agent: usize,
    pub(super) lane: &'a mut usize,
    pub(super) edge: &'a mut usize,
    pub(super) node: &'a mut u32,
    pub(super) distance: &'a mut f32,
    pub(super) transit: &'a mut u8,
    pub(super) attach_lane: &'a mut u32,
    pub(super) attach_d: &'a mut f32,
    pub(super) flags: &'a mut u8,
    pub(super) speed: &'a mut f32,
}

impl LaneHoldMut<'_> {
    /// Leaves the agent off every lane, waiting at `distance` on vehicle road lane `id` whose
    /// origin is `node`; movement attaches it there through the egress gap check and claim.
    /// A non-vehicle mode or lane is a simulation bug, as in [`LaneStateMut::enter`].
    #[inline]
    #[track_caller]
    pub(super) fn hold(self, lanes: &LaneSystem, mode: u8, id: usize, distance: f32, node: u32) {
        let agent = self.agent;
        let lane = lane_for_mode(lanes, id, mode).unwrap_or_else(|reason| {
            panic!("lane attach rejected: agent={agent} lane={id}: {reason}")
        });
        assert!(
            mode != MODE_WALK && lane.edge_id != usize::MAX,
            "lane attach rejected: agent={agent} lane={id} is not a vehicle road lane"
        );
        assert!(
            distance.is_finite() && distance >= 0.0,
            "lane attach rejected: agent={agent} lane={id} invalid distance={distance}"
        );
        *self.lane = usize::MAX;
        *self.edge = lane.edge_id;
        *self.node = node;
        *self.distance = distance;
        *self.transit = TRANSIT_NETWORK;
        *self.attach_lane = id as u32;
        *self.attach_d = distance;
        *self.flags |= ACCESS_LANE_ATTACH_PENDING;
        *self.speed = 0.0;
    }
}

impl AgentVec {
    /// Stands vehicle `i` lane-less at `distance` on lane `id` (whose origin is `node`) until
    /// movement finds the attach slot clear and claims it: placement outside the tick never
    /// steps onto a lane, so it cannot overfill one (`TRAFFIC-11`). O(1).
    #[track_caller]
    pub(crate) fn hold_for_lane_attach(
        &mut self,
        i: usize,
        lanes: &LaneSystem,
        id: usize,
        distance: f32,
        node: u32,
    ) {
        LaneHoldMut {
            agent: i,
            lane: &mut self.current_lane_id[i],
            edge: &mut self.current_edge[i],
            node: &mut self.current_node[i],
            distance: &mut self.lane_distance[i],
            transit: &mut self.transit[i],
            attach_lane: &mut self.planned_attach_lane_id[i],
            attach_d: &mut self.planned_attach_lane_d[i],
            flags: &mut self.access_flags[i],
            speed: &mut self.speed[i],
        }
        .hold(lanes, self.transit_mode[i], id, distance, node);
    }

    /// Returns the attach point of a vehicle held by [`Self::hold_for_lane_attach`]. O(1).
    pub(crate) fn pending_lane_attach(&self, i: usize) -> Option<(u32, f32)> {
        ((self.access_flags[i] & ACCESS_LANE_ATTACH_PENDING) != 0
            && self.transit[i] == TRANSIT_NETWORK
            && self.current_lane_id[i] == usize::MAX
            && self.planned_attach_lane_id[i] != u32::MAX
            && self.planned_attach_lane_d[i] >= 0.0
            && self.planned_attach_lane_d[i].is_finite())
            .then(|| {
                (
                    self.planned_attach_lane_id[i],
                    self.planned_attach_lane_d[i],
                )
            })
    }

    /// Restores a held attach point after a trip-plan clear: the car still stands at it, and
    /// dropping it would move the car to its lane's origin node. O(1).
    pub(crate) fn keep_pending_lane_attach(&mut self, i: usize, pending: Option<(u32, f32)>) {
        if let Some((lane, distance)) = pending {
            self.planned_attach_lane_id[i] = lane;
            self.planned_attach_lane_d[i] = distance;
            self.access_flags[i] |= ACCESS_LANE_ATTACH_PENDING;
        }
    }

    /// Commits a serial lane transition through the same contract as parallel movement. A car on
    /// a lane no longer waits to attach, so any lane-attach hold ends here.
    #[track_caller]
    pub(crate) fn enter_lane(
        &mut self,
        i: usize,
        lanes: &LaneSystem,
        id: usize,
        distance: f32,
        node: u32,
    ) {
        self.access_flags[i] &= !ACCESS_LANE_ATTACH_PENDING;
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
