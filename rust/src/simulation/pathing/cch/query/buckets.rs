// SPDX-License-Identifier: GPL-2.0-only

//! Exact many-to-many meetings with retained backward witnesses for pedestrian validation.

use super::*;

#[derive(Clone, Copy)]
struct TargetRecord {
    state: u32,
    cost: f32,
    arc: usize,
    previous: u32,
}
#[derive(Clone, Copy)]
struct BucketEntry {
    target: u32,
    cost: f32,
}
#[derive(Clone, Copy)]
struct Meeting {
    cost: f32,
    state: u32,
}
impl Default for Meeting {
    fn default() -> Self {
        Self {
            cost: f32::INFINITY,
            state: u32::MAX,
        }
    }
}

/// Backward target searches shared by all sources under one topology and metric revision.
/// Buckets contain exactly the labels used by point queries, including their path witnesses.
pub(crate) struct CchBuckets {
    /// Identifies this target set so worker caches cannot answer a different batch.
    pub(crate) id: u64,
    revision: (u32, u64),
    mask: u8,
    targets: Vec<u32>,
    backward: NodeSlices<TargetRecord>,
    buckets: NodeSlices<BucketEntry>,
}

/// Reusable forward labels and one result per target for a single source/context.
#[derive(Default)]
pub(crate) struct CchBucketSource {
    prepared: Option<(u64, u32, usize)>,
    forward: SearchData,
    meetings: Vec<Meeting>,
    start: u32,
}
impl CchBucketSource {
    /// Reserves O(states + targets) storage once per target-set revision.
    pub(crate) fn prepare(&mut self, targets: &CchBuckets, cch: &CchGraph) {
        let revision = (targets.id, cch.build_generation, cch.parent.len());
        if self.prepared == Some(revision) {
            return;
        }
        self.forward.prepare(cch.parent.len());
        self.meetings
            .resize(targets.targets.len(), Meeting::default());
        self.prepared = Some(revision);
    }
}

impl CchBuckets {
    /// Builds one backward search per distinct target, in parallel with per-worker scratch.
    /// Retained storage is O(sum of target ancestor labels), independent of source count.
    pub(crate) fn build(cch: &CchGraph, targets: &[u32], mask: u8) -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let mut targets = targets.to_vec();
        targets.sort_unstable();
        targets.dedup();
        let rows: Vec<Vec<TargetRecord>> = targets
            .par_iter()
            .with_min_len(targets.len().div_ceil(rayon::current_num_threads()).max(1))
            .map_init(
                || {
                    let mut scratch = SearchData::default();
                    scratch.prepare(cch.parent.len());
                    scratch
                },
                |scratch, &target| {
                    scratch.clear();
                    if target as usize >= cch.states.len() || cch.parent.is_empty() {
                        return Vec::new();
                    }
                    for &state in &cch.states[target as usize] {
                        scratch.seed(state, 0.0, 0.0, usize::MAX, &cch.parent);
                    }
                    cch.sweep_up(
                        &cch.profiles[cch.profile_for_mask[mask as usize] as usize],
                        scratch,
                        true,
                    );
                    scratch
                        .order
                        .iter()
                        .filter_map(|&state| {
                            let label = scratch.records[state as usize];
                            (label.cost < f32::MAX).then_some(TargetRecord {
                                state,
                                cost: label.cost,
                                arc: label.arc,
                                previous: label.previous,
                            })
                        })
                        .collect()
                },
            )
            .collect();
        let mut buckets = vec![Vec::new(); cch.parent.len()];
        for (target, labels) in rows.iter().enumerate() {
            for label in labels {
                buckets[label.state as usize].push(BucketEntry {
                    target: target as u32,
                    cost: label.cost,
                });
            }
        }
        Self {
            id: NEXT_ID.fetch_add(1, Ordering::Relaxed),
            revision: (cch.build_generation, cch.metric_revision),
            mask,
            targets,
            backward: NodeSlices::from_rows(rows),
            buckets: NodeSlices::from_rows(buckets),
        }
    }

    /// Whether the topology and customized costs still match this prepared target set.
    pub(crate) fn is_current(&self, cch: &CchGraph) -> bool {
        self.revision == (cch.build_generation, cch.metric_revision)
    }

    /// Finds a prepared target in O(log targets), independently of lane attachment details.
    pub(crate) fn target_index(&self, node: u32) -> Option<usize> {
        self.targets.binary_search(&node).ok()
    }

    /// Checks target-set equality without rebuilding buckets when open job sites are unchanged.
    pub(crate) fn targets(&self) -> &[u32] {
        &self.targets
    }

    /// Runs one upward source search and joins its compatible-state target buckets.
    /// O(H log H + A_H + visited bucket entries + targets); prepared calls allocate nothing.
    pub(crate) fn query_source(
        &self,
        cch: &CchGraph,
        start: u32,
        incoming: usize,
        source: &mut CchBucketSource,
    ) {
        source.prepare(self, cch);
        source.start = start;
        source.meetings.fill(Meeting::default());
        source.forward.clear();
        if !self.is_current(cch) || start as usize >= cch.states.len() {
            return;
        }
        if !cch.parent.is_empty() {
            cch.seed_source(start, incoming, self.mask, &mut source.forward);
            cch.sweep_up(
                &cch.profiles[cch.profile_for_mask[self.mask as usize] as usize],
                &mut source.forward,
                false,
            );
            // Ascending state order and strict improvements reproduce point-query meeting ties.
            for &state in &source.forward.order {
                let cost = source.forward.records[state as usize].cost;
                if cost >= f32::MAX {
                    continue;
                }
                for entry in &self.buckets[state as usize] {
                    let sum = cost + entry.cost;
                    let meeting = &mut source.meetings[entry.target as usize];
                    if sum < meeting.cost {
                        *meeting = Meeting { cost: sum, state };
                    }
                }
            }
        }
        if let Some(index) = self.target_index(start) {
            source.meetings[index] = Meeting {
                cost: 0.0,
                state: u32::MAX,
            };
        }
    }

    /// Reads the exact point-query cost bits for one target after a source query.
    pub(crate) fn cost(&self, target: usize, source: &CchBucketSource) -> Option<f32> {
        if source.prepared.is_none_or(|(id, _, _)| id != self.id) {
            return None;
        }
        source
            .meetings
            .get(target)
            .map(|m| m.cost)
            .filter(|&cost| cost < f32::MAX)
    }

    /// Unpacks the identical point-query path for connector validation; no new search runs.
    pub(crate) fn path_in<'s>(
        &self,
        target: usize,
        cch: &CchGraph,
        graph: &RegionGraph,
        source: &CchBucketSource,
        scratch: &'s mut CchPathScratch,
    ) -> Option<(f32, &'s [u32])> {
        if !self.is_current(cch) {
            return None;
        }
        let cost = self.cost(target, source)?;
        scratch.edges.clear();
        scratch.shortcuts.clear();
        scratch.nodes.clear();
        let meeting = source.meetings[target].state;
        if meeting == u32::MAX {
            scratch.nodes.push(source.start);
            return Some((cost, &scratch.nodes));
        }
        scratch.prepare(cch, graph);
        let metric = &cch.profiles[cch.profile_for_mask[self.mask as usize] as usize];
        let forward = &source.forward.records;
        let mut state = meeting;
        while forward[state as usize].arc != usize::MAX {
            scratch.shortcuts.push(forward[state as usize].arc);
            state = forward[state as usize].previous;
        }
        if forward[state as usize].seed_edge != usize::MAX {
            scratch.edges.push(forward[state as usize].seed_edge);
        }
        CchGraph::unpack(metric, &mut scratch.shortcuts, &mut scratch.edges);
        state = meeting;
        let backward = &self.backward[target];
        loop {
            let label = backward[backward
                .binary_search_by_key(&state, |label| label.state)
                .unwrap()];
            if label.arc == usize::MAX {
                break;
            }
            scratch.shortcuts.push(label.arc);
            state = label.previous;
        }
        scratch.shortcuts.reverse();
        CchGraph::unpack(metric, &mut scratch.shortcuts, &mut scratch.edges);
        let mut node = source.start;
        scratch.nodes.push(node);
        for &edge in &scratch.edges {
            let edge = graph.edge(edge);
            node = if edge.start_node == node {
                edge.end_node
            } else {
                edge.start_node
            };
            scratch.nodes.push(node);
        }
        debug_assert_eq!(node, self.targets[target]);
        Some((cost, &scratch.nodes))
    }
}

#[cfg(test)]
impl CchBucketSource {
    /// Counts retained buffers for allocation regression tests.
    pub(crate) fn capacities(&self) -> [usize; 3] {
        [
            self.forward.records.capacity(),
            self.forward.order.capacity(),
            self.meetings.capacity(),
        ]
    }
}
