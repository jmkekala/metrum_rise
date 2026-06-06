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
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
        self.clear_dirty_lane_buckets();

        let mut live_lane_agent_count = 0;
        for i in 0..n {
            if live_lane_bucket_transit(self.agents.transit[i]) {
                live_lane_agent_count += 1;
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    self.push_dirty_lane_agent(lid, self.agents.lane_distance[i], i);
                }
                let source_lane_id = self.agents.lane_change_from_lane_id[i];
                let source_lid = source_lane_id as usize;
                if self.agents.transit[i] == TRANSIT_NETWORK
                    && source_lane_id != u32::MAX
                    && source_lid < lane_count
                    && source_lid != lid
                    && self.agents.lane_distance[i] + LANE_CHANGE_FINISH_EPS_M
                        < self.agents.lane_change_start_d[i] + self.agents.lane_change_length_m[i]
                {
                    self.push_dirty_lane_agent(source_lid, self.agents.lane_distance[i], i);
                }
            }
        }
        self.sort_dirty_lane_buckets();

        if self.lane_attach_claimed.len() < lane_count {
            self.lane_attach_claimed
                .resize_with(lane_count, || AtomicBool::new(false));
        }
        for claimed in &self.lane_attach_claimed {
            claimed.store(false, Ordering::Relaxed);
        }

        (lane_count, live_lane_agent_count)
    }

    /// Rebuilds lane buckets after movement, fixes overlaps, then writes edge congestion.
    pub(super) fn rebuild_lanes_and_congestion(
        &mut self,
        graph: &mut RegionGraph,
        lane_count: usize,
        n: usize,
    ) {
        self.clear_dirty_lane_buckets();

        let edge_count = graph.edge_count();
        self.edge_speed_sum.clear();
        self.edge_speed_sum.resize(edge_count, 0.0_f32);
        self.edge_agent_cnt.clear();
        self.edge_agent_cnt.resize(edge_count, 0_u32);

        for i in 0..n {
            if live_lane_bucket_transit(self.agents.transit[i]) {
                let lid = self.agents.current_lane_id[i];
                if lid != usize::MAX && lid < lane_count {
                    self.push_dirty_lane_agent(lid, self.agents.lane_distance[i], i);
                }
                if self.agents.transit[i] == TRANSIT_NETWORK {
                    let eid = self.agents.current_edge[i];
                    if eid != usize::MAX && eid < edge_count {
                        self.edge_speed_sum[eid] += self.agents.speed[i];
                        self.edge_agent_cnt[eid] += 1;
                    }
                }
            }
        }

        self.sort_dirty_lane_buckets();
        self.correct_lane_overlaps();
        self.commit_edge_congestion(graph, edge_count);
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
        for eid in 0..edge_count {
            if !graph.edge(eid).deleted && self.edge_agent_cnt[eid] > 0 {
                let avg = self.edge_speed_sum[eid] / self.edge_agent_cnt[eid] as f32;
                let limit = graph.edge(eid).speed_limit.max(1.0);
                graph.set_edge_congestion(eid, (1.0 - avg / limit).max(0.0));
            }
        }
    }
}
