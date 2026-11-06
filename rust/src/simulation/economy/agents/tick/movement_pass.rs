// SPDX-License-Identifier: GPL-2.0-only

//! Parallel dispatch phase for the movement state machine.

use super::TripQueryScratch;
use super::claims::LaneClaimContext;
use super::runtime::{PAR_THRESHOLD, dispatch_agents_with_owned_scratch};
use super::slices::MovementSlices;
use super::trip_failures::FailureSink;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::data::{AgentSystem, MovementClaimMode};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;

impl AgentSystem {
    /// Sizes the movement pass's route scratch, one per worker of the current pool and one for
    /// the serial pass, for the network's current revision: loading calls it so the first frame
    /// does not (`SIM-02-K`). Scratch never changes results. O(lanes) per scratch on a new
    /// revision, O(1) otherwise.
    pub(crate) fn prepare_movement_scratch(
        &mut self,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.movement_query_shards
            .resize_with(rayon::current_num_threads(), Default::default);
        self.movement_query_shards
            .par_iter_mut()
            .for_each(|scratch| {
                scratch.prepare_routes(transit_network, graph);
            });
        self.movement_serial_query
            .prepare_routes(transit_network, graph);
    }

    /// Dispatches the parallel per-agent movement state machine.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dispatch_movement_pass(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
        time: &TimeSystem,
        n: usize,
        parking: Option<&crate::simulation::parking::ParkingSystem>,
    ) {
        self.prepare_lane_claims(allocator, transit_network, graph, delta, n);
        // Before the movement slices borrow the agent columns.
        if n >= PAR_THRESHOLD {
            self.prepare_movement_scratch(transit_network, graph);
        } else {
            self.movement_serial_query
                .prepare_routes(transit_network, graph);
        }

        let mut slices = MovementSlices::new(&mut self.agents);
        slices.parking = parking;
        slices.failures = FailureSink::new(std::mem::take(&mut self.trip_failures.buffer));

        let lane_buckets = &self.lane_buckets;
        let lane_claims = LaneClaimContext::new(&self.lane_claim_owner, &self.movement_claim_modes);
        let sim_time = self.sim_time;
        let economy_tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let economy_catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        let movement_claim_modes = &self.movement_claim_modes;
        let pathfind_count = &self.pathfind_count;

        dispatch_agents_with_owned_scratch(
            n,
            &mut self.movement_query_shards,
            &mut self.movement_serial_query,
            |i| unsafe {
                if movement_claim_modes[i] == MovementClaimMode::HeldEntry {
                    *slices.speed.get_mut(i) = 0.0;
                    return;
                }
                if movement_claim_modes[i] == MovementClaimMode::Dynamic {
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
                    pathfind_count,
                    lane_buckets,
                    &lane_claims,
                    &economy_tuning.operational_clock,
                    &economy_catalog,
                    &slices,
                );
            },
        );

        // A borrowed slice keeps the bound in a register for this serial scan of every agent.
        TripQueryScratch::with_owned(&mut self.movement_serial_query, || {
            for (i, &mode) in movement_claim_modes[..n].iter().enumerate() {
                if mode != MovementClaimMode::Dynamic {
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
                        pathfind_count,
                        lane_buckets,
                        &lane_claims,
                        &economy_tuning.operational_clock,
                        &economy_catalog,
                        &slices,
                    );
                }
            }
        });
        #[cfg(test)]
        super::contract::add(super::contract::Phase::MovementAgents, n);
        self.trip_failures.buffer = slices.failures.into_inner();
    }
}

#[cfg(test)]
mod ring_tests;
#[cfg(test)]
mod tests;
