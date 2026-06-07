//! Deterministic lane-entry claim serialization for the parallel movement pass.

use super::super::{
    ACCESS_PLAN_VALID, MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_IMMIGRATING, TRANSIT_INTERSECTION,
    TRANSIT_NETWORK,
};
use super::access::planned_detach_is_legal;
use super::traffic::{connector_turn_speed, junction_car_speed, planned_lane_change_target};
use crate::config::CAR_JUNCTION_SPEED_MS;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use std::sync::atomic::{AtomicBool, Ordering};

const CLAIM_REACH_EPS_M: f32 = 0.05;

/// Read-only claim state passed into the movement FSM.
pub(in crate::simulation::economy::agents::tick) struct LaneClaimContext<'a> {
    claimed: &'a [AtomicBool],
    serial_agents: &'a [bool],
}

impl<'a> LaneClaimContext<'a> {
    /// Builds a claim context for one movement pass.
    pub(in crate::simulation::economy::agents::tick) fn new(
        claimed: &'a [AtomicBool],
        serial_agents: &'a [bool],
    ) -> Self {
        Self {
            claimed,
            serial_agents,
        }
    }

    /// Returns whether `agent_idx` was marked for deterministic serial claim execution.
    #[inline(always)]
    pub(in crate::simulation::economy::agents::tick) fn agent_is_serial(
        &self,
        agent_idx: usize,
    ) -> bool {
        self.serial_agents.get(agent_idx).copied().unwrap_or(false)
    }

    /// Claims `lane_id` for `agent_idx` after asserting that the agent is in the serial group.
    #[inline(always)]
    pub(in crate::simulation::economy::agents::tick) fn claim_lane(
        &self,
        agent_idx: usize,
        lane_id: usize,
    ) -> bool {
        debug_assert!(
            self.agent_is_serial(agent_idx),
            "agent {agent_idx} attempted a lane claim outside the deterministic serial pass"
        );
        self.claimed
            .get(lane_id)
            .map(|claimed| !claimed.swap(true, Ordering::AcqRel))
            .unwrap_or(false)
    }
}

impl AgentSystem {
    /// Marks agents whose movement may touch a lane-entry claim this tick.
    pub(super) fn prepare_claim_serial_agents(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
        n: usize,
    ) {
        if self.claim_serial_agents.len() != n {
            self.claim_serial_agents.resize(n, false);
        }
        for serial in &mut self.claim_serial_agents {
            *serial = false;
        }

        for i in 0..n {
            self.claim_serial_agents[i] =
                self.agent_may_touch_lane_claim(i, allocator, transit_network, graph, delta);
        }
    }

    fn agent_may_touch_lane_claim(
        &self,
        i: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
    ) -> bool {
        match self.agents.transit[i] {
            TRANSIT_ACCESS_EGRESS => self.agents.transit_mode[i] == MODE_CAR,
            TRANSIT_NETWORK | TRANSIT_IMMIGRATING | TRANSIT_INTERSECTION => {
                self.network_agent_may_touch_lane_claim(i, allocator, transit_network, graph, delta)
            }
            _ => false,
        }
    }

    fn network_agent_may_touch_lane_claim(
        &self,
        i: usize,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
    ) -> bool {
        let lane_id = self.agents.current_lane_id[i];
        if lane_id == usize::MAX || lane_id >= transit_network.lane_system.lanes.len() {
            return true;
        }

        let remaining = self.agent_network_claim_distance(i, transit_network) * delta;
        if remaining <= 0.0 {
            return false;
        }

        let lane = &transit_network.lane_system.lanes[lane_id];
        let dist_to_end = (lane.length - self.agents.lane_distance[i]).max(0.0);
        if remaining + CLAIM_REACH_EPS_M >= dist_to_end {
            return true;
        }

        let target_building = self.agents.target_building[i];
        if target_building != usize::MAX && target_building >= allocator.entrances.len() {
            return true;
        }
        if (self.agents.access_flags[i] & ACCESS_PLAN_VALID) == 0 {
            return target_building != usize::MAX;
        }

        let planned_detach_lane_id = self.agents.planned_detach_lane_id[i] as usize;
        if planned_detach_lane_id == usize::MAX {
            return target_building != usize::MAX;
        }
        if target_building < allocator.entrances.len()
            && !planned_detach_is_legal(
                self.agents.transit_mode[i],
                &allocator.entrances[target_building],
                planned_detach_lane_id,
                self.agents.planned_detach_lane_d[i],
                self.agents.planned_detach_node[i],
                transit_network,
                graph,
            )
        {
            return true;
        }

        let lane_d = self.agents.lane_distance[i];
        let detach_d = self.agents.planned_detach_lane_d[i];
        let already_on_detach_lane = lane_id == planned_detach_lane_id;
        let could_change_to_detach_lane = planned_lane_change_target(
            lane_id,
            planned_detach_lane_id,
            lane_d,
            detach_d,
            transit_network,
        )
        .is_some();

        (already_on_detach_lane || could_change_to_detach_lane)
            && lane_d + remaining + CLAIM_REACH_EPS_M >= detach_d
    }

    fn agent_network_claim_distance(&self, i: usize, transit_network: &TransitNetwork) -> f32 {
        if self.agents.transit_mode[i] != MODE_CAR {
            return 4.0;
        }
        if self.agents.transit[i] == TRANSIT_INTERSECTION {
            let lane_id = self.agents.current_lane_id[i];
            let turn_speed = transit_network
                .lane_system
                .lanes
                .get(lane_id)
                .map(connector_turn_speed)
                .unwrap_or(CAR_JUNCTION_SPEED_MS);
            junction_car_speed(self.agents.speed[i]).min(turn_speed)
        } else {
            self.agents.speed[i]
        }
    }
}
