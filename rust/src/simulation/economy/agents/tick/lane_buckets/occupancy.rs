// SPDX-License-Identifier: GPL-3.0-or-later

//! Retained per-lane occupancy snapshot maintenance.
//!
//! The snapshot is a function of the agents' lane state: each live lane agent's
//! `(lane_distance, index, half_length)` in its current lane's bucket (`LaneEntry`), except a walker standing with no route
//! (`lane_bucket_member`, `TRAFFIC-37`), plus a source-lane ghost while a lane
//! change is under way, plus the entries of an agent off every lane (`off_lane_bucket_entries`):
//! an access agent crossing a lane in that lane's bucket at its `lane_distance` (`PARK-32`), and
//! a vehicle holding its place to join a lane, a zero-length stop line (`TRAFFIC-45`), every
//! bucket sorted by distance with an index tiebreak. A fill rebuilds
//! it from every agent on the shared pool; between fills it follows appends and swap-removes
//! entry by entry (`SIM-02-K` part 4).

use super::super::runtime::PAR_THRESHOLD;
use super::super::access::CROSSING_GIVE_UP_S;
use super::super::traffic::{
    LANE_CHANGE_FINISH_EPS_M, OffLaneRow, lane_bucket_member, lane_half_length,
    live_lane_bucket_transit, off_lane_bucket_entries,
};
use crate::config::IDM_S_MIN;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::economy::agents::walker_crowd::WALKER_STANDING_MS;
use crate::simulation::economy::agents::{LaneEntry, MODE_CAR, TRANSIT_NETWORK};
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

// The agent columns that decide an agent's entries, whole (indexed by agent).
#[derive(Clone, Copy)]
struct EntryColumns<'a> {
    transit: &'a [u8],
    lanes: &'a [usize],
    distances: &'a [f32],
    modes: &'a [u8],
    vehicle_types: &'a [u8],
    flags: &'a [u8],
    crossing: &'a [u32],
    waits: &'a [f32],
    attach_lanes: &'a [u32],
    attach_ds: &'a [f32],
}

impl EntryColumns<'_> {
    // Whether agent `i` is on lane `lid` and holds its body there (lane-change ghosts aside).
    #[inline(always)]
    fn on_lane(&self, i: usize, lane_count: usize) -> bool {
        live_lane_bucket_transit(self.transit[i])
            && self.lanes[i] < lane_count
            && lane_bucket_member(self.modes[i], self.flags[i])
    }

    // The entries of agent `i` off every lane; two column reads for one with none (most of them).
    #[inline(always)]
    fn off_lane(&self, i: usize) -> [Option<(usize, f32, f32)>; 2] {
        if self.crossing[i] == u32::MAX && self.waits[i] < CROSSING_GIVE_UP_S {
            return [None, None];
        }
        off_lane_bucket_entries(OffLaneRow {
            transit: self.transit[i],
            mode: self.modes[i],
            vehicle_type: self.vehicle_types[i],
            flags: self.flags[i],
            lane: self.lanes[i],
            distance: self.distances[i],
            crossing_lane: self.crossing[i],
            wait_s: self.waits[i],
            attach_lane: self.attach_lanes[i],
            attach_d: self.attach_ds[i],
        })
    }
}

// The agent columns a fill reads.
struct FillInput<'a> {
    groups: &'a [Vec<u32>],
    chunk_count: usize,
    range_count: usize,
    columns: EntryColumns<'a>,
    change_from: &'a [u32],
    speeds: &'a [f32],
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
            let owners = &self.lane_claim_owner;
            crate::thread_pools::install(|| owners.par_chunks(KEY_CHUNK).for_each(reset));
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

    // The columns that decide every agent's entries.
    fn entry_columns(&self) -> EntryColumns<'_> {
        let a = &self.agents;
        EntryColumns {
            transit: &a.transit,
            lanes: &a.current_lane_id,
            distances: &a.lane_distance,
            modes: &a.transit_mode,
            vehicle_types: &a.vehicle_type,
            flags: &a.access_flags,
            crossing: &a.crossing_lane,
            waits: &a.crossing_wait_s,
            attach_lanes: &a.planned_attach_lane_id,
            attach_ds: &a.planned_attach_lane_d,
        }
    }

    /// Agent `agent_idx`'s entries, `(lane, distance, half length)`, in a snapshot of its current
    /// state: its body in its current lane and, during a lane change, the source lane's ghost;
    /// none for a walker standing with no route ([`lane_bucket_member`]); off every lane, a
    /// crossing and a join hold ([`off_lane_bucket_entries`]).
    pub(super) fn agent_bucket_entries(
        &self,
        agent_idx: usize,
        lane_count: usize,
    ) -> [Option<(usize, f32, f32)>; 2] {
        let columns = self.entry_columns();
        if columns.on_lane(agent_idx, lane_count) {
            let lid = self.agents.current_lane_id[agent_idx];
            let entry = |lane| (lane, self.agents.lane_distance[agent_idx], self.lane_half_length_of(agent_idx));
            return [
                Some(entry(lid)),
                self.agent_source_lane_ghost_lid(agent_idx, lane_count, lid).map(entry),
            ];
        }
        if live_lane_bucket_transit(self.agents.transit[agent_idx])
            && self.agents.current_lane_id[agent_idx] != usize::MAX
        {
            return [None, None];
        }
        columns
            .off_lane(agent_idx)
            .map(|entry| entry.filter(|&(lane, _, _)| lane < lane_count))
    }

    /// Half the length agent `agent_idx`'s body takes in a lane bucket ([`lane_half_length`]).
    /// O(1), two column reads.
    #[inline(always)]
    pub(in crate::simulation::economy::agents) fn lane_half_length_of(
        &self,
        agent_idx: usize,
    ) -> f32 {
        lane_half_length(
            self.agents.transit_mode[agent_idx],
            self.agents.vehicle_type[agent_idx],
        )
    }

    /// Inserts one entry at its sorted place, marking the lane dirty. O(bucket).
    pub(super) fn insert_lane_entry(&mut self, lid: usize, entry: LaneEntry) {
        if !self.lane_is_dirty[lid] {
            self.lane_is_dirty[lid] = true;
            self.dirty_lanes.push(lid);
        }
        let bucket = &mut self.lane_buckets[lid];
        let at = bucket.partition_point(|other| lane_bucket_order(other, &entry).is_lt());
        bucket.insert(at, entry);
    }

    /// Removes agent `agent_idx`'s entry from lane `lid` and returns it. O(bucket).
    pub(super) fn remove_lane_entry(&mut self, lid: usize, agent_idx: usize) -> Option<LaneEntry> {
        let bucket = self.lane_buckets.get_mut(lid)?;
        let at = bucket.iter().position(|entry| entry.1 == agent_idx)?;
        Some(bucket.remove(at))
    }

    /// Rebuilds every bucket from the live lane agents `0..n`: clears the previous entries,
    /// fills and sorts each lane, then, with `correct_overlaps`, moves overlapping bodies back
    /// (the rear body of each pair to the leader's distance less both half lengths and the
    /// minimum gap, from the front) before adding the source-lane ghosts of lane changes still under
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
            2 * crate::thread_pools::install(rayon::current_num_threads)
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
        let a = &self.agents;
        let columns = EntryColumns {
            transit: &a.transit[..n],
            lanes: &a.current_lane_id[..n],
            distances: &a.lane_distance[..n],
            modes: &a.transit_mode[..n],
            vehicle_types: &a.vehicle_type[..n],
            flags: &a.access_flags[..n],
            crossing: &a.crossing_lane[..n],
            waits: &a.crossing_wait_s[..n],
            attach_lanes: &a.planned_attach_lane_id[..n],
            attach_ds: &a.planned_attach_lane_d[..n],
        };
        let group = |(chunk, out): (usize, &mut [Vec<u32>])| {
            group_lane_agents(chunk, out, &columns, lane_count, range_len)
        };
        let live = if parallel {
            crate::thread_pools::install(|| {
                groups
                    .par_chunks_mut(range_count)
                    .enumerate()
                    .map(group)
                    .sum()
            })
        } else {
            groups.chunks_mut(range_count).enumerate().map(group).sum()
        };

        let input = FillInput {
            groups: &scratch.groups[..chunk_count * range_count],
            chunk_count,
            range_count,
            columns,
            change_from: &self.agents.lane_change_from_lane_id[..n],
            speeds: &self.agents.speed[..n],
            correct_overlaps,
        };
        let ranges = &mut scratch.ranges[..range_count];
        if parallel {
            let (buckets, dirty) = (&mut self.lane_buckets, &mut self.lane_is_dirty);
            crate::thread_pools::install(|| {
                buckets
                    .par_chunks_mut(range_len)
                    .zip(dirty.par_chunks_mut(range_len))
                    .zip(ranges.par_iter_mut())
                    .enumerate()
                    .for_each(|(r, ((buckets, flags), range))| {
                        fill_lane_range(r, r * range_len, buckets, flags, range, &input);
                    })
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
                let entry = (
                    self.agents.lane_distance[agent_idx],
                    agent_idx,
                    self.lane_half_length_of(agent_idx),
                );
                self.insert_lane_entry(source_lid, entry);
            }
        }
    }
}

// Groups the bucket members among the agents of agent chunk `chunk` by lane range, once per
// range they hold an entry in; returns the chunk's live lane-agent count, members or not.
fn group_lane_agents(
    chunk: usize,
    out: &mut [Vec<u32>],
    columns: &EntryColumns<'_>,
    lane_count: usize,
    range_len: usize,
) -> usize {
    for group in out.iter_mut() {
        group.clear();
    }
    let start = chunk * KEY_CHUNK;
    let end = (start + KEY_CHUNK).min(columns.transit.len());
    let mut live = 0;
    for (offset, (&transit, &lid)) in columns.transit[start..end]
        .iter()
        .zip(&columns.lanes[start..end])
        .enumerate()
    {
        let agent = start + offset;
        let is_live = live_lane_bucket_transit(transit);
        if is_live {
            live += 1;
            if lid < lane_count
                && lane_bucket_member(columns.modes[agent], columns.flags[agent])
            {
                out[lid / range_len].push(agent as u32);
                continue;
            }
            if lid != usize::MAX {
                continue;
            }
        }
        // Off every lane: a crossing, a join hold, or both, in one range or two.
        let mut last_range = usize::MAX;
        for (lane, _, _) in columns.off_lane(agent).into_iter().flatten() {
            if lane < lane_count && lane / range_len != last_range {
                last_range = lane / range_len;
                out[last_range].push(agent as u32);
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
    buckets: &mut [Vec<LaneEntry>],
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
    let columns = &input.columns;
    let end = start + buckets.len();
    let push = |lid: usize,
                entry: LaneEntry,
                buckets: &mut [Vec<LaneEntry>],
                flags: &mut [bool],
                dirty: &mut Vec<usize>| {
        let local = lid - start;
        if !flags[local] {
            flags[local] = true;
            dirty.push(lid);
        }
        buckets[local].push(entry);
    };
    for chunk in 0..input.chunk_count {
        for &agent in &input.groups[chunk * input.range_count + r] {
            let agent = agent as usize;
            let lid = columns.lanes[agent];
            if live_lane_bucket_transit(columns.transit[agent]) && lid != usize::MAX {
                // The body's half length, read at the index whose distance the fill already
                // reads.
                let half = lane_half_length(columns.modes[agent], columns.vehicle_types[agent]);
                push(lid, (columns.distances[agent], agent, half), buckets, flags, &mut range.dirty);
                if input.change_from[agent] != u32::MAX
                    && columns.transit[agent] == TRANSIT_NETWORK
                {
                    range.ghosts.push(agent);
                }
                continue;
            }
            for (lane, d, half) in columns.off_lane(agent).into_iter().flatten() {
                if (start..end).contains(&lane) {
                    push(lane, (d, agent, half), buckets, flags, &mut range.dirty);
                }
            }
        }
    }
    range.dirty.sort_unstable();
    for &lid in &range.dirty {
        let bucket = &mut buckets[lid - start];
        bucket.sort_unstable_by(lane_bucket_order);
        if !input.correct_overlaps {
            continue;
        }
        let corrected = range.corrections.len();
        // Whether the body ahead was moved back by this pass: it stands in a queue.
        let mut lead_held = false;
        for j in (0..bucket.len().saturating_sub(1)).rev() {
            let (rear, lead) = (bucket[j].1, bucket[j + 1].1);
            // A crossing vehicle or a join hold is not on the lane it holds: nothing moves it
            // back.
            if !live_lane_bucket_transit(columns.transit[rear])
                || columns.lanes[rear] == usize::MAX
            {
                lead_held = false;
                continue;
            }
            // Walkers walk through each other's lane distance to pass (`walker_crowd`); they
            // queue, bodies the minimum gap apart, only behind one standing still or held in
            // the queue ahead of it, whose speed is still the step it took before the hold.
            if columns.modes[rear] != MODE_CAR
                && columns.modes[lead] != MODE_CAR
                && !lead_held
                && input.speeds[lead] >= WALKER_STANDING_MS
            {
                continue;
            }
            let min_sep = bucket[j + 1].2 + bucket[j].2 + IDM_S_MIN;
            let max_rear = (bucket[j + 1].0 - min_sep).max(0.0);
            lead_held = bucket[j].0 > max_rear;
            if lead_held {
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

pub(super) fn lane_bucket_order(a: &LaneEntry, b: &LaneEntry) -> CmpOrdering {
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
        let mut bucket = vec![(12.0, 3, 2.25), (12.0, 1, 4.0), (8.0, 2, 2.25)];
        bucket.sort_unstable_by(lane_bucket_order);
        assert_eq!(
            bucket,
            vec![(8.0, 2, 2.25), (12.0, 1, 4.0), (12.0, 3, 2.25)]
        );
        assert_eq!(
            lane_bucket_order(&(4.0, 7, 2.25), &(4.0, 7, 2.25)),
            Ordering::Equal
        );
    }

    #[test]
    fn walkers_queue_behind_one_standing_and_walk_through_moving_ones() {
        use crate::config::{IDM_S_MIN, WALKER_BODY_LENGTH_M};
        use crate::simulation::economy::agents::data::AgentSystem;
        use crate::simulation::economy::agents::{ACCESS_PLAN_VALID, MODE_WALK, TRANSIT_NETWORK};

        // Walker 0 at 50 m, the others closing in on it from inside its body.
        let walkers = |lead_speed: f32| {
            let mut agents = AgentSystem::new();
            for (d, speed) in [(50.0, lead_speed), (49.9, 1.4), (49.8, 1.4), (49.7, 1.4)] {
                let i = agents.spawn_border_arrival_agent(usize::MAX, 0, 0.0, 0.0);
                agents.transit[i] = TRANSIT_NETWORK;
                agents.transit_mode[i] = MODE_WALK;
                agents.access_flags[i] = ACCESS_PLAN_VALID;
                agents.current_lane_id[i] = 0;
                agents.lane_distance[i] = d;
                agents.speed[i] = speed;
            }
            agents.rebuild_lanes(1, 4);
            agents
        };

        // Behind one standing, each walker queues behind the one ahead, bodies the gap apart.
        let queued = walkers(0.0);
        let spacing = WALKER_BODY_LENGTH_M + IDM_S_MIN;
        for (k, i) in [1, 2, 3].into_iter().enumerate() {
            let want = 50.0 - (k + 1) as f32 * spacing;
            let d = queued.lane_distance[i];
            assert!((d - want).abs() < 1e-4, "walker {i} at {d}, want {want}");
        }
        let order: Vec<usize> = queued.lane_buckets[0].iter().map(|e| e.1).collect();
        assert_eq!(order, [3, 2, 1, 0]);

        // Behind one walking, they keep their distances: they walk through to pass.
        let passing = walkers(1.2);
        for (i, d) in [(1, 49.9), (2, 49.8), (3, 49.7)] {
            assert_eq!(passing.lane_distance[i], d);
        }
    }
}
