// SPDX-License-Identifier: GPL-2.0-only

//! Per-second live frontage delay updates used by agent access planning.

use super::super::MODE_CAR;
use rayon::prelude::*;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;

const FRONTAGE_DELAY_UPDATE_S: f32 = 1.0;

impl AgentSystem {
    /// Updates each lane's live frontage delay from aggregated live lane speeds.
    ///
    /// Runs at fixed cadence rather than every tick so planner-visible congestion stays stable
    /// and cheap to maintain. Economy prices read the hourly copy published by
    /// [`TransitNetwork::publish_frontage_prices`]. O(agents + lanes).
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
        self.lane_speed_sum.clear();
        self.lane_speed_sum.resize(lane_count, 0.0);
        self.lane_vehicle_cnt.clear();
        self.lane_vehicle_cnt.resize(lane_count, 0);

        // Serial on purpose: the per-lane f32 sums must accumulate in agent order so delays do
        // not depend on the worker count. O(agents), once per simulated second.
        for i in 0..self.agents.len() {
            if self.agents.transit_mode[i] != MODE_CAR {
                continue;
            }
            let lid = self.agents.current_lane_id[i];
            if lid == usize::MAX || lid >= lane_count {
                continue;
            }
            self.lane_speed_sum[lid] += self.agents.speed[i];
            self.lane_vehicle_cnt[lid] += 1;
        }

        let smoothing_retain = 0.75_f32.powi(update_steps);
        let smoothing_gain = 1.0 - smoothing_retain;
        let speed_sum = &self.lane_speed_sum;
        let vehicle_count = &self.lane_vehicle_cnt;
        // Lanes are independent once speeds are aggregated, so this O(lanes) pass is parallel
        // and deterministic. Economy prices read the hourly published copy instead.
        transit_network
            .lane_system
            .lanes
            .par_iter_mut()
            .enumerate()
            .for_each(|(lane_id, lane)| {
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
                        let raw_lane_delay_penalty_s = if vehicle_count[lane_id] == 0 {
                            0.0
                        } else {
                            let lane_mean_speed =
                                speed_sum[lane_id] / vehicle_count[lane_id] as f32;
                            let observed_speed = lane_mean_speed.clamp(1.0, edge.speed_limit);
                            let free_flow_lane_time = lane.length / edge.speed_limit;
                            let observed_lane_time = lane.length / observed_speed;
                            (observed_lane_time - free_flow_lane_time).clamp(0.0, 30.0)
                        };
                        smoothing_retain * lane.live_frontage_delay_s
                            + smoothing_gain * raw_lane_delay_penalty_s
                    }
                };
            });
    }
}
