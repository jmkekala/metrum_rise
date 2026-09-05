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
}

#[derive(Clone, Debug)]
struct ComponentBuckets {
    by_chunk: BTreeMap<(i32, i32), Vec<usize>>,
    min_chunk: (i32, i32),
    max_chunk: (i32, i32),
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

        let mut by_component = BTreeMap::new();
        for entry in entries {
            by_component
                .entry(entry.component)
                .and_modify(|buckets: &mut ComponentBuckets| {
                    buckets.min_chunk.0 = buckets.min_chunk.0.min(entry.chunk.0);
                    buckets.min_chunk.1 = buckets.min_chunk.1.min(entry.chunk.1);
                    buckets.max_chunk.0 = buckets.max_chunk.0.max(entry.chunk.0);
                    buckets.max_chunk.1 = buckets.max_chunk.1.max(entry.chunk.1);
                    buckets
                        .by_chunk
                        .entry(entry.chunk)
                        .or_default()
                        .push(entry.item_idx);
                })
                .or_insert_with(|| ComponentBuckets {
                    by_chunk: BTreeMap::from([(entry.chunk, vec![entry.item_idx])]),
                    min_chunk: entry.chunk,
                    max_chunk: entry.chunk,
                });
        }

        Self { by_component }
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
                    for &item_idx in items {
                        if !visitor(ReachableBucketScanEvent::Item { item_idx }) {
                            should_continue = false;
                            return;
                        }
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

fn scan_ring_chunks(origin_chunk: (i32, i32), ring: i32, mut visit: impl FnMut((i32, i32))) {
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

fn min_possible_ring_distance_sq(
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
