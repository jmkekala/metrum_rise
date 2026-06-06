//! Low-frequency frontage delay cache updates used by exact access planning.

use super::super::MODE_CAR;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::LaneType;

const FRONTAGE_DELAY_UPDATE_S: f32 = 1.0;

impl AgentSystem {
    /// Updates the low-frequency per-lane frontage delay cache from aggregated live lane speeds.
    ///
    /// Runs at fixed cadence rather than every tick so planner-visible congestion stays stable
    /// and cheap to maintain.
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
        for (lane_id, lane) in transit_network.lane_system.lanes.iter_mut().enumerate() {
            if lane.lane_type != LaneType::Vehicle {
                lane.frontage_delay_penalty_s = 0.0;
                continue;
            }
            if lane.edge_id == usize::MAX || lane.edge_id >= graph.edge_count() {
                lane.frontage_delay_penalty_s = 0.0;
                continue;
            }

            let edge = graph.edge(lane.edge_id);
            if edge.speed_limit <= 1e-6 || lane.length <= 1e-6 {
                lane.frontage_delay_penalty_s = 0.0;
                continue;
            }
            let raw_lane_delay_penalty_s = if self.lane_vehicle_cnt[lane_id] == 0 {
                0.0
            } else {
                let lane_mean_speed =
                    self.lane_speed_sum[lane_id] / self.lane_vehicle_cnt[lane_id] as f32;
                let observed_speed = lane_mean_speed.clamp(1.0, edge.speed_limit);
                let free_flow_lane_time = lane.length / edge.speed_limit;
                let observed_lane_time = lane.length / observed_speed;
                (observed_lane_time - free_flow_lane_time).clamp(0.0, 30.0)
            };
            lane.frontage_delay_penalty_s = smoothing_retain * lane.frontage_delay_penalty_s
                + smoothing_gain * raw_lane_delay_penalty_s;
        }
    }
}
