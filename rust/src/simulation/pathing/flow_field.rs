// SPDX-License-Identifier: GPL-2.0-only

//! Flow-field routing: multi-source reverse Dijkstra per zone type.
//!
//! Each field stores the next node and nearest destination building for one zone/mode.
//! Fields provide an optional path for an already selected trip; CCH remains authoritative.
//! Topology edits and building changes mark the affected fields dirty for the next tick.
//!
//! A field is a function of its sources and the road graph alone, which is what a loaded game
//! builds (`SIM-02-I`): a marked zone whose sources and graph are unchanged since its field was
//! built keeps the field. The reverse adjacency each search walks is cached per graph revision
//! in compact rows, and every field keeps its storage and search scratch, so a frame that builds
//! fields allocates nothing once the buffers have grown.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::pathing::cost::CostCalculator;
use crate::simulation::zoning::ZoneType;
use rayon::prelude::*;
use std::collections::BinaryHeap;

// ---------------------------------------------------------------------------
// HeapEntry — min-heap on cost for Dijkstra.
// Uses f32::to_bits() which preserves ordering for non-negative floats.
// ---------------------------------------------------------------------------
#[derive(PartialEq, Eq)]
struct HeapEntry {
    cost_bits: u32,
    node: u32,
}

impl Ord for HeapEntry {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Reversed for min-heap.
        other
            .cost_bits
            .cmp(&self.cost_bits)
            .then(self.node.cmp(&other.node))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

// ---------------------------------------------------------------------------
// Reverse adjacency and search scratch
// ---------------------------------------------------------------------------

// Incoming arcs of every node for one mode in compact rows: node `v`'s arcs are
// `arcs[offsets[v]..offsets[v + 1]]`, `(predecessor, cost)`, in graph edge order, each edge
// adding its forward arc at its end node before its backward arc at its start node. Direct
// traversal of graph adjacency reads the large Edge records in search order and was slower on
// large matched grids; these rows are a measured locality cache.
#[derive(Default)]
struct ReverseAdjacency {
    offsets: Vec<u32>,
    arcs: Vec<(u32, f32)>,
    // Fill position per node while building.
    cursor: Vec<u32>,
}

impl ReverseAdjacency {
    // Rebuilds the rows for mode `flags` in O(V + E), reusing the storage.
    fn rebuild(&mut self, graph: &RegionGraph, flags: u8) {
        let node_count = graph.node_count();
        let arcs_of = |edge: &crate::simulation::network::graph::Edge| {
            if edge.deleted || edge.allowed_types & flags == 0 {
                return [None, None];
            }
            let cost = if flags == TransitFlags::FOOT {
                CostCalculator::walk_cost_s(edge)
            } else {
                edge.base_cost
            }
            .max(1e-6);
            [
                (edge.traversal_flags(true) & flags != 0).then_some((
                    edge.end_node,
                    edge.start_node,
                    cost,
                )),
                (edge.traversal_flags(false) & flags != 0).then_some((
                    edge.start_node,
                    edge.end_node,
                    cost,
                )),
            ]
        };
        self.offsets.clear();
        self.offsets.resize(node_count + 1, 0);
        for edge in graph.edges() {
            for (node, _, _) in arcs_of(edge).into_iter().flatten() {
                self.offsets[node as usize + 1] += 1;
            }
        }
        for node in 0..node_count {
            self.offsets[node + 1] += self.offsets[node];
        }
        self.arcs.clear();
        self.arcs
            .resize(self.offsets[node_count] as usize, (0, 0.0));
        self.cursor.clear();
        self.cursor.extend_from_slice(&self.offsets[..node_count]);
        for edge in graph.edges() {
            for (node, predecessor, cost) in arcs_of(edge).into_iter().flatten() {
                let position = &mut self.cursor[node as usize];
                self.arcs[*position as usize] = (predecessor, cost);
                *position += 1;
            }
        }
    }

    fn node_count(&self) -> usize {
        self.offsets.len().saturating_sub(1)
    }

    fn incoming(&self, node: u32) -> &[(u32, f32)] {
        let node = node as usize;
        &self.arcs[self.offsets[node] as usize..self.offsets[node + 1] as usize]
    }
}

// The search state of one field build, kept between builds.
#[derive(Default)]
struct SearchScratch {
    dist: Vec<f32>,
    nearest_source_node: Vec<u32>,
    heap: BinaryHeap<HeapEntry>,
}

// ---------------------------------------------------------------------------
// FlowField
// ---------------------------------------------------------------------------

/// A precomputed routing map from every reachable node toward the nearest
/// building of one zone type, for one transit mode (car or foot).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlowField {
    /// `next_node[v]` — the next node to move to from `v` on the shortest path
    /// toward the nearest destination. `u32::MAX` if `v` is unreachable.
    pub next_node: Vec<u32>,
    /// `nearest_building[v]` — index of the nearest destination building from `v`.
    /// `usize::MAX` if unreachable.
    pub nearest_building: Vec<usize>,
}

impl FlowField {
    /// Builds a flow field by multi-source reverse Dijkstra.
    ///
    /// `sources` is a slice of `(frontage_node, building_index)` pairs — one
    /// per building of the target zone type. All sources start at cost 0.
    ///
    /// Complexity: O((V + E) log V), with O(V + E) temporary storage plus the search heap.
    /// [`FlowFieldSystem`] builds the same field on cached adjacency and reused storage.
    pub fn build(sources: &[(u32, usize)], graph: &RegionGraph, flags: u8) -> Self {
        let mut adjacency = ReverseAdjacency::default();
        adjacency.rebuild(graph, flags);
        let mut field = Self::default();
        field.build_on(sources, &adjacency, &mut SearchScratch::default());
        field
    }

    // Rebuilds this field from `sources` on `adjacency`, reusing this field's and `scratch`'s
    // storage. O((V + E) log V); allocates only to grow past earlier sizes.
    fn build_on(
        &mut self,
        sources: &[(u32, usize)],
        adjacency: &ReverseAdjacency,
        scratch: &mut SearchScratch,
    ) {
        let node_count = adjacency.node_count();
        let SearchScratch {
            dist,
            nearest_source_node,
            heap,
        } = scratch;
        let next_node = &mut self.next_node;
        let nearest_building = &mut self.nearest_building;
        dist.clear();
        dist.resize(node_count, f32::INFINITY);
        next_node.clear();
        next_node.resize(node_count, u32::MAX);
        nearest_building.clear();
        nearest_building.resize(node_count, usize::MAX);
        nearest_source_node.clear();
        nearest_source_node.resize(node_count, u32::MAX);
        heap.clear();

        // Seed all source nodes at cost 0.
        for &(source_node, building_idx) in sources {
            let fn_idx = source_node as usize;
            if fn_idx >= node_count {
                continue;
            }
            let better_source = 0.0 < dist[fn_idx]
                || (dist[fn_idx] == 0.0 && building_idx < nearest_building[fn_idx]);
            if better_source {
                dist[fn_idx] = 0.0;
                // At a source node: you're already there, no next hop needed.
                next_node[fn_idx] = source_node;
                nearest_building[fn_idx] = building_idx;
                nearest_source_node[fn_idx] = source_node;
                heap.push(HeapEntry {
                    cost_bits: 0f32.to_bits(),
                    node: source_node,
                });
            }
        }

        // Reverse Dijkstra.
        while let Some(HeapEntry { cost_bits, node: u }) = heap.pop() {
            let d = f32::from_bits(cost_bits);
            if d > dist[u as usize] {
                continue; // stale entry
            }
            for &(v, cost) in adjacency.incoming(u) {
                let new_dist = d + cost;
                let v_idx = v as usize;
                let new_building = nearest_building[u as usize];
                let new_source = nearest_source_node[u as usize];
                let better = new_dist < dist[v_idx]
                    || (new_dist == dist[v_idx]
                        && (new_building < nearest_building[v_idx]
                            || (new_building == nearest_building[v_idx]
                                && (new_source < nearest_source_node[v_idx]
                                    || (new_source == nearest_source_node[v_idx]
                                        && u < next_node[v_idx])))));
                if better {
                    dist[v_idx] = new_dist;
                    // From v, moving to u gets you closer to the destination.
                    next_node[v_idx] = u;
                    nearest_building[v_idx] = new_building;
                    nearest_source_node[v_idx] = new_source;
                    heap.push(HeapEntry {
                        cost_bits: new_dist.to_bits(),
                        node: v,
                    });
                }
            }
        }
    }

    /// Builds a path `Vec<u32>` from `from_node` to the nearest destination by
    /// following `next_node` hops. Returns `None` if `from_node` is unreachable
    /// or if the chain loops (cycle guard via `max_hops`).
    ///
    /// The path ends at the source node (a building frontage node).
    pub fn build_path(&self, from_node: u32, max_hops: usize) -> Option<Vec<u32>> {
        if self
            .next_node
            .get(from_node as usize)
            .is_none_or(|&node| node == u32::MAX)
        {
            return None;
        }
        let mut path = Vec::with_capacity(32);
        self.build_path_in(from_node, max_hops, &mut path)?;
        Some(path)
    }

    /// Follows the same bounded chain into caller-owned storage. Route planners reserve
    /// `max_hops + 1` entries before querying so rejected alternatives allocate nothing.
    pub(crate) fn build_path_in(
        &self,
        from_node: u32,
        max_hops: usize,
        path: &mut Vec<u32>,
    ) -> Option<()> {
        path.clear();
        let from_idx = from_node as usize;
        if from_idx >= self.next_node.len() || self.next_node[from_idx] == u32::MAX {
            return None;
        }
        path.push(from_node);
        let mut current = from_node;
        for _ in 0..max_hops {
            let next = self.next_node[current as usize];
            if next == u32::MAX {
                return None;
            }
            if next == current {
                // Already at a source node — path is complete.
                return Some(());
            }
            path.push(next);
            current = next;
        }
        // Exceeded max_hops without reaching a destination.
        None
    }
}

// ---------------------------------------------------------------------------
// FlowFieldSystem
// ---------------------------------------------------------------------------

const FLOW_FIELD_ZONE_COUNT: usize = 3;
const FLOW_FIELD_ZONES: [ZoneType; FLOW_FIELD_ZONE_COUNT] = [
    ZoneType::Residential,
    ZoneType::Commercial,
    ZoneType::Industrial,
];
// Field slot `2 * zone + mode`: car then foot.
const FLOW_FIELD_MODES: [u8; 2] = [TransitFlags::CAR, TransitFlags::FOOT];

fn flow_field_zone_slot(zone: ZoneType) -> Option<usize> {
    match zone {
        ZoneType::Residential => Some(0),
        ZoneType::Commercial => Some(1),
        ZoneType::Industrial => Some(2),
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => None,
    }
}

/// What a flow field's graph is identified by: the routing hierarchy's build generation and
/// metric revision, which every topology or cost change moves, with the node and edge counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlowGraphRevision {
    /// CCH build generation.
    pub cch_build: u32,
    /// CCH metric revision.
    pub cch_metric: u64,
}

// One zone's field for one mode, with the sources it was built from and its search storage.
#[derive(Default)]
struct FieldSlot {
    field: FlowField,
    // Whether `field` holds a field: a zone without sources has none.
    present: bool,
    // The sources `field` was built from, on the cached adjacency when `current`.
    sources: Vec<(u32, usize)>,
    current: bool,
    // The sources collected at a drain, compared with `sources`.
    collected: Vec<(u32, usize)>,
    scratch: SearchScratch,
}

/// Manages one [`FlowField`] per zone type per transit mode, rebuilt lazily.
///
/// Attached to [`crate::simulation::network::TransitNetwork`].
/// Call [`FlowFieldSystem::mark_all_dirty`] on topology changes and
/// [`FlowFieldSystem::mark_zone_dirty`] when buildings of a zone type are added/removed.
pub struct FlowFieldSystem {
    // Slot `2 * zone + mode`, car before foot.
    slots: [FieldSlot; 2 * FLOW_FIELD_ZONE_COUNT],
    // Per-family dirty flags, read by the next `rebuild_dirty` call.
    dirty: [bool; FLOW_FIELD_ZONE_COUNT],
    // Reverse adjacency per mode for the graph `graph` identifies; `None` until built or after
    // `mark_all_dirty`.
    adjacency: [ReverseAdjacency; 2],
    graph: Option<(FlowGraphRevision, usize, usize)>,
}

impl Default for FlowFieldSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl FlowFieldSystem {
    /// Creates a new system with all fields absent and all zones dirty.
    pub fn new() -> Self {
        Self {
            slots: Default::default(),
            dirty: [true; FLOW_FIELD_ZONE_COUNT],
            adjacency: Default::default(),
            graph: None,
        }
    }

    /// Marks all zone types as dirty (called on road topology change): every field and the
    /// cached adjacency are rebuilt at the next call.
    pub fn mark_all_dirty(&mut self) {
        self.dirty = [true; FLOW_FIELD_ZONE_COUNT];
        self.graph = None;
    }

    /// Marks a single zone type as dirty (called when its buildings may have changed): its
    /// sources are collected again at the next call, and its fields rebuilt if they differ.
    pub fn mark_zone_dirty(&mut self, zone: ZoneType) {
        if let Some(zone_idx) = flow_field_zone_slot(zone) {
            self.dirty[zone_idx] = true;
        }
    }

    /// Rebuilds the flow fields of the dirty zone types whose sources changed, or every field
    /// when the graph `revision` identifies differs from the one they were built on.
    ///
    /// `sources_fn(zone, mode_flags, out)` fills `out` (cleared) with the current legal
    /// destination endpoints of `zone` for one mode, in a fixed order; it is called once per
    /// dirty zone and mode, from Rayon workers. Passing a closure keeps this module free of a direct
    /// `BuildingAllocator` dependency. A field equals [`FlowField::build`] of its sources on
    /// the current graph.
    ///
    /// Complexity: O(1) with nothing marked; O(V + E) for the adjacency after a graph change;
    /// then, for each marked zone's two modes in parallel, O(S) to collect and compare its S
    /// sources and O((V + E) log V) when they changed. Allocates only to grow buffers past
    /// earlier sizes.
    pub fn rebuild_dirty(
        &mut self,
        graph: &RegionGraph,
        revision: FlowGraphRevision,
        sources_fn: impl Fn(ZoneType, u8, &mut Vec<(u32, usize)>) + Sync,
    ) {
        let identity = (revision, graph.node_count(), graph.edge_count());
        if self.graph != Some(identity) {
            for (adjacency, flags) in self.adjacency.iter_mut().zip(FLOW_FIELD_MODES) {
                adjacency.rebuild(graph, flags);
            }
            self.graph = Some(identity);
            self.dirty = [true; FLOW_FIELD_ZONE_COUNT];
            for slot in &mut self.slots {
                slot.current = false;
            }
        }
        let dirty = std::mem::take(&mut self.dirty);
        // Most frames find nothing marked; the pool is not woken for them.
        if !dirty.contains(&true) {
            return;
        }
        // The cached adjacency is keyed on the routing hierarchy's revisions, not on the graph
        // itself: every change to an edge's endpoints, deletion, modes, directions or base cost
        // must move the CCH build generation or metric revision before this call (the frame
        // rebuilds a dirty hierarchy first) or call `mark_all_dirty`. Debug builds check the
        // rows against a fresh build, O(V + E) per call with a zone marked.
        #[cfg(debug_assertions)]
        for (adjacency, flags) in self.adjacency.iter().zip(FLOW_FIELD_MODES) {
            let mut fresh = ReverseAdjacency::default();
            fresh.rebuild(graph, flags);
            debug_assert!(
                fresh.offsets == adjacency.offsets && fresh.arcs == adjacency.arcs,
                "the flow fields' cached adjacency is stale for its graph revision {revision:?}"
            );
        }
        crate::debug::work::add(
            crate::debug::work::Work::FlowFieldZonesDrained,
            dirty.iter().filter(|&&marked| marked).count() as u64,
        );
        // Each marked slot collects its sources, compares them with its field's and rebuilds
        // only on a change; slots share nothing but the adjacency, so the order they run in
        // changes no result.
        let adjacency = &self.adjacency;
        self.slots
            .as_mut_slice()
            .par_iter_mut()
            .enumerate()
            .filter(|(index, _)| dirty[index / 2])
            .for_each(|(index, slot)| {
                slot.collected.clear();
                sources_fn(
                    FLOW_FIELD_ZONES[index / 2],
                    FLOW_FIELD_MODES[index % 2],
                    &mut slot.collected,
                );
                if slot.current && slot.collected == slot.sources {
                    crate::debug::work::add(crate::debug::work::Work::FlowFieldsSkipped, 1);
                    return;
                }
                std::mem::swap(&mut slot.collected, &mut slot.sources);
                slot.current = true;
                slot.present = !slot.sources.is_empty();
                if slot.present {
                    crate::debug::work::add(crate::debug::work::Work::FlowFieldsBuilt, 1);
                    slot.field
                        .build_on(&slot.sources, &adjacency[index % 2], &mut slot.scratch);
                }
            });
    }

    /// Returns the car flow field for `zone`, or `None` if not yet built or zone has no buildings.
    #[inline]
    pub fn car(&self, zone: ZoneType) -> Option<&FlowField> {
        self.field(zone, 0)
    }

    /// Returns the foot flow field for `zone`, or `None` if not yet built or zone has no buildings.
    #[inline]
    pub fn foot(&self, zone: ZoneType) -> Option<&FlowField> {
        self.field(zone, 1)
    }

    fn field(&self, zone: ZoneType, mode: usize) -> Option<&FlowField> {
        let slot = &self.slots[2 * flow_field_zone_slot(zone)? + mode];
        slot.present.then_some(&slot.field)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::{Edge, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;

    fn make_edge(s: u32, e: u32, cost: f32) -> Edge {
        Edge {
            start_node: s,
            end_node: e,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: cost,
            physical_length: cost * 50.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::ZERO, Vector3::new(cost * 50.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::ZERO, Vector3::new(cost * 50.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        }
    }

    fn linear_graph(node_count: u32) -> RegionGraph {
        let mut graph = RegionGraph::new();
        for node in 0..node_count {
            graph.add_node(
                Vector3::new(node as f32 * 50.0, 0.0, 0.0),
                NodeType::Junction,
            );
            if node > 0 {
                graph.add_edge(make_edge(node - 1, node, 1.0));
            }
        }
        graph
    }

    /// Build path from n0 should give [n0, n1, n2, n3].
    #[test]
    fn test_flow_field_build_path() {
        let g = linear_graph(4);
        let [n0, n1, n2, n3] = [0, 1, 2, 3];

        let ff = FlowField::build(&[(n3, 0)], &g, TransitFlags::CAR);
        let path = ff.build_path(n0, 100).expect("path should exist");
        assert_eq!(path, vec![n0, n1, n2, n3]);
        assert_eq!(ff.nearest_building, vec![0; 4]);
        assert!(ff.build_path(n0, 2).is_none());
        assert!(ff.build_path(4, 100).is_none());
    }

    /// Multi-source chains select the nearer destination before comparing building IDs.
    #[test]
    fn test_flow_field_multi_source_picks_nearest() {
        let g = linear_graph(4);
        let [n0, n2, n3] = [0, 2, 3];

        // Two sources: building 0 at n2, building 1 at n3.
        let sources = vec![(n2, 0usize), (n3, 1usize)];
        let ff = FlowField::build(&sources, &g, TransitFlags::CAR);

        // From n0, nearest is n2 (cost 2 vs cost 3).
        assert_eq!(ff.nearest_building[n0 as usize], 0);
        let path = ff.build_path(n0, 100).unwrap();
        assert_eq!(*path.last().unwrap(), n2);
    }

    /// Equal-cost competing sources must break ties by lower building id first.
    #[test]
    fn test_flow_field_equal_cost_tie_breaks_by_building_id() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(50.0, 0.0, 50.0), NodeType::Junction);
        g.add_edge(make_edge(n0, n1, 1.0));
        g.add_edge(make_edge(n0, n2, 1.0));
        g.rebuild_adjacency_list();

        for (mut sources, expected_end) in [([(n1, 1), (n2, 0)], n2), ([(n1, 0), (n2, 0)], n1)] {
            for _ in 0..2 {
                let ff = FlowField::build(&sources, &g, TransitFlags::CAR);
                assert_eq!(ff.nearest_building[n0 as usize], 0);
                assert_eq!(ff.build_path(n0, 100).unwrap(), vec![n0, expected_end]);
                sources.reverse();
            }
        }
    }

    #[test]
    fn flow_field_respects_mode_directions() {
        for (primary_type, fwd, bkw) in [
            (TransitType::Road, 1, 0),
            (TransitType::Road, 0, 1),
            (TransitType::Foot, 0, 0),
        ] {
            let mut graph = RegionGraph::new();
            graph.add_node(Vector3::ZERO, NodeType::Junction);
            graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
            let mut edge = make_edge(0, 1, 1.0);
            edge.primary_type = primary_type;
            edge.fwd_lanes = fwd;
            edge.bkw_lanes = bkw;
            if primary_type == TransitType::Foot {
                edge.allowed_types = TransitFlags::FOOT;
            }
            graph.add_edge(edge);
            graph.rebuild_adjacency_list();
            for mode in [TransitFlags::FOOT, TransitFlags::CAR] {
                for (start, end, vehicle_lanes) in [(0, 1, fwd), (1, 0, bkw)] {
                    let field = FlowField::build(&[(end, 7)], &graph, mode);
                    let reachable = mode == TransitFlags::FOOT || vehicle_lanes > 0;
                    assert_eq!(
                        field.build_path(start, 3),
                        reachable.then_some(vec![start, end]),
                        "{primary_type:?} lanes={fwd}/{bkw} mode={mode} {start}->{end}"
                    );
                    assert_eq!(
                        field.nearest_building[start as usize],
                        if reachable { 7 } else { usize::MAX }
                    );
                }
            }
            graph.edge_mut(0).deleted = true;
            graph.rebuild_adjacency_list();
            for mode in [TransitFlags::FOOT, TransitFlags::CAR] {
                assert!(
                    FlowField::build(&[(1, 7)], &graph, mode)
                        .build_path(0, 3)
                        .is_none()
                );
            }
        }
    }

    #[test]
    #[ignore = "matched release flow-field build benchmark; excludes graph setup and verification"]
    fn benchmark_flow_field_build() {
        use std::hint::black_box;
        use std::time::Instant;

        for side in [32_u32, 100, 316] {
            let setup = Instant::now();
            let mut graph = RegionGraph::new();
            for y in 0..side {
                for x in 0..side {
                    graph.add_node(
                        Vector3::new(x as f32 * 50.0, 0.0, y as f32 * 50.0),
                        NodeType::Junction,
                    );
                }
            }
            for y in 0..side {
                for x in 0..side {
                    let node = y * side + x;
                    if x + 1 < side {
                        graph.add_edge(make_edge(node, node + 1, 1.0));
                    }
                    if y + 1 < side {
                        graph.add_edge(make_edge(node, node + side, 1.0));
                    }
                }
            }
            graph.rebuild_adjacency_list();
            let setup_us = setup.elapsed().as_secs_f64() * 1e6;
            let sources = [(0, 7), (side * side - 1, 3)];
            let fingerprint = |field: &FlowField| {
                let mut hash = 0xcbf29ce484222325_u64;
                for (node, (&next, &building)) in field
                    .next_node
                    .iter()
                    .zip(&field.nearest_building)
                    .enumerate()
                {
                    let x = node as u32 % side;
                    let y = node as u32 / side;
                    let expected = if x + y < side - 1 { 7 } else { 3 };
                    assert_eq!(building, expected);
                    let dest = if building == 7 { 0 } else { side * side - 1 };
                    let remaining = x.abs_diff(dest % side) + y.abs_diff(dest / side);
                    if remaining == 0 {
                        assert_eq!(next, node as u32);
                    } else {
                        let next_remaining = (next % side).abs_diff(dest % side)
                            + (next / side).abs_diff(dest / side);
                        assert_eq!(next_remaining + 1, remaining);
                    }
                    for value in [next as u64, building as u64] {
                        hash = (hash ^ value).wrapping_mul(0x100000001b3);
                    }
                }
                hash
            };
            let expected = fingerprint(&FlowField::build(&sources, &graph, TransitFlags::CAR));
            let mut samples = Vec::with_capacity(21);
            for _ in 0..21 {
                let begin = Instant::now();
                let field =
                    FlowField::build(black_box(&sources), black_box(&graph), TransitFlags::CAR);
                samples.push(begin.elapsed().as_secs_f64() * 1e6);
                assert_eq!(fingerprint(&field), expected);
                black_box(field);
            }
            samples.sort_by(f64::total_cmp);
            // The drain's path: adjacency cached per graph revision, field and scratch reused.
            let mut adjacency_samples = Vec::with_capacity(21);
            let mut adjacency = ReverseAdjacency::default();
            for _ in 0..21 {
                let begin = Instant::now();
                adjacency.rebuild(black_box(&graph), TransitFlags::CAR);
                adjacency_samples.push(begin.elapsed().as_secs_f64() * 1e6);
            }
            adjacency_samples.sort_by(f64::total_cmp);
            let mut field = FlowField::default();
            let mut scratch = SearchScratch::default();
            field.build_on(&sources, &adjacency, &mut scratch);
            let mut cached_samples = Vec::with_capacity(21);
            for _ in 0..21 {
                let begin = Instant::now();
                field.build_on(black_box(&sources), black_box(&adjacency), &mut scratch);
                cached_samples.push(begin.elapsed().as_secs_f64() * 1e6);
                assert_eq!(fingerprint(&field), expected);
            }
            cached_samples.sort_by(f64::total_cmp);
            println!(
                "FLOW_BUILD_BENCH {}",
                serde_json::json!({"side":side,"nodes":graph.node_count(),"edges":graph.edge_count(),"samples":samples.len(),"median_us":samples[samples.len()/2],"adjacency_median_us":adjacency_samples[adjacency_samples.len()/2],"cached_median_us":cached_samples[cached_samples.len()/2],"setup_us":setup_us,"fingerprint":format!("{expected:016x}")})
            );
        }
    }

    fn grid_graph(side: u32) -> RegionGraph {
        let mut graph = RegionGraph::new();
        for y in 0..side {
            for x in 0..side {
                graph.add_node(
                    Vector3::new(x as f32 * 50.0, 0.0, y as f32 * 50.0),
                    NodeType::Junction,
                );
            }
        }
        for y in 0..side {
            for x in 0..side {
                let node = y * side + x;
                // Uneven costs, so ties and detours both occur.
                let cost = 1.0 + ((x * 7 + y * 3) % 5) as f32 * 0.25;
                if x + 1 < side {
                    graph.add_edge(make_edge(node, node + 1, cost));
                }
                if y + 1 < side {
                    graph.add_edge(make_edge(node, node + side, cost));
                }
            }
        }
        graph.rebuild_adjacency_list();
        graph
    }

    /// The system's fields equal fresh builds of their sources on the current graph: a marked
    /// zone with the same sources keeps its field, changed sources and a changed graph rebuild.
    #[test]
    fn system_fields_equal_fresh_builds_and_skip_unchanged_sources() {
        let mut graph = grid_graph(9);
        let revision = |build| FlowGraphRevision {
            cch_build: build,
            cch_metric: 0,
        };
        let mut commercial = vec![(4_u32, 3_usize), (40, 1), (80, 2)];
        let fresh = |sources: &[(u32, usize)], graph: &RegionGraph, flags| {
            (!sources.is_empty()).then(|| FlowField::build(sources, graph, flags))
        };
        let mut system = FlowFieldSystem::new();
        let rebuild =
            |system: &mut FlowFieldSystem, graph: &RegionGraph, build, sources: &[(u32, usize)]| {
                system.rebuild_dirty(graph, revision(build), |zone, _, out| {
                    if zone == ZoneType::Commercial {
                        out.extend_from_slice(sources);
                    }
                });
            };
        rebuild(&mut system, &graph, 1, &commercial);
        let check = |system: &FlowFieldSystem, graph: &RegionGraph, sources: &[(u32, usize)]| {
            assert_eq!(
                system.car(ZoneType::Commercial).cloned(),
                fresh(sources, graph, TransitFlags::CAR)
            );
            assert_eq!(
                system.foot(ZoneType::Commercial).cloned(),
                fresh(sources, graph, TransitFlags::FOOT)
            );
            assert!(system.car(ZoneType::Residential).is_none());
            assert!(system.foot(ZoneType::Industrial).is_none());
        };
        check(&system, &graph, &commercial);
        // Unchanged sources on the same graph: the field is kept as it is.
        system.slots[2].field.next_node[0] = 12_345;
        system.mark_zone_dirty(ZoneType::Commercial);
        rebuild(&mut system, &graph, 1, &commercial);
        assert_eq!(system.slots[2].field.next_node[0], 12_345);
        // Changed sources rebuild.
        commercial.push((20, 0));
        system.mark_zone_dirty(ZoneType::Commercial);
        rebuild(&mut system, &graph, 1, &commercial);
        check(&system, &graph, &commercial);
        // A graph change rebuilds every field, marked or not.
        system.slots[2].field.next_node[0] = 12_345;
        graph.edge_mut(3).deleted = true;
        graph.rebuild_adjacency_list();
        rebuild(&mut system, &graph, 2, &commercial);
        check(&system, &graph, &commercial);
        // So does marking everything.
        system.slots[2].field.next_node[0] = 12_345;
        system.mark_all_dirty();
        rebuild(&mut system, &graph, 2, &commercial);
        check(&system, &graph, &commercial);
        // A zone without sources has no field.
        system.mark_zone_dirty(ZoneType::Commercial);
        rebuild(&mut system, &graph, 2, &[]);
        check(&system, &graph, &[]);
    }
}
