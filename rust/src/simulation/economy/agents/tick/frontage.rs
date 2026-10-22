// SPDX-License-Identifier: GPL-2.0-only

//! Per-second live frontage delay updates used by agent access planning.

use super::super::MODE_CAR;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;
use rayon::prelude::*;

const FRONTAGE_DELAY_UPDATE_S: f32 = 1.0;

impl AgentSystem {
    /// Updates each lane's live frontage delay from aggregated live lane speeds.
    ///
    /// Runs at fixed cadence rather than every tick so planner-visible congestion stays stable
    /// and cheap to maintain. Economy prices read the hourly copy published by
    /// [`TransitNetwork::publish_frontage_prices`]. O(agents) per worker plus O(lanes / p).
    pub fn update_frontage_delay_cache(
        &mut self,
        transit_network: &mut TransitNetwork,
        graph: &RegionGraph,
        delta: f32,
    ) {
        transit_network.frontage_delay_elapsed_s += delta;
        if transit_network.frontage_delay_elapsed_s < FRONTAGE_DELAY_UPDATE_S {
            return;
        }
        let update_steps =
            (transit_network.frontage_delay_elapsed_s / FRONTAGE_DELAY_UPDATE_S).floor() as i32;
        transit_network.frontage_delay_elapsed_s -= update_steps as f32 * FRONTAGE_DELAY_UPDATE_S;

        let lane_count = transit_network.lane_system.lanes.len();
        // Each range task below zeroes its own sums first.
        self.lane_speed_sum.resize(lane_count, 0.0);
        self.lane_vehicle_cnt.resize(lane_count, 0);

        // The per-lane f32 sums accumulate in agent order, so delays do not depend on the worker
        // count. Each agent's lane is noted in parallel (none for an agent that is not a car on
        // a lane); the lanes then split into one range per worker, and each range's task reads
        // the notes in agent order, adds the cars on its own lanes and updates those lanes'
        // delays, so each lane's sum is the serial pass's whatever the split (`SIM-02-K`). Once
        // per simulated second; O(agents / p) for the notes, O(agents) per range task over
        // 4-byte notes and O(lanes / p), where the serial sums and the separate lane pass held
        // 0.7 ms of the minute's first frame at 100k residents.
        debug_assert!(lane_count < u32::MAX as usize);
        let agents = &self.agents;
        let notes = &mut self.frontage_lanes;
        notes.resize(agents.len(), u32::MAX);
        notes.par_iter_mut().enumerate().for_each(|(i, note)| {
            let lid = agents.current_lane_id[i];
            *note = if agents.transit_mode[i] == MODE_CAR && lid < lane_count {
                lid as u32
            } else {
                u32::MAX
            };
        });
        #[cfg(test)]
        super::contract::add(super::contract::Phase::FrontageAgents, self.agents.len());

        let notes = &self.frontage_lanes;
        let speeds = &self.agents.speed;
        let smoothing_retain = 0.75_f32.powi(update_steps);
        let smoothing_gain = 1.0 - smoothing_retain;
        let span = lane_count
            .div_ceil(rayon::current_num_threads().max(1))
            .max(1);
        // Lanes are independent once speeds are aggregated. Economy prices read the hourly
        // published copy instead.
        transit_network
            .lane_system
            .lanes
            .par_chunks_mut(span)
            .zip(self.lane_speed_sum.par_chunks_mut(span))
            .zip(self.lane_vehicle_cnt.par_chunks_mut(span))
            .enumerate()
            .for_each(|(range, ((lanes, speed_sum), vehicle_count))| {
                let first = range * span;
                speed_sum.fill(0.0);
                vehicle_count.fill(0);
                for (i, &note) in notes.iter().enumerate() {
                    // A note outside this range, or none, wraps past the range's length.
                    let offset = (note as usize).wrapping_sub(first);
                    if offset < speed_sum.len() {
                        speed_sum[offset] += speeds[i];
                        vehicle_count[offset] += 1;
                    }
                }
                for (offset, lane) in lanes.iter_mut().enumerate() {
                    lane.live_frontage_delay_s = if lane.lane_type != LaneType::Vehicle
                        || lane.edge_id == usize::MAX
                        || lane.edge_id >= graph.edge_count()
                    {
                        0.0
                    } else {
                        let edge = graph.edge(lane.edge_id);
                        if edge.speed_limit <= 1e-6 || lane.length <= 1e-6 {
                            0.0
                        } else {
                            let raw_lane_delay_penalty_s = if vehicle_count[offset] == 0 {
                                0.0
                            } else {
                                let lane_mean_speed =
                                    speed_sum[offset] / vehicle_count[offset] as f32;
                                let observed_speed = lane_mean_speed.clamp(1.0, edge.speed_limit);
                                let free_flow_lane_time = lane.length / edge.speed_limit;
                                let observed_lane_time = lane.length / observed_speed;
                                (observed_lane_time - free_flow_lane_time).clamp(0.0, 30.0)
                            };
                            smoothing_retain * lane.live_frontage_delay_s
                                + smoothing_gain * raw_lane_delay_penalty_s
                        }
                    };
                }
            });
        #[cfg(test)]
        super::contract::add(super::contract::Phase::FrontageLanes, lane_count);
    }
}
