//! Main simulation loop for agents: transit state machine and movement.

mod access;
mod claims;
mod frontage;
mod lane_buckets;
mod lane_nav;
mod movement;
mod movement_pass;
mod planning;
mod runtime;
mod schedule;
mod scrub;
mod slices;
mod speed;
mod traffic;

use super::data::AgentSystem;
use super::{
    MODE_CAR, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IMMIGRATING,
    TRANSIT_IN_BUILDING, TRANSIT_INTERSECTION, TRANSIT_NETWORK, transit_is_visible,
};
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::traffic_log;

const TRAFFIC_DEBUG_STATIONARY_EPS_M: f32 = 0.02;
const TRAFFIC_DEBUG_STATIONARY_AFTER_S: f32 = 3.0;
const TRAFFIC_DEBUG_STATIONARY_LOG_INTERVAL_S: f32 = 5.0;

pub(crate) use planning::{
    BuiltTripPlan, building_origin_trip_is_feasible, estimate_building_origin_trip_minutes,
    plan_building_origin_trip, plan_building_to_border_trip, plan_immigration_trip,
};

impl AgentSystem {
    /// Advances the agent simulation by `delta` seconds.
    pub fn tick(
        &mut self,
        allocator: &BuildingAllocator,
        transit_network: &mut TransitNetwork,
        graph: &mut RegionGraph,
        delta: f32,
        day_index: u32,
        minute_of_day: u16,
    ) {
        self.sim_time += delta;
        let n = self.agents.len();
        if n == 0 {
            self.update_frontage_delay_cache(transit_network, graph, delta);
            return;
        }

        let building_ref_revision = allocator.building_ref_revision();
        if self.last_building_ref_scrub_revision != building_ref_revision {
            self.scrub_invalid_building_refs(allocator.buildings.len(), n);
            self.last_building_ref_scrub_revision = building_ref_revision;
            self.invalidate_lane_bucket_snapshot();
        }

        let (lane_count, live_lane_agent_count) =
            self.prepare_lane_buckets_for_tick(transit_network, n);
        self.update_idm_speeds(delta, transit_network, graph, n, live_lane_agent_count);

        self.dispatch_movement_pass(
            allocator,
            transit_network,
            graph,
            delta,
            day_index,
            minute_of_day,
            n,
        );
        self.log_stationary_visible_pedestrians(delta, n);

        self.rebuild_lanes_and_congestion(graph, lane_count, n);
        self.update_frontage_delay_cache(transit_network, graph, delta);
    }

    fn log_stationary_visible_pedestrians(&mut self, delta: f32, n: usize) {
        if !crate::debug::is_traffic_enabled() {
            return;
        }

        self.traffic_debug_last_pos_x.resize(n, f32::NAN);
        self.traffic_debug_last_pos_y.resize(n, f32::NAN);
        self.traffic_debug_stationary_s.resize(n, 0.0);
        self.traffic_debug_next_log_time.resize(n, 0.0);

        for i in 0..n {
            let transit = self.agents.transit[i];
            let is_visible_pedestrian =
                transit_is_visible(transit) && self.agents.transit_mode[i] != MODE_CAR;
            let x = self.agents.pos_x[i];
            let z = self.agents.pos_y[i];
            if !is_visible_pedestrian {
                self.traffic_debug_last_pos_x[i] = x;
                self.traffic_debug_last_pos_y[i] = z;
                self.traffic_debug_stationary_s[i] = 0.0;
                self.traffic_debug_next_log_time[i] = 0.0;
                continue;
            }

            let last_x = self.traffic_debug_last_pos_x[i];
            let last_z = self.traffic_debug_last_pos_y[i];
            if !last_x.is_finite() || !last_z.is_finite() {
                self.traffic_debug_last_pos_x[i] = x;
                self.traffic_debug_last_pos_y[i] = z;
                continue;
            }

            let moved = (x - last_x).hypot(z - last_z);
            self.traffic_debug_last_pos_x[i] = x;
            self.traffic_debug_last_pos_y[i] = z;

            if moved > TRAFFIC_DEBUG_STATIONARY_EPS_M {
                self.traffic_debug_stationary_s[i] = 0.0;
                self.traffic_debug_next_log_time[i] = 0.0;
                continue;
            }

            let stationary_s = self.traffic_debug_stationary_s[i] + delta.max(0.0);
            self.traffic_debug_stationary_s[i] = stationary_s;
            if stationary_s < TRAFFIC_DEBUG_STATIONARY_AFTER_S
                || self.sim_time < self.traffic_debug_next_log_time[i]
            {
                continue;
            }

            self.traffic_debug_next_log_time[i] =
                self.sim_time + TRAFFIC_DEBUG_STATIONARY_LOG_INTERVAL_S;
            traffic_log!(
                "[VISIBLE_PEDESTRIAN_STATIONARY] agent={} stationary_s={:.1} transit={} pos=({:.2},{:.2}) lane={} lane_d={:.2} node={} edge={} path_idx={}/{} current_bldg={} target_bldg={} flags=0x{:02x} next_replan={:.1}",
                i,
                stationary_s,
                transit_label(transit),
                x,
                z,
                self.agents.current_lane_id[i],
                self.agents.lane_distance[i],
                self.agents.current_node[i],
                self.agents.current_edge[i],
                self.agents.current_path_index[i],
                self.agents.current_path[i].len(),
                self.agents.current_building[i],
                self.agents.target_building[i],
                self.agents.access_flags[i],
                self.agents.next_replan_time[i],
            );
        }
    }
}

fn transit_label(transit: u8) -> &'static str {
    match transit {
        TRANSIT_IN_BUILDING => "in_building",
        TRANSIT_ACCESS_EGRESS => "access_egress",
        TRANSIT_NETWORK => "network",
        TRANSIT_ACCESS_INGRESS => "access_ingress",
        TRANSIT_IMMIGRATING => "immigrating",
        TRANSIT_INTERSECTION => "intersection",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::runtime::dispatch_agents;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Verifies that `dispatch_agents` visits every index in `0..n` exactly once,
    /// both below the PAR_THRESHOLD (sequential path) and above it (parallel path).
    #[test]
    fn test_dispatch_agents_visits_each_index_once() {
        for n in [10_usize, 499, 500, 501, 600] {
            let counts: Vec<AtomicUsize> = (0..n).map(|_| AtomicUsize::new(0)).collect();
            dispatch_agents(n, |i| {
                counts[i].fetch_add(1, Ordering::Relaxed);
            });
            for (i, c) in counts.iter().enumerate() {
                assert_eq!(
                    c.load(Ordering::Relaxed),
                    1,
                    "n={n}: index {i} was visited {} time(s), expected 1",
                    c.load(Ordering::Relaxed)
                );
            }
        }
    }
}
