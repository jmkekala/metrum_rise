//! Retained per-lane occupancy snapshot maintenance.

use super::super::runtime::PAR_THRESHOLD;
use super::super::slices::RawSlice;
use super::super::traffic::LANE_CHANGE_FINISH_EPS_M;
use crate::simulation::economy::agents::TRANSIT_NETWORK;
use crate::simulation::economy::agents::data::AgentSystem;
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};

impl AgentSystem {
    pub(super) fn ensure_lane_bucket_buffers(&mut self, lane_count: usize) {
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
    }

    pub(super) fn reset_lane_attach_claims(&mut self, lane_count: usize) {
        if self.lane_attach_claimed.len() < lane_count {
            self.lane_attach_claimed
                .resize_with(lane_count, || AtomicBool::new(false));
        }
        for claimed in &self.lane_attach_claimed {
            claimed.store(false, Ordering::Relaxed);
        }
    }

    pub(super) fn mark_lane_bucket_snapshot_valid(
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

    pub(super) fn agent_source_lane_ghost_lid(
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

    pub(super) fn add_lane_change_source_ghosts(&mut self, lane_count: usize) {
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

    pub(super) fn clear_dirty_lane_buckets(&mut self) {
        for &lid in &self.dirty_lanes {
            self.lane_buckets[lid].clear();
            self.lane_is_dirty[lid] = false;
        }
        self.dirty_lanes.clear();
    }

    pub(super) fn push_dirty_lane_agent(
        &mut self,
        lid: usize,
        lane_distance: f32,
        agent_idx: usize,
    ) {
        if !self.lane_is_dirty[lid] {
            self.lane_is_dirty[lid] = true;
            self.dirty_lanes.push(lid);
        }
        self.lane_buckets[lid].push((lane_distance, agent_idx));
    }

    pub(super) fn sort_dirty_lane_buckets(&mut self) {
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
}
