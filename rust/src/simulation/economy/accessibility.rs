// SPDX-License-Identifier: GPL-2.0-only

//! Economy-side reachability helpers for candidate planning.

use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

/// Sentinel component id for nodes or buildings with no usable access.
pub(crate) const NO_COMPONENT: u32 = u32::MAX;

/// Connected-component labels for one transit mode.
#[derive(Clone, Debug)]
pub(crate) struct ModeComponentIndex {
    component_by_node: Vec<u32>,
}

/// Up to two component labels reachable from a building entrance for one mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BuildingModeComponents {
    components: [u32; 2],
    count: u8,
}

/// One candidate entry in a reachability component and routing chunk.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct ReachableBucketEntry {
    component: u32,
    chunk: (i32, i32),
    item_idx: usize,
}

/// Event emitted by nearest-chunk candidate scans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ReachableBucketScanEvent {
    /// A candidate item was found in the current chunk ring.
    Item {
        /// Caller-owned candidate index.
        item_idx: usize,
    },
    /// A ring finished; the distance is a lower bound for every later ring.
    RingComplete {
        /// Squared world-space distance to the next ring's closest chunk bounds.
        next_min_distance_sq: f32,
    },
}

/// Candidate buckets grouped by reachability component and 512 m routing chunk.
#[derive(Clone, Debug, Default)]
pub(crate) struct ReachableBucketIndex {
    by_component: BTreeMap<u32, ComponentBuckets>,
    available: usize,
}

#[derive(Clone, Debug)]
struct ComponentBuckets {
    by_chunk: BTreeMap<(i32, i32), BucketItems>,
    min_chunk: (i32, i32),
    max_chunk: (i32, i32),
    available: usize,
}

#[derive(Clone, Debug, Default)]
struct BucketItems {
    items: Vec<usize>,
    // Empty for ordinary immutable indices. Finite reservation users opt into an
    // availability tree, avoiding a linear walk over depleted suppliers on every query.
    available: Vec<usize>,
    leaves: usize,
}

impl BucketItems {
    fn enable_reservations(&mut self) {
        self.leaves = self.items.len().max(1).next_power_of_two();
        self.available.resize(self.leaves * 2, 0);
        self.available.fill(0);
        self.available[self.leaves..self.leaves + self.items.len()].fill(1);
        for node in (1..self.leaves).rev() {
            self.available[node] = self.available[node * 2] + self.available[node * 2 + 1];
        }
    }

    fn set_available(&mut self, item: usize, available: bool) -> isize {
        let Ok(position) = self.items.binary_search(&item) else {
            return 0;
        };
        let mut node = self.leaves + position;
        let previous = self.available[node];
        let next = usize::from(available);
        if previous == next {
            return 0;
        }
        self.available[node] = next;
        node /= 2;
        while node > 0 {
            self.available[node] = self.available[node * 2] + self.available[node * 2 + 1];
            node /= 2;
        }
        next as isize - previous as isize
    }

    fn visit(&self, mut visitor: impl FnMut(usize) -> bool) -> bool {
        if self.available.is_empty() {
            return self.items.iter().all(|&item| visitor(item));
        }
        self.visit_available(1, &mut visitor)
    }

    fn visit_available(&self, node: usize, visitor: &mut impl FnMut(usize) -> bool) -> bool {
        if self.available[node] == 0 {
            return true;
        }
        if node >= self.leaves {
            return visitor(self.items[node - self.leaves]);
        }
        // O(K) for a full bucket, O(log K) for one survivor. Prune empty subtrees
        // instead of walking exhausted candidates or selecting every live leaf by rank.
        self.visit_available(node * 2, visitor) && self.visit_available(node * 2 + 1, visitor)
    }
}

impl Default for BuildingModeComponents {
    fn default() -> Self {
        Self {
            components: [NO_COMPONENT; 2],
            count: 0,
        }
    }
}

impl BuildingModeComponents {
    /// Returns the component labels in deterministic order.
    pub(crate) fn as_slice(&self) -> &[u32] {
        &self.components[..self.count as usize]
    }

    /// Returns the fixed backing array and active component count for allocation-free iteration.
    pub(crate) fn raw_parts(self) -> ([u32; 2], usize) {
        (self.components, self.count as usize)
    }

    /// Appends this building's reachable components to an existing candidate-index buffer.
    pub(crate) fn append_bucket_entries(
        self,
        target: &mut Vec<ReachableBucketEntry>,
        chunk: (i32, i32),
        item_idx: usize,
    ) {
        for &component in self.as_slice() {
            target.push(ReachableBucketEntry::new(component, chunk, item_idx));
        }
    }

    fn push(&mut self, component: u32) {
        if component == NO_COMPONENT || self.as_slice().contains(&component) {
            return;
        }
        let idx = self.count as usize;
        if idx < self.components.len() {
            self.components[idx] = component;
            self.count += 1;
        }
    }
}

impl ReachableBucketEntry {
    /// Creates a candidate entry for a reachability component and routing chunk.
    pub(crate) fn new(component: u32, chunk: (i32, i32), item_idx: usize) -> Self {
        Self {
            component,
            chunk,
            item_idx,
        }
    }
}

impl ReachableBucketIndex {
    /// Builds deterministic component/chunk buckets from caller-owned candidate entries.
    pub(crate) fn from_entries(mut entries: Vec<ReachableBucketEntry>) -> Self {
        entries.retain(|entry| entry.component != NO_COMPONENT);
        entries.sort_unstable();
        entries.dedup();
        let available = entries.len();

        let mut by_component = BTreeMap::new();
        for entry in entries {
            by_component
                .entry(entry.component)
                .and_modify(|buckets: &mut ComponentBuckets| {
                    buckets.available += 1;
                    buckets.min_chunk.0 = buckets.min_chunk.0.min(entry.chunk.0);
                    buckets.min_chunk.1 = buckets.min_chunk.1.min(entry.chunk.1);
                    buckets.max_chunk.0 = buckets.max_chunk.0.max(entry.chunk.0);
                    buckets.max_chunk.1 = buckets.max_chunk.1.max(entry.chunk.1);
                    buckets
                        .by_chunk
                        .entry(entry.chunk)
                        .or_default()
                        .items
                        .push(entry.item_idx);
                })
                .or_insert_with(|| ComponentBuckets {
                    by_chunk: BTreeMap::from([(
                        entry.chunk,
                        BucketItems {
                            items: vec![entry.item_idx],
                            ..Default::default()
                        },
                    )]),
                    min_chunk: entry.chunk,
                    max_chunk: entry.chunk,
                    available: 1,
                });
        }

        Self {
            by_component,
            available,
        }
    }

    /// Whether any indexed entry remains eligible, in O(1).
    pub(crate) fn has_available(&self) -> bool {
        self.available > 0
    }

    /// Enables allocation-free availability updates for finite reservation users.
    /// Immutable indices retain their existing flat traversal without tree overhead.
    pub(crate) fn enable_reservations(&mut self) {
        for buckets in self.by_component.values_mut() {
            for items in buckets.by_chunk.values_mut() {
                items.enable_reservations();
            }
        }
    }

    /// Refills a prepared reservation tree in O(indexed entries), retaining all allocations.
    /// The predicate is evaluated once per entry, before dependent parent counts are summed.
    pub(crate) fn reset_available(&mut self, mut eligible: impl FnMut(usize) -> bool) {
        self.available = 0;
        for buckets in self.by_component.values_mut() {
            buckets.available = 0;
            for items in buckets.by_chunk.values_mut() {
                debug_assert!(items.leaves > 0);
                for (index, &item) in items.items.iter().enumerate() {
                    items.available[items.leaves + index] = usize::from(eligible(item));
                }
                for node in (1..items.leaves).rev() {
                    items.available[node] = items.available[node * 2] + items.available[node * 2 + 1];
                }
                buckets.available += items.available[1];
            }
            self.available += buckets.available;
        }
    }

    /// Toggles one candidate in O(log chunks + log candidates-in-chunk), without allocation.
    /// Requires `enable_reservations`; membership and spatial bounds remain unchanged.
    pub(crate) fn set_available(&mut self, entry: ReachableBucketEntry, available: bool) {
        if let Some(buckets) = self.by_component.get_mut(&entry.component)
            && let Some(items) = buckets.by_chunk.get_mut(&entry.chunk)
        {
            let change = items.set_available(entry.item_idx, available);
            buckets.available = buckets.available.checked_add_signed(change).unwrap();
            self.available = self.available.checked_add_signed(change).unwrap();
        }
    }

    /// Scans reachable chunks outward from an origin without a fixed radius.
    pub(crate) fn scan_nearest(
        &self,
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mut visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        let origin_chunk = chunk_for_point(origin_x, origin_y);
        let Some(max_ring) = self.max_ring_for_components(components, origin_chunk) else {
            return;
        };

        for ring in 0..=max_ring {
            let mut should_continue = true;
            scan_ring_chunks(origin_chunk, ring, |chunk| {
                if !should_continue {
                    return;
                }
                for &component in components.as_slice() {
                    let Some(buckets) = self.by_component.get(&component) else {
                        continue;
                    };
                    let Some(items) = buckets.by_chunk.get(&chunk) else {
                        continue;
                    };
                    if !items.visit(|item_idx| visitor(ReachableBucketScanEvent::Item { item_idx }))
                    {
                        should_continue = false;
                        return;
                    }
                }
            });
            if !should_continue {
                return;
            }

            if ring < max_ring {
                let next_min_distance_sq =
                    min_possible_ring_distance_sq(origin_x, origin_y, origin_chunk, ring + 1);
                if !visitor(ReachableBucketScanEvent::RingComplete {
                    next_min_distance_sq,
                }) {
                    return;
                }
            }
        }
    }

    fn max_ring_for_components(
        &self,
        components: BuildingModeComponents,
        origin_chunk: (i32, i32),
    ) -> Option<i32> {
        let mut max_ring = None::<i32>;
        for &component in components.as_slice() {
            let Some(buckets) = self.by_component.get(&component) else {
                continue;
            };
            if buckets.available == 0 {
                continue;
            }
            let component_ring = [
                (origin_chunk.0 - buckets.min_chunk.0).abs(),
                (origin_chunk.1 - buckets.min_chunk.1).abs(),
                (origin_chunk.0 - buckets.max_chunk.0).abs(),
                (origin_chunk.1 - buckets.max_chunk.1).abs(),
            ]
            .into_iter()
            .max()
            .unwrap_or(0);
            max_ring = Some(max_ring.unwrap_or(0).max(component_ring));
        }
        max_ring
    }
}

impl ModeComponentIndex {
    /// Builds undirected reachability labels for the requested transit mode.
    pub(crate) fn build(graph: &RegionGraph, transit_flag: u8) -> Self {
        let node_count = graph.node_count();
        let mut parent: Vec<usize> = (0..node_count).collect();
        let mut active = vec![false; node_count];

        for edge in graph.edges() {
            if edge.deleted || (edge.allowed_types & transit_flag) == 0 {
                continue;
            }
            let start = graph.get_valid_node(edge.start_node) as usize;
            let end = graph.get_valid_node(edge.end_node) as usize;
            if start >= node_count || end >= node_count {
                continue;
            }
            active[start] = true;
            active[end] = true;
            unite(start, end, &mut parent);
        }

        let mut root_to_component = BTreeMap::new();
        let mut component_by_node = vec![NO_COMPONENT; node_count];
        for node in 0..node_count {
            if !active[node] {
                continue;
            }
            let root = find(node, &mut parent);
            let next_id = root_to_component.len() as u32;
            let component = *root_to_component.entry(root).or_insert(next_id);
            component_by_node[node] = component;
        }

        Self { component_by_node }
    }

    /// Returns the component label for a graph node, or [`NO_COMPONENT`].
    pub(crate) fn node_component(&self, node: u32) -> u32 {
        self.component_by_node
            .get(node as usize)
            .copied()
            .unwrap_or(NO_COMPONENT)
    }

    /// Returns the components reachable from a building's entrance for this mode.
    pub(crate) fn building_components(
        &self,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
        building_idx: usize,
        transit_flag: u8,
    ) -> BuildingModeComponents {
        if building_idx >= allocator.buildings.len() || building_idx >= allocator.entrances.len() {
            return BuildingModeComponents::default();
        }
        let entrance = &allocator.entrances[building_idx];
        self.entrance_components(entrance, graph, transit_flag)
    }

    /// Broad-phase components for an actual or proposed door; callers still verify directed routes.
    pub(crate) fn entrance_components(
        &self,
        entrance: &crate::simulation::buildings::allocator::BuildingEntrance,
        graph: &RegionGraph,
        transit_flag: u8,
    ) -> BuildingModeComponents {
        if !entrance_supports_mode(entrance, transit_flag) {
            return BuildingModeComponents::default();
        }
        let Some(edge) = graph.get_edge(entrance.edge_idx) else {
            return BuildingModeComponents::default();
        };
        if edge.deleted || (edge.allowed_types & transit_flag) == 0 {
            return BuildingModeComponents::default();
        }

        let mut components = BuildingModeComponents::default();
        components.push(self.node_component(graph.get_valid_node(edge.start_node)));
        components.push(self.node_component(graph.get_valid_node(edge.end_node)));
        components
    }
}

/// Returns the routing chunk for a building or query point.
pub(crate) fn chunk_for_point(x: f32, y: f32) -> (i32, i32) {
    (
        (x / RegionGraph::CHUNK_SIZE).floor() as i32,
        (y / RegionGraph::CHUNK_SIZE).floor() as i32,
    )
}

/// Returns a conservative lower-bound travel time for a squared straight-line distance.
pub(crate) fn lower_bound_travel_seconds(distance_sq: f32, max_speed: f32) -> f32 {
    distance_sq.sqrt() / max_speed.max(1.0)
}

/// Returns the fastest edge speed available to any of the requested transit modes.
pub(crate) fn max_speed_for_modes(graph: &RegionGraph, transit_flags: u8) -> f32 {
    graph
        .edges()
        .iter()
        .filter(|edge| !edge.deleted && (edge.allowed_types & transit_flags) != 0)
        .map(|edge| edge.speed_limit)
        .fold(1.0_f32, f32::max)
}

fn entrance_supports_mode(
    entrance: &crate::simulation::buildings::allocator::BuildingEntrance,
    transit_flag: u8,
) -> bool {
    if transit_flag == TransitFlags::CAR {
        entrance.car_lane_fwd != usize::MAX || entrance.car_lane_bkw != usize::MAX
    } else if transit_flag == TransitFlags::FOOT {
        entrance.foot_lane_fwd != usize::MAX || entrance.foot_lane_bkw != usize::MAX
    } else {
        false
    }
}

/// Visits each chunk on one square ring once, in O(ring) time without allocation.
pub(super) fn scan_ring_chunks(
    origin_chunk: (i32, i32),
    ring: i32,
    mut visit: impl FnMut((i32, i32)),
) {
    if ring == 0 {
        visit(origin_chunk);
        return;
    }

    let min_x = origin_chunk.0 - ring;
    let max_x = origin_chunk.0 + ring;
    let min_y = origin_chunk.1 - ring;
    let max_y = origin_chunk.1 + ring;

    for x in min_x..=max_x {
        visit((x, min_y));
    }
    for y in (min_y + 1)..=(max_y - 1) {
        visit((max_x, y));
    }
    for x in (min_x..=max_x).rev() {
        visit((x, max_y));
    }
    for y in ((min_y + 1)..=(max_y - 1)).rev() {
        visit((min_x, y));
    }
}

/// Returns the squared distance to the nearest chunk bounds on the requested ring.
pub(super) fn min_possible_ring_distance_sq(
    origin_x: f32,
    origin_y: f32,
    origin_chunk: (i32, i32),
    ring: i32,
) -> f32 {
    let mut best = f32::MAX;
    scan_ring_chunks(origin_chunk, ring, |chunk| {
        best = best.min(squared_distance_to_chunk(origin_x, origin_y, chunk));
    });
    best
}

fn squared_distance_to_chunk(origin_x: f32, origin_y: f32, chunk: (i32, i32)) -> f32 {
    let min_x = chunk.0 as f32 * RegionGraph::CHUNK_SIZE;
    let max_x = min_x + RegionGraph::CHUNK_SIZE;
    let min_y = chunk.1 as f32 * RegionGraph::CHUNK_SIZE;
    let max_y = min_y + RegionGraph::CHUNK_SIZE;
    let dx = if origin_x < min_x {
        min_x - origin_x
    } else if origin_x > max_x {
        origin_x - max_x
    } else {
        0.0
    };
    let dy = if origin_y < min_y {
        min_y - origin_y
    } else if origin_y > max_y {
        origin_y - max_y
    } else {
        0.0
    };
    dx * dx + dy * dy
}

fn find(mut i: usize, parent: &mut [usize]) -> usize {
    let mut root = i;
    while parent[root] != root {
        root = parent[root];
    }
    while parent[i] != i {
        let next = parent[i];
        parent[i] = root;
        i = next;
    }
    root
}

fn unite(i: usize, j: usize, parent: &mut [usize]) {
    let root_i = find(i, parent);
    let root_j = find(j, parent);
    if root_i != root_j {
        parent[root_i] = root_j;
    }
}

#[cfg(test)]
mod reservation_tests {
    use super::*;

    #[test]
    fn depleted_candidates_skip_and_restore_without_changing_stable_order() {
        let entries: Vec<_> = (0..4096)
            .map(|item| ReachableBucketEntry::new(0, (0, 0), item))
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries.clone());
        index.enable_reservations();
        assert!(index.has_available());
        let mut components = BuildingModeComponents::default();
        components.push(0);
        for &entry in &entries {
            index.set_available(entry, false);
        }
        let scan = |index: &ReachableBucketIndex| {
            let mut found = Vec::new();
            index.scan_nearest(components, 0.0, 0.0, |event| {
                if let ReachableBucketScanEvent::Item { item_idx } = event {
                    found.push(item_idx);
                }
                true
            });
            found
        };
        assert!(scan(&index).is_empty());
        assert!(!index.has_available());
        for item in [4095, 23, 100] {
            index.set_available(entries[item], true);
        }
        assert_eq!(scan(&index), [23, 100, 4095]);
        assert!(index.has_available());
        index.set_available(entries[23], false);
        index.set_available(entries[100], true);
        assert_eq!(scan(&index), [100, 4095]);
    }

    #[test]
    #[ignore = "manual release locality of finite supplier availability in one dense chunk"]
    fn benchmark_depleted_supplier_bucket() {
        use std::{hint::black_box, time::Instant};
        for count in [1_024, 8_192, 65_536] {
            let entries: Vec<_> = (0..count)
                .map(|item| ReachableBucketEntry::new(0, (0, 0), item))
                .collect();
            let mut index = ReachableBucketIndex::from_entries(entries.clone());
            index.enable_reservations();
            for &entry in &entries[..count - 1] {
                index.set_available(entry, false);
            }
            let mut components = BuildingModeComponents::default();
            components.push(0);
            let mut samples = [0.0; 11];
            for sample in &mut samples {
                let start = Instant::now();
                let mut visited = 0;
                for _ in 0..4096 {
                    index.scan_nearest(components, 0.0, 0.0, |event| {
                        if let ReachableBucketScanEvent::Item { item_idx } = event {
                            assert_eq!(black_box(item_idx), count - 1);
                            visited += 1;
                        }
                        true
                    });
                }
                *sample = start.elapsed().as_secs_f64() * 1000.0;
                assert_eq!(visited, 4096);
            }
            samples.sort_by(f64::total_cmp);
            eprintln!(
                "depleted_supplier_bucket suppliers={count} active=1 queries=4096 median_ms={:.4}",
                samples[5]
            );
        }
    }
}
