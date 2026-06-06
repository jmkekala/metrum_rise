//! Main simulation loop for agents: transit state machine and movement.

mod access;
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
use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

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

        self.rebuild_lanes_and_congestion(graph, lane_count, n);
        self.update_frontage_delay_cache(transit_network, graph, delta);
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
