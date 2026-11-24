// SPDX-License-Identifier: GPL-3.0-or-later

//! Parallel dispatch phase for the movement state machine.

use super::TripQueryScratch;
use super::claims::LaneClaimContext;
use super::runtime::{PAR_THRESHOLD, dispatch_agents_with_owned_scratch};
use super::slices::MovementSlices;
use super::trip_failures::FailureSink;
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::ACCESS_LANE_ATTACH_PENDING;
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
        let shards = &mut self.movement_query_shards;
        crate::thread_pools::install(|| {
            shards.resize_with(rayon::current_num_threads(), Default::default);
            shards.par_iter_mut().for_each(|scratch| {
                scratch.prepare_routes(transit_network, graph);
            })
        });
        self.movement_serial_query
            .prepare_routes(transit_network, graph);
        self.parking_work.quotes.sync(transit_network);
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
        slices.parking_quotes = Some(&self.parking_work.quotes);
        slices.kerb_lines = &self.parking_work.kerb_lines;
        slices.exit_holds = &self.parking_work.exit_holds;
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
                    // A car held at its attach point counts its wait to join, below
                    // `CROSSING_GIVE_UP_S` here (`TRAFFIC-45`, `try_pending_lane_attach`).
                    if *slices.access_flags.get(i) & ACCESS_LANE_ATTACH_PENDING != 0 {
                        *slices.crossing_wait.get_mut(i) =
                            slices.crossing_wait.get(i).max(0.0) + delta;
                    }
                    slices.note_frozen_parking_agent(i);
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

        // The egress steps the parallel part left untouched, in ascending agent order whatever
        // the worker count (`PERF-33`). O(D log D) for D of them; the list keeps its storage.
        let deferred = &mut self.movement_deferred;
        deferred.clear();
        for shard in &mut self.movement_query_shards {
            shard.drain_deferred_egress(deferred);
        }
        self.movement_serial_query.drain_deferred_egress(deferred);
        deferred.sort_unstable();
        let deferred = &self.movement_deferred;
        let lane_claims = lane_claims.for_serial_pass();

        // A borrowed slice keeps the bound in a register for this serial scan of every agent.
        // The serial part runs the dynamic handoffs and the deferred egress steps together in
        // ascending agent order: the order of every claim and every read of a live speed is the
        // order it had when every car egress ran here.
        TripQueryScratch::with_owned(&mut self.movement_serial_query, || {
            let mut next_deferred = 0;
            for (i, &mode) in movement_claim_modes[..n].iter().enumerate() {
                if mode != MovementClaimMode::Dynamic {
                    if deferred.get(next_deferred) != Some(&i) {
                        continue;
                    }
                    next_deferred += 1;
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
