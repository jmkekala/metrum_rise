// SPDX-License-Identifier: GPL-2.0-only

//! Lane bucket maintenance and overlap correction.

mod occupancy;

use super::traffic::live_lane_bucket_transit;
use crate::simulation::economy::agents::data::AgentSystem;
use crate::simulation::network::TransitNetwork;
pub(crate) use occupancy::LaneBucketScratch;

impl AgentSystem {
    /// Brings the sorted per-lane buckets used by IDM and access handoff claims up to date.
    ///
    /// The snapshot the previous tick left stays valid across agent appends and swap-removes
    /// (`SIM-02-K` part 4): removals patch it as they happen, and agents appended since are
    /// added here, O(appended × bucket). Only an invalidated snapshot or a changed lane count
    /// rebuilds it from every agent.
    pub(super) fn prepare_lane_buckets_for_tick(
        &mut self,
        transit_network: &TransitNetwork,
        n: usize,
    ) -> (usize, usize) {
        let lane_count = transit_network.lane_system.lanes.len();
        self.ensure_lane_bucket_buffers(lane_count);
        if !self.lane_bucket_snapshot_valid
            || self.lane_bucket_snapshot_lane_count != lane_count
            || self.lane_bucket_snapshot_agent_count > n
        {
            self.rebuild_lane_occupancy_snapshot(lane_count, n);
        } else if self.lane_bucket_snapshot_agent_count < n {
            self.add_appended_lane_agents(lane_count, n);
        }
        #[cfg(test)]
        self.check_lane_buckets_match_rebuild(lane_count, n);

        self.reset_lane_claims(lane_count);

        (lane_count, self.lane_bucket_live_agent_count)
    }

    /// Rebuilds lane buckets after movement and fixes overlaps.
    pub(super) fn rebuild_lanes(&mut self, lane_count: usize, n: usize) {
        self.ensure_lane_bucket_buffers(lane_count);
        let live_lane_agent_count = self.fill_lane_buckets(lane_count, n, true);

        #[cfg(test)]
        {
            self.count_lane_bucket_entries();
            self.lane_bucket_patched = false;
        }
        self.mark_lane_bucket_snapshot_valid(lane_count, n, live_lane_agent_count);
    }

    fn rebuild_lane_occupancy_snapshot(&mut self, lane_count: usize, n: usize) {
        let live_lane_agent_count = self.fill_lane_buckets(lane_count, n, false);
        #[cfg(test)]
        {
            super::contract::add(super::contract::Phase::LaneBucketAgents, n);
            self.count_lane_bucket_entries();
            self.lane_bucket_patched = false;
        }
        self.mark_lane_bucket_snapshot_valid(lane_count, n, live_lane_agent_count);
    }

    // Agents appended since the snapshot was taken start indoors or are placed by their spawn
    // call before the tick; their final state is read here.
    fn add_appended_lane_agents(&mut self, lane_count: usize, n: usize) {
        let first = self.lane_bucket_snapshot_agent_count;
        for agent in first..n {
            self.add_lane_bucket_agent(agent, agent, lane_count);
        }
        #[cfg(test)]
        {
            super::contract::add(super::contract::Phase::LaneBucketAgents, n - first);
            self.lane_bucket_patched = true;
        }
        self.lane_bucket_snapshot_agent_count = n;
    }

    // Adds the entries of agent `source`'s current state under index `index`.
    fn add_lane_bucket_agent(&mut self, source: usize, index: usize, lane_count: usize) {
        if live_lane_bucket_transit(self.agents.transit[source]) {
            self.lane_bucket_live_agent_count += 1;
        }
        let lane_distance = self.agents.lane_distance[source];
        for lid in self
            .agent_bucket_lanes(source, lane_count)
            .into_iter()
            .flatten()
        {
            self.insert_lane_entry(lid, lane_distance, index);
        }
    }

    /// Keeps the retained snapshot in step with the swap-remove of agent `index`. Called just
    /// before the removal, while the last agent is still at the end; it renames that agent's
    /// entries to `index`, or adds them if it was appended after the snapshot. Entries are
    /// found from the agents' current lane state, which only the tick and invalidating
    /// mutators change. A missing entry invalidates the snapshot. O(entries on their lanes).
    pub(in crate::simulation::economy::agents) fn swap_remove_lane_bucket_agent(
        &mut self,
        index: usize,
    ) {
        if !self.lane_bucket_snapshot_valid || index >= self.agents.len() {
            return;
        }
        let last = self.agents.len() - 1;
        let covered = self.lane_bucket_snapshot_agent_count;
        let lane_count = self.lane_bucket_snapshot_lane_count;
        if index < covered {
            if live_lane_bucket_transit(self.agents.transit[index]) {
                self.lane_bucket_live_agent_count =
                    self.lane_bucket_live_agent_count.saturating_sub(1);
            }
            for lid in self
                .agent_bucket_lanes(index, lane_count)
                .into_iter()
                .flatten()
            {
                if self.remove_lane_entry(lid, index).is_none() {
                    self.invalidate_lane_bucket_snapshot();
                    return;
                }
            }
        }
        if last != index {
            if last < covered {
                for lid in self
                    .agent_bucket_lanes(last, lane_count)
                    .into_iter()
                    .flatten()
                {
                    let Some(lane_distance) = self.remove_lane_entry(lid, last) else {
                        self.invalidate_lane_bucket_snapshot();
                        return;
                    };
                    self.insert_lane_entry(lid, lane_distance, index);
                }
            } else if index < covered {
                self.add_lane_bucket_agent(last, index, lane_count);
            }
        }
        if last < covered {
            self.lane_bucket_snapshot_agent_count = last;
        }
        #[cfg(test)]
        {
            self.lane_bucket_patched = true;
        }
    }

    /// Test builds: prepares the tick's snapshot and compares it with a rebuild from every
    /// agent.
    #[cfg(test)]
    pub(crate) fn prepare_lane_buckets_for_test(&mut self, transit_network: &TransitNetwork) {
        let n = self.agents.len();
        self.prepare_lane_buckets_for_tick(transit_network, n);
        self.lane_bucket_patched = true;
        self.check_lane_buckets_match_rebuild(transit_network.lane_system.lanes.len(), n);
    }

    /// Adds a boundary-mutated agent after its new network state is installed. Appended agents
    /// are already picked up by the next tick. O(entries on its lanes), allocation-free.
    pub(in crate::simulation::economy::agents) fn join_lane_buckets(&mut self, agent_idx: usize) {
        if self.lane_bucket_snapshot_valid && agent_idx < self.lane_bucket_snapshot_agent_count {
            self.add_lane_bucket_agent(agent_idx, agent_idx, self.lane_bucket_snapshot_lane_count);
            #[cfg(test)]
            { self.lane_bucket_patched = true; }
        }
    }

    /// Removes agent `agent_idx`'s entries before a mutator moves it into a building or onto an
    /// access leg, neither of which is in the buckets, so the snapshot stays valid. An agent
    /// appended after the snapshot has none. O(entries on its lanes).
    pub(in crate::simulation::economy::agents) fn leave_lane_buckets(&mut self, agent_idx: usize) {
        if !self.lane_bucket_snapshot_valid || agent_idx >= self.lane_bucket_snapshot_agent_count {
            return;
        }
        if live_lane_bucket_transit(self.agents.transit[agent_idx]) {
            self.lane_bucket_live_agent_count = self.lane_bucket_live_agent_count.saturating_sub(1);
        }
        let lane_count = self.lane_bucket_snapshot_lane_count;
        for lid in self
            .agent_bucket_lanes(agent_idx, lane_count)
            .into_iter()
            .flatten()
        {
            if self.remove_lane_entry(lid, agent_idx).is_none() {
                self.invalidate_lane_bucket_snapshot();
                return;
            }
        }
        #[cfg(test)]
        {
            self.lane_bucket_patched = true;
        }
    }

    #[cfg(test)]
    fn count_lane_bucket_entries(&self) {
        let entries = self
            .dirty_lanes
            .iter()
            .map(|&lane_id| self.lane_buckets[lane_id].len())
            .sum();
        super::contract::add(super::contract::Phase::LaneBucketEntries, entries);
    }

    /// Test builds: after an append or swap-remove patch, the snapshot equals a rebuild from
    /// every agent, entry for entry, with every lane holding entries marked dirty once.
    /// Timing runs skip it with the other whole-city checks.
    #[cfg(test)]
    fn check_lane_buckets_match_rebuild(&mut self, lane_count: usize, n: usize) {
        if !std::mem::take(&mut self.lane_bucket_patched) || !crate::debug::work::city_checks() {
            return;
        }
        let mut expected = vec![Vec::new(); self.lane_buckets.len()];
        let mut live = 0;
        for agent in 0..n {
            if live_lane_bucket_transit(self.agents.transit[agent]) {
                live += 1;
            }
            for lid in self
                .agent_bucket_lanes(agent, lane_count)
                .into_iter()
                .flatten()
            {
                expected[lid].push((self.agents.lane_distance[agent], agent));
            }
        }
        for bucket in &mut expected {
            bucket.sort_unstable_by(occupancy::lane_bucket_order);
        }
        assert_eq!(live, self.lane_bucket_live_agent_count, "live lane agents");
        for (lid, bucket) in expected.iter().enumerate() {
            assert_eq!(
                &self.lane_buckets[lid], bucket,
                "lane {lid}: patched bucket differs from a rebuild"
            );
            assert!(
                bucket.is_empty() || self.lane_is_dirty[lid],
                "lane {lid} not dirty"
            );
        }
        let mut dirty = self.dirty_lanes.clone();
        dirty.sort_unstable();
        dirty.dedup();
        assert_eq!(dirty.len(), self.dirty_lanes.len(), "duplicate dirty lanes");
        assert!(
            dirty.iter().all(|&lid| self.lane_is_dirty[lid])
                && self.lane_is_dirty.iter().filter(|&&flag| flag).count() == dirty.len(),
            "dirty flags and list differ"
        );
    }
}
