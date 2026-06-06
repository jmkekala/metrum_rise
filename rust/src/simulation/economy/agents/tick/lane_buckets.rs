//! Lane bucket maintenance, overlap correction, and congestion aggregation.

use super::super::TRANSIT_NETWORK;
use super::runtime::PAR_THRESHOLD;
use super::slices::RawSlice;
use super::traffic::{LANE_CHANGE_FINISH_EPS_M, live_lane_bucket_transit};
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

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

    fn ensure_lane_bucket_buffers(&mut self, lane_count: usize) {
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
    }

    fn reset_lane_attach_claims(&mut self, lane_count: usize) {
        if self.lane_attach_claimed.len() < lane_count {
            self.lane_attach_claimed
                .resize_with(lane_count, || AtomicBool::new(false));
        }
        for claimed in &self.lane_attach_claimed {
            claimed.store(false, Ordering::Relaxed);
        }
    }

    fn mark_lane_bucket_snapshot_valid(
        &mut self,
        lane_count: usize,
        n: usize,
        live_lane_agent_count: usize,
    ) {
        self.lane_bucket_live_agent_count = live_lane_agent_count;
        self.lane_bucket_snapshot_lane_count = lane_count;
        self.lane_bucket_snapshot_agent_count = n;
        self.lane_bucket_snapshot_valid = true;
    }

    fn agent_source_lane_ghost_lid(
        &self,
        agent_idx: usize,
        lane_count: usize,
        current_lid: usize,
    ) -> Option<usize> {
        let source_lane_id = self.agents.lane_change_from_lane_id[agent_idx];
        let source_lid = source_lane_id as usize;
        if self.agents.transit[agent_idx] == TRANSIT_NETWORK
            && source_lane_id != u32::MAX
            && source_lid < lane_count
            && source_lid != current_lid
            && self.agents.lane_distance[agent_idx] + LANE_CHANGE_FINISH_EPS_M
                < self.agents.lane_change_start_d[agent_idx]
                    + self.agents.lane_change_length_m[agent_idx]
        {
            Some(source_lid)
        } else {
            None
        }
    }

    fn add_lane_change_source_ghosts(&mut self, lane_count: usize) {
        if self.lane_change_ghost_agents.is_empty() {
            return;
        }
        let ghost_agent_count = self.lane_change_ghost_agents.len();
        for ghost_idx in 0..ghost_agent_count {
            let agent_idx = self.lane_change_ghost_agents[ghost_idx];
            let current_lid = self.agents.current_lane_id[agent_idx];
            if let Some(source_lid) =
                self.agent_source_lane_ghost_lid(agent_idx, lane_count, current_lid)
            {
                self.push_dirty_lane_agent(
                    source_lid,
                    self.agents.lane_distance[agent_idx],
                    agent_idx,
                );
            }
        }
        self.sort_dirty_lane_buckets();
    }

    fn ensure_edge_congestion_buffers(&mut self, edge_count: usize) {
        if self.edge_speed_sum.len() < edge_count {
            self.edge_speed_sum.resize(edge_count, 0.0_f32);
            self.edge_agent_cnt.resize(edge_count, 0_u32);
            self.edge_is_dirty.resize(edge_count, false);
        }
    }

    fn clear_dirty_edge_congestion(&mut self) {
        self.stale_dirty_edges.clear();
        for &eid in &self.dirty_edges {
            if eid < self.edge_speed_sum.len() {
                self.edge_speed_sum[eid] = 0.0;
                self.edge_agent_cnt[eid] = 0;
                self.edge_is_dirty[eid] = false;
                self.stale_dirty_edges.push(eid);
            }
        }
        self.dirty_edges.clear();
    }

    fn push_dirty_edge_speed(&mut self, eid: usize, speed: f32) {
        if !self.edge_is_dirty[eid] {
            self.edge_is_dirty[eid] = true;
            self.dirty_edges.push(eid);
        }
        self.edge_speed_sum[eid] += speed;
        self.edge_agent_cnt[eid] += 1;
    }

    fn clear_dirty_lane_buckets(&mut self) {
        for &lid in &self.dirty_lanes {
            self.lane_buckets[lid].clear();
            self.lane_is_dirty[lid] = false;
        }
        self.dirty_lanes.clear();
    }

    fn push_dirty_lane_agent(&mut self, lid: usize, lane_distance: f32, agent_idx: usize) {
        if !self.lane_is_dirty[lid] {
            self.lane_is_dirty[lid] = true;
            self.dirty_lanes.push(lid);
        }
        self.lane_buckets[lid].push((lane_distance, agent_idx));
    }

    fn sort_dirty_lane_buckets(&mut self) {
        let buckets_raw = RawSlice::new(&mut self.lane_buckets);
        if self.dirty_lanes.len() >= PAR_THRESHOLD {
            self.dirty_lanes.par_iter().for_each(|&lid| {
                let bucket = unsafe { buckets_raw.get_mut(lid) };
                bucket.sort_unstable_by(|a, b| {
                    a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                });
            });
        } else {
            for &lid in &self.dirty_lanes {
                self.lane_buckets[lid].sort_unstable_by(|a, b| {
                    a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
        }
    }

    fn correct_lane_overlaps(&mut self) {
        let min_sep = CAR_LENGTH + IDM_S_MIN;
        for &lid in &self.dirty_lanes {
            let bucket = &mut self.lane_buckets[lid];
            for j in (0..bucket.len().saturating_sub(1)).rev() {
                let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
                if bucket[j].0 > max_rear {
                    bucket[j].0 = max_rear;
                    self.agents.lane_distance[bucket[j].1] = max_rear;
                }
            }
        }
    }

    fn commit_edge_congestion(&mut self, graph: &mut RegionGraph, edge_count: usize) {
        for &eid in &self.stale_dirty_edges {
            if eid < edge_count && !self.edge_is_dirty[eid] && !graph.edge(eid).deleted {
                graph.set_edge_congestion(eid, 0.0);
            }
        }
        for &eid in &self.dirty_edges {
            if eid >= edge_count {
                continue;
            }
            if !graph.edge(eid).deleted && self.edge_agent_cnt[eid] > 0 {
                let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                let limit = graph.edge(eid).speed_limit.max(1.0);
                graph.set_edge_congestion(eid, (1.0 - avg / limit).max(0.0));
            }
        }
    }
}
