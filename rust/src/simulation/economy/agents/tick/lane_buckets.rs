//! Lane bucket maintenance, overlap correction, and congestion aggregation.

mod congestion;
mod occupancy;
mod overlap;

use super::super::TRANSIT_NETWORK;
use super::traffic::live_lane_bucket_transit;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;

impl AgentSystem {
    /// Builds the sorted per-lane buckets used by IDM and access handoff claims.
    pub(super) fn prepare_lane_buckets_for_tick(
        &mut self,
        transit_network: &TransitNetwork,
        n: usize,
    ) -> (usize, usize) {
        let lane_count = transit_network.lane_system.lanes.len();
        self.ensure_lane_bucket_buffers(lane_count);
        if !self.lane_bucket_snapshot_valid
            || self.lane_bucket_snapshot_lane_count != lane_count
            || self.lane_bucket_snapshot_agent_count != n
        {
            self.rebuild_lane_occupancy_snapshot(lane_count, n);
        }

        self.reset_lane_attach_claims(lane_count);

        (lane_count, self.lane_bucket_live_agent_count)
    }

    /// Rebuilds lane buckets after movement, fixes overlaps, then writes edge congestion.
    pub(super) fn rebuild_lanes_and_congestion(
        &mut self,
        graph: &mut RegionGraph,
        lane_count: usize,
        n: usize,
    ) {
        self.ensure_lane_bucket_buffers(lane_count);
        self.clear_dirty_lane_buckets();
        self.lane_change_ghost_agents.clear();

        let edge_count = graph.edge_count();
        self.ensure_edge_congestion_buffers(edge_count);
        self.clear_dirty_edge_congestion();

        let mut live_lane_agent_count = 0;
        for i in 0..n {
            if live_lane_bucket_transit(self.agents.transit[i]) {
                live_lane_agent_count += 1;
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    self.push_dirty_lane_agent(lid, self.agents.lane_distance[i], i);
                    if self
                        .agent_source_lane_ghost_lid(i, lane_count, lid)
                        .is_some()
                    {
                        self.lane_change_ghost_agents.push(i);
                    }
                }
                if self.agents.transit[i] == TRANSIT_NETWORK {
                    let eid = self.agents.current_edge[i];
                    if eid != usize::MAX && eid < edge_count {
                        self.push_dirty_edge_speed(eid, self.agents.speed[i]);
                    }
                }
            }
        }

        self.sort_dirty_lane_buckets();
        self.correct_lane_overlaps();
        self.add_lane_change_source_ghosts(lane_count);
        self.commit_edge_congestion(graph, edge_count);
        self.mark_lane_bucket_snapshot_valid(lane_count, n, live_lane_agent_count);
    }

    fn rebuild_lane_occupancy_snapshot(&mut self, lane_count: usize, n: usize) {
        self.clear_dirty_lane_buckets();
        self.lane_change_ghost_agents.clear();

        let mut live_lane_agent_count = 0;
        for i in 0..n {
            if live_lane_bucket_transit(self.agents.transit[i]) {
                live_lane_agent_count += 1;
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    self.push_dirty_lane_agent(lid, self.agents.lane_distance[i], i);
                    if self
                        .agent_source_lane_ghost_lid(i, lane_count, lid)
                        .is_some()
                    {
                        self.lane_change_ghost_agents.push(i);
                    }
                }
            }
        }

        self.sort_dirty_lane_buckets();
        self.add_lane_change_source_ghosts(lane_count);
        self.mark_lane_bucket_snapshot_valid(lane_count, n, live_lane_agent_count);
    }
}
