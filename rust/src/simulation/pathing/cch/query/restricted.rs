// SPDX-License-Identifier: GPL-2.0-only

//! CPU restricted PHAST prototype, evaluated against exact bucket joins before adoption.

use super::*;

/// Source lanes per contiguous node-major block in the CPU batch experiment.
pub(crate) const RESTRICTED_BATCH_LANES: usize = 8;

/// Reusable node-major source block; every arc cost is shared across eight source lanes.
#[derive(Default)]
pub(crate) struct RestrictedBatchScratch {
    generation: Option<(u32, usize, usize)>,
    up: SearchData,
    down: Vec<[f32; RESTRICTED_BATCH_LANES]>,
    result: Vec<[f32; RESTRICTED_BATCH_LANES]>,
}

impl RestrictedBatchScratch {
    /// Reports every reusable buffer for the no-growth regression.
    pub(crate) fn capacities(&self) -> [usize; 4] {
        [
            self.up.records.capacity(),
            self.up.order.capacity(),
            self.down.capacity(),
            self.result.capacity(),
        ]
    }
}

/// Topology-only upward closure of a requested target set.
pub(crate) struct RestrictedTargets {
    generation: u32,
    targets: Vec<u32>,
    closure: Vec<u32>,
}
/// Reusable source labels and target results; queries allocate only on a new layout.
#[derive(Default)]
pub(crate) struct RestrictedScratch {
    generation: Option<(u32, usize, usize)>,
    data: SearchData,
    down: Vec<f32>,
    result: Vec<f32>,
}
impl RestrictedScratch {
    /// Reports each reusable buffer for allocation-growth regression checks.
    pub(crate) fn capacities(&self) -> [usize; 4] {
        [
            self.data.records.capacity(),
            self.data.order.capacity(),
            self.down.capacity(),
            self.result.capacity(),
        ]
    }
}
impl RestrictedTargets {
    /// Reserves an eight-source block once before repeated queries.
    pub(crate) fn prepare_batch(&self, cch: &CchGraph, scratch: &mut RestrictedBatchScratch) {
        let revision = (cch.build_generation, cch.parent.len(), self.targets.len());
        if scratch.generation == Some(revision) {
            return;
        }
        scratch.up.prepare(cch.parent.len());
        scratch
            .down
            .resize(cch.parent.len(), [f32::INFINITY; RESTRICTED_BATCH_LANES]);
        scratch
            .result
            .resize(self.targets.len(), [f32::INFINITY; RESTRICTED_BATCH_LANES]);
        scratch.generation = Some(revision);
    }

    /// Queries at most eight sources; output is indexed by target, then source lane.
    /// The contiguous lane loop permits SIMD without changing any source's addition order.
    /// Prepared work is O(sum source upward work + lanes × (closure arcs + target states)).
    pub(crate) fn query_batch<'s>(
        &self,
        cch: &CchGraph,
        starts: &[(u32, usize)],
        mask: u8,
        scratch: &'s mut RestrictedBatchScratch,
    ) -> &'s [[f32; RESTRICTED_BATCH_LANES]] {
        assert!(starts.len() <= RESTRICTED_BATCH_LANES);
        self.prepare_batch(cch, scratch);
        scratch.result.fill([f32::INFINITY; RESTRICTED_BATCH_LANES]);
        if self.generation != cch.build_generation {
            return &scratch.result;
        }
        if !cch.parent.is_empty() {
            for &state in &self.closure {
                scratch.down[state as usize] = [f32::INFINITY; RESTRICTED_BATCH_LANES];
            }
            let metric = &cch.profiles[cch.profile_for_mask[mask as usize] as usize];
            for (lane, &(start, incoming)) in starts.iter().enumerate() {
                if start as usize >= cch.states.len() {
                    continue;
                }
                scratch.up.clear();
                cch.seed_source(start, incoming, mask, &mut scratch.up);
                cch.sweep_up(metric, &mut scratch.up, false);
                for &state in &scratch.up.order {
                    scratch.down[state as usize][lane] = scratch.up.records[state as usize].cost;
                }
            }
            for &state in self.closure.iter().rev() {
                let mut low = scratch.down[state as usize];
                for &(high, arc) in &cch.up[state as usize] {
                    let high = scratch.down[high as usize];
                    let edge = metric.costs[arc * 2 + 1];
                    for lane in 0..RESTRICTED_BATCH_LANES {
                        low[lane] = low[lane].min(high[lane] + edge);
                    }
                }
                scratch.down[state as usize] = low;
            }
            for (index, &target) in self.targets.iter().enumerate() {
                if target as usize >= cch.states.len() {
                    continue;
                }
                for &state in &cch.states[target as usize] {
                    for lane in 0..starts.len() {
                        scratch.result[index][lane] =
                            scratch.result[index][lane].min(scratch.down[state as usize][lane]);
                    }
                }
            }
        }
        for (lane, &(start, _)) in starts.iter().enumerate() {
            if start as usize >= cch.states.len() {
                continue;
            }
            if let Ok(index) = self.targets.binary_search(&start) {
                scratch.result[index][lane] = 0.0;
            }
        }
        &scratch.result
    }

    /// Builds the exact ancestor closure, including arcs infinite in the current metric.
    pub(crate) fn build(cch: &CchGraph, targets: &[u32]) -> Self {
        let mut targets = targets.to_vec();
        targets.sort_unstable();
        targets.dedup();
        let mut marked = vec![false; cch.parent.len()];
        let mut closure = Vec::new();
        for &target in &targets {
            if target as usize >= cch.states.len() {
                continue;
            }
            for &state in &cch.states[target as usize] {
                let mut state = state;
                while state != u32::MAX && !marked[state as usize] {
                    marked[state as usize] = true;
                    closure.push(state);
                    state = cch.parent[state as usize];
                }
            }
        }
        closure.sort_unstable();
        Self {
            generation: cch.build_generation,
            targets,
            closure,
        }
    }
    /// Target node IDs, in the output array's sorted order.
    pub(crate) fn targets(&self) -> &[u32] {
        &self.targets
    }
    /// Number of states retained by the restricted descending pass.
    pub(crate) fn closure_len(&self) -> usize {
        self.closure.len()
    }
    /// Full state count for reporting how much work restriction actually removes.
    pub(crate) fn state_count(cch: &CchGraph) -> usize {
        cch.parent.len()
    }
    /// Reserves the dense scratch and output once before repeated source queries.
    pub(crate) fn prepare(&self, cch: &CchGraph, scratch: &mut RestrictedScratch) {
        let revision = (cch.build_generation, cch.parent.len(), self.targets.len());
        if scratch.generation == Some(revision) {
            return;
        }
        scratch.data.prepare(cch.parent.len());
        scratch.result.resize(self.targets.len(), f32::INFINITY);
        scratch.down.resize(cch.parent.len(), f32::INFINITY);
        scratch.generation = Some(revision);
    }
    /// Runs upward search, then the descending target closure. Costs equal the full PHAST
    /// sweep bit-for-bit; addition order can differ from bidirectional point/bucket queries.
    /// O(H log H + A_H + closure arcs + target arrival states), allocation-free when prepared.
    pub(crate) fn query<'s>(
        &self,
        cch: &CchGraph,
        start: u32,
        incoming: usize,
        mask: u8,
        scratch: &'s mut RestrictedScratch,
    ) -> &'s [f32] {
        self.prepare(cch, scratch);
        scratch.result.fill(f32::INFINITY);
        if self.generation != cch.build_generation || start as usize >= cch.states.len() {
            return &scratch.result;
        }
        let data = &mut scratch.data;
        data.clear();
        if !cch.parent.is_empty() {
            cch.seed_source(start, incoming, mask, data);
            let metric = &cch.profiles[cch.profile_for_mask[mask as usize] as usize];
            cch.sweep_up(metric, data, false);
            for &state in self.closure.iter().rev() {
                let original = data.records[state as usize];
                let mut cost = if original.epoch == data.epoch {
                    original.cost
                } else {
                    f32::INFINITY
                };
                for &(high, arc) in &cch.up[state as usize] {
                    // The target closure contains every higher filled neighbor. Its compact
                    // f32 slot was written earlier in this descending pass; no epoch test
                    // or access to the path-witness label is needed in this inner loop.
                    cost = cost.min(scratch.down[high as usize] + metric.costs[arc * 2 + 1]);
                }
                scratch.down[state as usize] = cost;
            }
            for (index, &target) in self.targets.iter().enumerate() {
                if target as usize >= cch.states.len() {
                    continue;
                }
                for &state in &cch.states[target as usize] {
                    scratch.result[index] = scratch.result[index].min(scratch.down[state as usize]);
                }
            }
        }
        if let Ok(index) = self.targets.binary_search(&start) {
            scratch.result[index] = 0.0;
        }
        &scratch.result
    }
}
