// SPDX-License-Identifier: GPL-2.0-only

//! Economy-side reachability helpers for candidate planning.

use std::collections::BTreeMap;
use rayon::prelude::*;

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
    by_component: BTreeMap<u32, usize>,
    components: Vec<ComponentBuckets>,
    buckets: Vec<BucketItems>,
    available: usize,
}

/// Position in a prepared reservation index. Valid until membership is rebuilt; clones and
/// indices built from identical entries preserve positions, independently of availability.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ReachableReservation {
    // A niche keeps optional positions at two words in per-household access storage.
    bucket: std::num::NonZeroUsize,
    position: usize,
}

#[derive(Clone, Debug)]
struct ComponentBuckets {
    by_chunk: BTreeMap<(i32, i32), usize>,
    min_chunk: (i32, i32),
    max_chunk: (i32, i32),
    available: usize,
    // Independent membership census; financial availability changes do not reset it.
    counted: usize,
}

#[derive(Clone, Debug, Default)]
struct BucketItems {
    component: usize,
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

    fn set_available(&mut self, position: usize, available: bool) -> isize {
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
        let mut components = Vec::new();
        let mut buckets = Vec::new();
        for entry in entries {
            let component = *by_component.entry(entry.component).or_insert_with(|| {
                let index = components.len();
                components.push(ComponentBuckets {
                    by_chunk: BTreeMap::new(),
                    min_chunk: entry.chunk,
                    max_chunk: entry.chunk,
                    available: 0,
                    counted: 0,
                });
                index
            });
            let group = &mut components[component];
            group.available += 1;
            group.min_chunk.0 = group.min_chunk.0.min(entry.chunk.0);
            group.min_chunk.1 = group.min_chunk.1.min(entry.chunk.1);
            group.max_chunk.0 = group.max_chunk.0.max(entry.chunk.0);
            group.max_chunk.1 = group.max_chunk.1.max(entry.chunk.1);
            let bucket = *group.by_chunk.entry(entry.chunk).or_insert_with(|| {
                let index = buckets.len();
                buckets.push(BucketItems { component, ..Default::default() });
                index
            });
            buckets[bucket].items.push(entry.item_idx);
        }

        Self {
            by_component,
            components,
            buckets,
            available,
        }
    }

    /// Visits all prepared members of one component, including unavailable entries.
    /// Cost is O(log components + component chunks + component entries), without allocation.
    pub(crate) fn visit_component_members(&self, component: u32, mut visit: impl FnMut(usize)) {
        let Some(&component) = self.by_component.get(&component) else { return; };
        for &bucket in self.components[component].by_chunk.values() {
            for &item in &self.buckets[bucket].items { visit(item); }
        }
    }

    /// Counts an independent subset over existing membership in O(indexed entries).
    /// Reservation updates preserve these counts until the caller starts another census.
    pub(crate) fn reset_member_counts(&mut self, counted: impl Fn(usize) -> bool) {
        for component in &mut self.components { component.counted = 0; }
        for bucket in &self.buckets {
            self.components[bucket.component].counted += bucket.items.iter()
                .filter(|&&item| counted(item)).count();
        }
    }

    /// Removes one previously counted membership in O(log components), returning true
    /// only when its component becomes empty. The caller owns membership eligibility.
    pub(crate) fn remove_counted_member(&mut self, component: u32) -> bool {
        let Some(&component) = self.by_component.get(&component) else { return false; };
        let count = &mut self.components[component].counted;
        *count = count.checked_sub(1).unwrap();
        *count == 0
    }

    /// Whether either entrance component retains counted members, in O(log components).
    pub(crate) fn has_counted_members(&self, components: BuildingModeComponents) -> bool {
        components.as_slice().iter().any(|component| self.by_component.get(component)
            .is_some_and(|&component| self.components[component].counted > 0))
    }

    /// Whether any indexed entry remains eligible, in O(1).
    pub(crate) fn has_available(&self) -> bool {
        self.available > 0
    }

    /// Enables allocation-free availability updates for finite reservation users.
    /// Immutable indices retain their existing flat traversal without tree overhead.
    pub(crate) fn enable_reservations(&mut self) {
        self.buckets.par_iter_mut().for_each(BucketItems::enable_reservations);
    }

    /// Refills a prepared reservation tree in O(indexed entries), retaining all allocations.
    /// The predicate is evaluated once per entry, before dependent parent counts are summed.
    pub(crate) fn reset_available(&mut self, mut eligible: impl FnMut(usize) -> bool) {
        self.available = 0;
        self.components.iter_mut().for_each(|component| component.available = 0);
        // Predicate calls and shared component totals retain their deterministic order.
        for items in &mut self.buckets {
            debug_assert!(items.leaves > 0);
            for (index, &item) in items.items.iter().enumerate() {
                items.available[items.leaves + index] = usize::from(eligible(item));
            }
            for node in (1..items.leaves).rev() {
                items.available[node] = items.available[node * 2] + items.available[node * 2 + 1];
            }
            self.components[items.component].available += items.available[1];
            self.available += items.available[1];
        }
    }

    /// Toggles one candidate in O(log chunks + log candidates-in-chunk), without allocation.
    /// Requires `enable_reservations`; membership and spatial bounds remain unchanged.
    pub(crate) fn set_available(&mut self, entry: ReachableBucketEntry, available: bool) {
        if let Some(reservation) = self.reservation(entry) {
            self.set_reserved_available(reservation, available);
        }
    }

    /// Resolves immutable membership once in O(log components + log chunks + log bucket size).
    /// Requires a prepared reservation tree; callers rebuild positions with the index.
    pub(crate) fn reservation(&self, entry: ReachableBucketEntry) -> Option<ReachableReservation> {
        let component = *self.by_component.get(&entry.component)?;
        let bucket = *self.components[component].by_chunk.get(&entry.chunk)?;
        let items = &self.buckets[bucket];
        debug_assert!(items.leaves > 0);
        Some(ReachableReservation { bucket: std::num::NonZeroUsize::new(bucket + 1)?,
            position: items.items.binary_search(&entry.item_idx).ok()? })
    }

    /// Toggles prepared membership with an O(1) lookup and O(log bucket size) tree update.
    /// Unchanged availability returns in O(1); no spatial lookup or allocation is performed.
    pub(crate) fn set_reserved_available(&mut self, reservation: ReachableReservation, available: bool) {
        let items = &mut self.buckets[reservation.bucket.get() - 1];
        let change = items.set_available(reservation.position, available);
        if change != 0 {
            let component = &mut self.components[items.component];
            component.available = component.available.checked_add_signed(change).unwrap();
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
        let Some((max_ring, mut remaining)) = self.scan_extent(components, origin_chunk) else {
            return;
        };

        for ring in 0..=max_ring {
            let mut should_continue = true;
            scan_ring_chunks(origin_chunk, ring, |chunk| {
                if !should_continue {
                    return;
                }
                for &component in components.as_slice() {
                    let Some(&component) = self.by_component.get(&component) else {
                        continue;
                    };
                    let Some(&bucket) = self.components[component].by_chunk.get(&chunk) else {
                        continue;
                    };
                    if !self.buckets[bucket].visit(|item_idx| {
                        remaining -= 1;
                        visitor(ReachableBucketScanEvent::Item { item_idx }) && remaining > 0
                    })
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

    // Counts include entries repeated across components, exactly as the bucket traversal does.
    // Exhausting them avoids scanning distant inactive geometry after the final active item.
    fn scan_extent(
        &self,
        components: BuildingModeComponents,
        origin_chunk: (i32, i32),
    ) -> Option<(i32, usize)> {
        let mut max_ring = None::<i32>;
        let mut available = 0;
        for &component in components.as_slice() {
            let Some(&component) = self.by_component.get(&component) else {
                continue;
            };
            let buckets = &self.components[component];
            if buckets.available == 0 {
                continue;
            }
            available += buckets.available;
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
        max_ring.map(|ring| (ring, available))
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
    fn component_member_counts_are_independent_of_financial_availability() {
        let mut index = ReachableBucketIndex::from_entries(vec![
            ReachableBucketEntry::new(0, (0, 0), 0),
            ReachableBucketEntry::new(0, (1, 0), 1),
            ReachableBucketEntry::new(1, (2, 0), 1),
            ReachableBucketEntry::new(1, (2, 0), 2),
        ]);
        index.enable_reservations();
        let mut a = BuildingModeComponents::default(); a.push(0);
        let mut b = BuildingModeComponents::default(); b.push(1);
        index.reset_member_counts(|item| item < 2);
        assert!(index.has_counted_members(a) && index.has_counted_members(b));
        assert!(!index.remove_counted_member(0));
        assert!(index.remove_counted_member(0));
        assert!(!index.has_counted_members(a) && index.has_counted_members(b));
        assert!(index.has_available());
        index.reset_available(|_| false);
        assert!(!index.has_available() && index.has_counted_members(b));
        let mut members = Vec::new();
        index.visit_component_members(1, |item| members.push(item));
        assert_eq!(members, [1, 2]);
        assert!(index.remove_counted_member(1));
        index.reset_member_counts(|item| item == 2);
        assert!(!index.has_counted_members(a) && index.has_counted_members(b));
    }

    #[test]
    fn depleted_candidates_skip_and_restore_without_changing_stable_order() {
        let entries: Vec<_> = (0..4096)
            .map(|item| ReachableBucketEntry::new(0, (0, 0), item))
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries.clone());
        index.enable_reservations();
        let reservations: Vec<_> = entries.iter().map(|&entry| index.reservation(entry).unwrap()).collect();
        assert!(index.has_available());
        let mut components = BuildingModeComponents::default();
        components.push(0);
        for &reservation in &reservations {
            index.set_reserved_available(reservation, false);
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
        let mut fork = index.clone();
        fork.reset_available(|_| false);
        for item in [4095, 23, 100] {
            fork.set_reserved_available(reservations[item], true);
        }
        assert_eq!(scan(&fork), [23, 100, 4095]);
        assert_eq!(scan(&index), [100, 4095], "forked availability remains independent");
    }

    #[test]
    fn inactive_background_does_not_extend_the_active_candidate_scan() {
        let mut components = BuildingModeComponents::default();
        components.push(0);
        components.push(1);
        for background in [0, 16, 256, 4_096] {
            let near = [ReachableBucketEntry::new(0, (0, 0), 0),
                ReachableBucketEntry::new(1, (0, 0), 0),
                ReachableBucketEntry::new(1, (2, 0), 1)];
            let entries = near.into_iter().chain((0..background)
                .map(|i| ReachableBucketEntry::new((i % 2) as u32, (1_024 + i as i32, 0), i + 2)))
                .collect();
            let mut index = ReachableBucketIndex::from_entries(entries);
            index.enable_reservations();
            index.reset_available(|item| item < 2);
            for remote_active in [true, false, true] {
                index.set_available(near[2], remote_active);
                let mut found = Vec::new();
                let mut rings = 0;
                index.scan_nearest(components, 0.0, 0.0, |event| {
                    match event {
                        ReachableBucketScanEvent::Item { item_idx } => found.push(item_idx),
                        ReachableBucketScanEvent::RingComplete { .. } => rings += 1,
                    }
                    true
                });
                assert_eq!(found, if remote_active { vec![0, 0, 1] } else { vec![0, 0] });
                assert_eq!(rings, if remote_active { 2 } else { 0 },
                    "inactive entries may not make a completed scan visit empty outer rings");
            }
        }
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

    #[test]
    #[ignore = "manual release locality with fixed active neighborhood and distant inactive inputs"]
    fn benchmark_inactive_supplier_extent() {
        use std::{hint::black_box, time::Instant};
        for count in [1_024, 8_192, 65_536] {
            let entries = std::iter::once(ReachableBucketEntry::new(0, (0, 0), 0))
                .chain((1..count).map(|item| ReachableBucketEntry::new(0,
                    (4_096 + item as i32 / 1_024, 0), item))).collect();
            let mut index = ReachableBucketIndex::from_entries(entries);
            index.enable_reservations();
            index.reset_available(|item| item == 0);
            let mut components = BuildingModeComponents::default();
            components.push(0);
            let mut samples = [0.0; 11];
            for sample in &mut samples {
                let start = Instant::now();
                let mut visited = 0;
                for _ in 0..4_096 {
                    index.scan_nearest(components, 0.0, 0.0, |event| {
                        let ReachableBucketScanEvent::Item { item_idx } = event else {
                            panic!("one local active entry must finish inside its own ring");
                        };
                        assert_eq!(black_box(item_idx), 0);
                        visited += 1;
                        true
                    });
                }
                assert_eq!(visited, 4_096);
                *sample = start.elapsed().as_secs_f64() * 1_000.0;
            }
            samples.sort_by(f64::total_cmp);
            eprintln!("inactive_supplier_extent entries={count} active=1 queries=4096 median_ms={:.4}", samples[5]);
        }
    }
}
