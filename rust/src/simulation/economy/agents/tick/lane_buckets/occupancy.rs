// SPDX-License-Identifier: GPL-2.0-only

//! Retained per-lane occupancy snapshot maintenance.
//!
//! The snapshot is a function of the agents' lane state: each live lane agent's
//! `(lane_distance, index)` in its current lane's bucket, plus a source-lane ghost while a lane
//! change is under way, every bucket sorted by distance with an index tiebreak. A fill rebuilds
//! it from every agent on the shared pool; between fills it follows appends and swap-removes
//! entry by entry (`SIM-02-K` part 4).

use super::super::runtime::PAR_THRESHOLD;
use super::super::traffic::{LANE_CHANGE_FINISH_EPS_M, live_lane_bucket_transit};
use crate::config::{CAR_LENGTH, IDM_S_MIN};
use crate::simulation::economy::agents::TRANSIT_NETWORK;
use crate::simulation::economy::agents::data::AgentSystem;
use rayon::prelude::*;
use std::cmp::Ordering as CmpOrdering;
use std::sync::atomic::{AtomicUsize, Ordering};

// Agents per grouping chunk of the parallel fill, and lane owners per chunk of the claim reset.
const KEY_CHUNK: usize = 4096;
// Lanes from which the claim reset runs on the pool.
const PAR_CLAIM_RESET_LANES: usize = 65_536;

/// Retained scratch of the parallel lane-bucket fill; derived state, never saved.
#[derive(Default)]
pub(crate) struct LaneBucketScratch {
    // Each agent chunk's lane agents grouped by lane range, `groups[chunk * ranges + range]`,
    // in agent order.
    groups: Vec<Vec<u32>>,
    // One per contiguous lane range of the fill.
    ranges: Vec<LaneRange>,
}

#[derive(Default)]
struct LaneRange {
    // Lanes of this range holding entries, ascending.
    dirty: Vec<usize>,
    // Overlap corrections `(agent, lane_distance)` to write back to the agents.
    corrections: Vec<(usize, f32)>,
    // Agents of this range's lanes with a lane change recorded, in agent order.
    ghosts: Vec<usize>,
}

// The agent columns a fill reads.
struct FillInput<'a> {
    groups: &'a [Vec<u32>],
    chunk_count: usize,
    range_count: usize,
    lanes: &'a [usize],
    distances: &'a [f32],
    transit: &'a [u8],
    change_from: &'a [u32],
    correct_overlaps: bool,
}

impl AgentSystem {
    pub(super) fn ensure_lane_bucket_buffers(&mut self, lane_count: usize) {
        if self.lane_buckets.len() < lane_count {
            self.lane_buckets.resize_with(lane_count, Vec::new);
            self.lane_is_dirty.resize(lane_count, false);
        }
        // Tests may install buckets directly; the fill clears by these flags.
        if self.lane_is_dirty.len() < self.lane_buckets.len() {
            self.lane_is_dirty.resize(self.lane_buckets.len(), false);
        }
    }

    pub(super) fn reset_lane_claims(&mut self, lane_count: usize) {
        if self.lane_claim_owner.len() < lane_count {
            self.lane_claim_owner
                .resize_with(lane_count, || AtomicUsize::new(usize::MAX));
        }
        let reset = |owners: &[AtomicUsize]| {
            for owner in owners {
                owner.store(usize::MAX, Ordering::Relaxed);
            }
        };
        if self.lane_claim_owner.len() >= PAR_CLAIM_RESET_LANES {
            self.lane_claim_owner.par_chunks(KEY_CHUNK).for_each(reset);
        } else {
            reset(&self.lane_claim_owner);
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

    /// The lanes holding agent `agent_idx`'s entries in a snapshot of its current state: its
    /// current lane and, during a lane change, the source lane's ghost.
    pub(super) fn agent_bucket_lanes(
        &self,
        agent_idx: usize,
        lane_count: usize,
    ) -> [Option<usize>; 2] {
        if !live_lane_bucket_transit(self.agents.transit[agent_idx]) {
            return [None, None];
        }
        let lid = self.agents.current_lane_id[agent_idx];
        if lid >= lane_count {
            return [None, None];
        }
        [
            Some(lid),
            self.agent_source_lane_ghost_lid(agent_idx, lane_count, lid),
        ]
    }

    /// Inserts one entry at its sorted place, marking the lane dirty. O(bucket).
    pub(super) fn insert_lane_entry(&mut self, lid: usize, lane_distance: f32, agent_idx: usize) {
        if !self.lane_is_dirty[lid] {
            self.lane_is_dirty[lid] = true;
            self.dirty_lanes.push(lid);
        }
        let entry = (lane_distance, agent_idx);
        let bucket = &mut self.lane_buckets[lid];
        let at = bucket.partition_point(|other| lane_bucket_order(other, &entry).is_lt());
        bucket.insert(at, entry);
    }

    /// Removes agent `agent_idx`'s entry from lane `lid` and returns its distance. O(bucket).
    pub(super) fn remove_lane_entry(&mut self, lid: usize, agent_idx: usize) -> Option<f32> {
        let bucket = self.lane_buckets.get_mut(lid)?;
        let at = bucket.iter().position(|entry| entry.1 == agent_idx)?;
        Some(bucket.remove(at).0)
    }

    /// Rebuilds every bucket from the live lane agents `0..n`: clears the previous entries,
    /// fills and sorts each lane, then, with `correct_overlaps`, moves overlapping cars back
    /// (the rear car of each pair to the leader's distance less a car length and the minimum
    /// gap, from the front) before adding the source-lane ghosts of lane changes still under
    /// way. Returns the live lane-agent count.
    ///
    /// Agents and lanes are split into contiguous ranges on the shared pool, so the result is
    /// independent of the worker count: one pass over agent chunks groups each chunk's lane
    /// agents by lane range, then each lane range takes its groups chunk by chunk, in agent
    /// order. O(n + L) work, spread over the workers; ghosts and overlap write-backs are serial
    /// and O(their count).
    pub(super) fn fill_lane_buckets(
        &mut self,
        lane_count: usize,
        n: usize,
        correct_overlaps: bool,
    ) -> usize {
        debug_assert!(n < u32::MAX as usize);
        let parallel = n >= PAR_THRESHOLD;
        let bucket_len = self.lane_buckets.len();
        let range_count = if parallel {
            2 * rayon::current_num_threads()
        } else {
            1
        };
        let range_len = bucket_len.div_ceil(range_count).max(1);
        let range_count = bucket_len.div_ceil(range_len).max(1);
        let chunk_count = n.div_ceil(KEY_CHUNK);
        let scratch = &mut self.lane_bucket_scratch;
        if scratch.ranges.len() < range_count {
            scratch.ranges.resize_with(range_count, LaneRange::default);
        }
        if scratch.groups.len() < chunk_count * range_count {
            scratch
                .groups
                .resize_with(chunk_count * range_count, Vec::new);
        }
        let groups = &mut scratch.groups[..chunk_count * range_count];
        let transit = &self.agents.transit[..n];
        let lanes = &self.agents.current_lane_id[..n];
        let live = if parallel {
            groups
                .par_chunks_mut(range_count)
                .zip(transit.par_chunks(KEY_CHUNK))
                .zip(lanes.par_chunks(KEY_CHUNK))
                .enumerate()
                .map(|(chunk, ((out, transit), lanes))| {
                    group_lane_agents(chunk, out, transit, lanes, lane_count, range_len)
                })
                .sum()
        } else {
            groups
                .chunks_mut(range_count)
                .zip(transit.chunks(KEY_CHUNK))
                .zip(lanes.chunks(KEY_CHUNK))
                .enumerate()
                .map(|(chunk, ((out, transit), lanes))| {
                    group_lane_agents(chunk, out, transit, lanes, lane_count, range_len)
                })
                .sum()
        };

        let input = FillInput {
            groups: &scratch.groups[..chunk_count * range_count],
            chunk_count,
            range_count,
            lanes,
            distances: &self.agents.lane_distance[..n],
            transit,
            change_from: &self.agents.lane_change_from_lane_id[..n],
            correct_overlaps,
        };
        let ranges = &mut scratch.ranges[..range_count];
        if parallel {
            self.lane_buckets
                .par_chunks_mut(range_len)
                .zip(self.lane_is_dirty.par_chunks_mut(range_len))
                .zip(ranges.par_iter_mut())
                .enumerate()
                .for_each(|(r, ((buckets, flags), range))| {
                    fill_lane_range(r, r * range_len, buckets, flags, range, &input);
                });
        } else {
            for (r, ((buckets, flags), range)) in self
                .lane_buckets
                .chunks_mut(range_len)
                .zip(self.lane_is_dirty.chunks_mut(range_len))
                .zip(ranges.iter_mut())
                .enumerate()
            {
                fill_lane_range(r, r * range_len, buckets, flags, range, &input);
            }
        }

        self.dirty_lanes.clear();
        self.lane_change_ghost_agents.clear();
        for range in &scratch.ranges[..range_count] {
            self.dirty_lanes.extend_from_slice(&range.dirty);
            self.lane_change_ghost_agents
                .extend_from_slice(&range.ghosts);
            for &(agent, lane_distance) in &range.corrections {
                self.agents.lane_distance[agent] = lane_distance;
            }
        }
        self.add_lane_change_source_ghosts(lane_count);
        live
    }

    // Ghost entries read the corrected distances. Candidates come range by range; sorting
    // them gives the dirty-lane order of agent order whatever the range layout.
    fn add_lane_change_source_ghosts(&mut self, lane_count: usize) {
        if self.lane_change_ghost_agents.is_empty() {
            return;
        }
        self.lane_change_ghost_agents.sort_unstable();
        for ghost_idx in 0..self.lane_change_ghost_agents.len() {
            let agent_idx = self.lane_change_ghost_agents[ghost_idx];
            let current_lid = self.agents.current_lane_id[agent_idx];
            if let Some(source_lid) =
                self.agent_source_lane_ghost_lid(agent_idx, lane_count, current_lid)
            {
                self.insert_lane_entry(source_lid, self.agents.lane_distance[agent_idx], agent_idx);
            }
        }
    }
}

// Groups the lane agents of agent chunk `chunk` by lane range; returns the chunk's live
// lane-agent count.
fn group_lane_agents(
    chunk: usize,
    out: &mut [Vec<u32>],
    transit: &[u8],
    lanes: &[usize],
    lane_count: usize,
    range_len: usize,
) -> usize {
    for group in out.iter_mut() {
        group.clear();
    }
    let mut live = 0;
    for (offset, (&transit, &lid)) in transit.iter().zip(lanes).enumerate() {
        if live_lane_bucket_transit(transit) {
            live += 1;
            if lid < lane_count {
                out[lid / range_len].push((chunk * KEY_CHUNK + offset) as u32);
            }
        }
    }
    live
}

// Clears and refills the buckets of range `r`, lanes `start..start + buckets.len()`, in agent
// order.
fn fill_lane_range(
    r: usize,
    start: usize,
    buckets: &mut [Vec<(f32, usize)>],
    flags: &mut [bool],
    range: &mut LaneRange,
    input: &FillInput<'_>,
) {
    for (bucket, flag) in buckets.iter_mut().zip(flags.iter_mut()) {
        if *flag {
            bucket.clear();
            *flag = false;
        }
    }
    range.dirty.clear();
    range.corrections.clear();
    range.ghosts.clear();
    for chunk in 0..input.chunk_count {
        for &agent in &input.groups[chunk * input.range_count + r] {
            let agent = agent as usize;
            let lid = input.lanes[agent];
            let local = lid - start;
            if !flags[local] {
                flags[local] = true;
                range.dirty.push(lid);
            }
            buckets[local].push((input.distances[agent], agent));
            if input.change_from[agent] != u32::MAX && input.transit[agent] == TRANSIT_NETWORK {
                range.ghosts.push(agent);
            }
        }
    }
    range.dirty.sort_unstable();
    let min_sep = CAR_LENGTH + IDM_S_MIN;
    for &lid in &range.dirty {
        let bucket = &mut buckets[lid - start];
        bucket.sort_unstable_by(lane_bucket_order);
        if !input.correct_overlaps {
            continue;
        }
        let corrected = range.corrections.len();
        for j in (0..bucket.len().saturating_sub(1)).rev() {
            let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
            if bucket[j].0 > max_rear {
                bucket[j].0 = max_rear;
                range.corrections.push((bucket[j].1, max_rear));
            }
        }
        // Clamping at the lane start can tie a rear car with a leader of lower index; sorting
        // again keeps the bucket the function of the agents' state that a rebuild gives.
        if range.corrections.len() > corrected {
            bucket.sort_unstable_by(lane_bucket_order);
        }
    }
}

pub(super) fn lane_bucket_order(a: &(f32, usize), b: &(f32, usize)) -> CmpOrdering {
    a.0.partial_cmp(&b.0)
        .unwrap_or(CmpOrdering::Equal)
        .then_with(|| a.1.cmp(&b.1))
}

#[cfg(test)]
mod tests {
    use super::lane_bucket_order;
    use std::cmp::Ordering;

    #[test]
    fn lane_bucket_order_uses_agent_index_tiebreak() {
        let mut bucket = vec![(12.0, 3), (12.0, 1), (8.0, 2)];
        bucket.sort_unstable_by(lane_bucket_order);
        assert_eq!(bucket, vec![(8.0, 2), (12.0, 1), (12.0, 3)]);
        assert_eq!(lane_bucket_order(&(4.0, 7), &(4.0, 7)), Ordering::Equal);
    }
}
