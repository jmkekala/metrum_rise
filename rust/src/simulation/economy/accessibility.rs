// SPDX-License-Identifier: GPL-2.0-only

//! Economy-side reachability helpers for candidate planning.

use rayon::prelude::*;
use std::collections::BTreeMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

/// Sentinel component id for nodes or buildings with no usable access.
pub(crate) const NO_COMPONENT: u32 = u32::MAX;

// Entries from which a bucket index sorts them on the pool (`SIM-02-K`).
const PARALLEL_SORT_ENTRIES: usize = 4096;

/// Connected-component labels for one transit mode; the default labels no node.
#[derive(Clone, Debug, Default)]
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

/// Event emitted by [`ReachableBucketIndex::scan_nearest_bounded`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum BoundedScanEvent {
    /// A candidate item was found; `false` ends the scan.
    Item {
        /// Caller-owned candidate index.
        item_idx: usize,
    },
    /// A subtree of one chunk bucket may follow; `false` skips it, never ending the scan.
    Region {
        /// Squared distance to the subtree's door bounds, never above any member's own.
        min_distance_sq: f32,
    },
    /// A ring finished; the distance is a lower bound for every later ring.
    RingComplete {
        /// Squared world-space distance to the next ring's closest chunk bounds.
        next_min_distance_sq: f32,
    },
}

// One step of the shared outward chunk-ring traversal.
enum RingStep<'a> {
    Bucket(&'a BucketItems),
    Complete(f32),
}

struct ScanExtent {
    max_ring: i32,
    available: usize,
    min_chunk: (i32, i32),
    max_chunk: (i32, i32),
}

// Covers no position; parents of padding leaves keep their real children's bounds.
const EMPTY_BOUNDS: [f32; 4] = [
    f32::INFINITY,
    f32::INFINITY,
    f32::NEG_INFINITY,
    f32::NEG_INFINITY,
];
// A member without a door position must never be pruned spatially.
const UNBOUNDED: [f32; 4] = [
    f32::NEG_INFINITY,
    f32::NEG_INFINITY,
    f32::INFINITY,
    f32::INFINITY,
];

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
    // Freshly summed upper bound carried by counted members; stale-high until recounted.
    counted_units: f64,
    units_dirty: bool,
    // Optional class counts avoid traversing empty city-wide chunk rings. Independent
    // quantity depletion may leave a stale-high mask, which only reduces pruning.
    query_counts: Vec<usize>,
    query_mask: u64,
}

impl ComponentBuckets {
    fn change_query_mask(&mut self, before: u64, after: u64) {
        let mut changed = before ^ after;
        while changed != 0 {
            let bit = changed.trailing_zeros() as usize;
            let flag = 1u64 << bit;
            changed &= changed - 1;
            if after & flag != 0 {
                self.query_counts[bit] += 1;
                self.query_mask |= flag;
            } else {
                self.query_counts[bit] -= 1;
                if self.query_counts[bit] == 0 {
                    self.query_mask &= !flag;
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct BucketItems {
    component: usize,
    items: Vec<usize>,
    // Empty for ordinary immutable indices. Finite reservation users opt into an
    // availability tree, avoiding a linear walk over depleted suppliers on every query.
    available: Vec<usize>,
    leaves: usize,
    // Optional [min x, min y, max x, max y] door bounds per availability-tree node.
    bounds: Vec<[f32; 4]>,
    // Optional caller-owned eligibility classes, OR-reduced over the same reservation tree.
    query_masks: Vec<u64>,
}

impl BucketItems {
    fn visit_masked(
        &self,
        node: usize,
        mask: u64,
        remaining: &mut usize,
        visitor: &mut impl FnMut(ReachableBucketScanEvent) -> bool,
    ) -> bool {
        let available = self.available[node];
        if available == 0 {
            return true;
        }
        if self.query_masks[node] & mask == 0 {
            *remaining -= available;
            return *remaining > 0;
        }
        if node >= self.leaves {
            *remaining -= 1;
            return visitor(ReachableBucketScanEvent::Item {
                item_idx: self.items[node - self.leaves],
            }) && *remaining > 0;
        }
        self.visit_masked(node * 2, mask, remaining, visitor)
            && self.visit_masked(node * 2 + 1, mask, remaining, visitor)
    }

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
        let change = next as isize - previous as isize;
        node /= 2;
        while node > 0 {
            // Exactly one Boolean leaf changed. Every ancestor has the same unit delta;
            // removal has a live descendant, and insertion has an empty one, so counts fit.
            self.available[node] = self.available[node].wrapping_add_signed(change);
            node /= 2;
        }
        change
    }

    fn enable_bounds(&mut self, door: &impl Fn(usize) -> Option<[f32; 2]>) {
        debug_assert!(self.leaves > 0);
        self.bounds.clear();
        self.bounds.resize(self.leaves * 2, EMPTY_BOUNDS);
        for (index, &item) in self.items.iter().enumerate() {
            self.bounds[self.leaves + index] = door(item).map_or(UNBOUNDED, |[x, y]| [x, y, x, y]);
        }
        for node in (1..self.leaves).rev() {
            let [a, b] = [self.bounds[node * 2], self.bounds[node * 2 + 1]];
            self.bounds[node] = [
                a[0].min(b[0]),
                a[1].min(b[1]),
                a[2].max(b[2]),
                a[3].max(b[3]),
            ];
        }
    }

    // Same leaf order as `visit_available`. A rejected region drops its whole availability
    // count from `remaining`, as visiting and rejecting each member would have done.
    fn visit_bounded(
        &self,
        node: usize,
        origin: [f32; 2],
        remaining: &mut usize,
        visitor: &mut impl FnMut(BoundedScanEvent) -> bool,
    ) -> bool {
        let available = self.available[node];
        if available == 0 {
            return true;
        }
        if node >= self.leaves {
            *remaining -= 1;
            return visitor(BoundedScanEvent::Item {
                item_idx: self.items[node - self.leaves],
            }) && *remaining > 0;
        }
        if available > 1
            && !visitor(BoundedScanEvent::Region {
                min_distance_sq: min_distance_sq(self.bounds[node], origin),
            })
        {
            *remaining -= available;
            return *remaining > 0;
        }
        self.visit_bounded(node * 2, origin, remaining, visitor)
            && self.visit_bounded(node * 2 + 1, origin, remaining, visitor)
    }

    fn visit(&self, mut visitor: impl FnMut(usize) -> bool) -> bool {
        if self.available.is_empty() {
            return self.items.iter().all(|&item| visitor(item));
        }
        self.visit_available(1, 0, self.leaves, &mut visitor)
    }

    fn visit_available(
        &self,
        node: usize,
        first: usize,
        width: usize,
        visitor: &mut impl FnMut(usize) -> bool,
    ) -> bool {
        let available = self.available[node];
        if available == 0 {
            return true;
        }
        if available == width {
            // Every real leaf is eligible; preserve its order and early stop without
            // descending through two tree nodes per item. Padding can never be available.
            return self.items[first..first + width]
                .iter()
                .all(|&item| visitor(item));
        }
        // Partial subtrees retain the original pruning: O(K) for a full bucket and
        // O(log K) for one survivor, without another index or any query allocation.
        let half = width / 2;
        self.visit_available(node * 2, first, half, visitor)
            && self.visit_available(node * 2 + 1, first + half, half, visitor)
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

    /// Inverse of [`Self::raw_parts`], for saved components; `None` for a count over two.
    pub(crate) fn from_raw_parts(components: [u32; 2], count: usize) -> Option<Self> {
        (count <= components.len()).then_some(Self {
            components,
            count: count as u8,
        })
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

    /// Component, routing chunk and caller-owned item index.
    pub(crate) fn parts(self) -> (u32, (i32, i32), usize) {
        (self.component, self.chunk, self.item_idx)
    }
}

impl ReachableBucketIndex {
    /// Builds deterministic component/chunk buckets from caller-owned candidate entries.
    pub(crate) fn from_entries(mut entries: Vec<ReachableBucketEntry>) -> Self {
        entries.retain(|entry| entry.component != NO_COMPONENT);
        // Equal entries are identical, so the parallel sort orders them as the serial one.
        if entries.len() >= PARALLEL_SORT_ENTRIES {
            entries.par_sort_unstable();
        } else {
            entries.sort_unstable();
        }
        entries.dedup();
        let available = entries.len();

        let mut by_component = BTreeMap::new();
        let mut components: Vec<ComponentBuckets> = Vec::new();
        let mut buckets: Vec<BucketItems> = Vec::new();
        // Sorted entries arrive bucket by bucket: only a bucket's first entry needs the maps.
        let mut last = None::<(u32, (i32, i32), usize)>;
        for entry in entries {
            if let Some((component, chunk, bucket)) = last
                && component == entry.component
                && chunk == entry.chunk
            {
                components[buckets[bucket].component].available += 1;
                buckets[bucket].items.push(entry.item_idx);
                continue;
            }
            let component = *by_component.entry(entry.component).or_insert_with(|| {
                let index = components.len();
                components.push(ComponentBuckets {
                    by_chunk: BTreeMap::new(),
                    min_chunk: entry.chunk,
                    max_chunk: entry.chunk,
                    available: 0,
                    counted: 0,
                    counted_units: 0.0,
                    units_dirty: false,
                    query_counts: Vec::new(),
                    query_mask: 0,
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
                buckets.push(BucketItems {
                    component,
                    ..Default::default()
                });
                index
            });
            buckets[bucket].items.push(entry.item_idx);
            last = Some((entry.component, entry.chunk, bucket));
        }

        Self {
            by_component,
            components,
            buckets,
            available,
        }
    }

    /// Visits every prepared entry in component, chunk and item order, in O(entries).
    pub(crate) fn visit_entries(&self, mut visit: impl FnMut(ReachableBucketEntry)) {
        for (&component, &group) in &self.by_component {
            for (&chunk, &bucket) in &self.components[group].by_chunk {
                for &item in &self.buckets[bucket].items {
                    visit(ReachableBucketEntry::new(component, chunk, item));
                }
            }
        }
    }

    /// Visits all prepared members of one component, including unavailable entries.
    /// Cost is O(log components + component chunks + component entries), without allocation.
    pub(crate) fn visit_component_members(&self, component: u32, mut visit: impl FnMut(usize)) {
        let Some(&component) = self.by_component.get(&component) else {
            return;
        };
        for &bucket in self.components[component].by_chunk.values() {
            for &item in &self.buckets[bucket].items {
                visit(item);
            }
        }
    }

    /// Counts an independent subset over existing membership in O(indexed entries), with
    /// each counted member's units. Reservation updates preserve these counts until the
    /// caller starts another census.
    pub(crate) fn reset_member_counts(&mut self, counted: impl Fn(usize) -> Option<f64>) {
        for component in &mut self.components {
            component.counted = 0;
            component.counted_units = 0.0;
            component.units_dirty = false;
        }
        // Bucket order is component/chunk order, matching `recount_member_units`.
        for bucket in &self.buckets {
            let component = &mut self.components[bucket.component];
            for units in bucket.items.iter().filter_map(|&item| counted(item)) {
                component.counted += 1;
                component.counted_units += units;
            }
        }
    }

    /// Marks one component's unit total stale in O(log components); true when newly marked.
    pub(crate) fn mark_member_units(&mut self, component: u32) -> bool {
        let Some(&component) = self.by_component.get(&component) else {
            return false;
        };
        !std::mem::replace(&mut self.components[component].units_dirty, true)
    }

    /// Replaces one component's unit total with a fresh ordered sum, never an incremental
    /// difference. O(log components + component chunks + component entries), no allocation.
    pub(crate) fn recount_member_units(
        &mut self,
        component: u32,
        counted: impl Fn(usize) -> Option<f64>,
    ) {
        let Some(&component) = self.by_component.get(&component) else {
            return;
        };
        let mut total = 0.0;
        for &bucket in self.components[component].by_chunk.values() {
            total += self.buckets[bucket]
                .items
                .iter()
                .filter_map(|&item| counted(item))
                .sum::<f64>();
        }
        self.components[component].counted_units = total;
        self.components[component].units_dirty = false;
    }

    /// Counted members and their units in either entrance component, in O(log components).
    pub(crate) fn counted_units(&self, components: BuildingModeComponents) -> (usize, f64) {
        components
            .as_slice()
            .iter()
            .filter_map(|component| self.by_component.get(component))
            .fold((0, 0.0), |(count, units), &component| {
                (
                    count + self.components[component].counted,
                    units + self.components[component].counted_units,
                )
            })
    }

    /// Removes one previously counted membership in O(log components), returning true
    /// only when its component becomes empty. The caller owns membership eligibility.
    pub(crate) fn remove_counted_member(&mut self, component: u32) -> bool {
        let Some(&component) = self.by_component.get(&component) else {
            return false;
        };
        let count = &mut self.components[component].counted;
        *count = count.checked_sub(1).unwrap();
        *count == 0
    }

    /// Whether either entrance component retains counted members, in O(log components).
    pub(crate) fn has_counted_members(&self, components: BuildingModeComponents) -> bool {
        components.as_slice().iter().any(|component| {
            self.by_component
                .get(component)
                .is_some_and(|&component| self.components[component].counted > 0)
        })
    }

    /// Whether any indexed entry remains eligible, in O(1).
    pub(crate) fn has_available(&self) -> bool {
        self.available > 0
    }

    /// Enables allocation-free availability updates for finite reservation users.
    /// Immutable indices retain their existing flat traversal without tree overhead.
    pub(crate) fn enable_reservations(&mut self) {
        self.buckets
            .par_iter_mut()
            .for_each(BucketItems::enable_reservations);
    }

    /// Adds door bounds for [`Self::scan_nearest_bounded`] in O(indexed entries). Requires
    /// `enable_reservations`; bounds are geometric, so availability updates keep them valid.
    pub(crate) fn enable_spatial_bounds(
        &mut self,
        door: impl Fn(usize) -> Option<[f32; 2]> + Sync,
    ) {
        self.buckets
            .par_iter_mut()
            .for_each(|items| items.enable_bounds(&door));
    }

    /// Prepares caller-defined eligibility classes in O(entries), retaining tree storage.
    /// Requires reservations; class masks remain independent of quantity availability.
    pub(crate) fn reset_query_masks(&mut self, mask: impl Fn(usize) -> u64) {
        for component in &mut self.components {
            component.query_counts.resize(64, 0);
            component.query_counts.fill(0);
            component.query_mask = 0;
        }
        for items in &mut self.buckets {
            items.query_masks.resize(items.leaves * 2, 0);
            for (index, &item) in items.items.iter().enumerate() {
                let mask = mask(item);
                items.query_masks[items.leaves + index] = mask;
                self.components[items.component].change_query_mask(0, mask);
            }
            for node in (1..items.leaves).rev() {
                items.query_masks[node] =
                    items.query_masks[node * 2] | items.query_masks[node * 2 + 1];
            }
        }
    }

    /// Changes one prepared eligibility mask in O(log chunks + log bucket size).
    pub(crate) fn set_query_mask(&mut self, entry: ReachableBucketEntry, mask: u64) {
        let Some(reservation) = self.reservation(entry) else {
            return;
        };
        let items = &mut self.buckets[reservation.bucket.get() - 1];
        let mut node = items.leaves + reservation.position;
        if items.query_masks[node] == mask {
            return;
        }
        self.components[items.component].change_query_mask(items.query_masks[node], mask);
        items.query_masks[node] = mask;
        node /= 2;
        while node > 0 {
            let combined = items.query_masks[node * 2] | items.query_masks[node * 2 + 1];
            if items.query_masks[node] == combined {
                break;
            }
            items.query_masks[node] = combined;
            node /= 2;
        }
    }

    /// Refills a prepared reservation tree in O(indexed entries), retaining all allocations.
    /// The predicate is evaluated once per entry, before dependent parent counts are summed.
    pub(crate) fn reset_available(&mut self, mut eligible: impl FnMut(usize) -> bool) {
        self.available = 0;
        self.components
            .iter_mut()
            .for_each(|component| component.available = 0);
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
        Some(ReachableReservation {
            bucket: std::num::NonZeroUsize::new(bucket + 1)?,
            position: items.items.binary_search(&entry.item_idx).ok()?,
        })
    }

    /// Toggles prepared membership with an O(1) lookup and O(log bucket size) tree update.
    /// Unchanged availability returns in O(1); no spatial lookup or allocation is performed.
    pub(crate) fn set_reserved_available(
        &mut self,
        reservation: ReachableReservation,
        available: bool,
    ) {
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
        visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        Self::scan_nearest_over(&[self], components, origin_x, origin_y, visitor);
    }

    // Disjoint indices whose item ids ascend from one index to the next scan exactly as one
    // merged index: each chunk/component visits its buckets in index order, and early stop
    // counts every index's available entries.
    fn scan_nearest_over(
        indices: &[&Self],
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mut visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        Self::scan_rings_over(
            indices,
            components,
            origin_x,
            origin_y,
            |step, remaining| match step {
                RingStep::Bucket(items) => items.visit(|item_idx| {
                    *remaining -= 1;
                    visitor(ReachableBucketScanEvent::Item { item_idx }) && *remaining > 0
                }),
                RingStep::Complete(next_min_distance_sq) => {
                    visitor(ReachableBucketScanEvent::RingComplete {
                        next_min_distance_sq,
                    })
                }
            },
        );
    }

    /// Preserves nearest-scan item order while skipping subtrees with no requested class.
    /// `None` or unprepared masks use the ordinary scan; filtering never changes membership.
    #[cfg(test)]
    pub(crate) fn scan_nearest_masked(
        &self,
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mask: Option<u64>,
        visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        Self::scan_nearest_masked_over(&[self], components, origin_x, origin_y, mask, visitor);
    }

    /// [`Self::scan_nearest_masked`] over disjoint indices whose item ids ascend from one
    /// index to the next, in the exact event order of one index holding every entry.
    pub(crate) fn scan_nearest_masked_over(
        indices: &[&Self],
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mask: Option<u64>,
        visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        let Some(mask) = mask else {
            return Self::scan_nearest_over(indices, components, origin_x, origin_y, visitor);
        };
        if !indices.iter().any(|index| {
            components.as_slice().iter().any(|component| {
                index.by_component.get(component).is_some_and(|&group| {
                    index.components[group].query_counts.is_empty()
                        || index.components[group].query_mask & mask != 0
                })
            })
        }) {
            return;
        }
        Self::scan_masked_buckets(indices, components, origin_x, origin_y, mask, visitor);
    }

    fn scan_masked_buckets(
        indices: &[&Self],
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mask: u64,
        mut visitor: impl FnMut(ReachableBucketScanEvent) -> bool,
    ) {
        Self::scan_rings_over(
            indices,
            components,
            origin_x,
            origin_y,
            |step, remaining| match step {
                RingStep::Bucket(items) if !items.query_masks.is_empty() => {
                    items.visit_masked(1, mask, remaining, &mut visitor)
                }
                RingStep::Bucket(items) => items.visit(|item_idx| {
                    *remaining -= 1;
                    visitor(ReachableBucketScanEvent::Item { item_idx }) && *remaining > 0
                }),
                RingStep::Complete(next_min_distance_sq) => {
                    visitor(ReachableBucketScanEvent::RingComplete {
                        next_min_distance_sq,
                    })
                }
            },
        );
    }

    /// [`Self::scan_nearest`] that also offers each multi-member subtree of a chunk bucket
    /// as a [`BoundedScanEvent::Region`] when `enable_spatial_bounds` prepared it. Items keep
    /// their traversal order; a caller must reject a region only when every member's own
    /// distance test would reject it. Unprepared buckets visit every available item.
    pub(crate) fn scan_nearest_bounded(
        &self,
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mut visitor: impl FnMut(BoundedScanEvent) -> bool,
    ) {
        self.scan_rings(
            components,
            origin_x,
            origin_y,
            |step, remaining| match step {
                RingStep::Bucket(items) if !items.bounds.is_empty() => {
                    items.visit_bounded(1, [origin_x, origin_y], remaining, &mut visitor)
                }
                RingStep::Bucket(items) => items.visit(|item_idx| {
                    *remaining -= 1;
                    visitor(BoundedScanEvent::Item { item_idx }) && *remaining > 0
                }),
                RingStep::Complete(next_min_distance_sq) => {
                    visitor(BoundedScanEvent::RingComplete {
                        next_min_distance_sq,
                    })
                }
            },
        );
    }

    // Outward ring traversal shared by both scans; a `false` step ends the scan.
    fn scan_rings(
        &self,
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        step: impl FnMut(RingStep<'_>, &mut usize) -> bool,
    ) {
        Self::scan_rings_over(&[self], components, origin_x, origin_y, step);
    }

    // Ring traversal over disjoint indices. Ring chunk order is fixed geometry clipped to
    // the union envelope, so chunks holding available entries keep their relative order.
    fn scan_rings_over(
        indices: &[&Self],
        components: BuildingModeComponents,
        origin_x: f32,
        origin_y: f32,
        mut step: impl FnMut(RingStep<'_>, &mut usize) -> bool,
    ) {
        let origin_chunk = chunk_for_point(origin_x, origin_y);
        let Some(extent) = indices
            .iter()
            .filter_map(|index| index.scan_extent(components, origin_chunk))
            .reduce(|a, b| ScanExtent {
                max_ring: a.max_ring.max(b.max_ring),
                available: a.available + b.available,
                min_chunk: (
                    a.min_chunk.0.min(b.min_chunk.0),
                    a.min_chunk.1.min(b.min_chunk.1),
                ),
                max_chunk: (
                    a.max_chunk.0.max(b.max_chunk.0),
                    a.max_chunk.1.max(b.max_chunk.1),
                ),
            })
        else {
            return;
        };
        let max_ring = extent.max_ring;
        let mut remaining = extent.available;

        for ring in 0..=max_ring {
            let mut should_continue = true;
            // No requested component has a live bucket outside this existing envelope.
            // Keep every ring-completion event, but avoid empty out-of-envelope lookups.
            scan_ring_chunks_in_bounds(
                origin_chunk,
                ring,
                extent.min_chunk,
                extent.max_chunk,
                |chunk| {
                    if !should_continue {
                        return;
                    }
                    for &component in components.as_slice() {
                        for index in indices {
                            let Some(&group) = index.by_component.get(&component) else {
                                continue;
                            };
                            let Some(&bucket) = index.components[group].by_chunk.get(&chunk) else {
                                continue;
                            };
                            if !step(RingStep::Bucket(&index.buckets[bucket]), &mut remaining) {
                                should_continue = false;
                                return;
                            }
                        }
                    }
                },
            );
            if !should_continue {
                return;
            }

            if ring < max_ring {
                let next_min_distance_sq =
                    min_possible_ring_distance_sq(origin_x, origin_y, origin_chunk, ring + 1);
                if !step(RingStep::Complete(next_min_distance_sq), &mut remaining) {
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
    ) -> Option<ScanExtent> {
        let mut max_ring = None::<i32>;
        let mut available = 0;
        let mut min_chunk = (i32::MAX, i32::MAX);
        let mut max_chunk = (i32::MIN, i32::MIN);
        for &component in components.as_slice() {
            let Some(&component) = self.by_component.get(&component) else {
                continue;
            };
            let buckets = &self.components[component];
            if buckets.available == 0 {
                continue;
            }
            available += buckets.available;
            min_chunk.0 = min_chunk.0.min(buckets.min_chunk.0);
            min_chunk.1 = min_chunk.1.min(buckets.min_chunk.1);
            max_chunk.0 = max_chunk.0.max(buckets.max_chunk.0);
            max_chunk.1 = max_chunk.1.max(buckets.max_chunk.1);
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
        max_ring.map(|max_ring| ScanExtent {
            max_ring,
            available,
            min_chunk,
            max_chunk,
        })
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

// Each axis term rounds monotonically like a member's own `door - origin` component, so
// this never exceeds a member's squared distance.
fn min_distance_sq(bounds: [f32; 4], [x, y]: [f32; 2]) -> f32 {
    let dx = (bounds[0] - x).max(x - bounds[2]).max(0.0);
    let dy = (bounds[1] - y).max(y - bounds[3]).max(0.0);
    dx * dx + dy * dy
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

/// Returns a speed ceiling for the requested routing modes, including walking access legs.
pub(crate) fn max_speed_for_modes(graph: &RegionGraph, transit_flags: u8) -> f32 {
    if transit_flags == TransitFlags::FOOT {
        return crate::config::AGENT_WALK_SPEED_MS;
    }
    graph
        .edges()
        .par_iter()
        .filter(|edge| !edge.deleted && (edge.allowed_types & transit_flags) != 0)
        .map(|edge| edge.speed_limit.max(crate::config::MIN_ROUTE_SPEED_MS))
        .reduce(|| crate::config::AGENT_WALK_SPEED_MS, f32::max)
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
pub(super) fn scan_ring_chunks(origin_chunk: (i32, i32), ring: i32, visit: impl FnMut((i32, i32))) {
    scan_ring_chunks_in_bounds(
        origin_chunk,
        ring,
        (i32::MIN, i32::MIN),
        (i32::MAX, i32::MAX),
        visit,
    );
}

// O(1 + intersecting perimeter chunks), preserving the original clockwise traversal.
fn scan_ring_chunks_in_bounds(
    origin_chunk: (i32, i32),
    ring: i32,
    lower: (i32, i32),
    upper: (i32, i32),
    mut visit: impl FnMut((i32, i32)),
) {
    if ring == 0 {
        if origin_chunk.0 >= lower.0
            && origin_chunk.0 <= upper.0
            && origin_chunk.1 >= lower.1
            && origin_chunk.1 <= upper.1
        {
            visit(origin_chunk);
        }
        return;
    }

    let min_x = origin_chunk.0 - ring;
    let max_x = origin_chunk.0 + ring;
    let min_y = origin_chunk.1 - ring;
    let max_y = origin_chunk.1 + ring;

    let xs = min_x.max(lower.0)..=max_x.min(upper.0);
    let ys = (min_y + 1).max(lower.1)..=(max_y - 1).min(upper.1);
    if min_y >= lower.1 && min_y <= upper.1 {
        for x in xs.clone() {
            visit((x, min_y));
        }
    }
    if max_x >= lower.0 && max_x <= upper.0 {
        for y in ys.clone() {
            visit((max_x, y));
        }
    }
    if max_y >= lower.1 && max_y <= upper.1 {
        for x in xs.rev() {
            visit((x, max_y));
        }
    }
    if min_x >= lower.0 && min_x <= upper.0 {
        for y in ys.rev() {
            visit((min_x, y));
        }
    }
}

/// Returns the squared distance to the nearest chunk bounds on the requested ring.
/// O(1) for an origin inside its supplied chunk; unusual external inputs retain the scan.
pub(super) fn min_possible_ring_distance_sq(
    origin_x: f32,
    origin_y: f32,
    origin_chunk: (i32, i32),
    ring: i32,
) -> f32 {
    if ring >= 0
        && origin_x.is_finite()
        && origin_y.is_finite()
        && squared_distance_to_chunk(origin_x, origin_y, origin_chunk) == 0.0
    {
        // Each side's aligned chunk has zero distance on the other axis. Every other
        // chunk on that side adds a nonnegative squared distance to the identical edge
        // distance. Evaluate the original arithmetic at these four exact minimizers.
        return [
            (origin_chunk.0, origin_chunk.1 - ring),
            (origin_chunk.0 + ring, origin_chunk.1),
            (origin_chunk.0, origin_chunk.1 + ring),
            (origin_chunk.0 - ring, origin_chunk.1),
        ]
        .into_iter()
        .map(|chunk| squared_distance_to_chunk(origin_x, origin_y, chunk))
        .fold(f32::MAX, f32::min);
    }
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
    fn walking_speed_bound_ignores_slow_edge_speed_limits() {
        use crate::simulation::network::graph::Edge;
        use crate::simulation::network::types::NodeType;
        use crate::simulation::pathing::cost::CostCalculator;
        use godot::prelude::Vector3;

        let mut graph = RegionGraph::new();
        let start = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let end = graph.add_node(Vector3::new(140.0, 0.0, 0.0), NodeType::Junction);
        let edge = graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            allowed_types: TransitFlags::FOOT | TransitFlags::CAR,
            speed_limit: 0.5,
            base_cost: 140.0 / crate::config::MIN_ROUTE_SPEED_MS,
            physical_length: 140.0,
            ..Default::default()
        });
        for mask in [TransitFlags::FOOT, TransitFlags::FOOT | TransitFlags::CAR] {
            let bound =
                lower_bound_travel_seconds(140.0 * 140.0, max_speed_for_modes(&graph, mask));
            assert!(bound <= CostCalculator::walk_cost_s(graph.edge(edge)));
        }
        assert_eq!(
            max_speed_for_modes(&graph, TransitFlags::FOOT),
            crate::config::AGENT_WALK_SPEED_MS
        );
    }

    #[test]
    fn clipped_chunk_rings_preserve_every_original_intersecting_chunk() {
        for origin in [(-100, 70), (-1, 0), (0, 0), (40, 42)] {
            for ring in [0, 1, 2, 16, 128] {
                let mut original = Vec::new();
                if ring == 0 {
                    original.push(origin);
                } else {
                    let (x, y) = origin;
                    original.extend((x - ring..=x + ring).map(|x| (x, y - ring)));
                    original.extend((y - ring + 1..y + ring).map(|y| (x + ring, y)));
                    original.extend((x - ring..=x + ring).rev().map(|x| (x, y + ring)));
                    original.extend((y - ring + 1..y + ring).rev().map(|y| (x - ring, y)));
                }
                for lower in [(-200, -200), (-10, -3), origin, (0, 0), (1, 1)] {
                    for size in [(0, 0), (1, 2), (20, 3), (3, 20), (400, 400)] {
                        let upper = (lower.0 + size.0, lower.1 + size.1);
                        let expected: Vec<_> = original
                            .iter()
                            .copied()
                            .filter(|&(x, y)| {
                                x >= lower.0 && x <= upper.0 && y >= lower.1 && y <= upper.1
                            })
                            .collect();
                        let mut actual = Vec::new();
                        scan_ring_chunks_in_bounds(origin, ring, lower, upper, |chunk| {
                            actual.push(chunk)
                        });
                        assert_eq!(actual, expected, "{origin:?} {ring} {lower:?} {upper:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn clipped_bucket_scans_preserve_item_and_ring_event_prefixes() {
        let entries: Vec<_> = (0..75)
            .flat_map(|item| {
                [0, 1].map(|component| {
                    ReachableBucketEntry::new(
                        component,
                        (item as i32 % 5 - 2, item as i32 / 5 - 7),
                        item,
                    )
                })
            })
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries.clone());
        index.enable_reservations();
        let mut components = BuildingModeComponents::default();
        components.push(1);
        components.push(0);
        for round in 0..3 {
            let eligible = |entry: &ReachableBucketEntry| {
                !(entry.item_idx + round).is_multiple_of(4) && !(round == 2 && entry.component == 1)
            };
            for &entry in &entries {
                index.set_available(entry, eligible(&entry));
            }
            for origin in [(-30, 20), (0, 0), (16, -16)] {
                let (x, y) = (
                    origin.0 as f32 * RegionGraph::CHUNK_SIZE,
                    origin.1 as f32 * RegionGraph::CHUNK_SIZE,
                );
                let mut remaining = entries.iter().filter(|entry| eligible(entry)).count();
                let max_ring = entries
                    .iter()
                    .filter(|entry| eligible(entry))
                    .map(|entry| {
                        (origin.0 - entry.chunk.0)
                            .abs()
                            .max((origin.1 - entry.chunk.1).abs())
                    })
                    .max()
                    .unwrap();
                let mut expected = Vec::new();
                for ring in 0..=max_ring {
                    scan_ring_chunks(origin, ring, |chunk| {
                        if remaining == 0 {
                            return;
                        }
                        for &component in components.as_slice() {
                            for entry in entries.iter().filter(|entry| {
                                entry.component == component
                                    && entry.chunk == chunk
                                    && eligible(entry)
                            }) {
                                expected.push((Some(entry.item_idx), 0));
                                remaining -= 1;
                            }
                        }
                    });
                    if remaining == 0 {
                        break;
                    }
                    if ring < max_ring {
                        expected.push((
                            None,
                            min_possible_ring_distance_sq(x, y, origin, ring + 1).to_bits(),
                        ));
                    }
                }
                for stop in [1, 7, usize::MAX] {
                    let mut actual = Vec::new();
                    index.scan_nearest(components, x, y, |event| {
                        actual.push(match event {
                            ReachableBucketScanEvent::Item { item_idx } => (Some(item_idx), 0),
                            ReachableBucketScanEvent::RingComplete {
                                next_min_distance_sq,
                            } => (None, next_min_distance_sq.to_bits()),
                        });
                        actual.len() < stop
                    });
                    assert_eq!(actual, expected[..expected.len().min(stop)]);
                }
            }
        }
    }

    #[test]
    fn component_masks_skip_empty_rings_and_track_last_member_updates() {
        let entries = [
            ReachableBucketEntry::new(0, (0, 0), 0),
            ReachableBucketEntry::new(0, (64, 64), 1),
            ReachableBucketEntry::new(1, (64, 64), 2),
        ];
        let mut index = ReachableBucketIndex::from_entries(entries.to_vec());
        index.enable_reservations();
        let mut components = BuildingModeComponents::default();
        components.push(0);
        let collect = |index: &ReachableBucketIndex, mask| {
            let mut items = Vec::new();
            let mut rings = 0;
            index.scan_nearest_masked(components, 0.0, 0.0, Some(mask), |event| {
                match event {
                    ReachableBucketScanEvent::Item { item_idx } => items.push(item_idx),
                    ReachableBucketScanEvent::RingComplete { .. } => rings += 1,
                }
                true
            });
            (items, rings)
        };
        // An index without class preparation must keep its ordinary traversal.
        assert_eq!(collect(&index, 1).0, [0, 1]);
        index.reset_query_masks(|item| if item == 2 { 1 } else { 0 });
        assert_eq!(collect(&index, 1), (Vec::new(), 0));
        let capacity = index.components[0].query_counts.capacity();
        for mask in [1, 1 << 63, u64::MAX] {
            index.set_query_mask(entries[0], mask);
            index.set_query_mask(entries[1], mask);
            assert_eq!(collect(&index, mask).0, [0, 1]);
            index.set_query_mask(entries[0], 0);
            assert_eq!(collect(&index, mask).0, [1]);
            index.set_query_mask(entries[1], 0);
            assert_eq!(collect(&index, mask), (Vec::new(), 0));
            assert_eq!(index.components[0].query_counts.capacity(), capacity);
        }
        index.reset_query_masks(|item| if item < 2 { 1 } else { 0 });
        assert_eq!(collect(&index, 1).0, [0, 1]);
        index.reset_available(|_| false);
        assert!(collect(&index, 1).0.is_empty());
        index.reset_available(|_| true);
        assert_eq!(collect(&index, 1).0, [0, 1]);
    }

    #[test]
    #[ignore = "isolated populated chunk traversal timing"]
    fn benchmark_empty_price_class_queries() {
        use std::{hint::black_box, time::Instant};
        let repeats = std::env::var("METRUM_ECON_BENCH_REPEATS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(11);
        for side in [16, 64, 128] {
            let entries = (0..side * side)
                .map(|item| ReachableBucketEntry::new(0, (item % side, item / side), item as usize))
                .collect();
            let mut index = ReachableBucketIndex::from_entries(entries);
            index.enable_reservations();
            index.reset_query_masks(|_| 0);
            let mut components = BuildingModeComponents::default();
            components.push(0);
            for sample in 0..repeats {
                for guarded in if sample % 2 == 0 {
                    [false, true]
                } else {
                    [true, false]
                } {
                    let mut rings = 0;
                    let begin = Instant::now();
                    for _ in 0..32 {
                        let visitor = |event| {
                            assert!(matches!(
                                event,
                                ReachableBucketScanEvent::RingComplete { .. }
                            ));
                            rings += 1;
                            true
                        };
                        if guarded {
                            black_box(&index).scan_nearest_masked(
                                components,
                                0.0,
                                0.0,
                                Some(black_box(1)),
                                visitor,
                            );
                        } else {
                            ReachableBucketIndex::scan_masked_buckets(
                                &[black_box(&index)],
                                components,
                                0.0,
                                0.0,
                                black_box(1),
                                visitor,
                            );
                        }
                    }
                    let ns_per_query = begin.elapsed().as_nanos() as f64 / 32.0;
                    assert_eq!(rings == 0, guarded);
                    println!(
                        "EMPTY_PRICE_CLASS {{\"side\":{side},\"guarded\":{guarded},\"sample\":{sample},\"ns_per_query\":{ns_per_query}}}"
                    );
                }
            }
        }
    }

    #[test]
    fn masked_bucket_scans_preserve_filtered_order_after_updates_and_early_stops() {
        let entries: Vec<_> = (0..257)
            .flat_map(|item| {
                [0, 1].map(|component| {
                    ReachableBucketEntry::new(component, (item as i32 / 61 - 2, 0), item)
                })
            })
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries.clone());
        index.enable_reservations();
        let mut components = BuildingModeComponents::default();
        components.push(0);
        components.push(1);
        let mut masks: Vec<u64> = (0..257).map(|i| 1 << (i % 7)).collect();
        index.reset_query_masks(|item| masks[item]);
        for round in 0..4 {
            for entry in &entries {
                index.set_available(*entry, (entry.item_idx + round) % 5 != 0);
                masks[entry.item_idx] = 1 << ((entry.item_idx + round) % 9);
                index.set_query_mask(*entry, masks[entry.item_idx]);
            }
            for mask in [0, 1, 3, 128, 512, u64::MAX] {
                for stop in [1, 7, usize::MAX] {
                    let mut expected = Vec::new();
                    index.scan_nearest(components, 14.0, 250.0, |event| {
                        if let ReachableBucketScanEvent::Item { item_idx } = event
                            && masks[item_idx] & mask != 0
                        {
                            expected.push(item_idx);
                        }
                        expected.len() < stop
                    });
                    let mut actual = Vec::new();
                    index.scan_nearest_masked(components, 14.0, 250.0, Some(mask), |event| {
                        if let ReachableBucketScanEvent::Item { item_idx } = event {
                            actual.push(item_idx);
                        }
                        actual.len() < stop
                    });
                    assert_eq!(actual, expected);
                }
            }
        }
    }

    #[test]
    fn split_indices_scan_exactly_as_one_merged_index() {
        // Low ids form the first index and high ids the second, as live and proposed ports do.
        let entries: Vec<_> = (0..300)
            .flat_map(|item| {
                [0, 1].map(|component| {
                    ReachableBucketEntry::new(
                        component,
                        ((item * 37 % 11) as i32 - 5, (item * 13 % 7) as i32 - 3),
                        item,
                    )
                })
            })
            .filter(|entry| entry.item_idx % 3 != 0 || entry.component == 0)
            .collect();
        for split in [0, 1, 150, 299, 300] {
            let build = |filter: &dyn Fn(usize) -> bool| {
                let mut index = ReachableBucketIndex::from_entries(
                    entries
                        .iter()
                        .filter(|entry| filter(entry.item_idx))
                        .copied()
                        .collect(),
                );
                index.enable_reservations();
                index
            };
            let mut merged = build(&|_| true);
            let mut low = build(&|item| item < split);
            let mut high = build(&|item| item >= split);
            let mut components = BuildingModeComponents::default();
            components.push(0);
            components.push(1);
            for round in 0..4 {
                let available = |item: usize| !(item * 7 + round).is_multiple_of(5);
                let mask = |item: usize| 1u64 << ((item + round) % 9);
                for index in [&mut merged, &mut low, &mut high] {
                    index.reset_available(available);
                    index.reset_query_masks(mask);
                }
                for (x, y) in [(14.0, 250.0), (-900.0, -60.0), (3000.0, 1000.0)] {
                    for query in [None, Some(0), Some(1), Some(3), Some(256), Some(u64::MAX)] {
                        for stop in [1, 7, usize::MAX] {
                            let collect = |indices: &[&ReachableBucketIndex]| {
                                let mut events = Vec::new();
                                ReachableBucketIndex::scan_nearest_masked_over(
                                    indices,
                                    components,
                                    x,
                                    y,
                                    query,
                                    |event| {
                                        events.push(match event {
                                            ReachableBucketScanEvent::Item { item_idx } => {
                                                (Some(item_idx), 0)
                                            }
                                            ReachableBucketScanEvent::RingComplete {
                                                next_min_distance_sq,
                                            } => (None, next_min_distance_sq.to_bits()),
                                        });
                                        events.iter().filter(|event| event.0.is_some()).count()
                                            < stop
                                    },
                                );
                                events
                            };
                            assert_eq!(
                                collect(&[&low, &high]),
                                collect(&[&merged]),
                                "split {split}, round {round}, origin ({x}, {y}), mask {query:?}, stop {stop}"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn constant_time_ring_distance_matches_every_original_chunk() {
        for chunk in [
            (-100_000, 100_000),
            (-40, -31),
            (-1, 0),
            (0, 0),
            (39, 42),
            (16_777_216, -16_777_216),
        ] {
            for offsets in [
                [0.0_f32, 0.0],
                [0.001, 511.999],
                [255.5, 310.75],
                [512.0, 512.0],
                [-10.0, 900.0],
            ] {
                let x = chunk.0 as f32 * RegionGraph::CHUNK_SIZE + offsets[0];
                let y = chunk.1 as f32 * RegionGraph::CHUNK_SIZE + offsets[1];
                for x in [x.next_down(), x, x.next_up()] {
                    for ring in [0, 1, 2, 16, 128] {
                        let mut expected = f32::MAX;
                        scan_ring_chunks(chunk, ring, |candidate| {
                            expected = expected.min(squared_distance_to_chunk(x, y, candidate));
                        });
                        assert_eq!(
                            min_possible_ring_distance_sq(x, y, chunk, ring).to_bits(),
                            expected.to_bits(),
                            "{chunk:?} {x} {y} ring={ring}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn component_member_counts_are_independent_of_financial_availability() {
        let mut index = ReachableBucketIndex::from_entries(vec![
            ReachableBucketEntry::new(0, (0, 0), 0),
            ReachableBucketEntry::new(0, (1, 0), 1),
            ReachableBucketEntry::new(1, (2, 0), 1),
            ReachableBucketEntry::new(1, (2, 0), 2),
        ]);
        index.enable_reservations();
        let mut a = BuildingModeComponents::default();
        a.push(0);
        let mut b = BuildingModeComponents::default();
        b.push(1);
        index.reset_member_counts(|item| (item < 2).then_some(item as f64 + 0.5));
        assert!(index.has_counted_members(a) && index.has_counted_members(b));
        assert_eq!(
            (index.counted_units(a), index.counted_units(b)),
            ((2, 2.0), (1, 1.5))
        );
        // Totals stay stale-high until an explicit fresh recount.
        assert!(index.mark_member_units(0) && !index.mark_member_units(0));
        index.recount_member_units(0, |item| (item == 1).then_some(0.25));
        assert_eq!(index.counted_units(a), (2, 0.25));
        assert!(index.mark_member_units(0));
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
        index.reset_member_counts(|item| (item == 2).then_some(3.0));
        assert!(!index.has_counted_members(a) && index.has_counted_members(b));
        assert_eq!(index.counted_units(b), (1, 3.0));
    }

    #[test]
    fn depleted_candidates_skip_and_restore_without_changing_stable_order() {
        let entries: Vec<_> = (0..4096)
            .map(|item| ReachableBucketEntry::new(0, (0, 0), item))
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries.clone());
        index.enable_reservations();
        let reservations: Vec<_> = entries
            .iter()
            .map(|&entry| index.reservation(entry).unwrap())
            .collect();
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
        assert_eq!(
            scan(&index),
            [100, 4095],
            "forked availability remains independent"
        );
    }

    #[test]
    fn dense_available_ranges_preserve_leaf_order_and_early_stop() {
        let mut components = BuildingModeComponents::default();
        components.push(0);
        for count in [1, 3, 63, 64, 129, 4096] {
            let mut index = ReachableBucketIndex::from_entries(
                (0..count)
                    .rev()
                    .map(|item| ReachableBucketEntry::new(0, (0, 0), item))
                    .collect(),
            );
            index.enable_reservations();
            for pattern in 0..4 {
                let eligible = |item: usize| match pattern {
                    0 => true,
                    1 => item > count / 3 && item < count * 2 / 3,
                    2 => item.is_multiple_of(37),
                    _ => false,
                };
                index.reset_available(eligible);
                for limit in [1, 3, 32, count + 1] {
                    let expected: Vec<_> = (0..count)
                        .filter(|&item| eligible(item))
                        .take(limit)
                        .collect();
                    let mut actual = Vec::new();
                    index.scan_nearest(components, 0.0, 0.0, |event| {
                        if let ReachableBucketScanEvent::Item { item_idx } = event {
                            actual.push(item_idx);
                        }
                        actual.len() < limit
                    });
                    assert_eq!(
                        actual, expected,
                        "count={count} pattern={pattern} limit={limit}"
                    );
                }
            }
        }
    }

    #[test]
    fn inactive_background_does_not_extend_the_active_candidate_scan() {
        let mut components = BuildingModeComponents::default();
        components.push(0);
        components.push(1);
        for background in [0, 16, 256, 4_096] {
            let near = [
                ReachableBucketEntry::new(0, (0, 0), 0),
                ReachableBucketEntry::new(1, (0, 0), 0),
                ReachableBucketEntry::new(1, (2, 0), 1),
            ];
            let entries = near
                .into_iter()
                .chain((0..background).map(|i| {
                    ReachableBucketEntry::new((i % 2) as u32, (1_024 + i as i32, 0), i + 2)
                }))
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
                assert_eq!(
                    found,
                    if remote_active {
                        vec![0, 0, 1]
                    } else {
                        vec![0, 0]
                    }
                );
                assert_eq!(
                    rings,
                    if remote_active { 2 } else { 0 },
                    "inactive entries may not make a completed scan visit empty outer rings"
                );
            }
        }
    }

    #[test]
    fn bounded_nearest_matches_exhaustive_selection_with_fewer_visits() {
        // Deterministic doors over three chunks; route time exceeds the straight-line bound.
        let mut seed = 0x9e37_79b9_u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed
        };
        let doors: Vec<[f32; 2]> = (0..600)
            .map(|_| [(next() % 1_500) as f32 + 0.25, (next() % 900) as f32 - 0.5])
            .collect();
        let entries = doors
            .iter()
            .enumerate()
            .map(|(item, door)| {
                ReachableBucketEntry::new(0, chunk_for_point(door[0], door[1]), item)
            })
            .collect();
        let mut index = ReachableBucketIndex::from_entries(entries);
        index.enable_reservations();
        index.enable_spatial_bounds(|item| (item % 97 != 0).then_some(doors[item]));
        let mut components = BuildingModeComponents::default();
        components.push(0);
        let speed = 13.0;
        let seconds = |item: usize, origin: [f32; 2]| {
            let distance_sq =
                (doors[item][0] - origin[0]).powi(2) + (doors[item][1] - origin[1]).powi(2);
            lower_bound_travel_seconds(distance_sq, speed) * (1.0 + (item % 7) as f32 / 10.0)
        };
        let [mut exhaustive_visits, mut bounded_visits] = [0, 0];
        for round in 0..64 {
            index.reset_available(|item| (item + round) % 5 != 0);
            let origin = [(next() % 1_500) as f32, (next() % 900) as f32];
            let mut best: Option<(f32, usize)> = None;
            let offer = |item: usize, best: &mut Option<(f32, usize)>| {
                let candidate = (seconds(item, origin), item);
                if best.is_none_or(|current| {
                    candidate
                        .0
                        .total_cmp(&current.0)
                        .then(candidate.1.cmp(&current.1))
                        .is_lt()
                }) {
                    *best = Some(candidate);
                }
            };
            index.scan_nearest(components, origin[0], origin[1], |event| {
                if let ReachableBucketScanEvent::Item { item_idx } = event {
                    exhaustive_visits += 1;
                    offer(item_idx, &mut best);
                }
                true
            });
            let expected = best;
            let mut best = None;
            index.scan_nearest_bounded(components, origin[0], origin[1], |event| match event {
                BoundedScanEvent::Item { item_idx } => {
                    bounded_visits += 1;
                    offer(item_idx, &mut best);
                    true
                }
                BoundedScanEvent::Region { min_distance_sq }
                | BoundedScanEvent::RingComplete {
                    next_min_distance_sq: min_distance_sq,
                } => best.is_none_or(|(seconds, _)| {
                    lower_bound_travel_seconds(min_distance_sq, speed) <= seconds
                }),
            });
            assert_eq!(best, expected, "round={round}");
        }
        assert!(
            bounded_visits * 4 < exhaustive_visits,
            "bounds must prune most candidates: {bounded_visits} of {exhaustive_visits}"
        );
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
                .chain((1..count).map(|item| {
                    ReachableBucketEntry::new(0, (4_096 + item as i32 / 1_024, 0), item)
                }))
                .collect();
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
            eprintln!(
                "inactive_supplier_extent entries={count} active=1 queries=4096 median_ms={:.4}",
                samples[5]
            );
        }
    }
}
