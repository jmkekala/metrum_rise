// SPDX-License-Identifier: GPL-2.0-only

//! Parallel dispatch phase for the movement state machine.

use super::claims::LaneClaimContext;
use super::runtime::dispatch_agents;
use super::slices::MovementSlices;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::data::{AgentSystem, MovementClaimMode};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    /// Dispatches the parallel per-agent movement state machine.
    pub(super) fn dispatch_movement_pass(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
        time: &TimeSystem,
        n: usize,
    ) {
        self.prepare_lane_claims(allocator, transit_network, graph, delta, n);

        let slices = MovementSlices::new(&mut self.agents);

        let lane_buckets = &self.lane_buckets;
        let lane_claims = LaneClaimContext::new(&self.lane_claim_owner, &self.movement_claim_modes);
        let sim_time = self.sim_time;
        let economy_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let economy_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        dispatch_agents(n, |i| unsafe {
            if self.movement_claim_modes[i] == MovementClaimMode::Dynamic {
                return;
            }
            Self::process_agent_movement(
                i,
                delta,
                sim_time,
                time,
                allocator,
                transit_network,
                graph,
                &self.pathfind_count,
                lane_buckets,
                &lane_claims,
                &economy_tuning.operational_clock,
                &economy_catalog,
                &slices,
            );
        });

        for i in 0..n {
            if self.movement_claim_modes[i] != MovementClaimMode::Dynamic {
                continue;
            }
            unsafe {
                Self::process_agent_movement(
                    i,
                    delta,
                    sim_time,
                    time,
                    allocator,
                    transit_network,
                    graph,
                    &self.pathfind_count,
                    lane_buckets,
                    &lane_claims,
                    &economy_tuning.operational_clock,
                    &economy_catalog,
                    &slices,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests;
