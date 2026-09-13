// SPDX-License-Identifier: GPL-2.0-only

//! Flow-field routing: multi-source reverse Dijkstra per zone type.
//!
//! Each field stores the next node and nearest destination building for one zone/mode.
//! Fields provide an optional path for an already selected trip; CCH remains authoritative.
//! Topology edits and building changes mark the affected fields dirty for the next tick.
//! Reverse Dijkstra stages compact incoming edges for cache locality during the search.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
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
// FlowField
// ---------------------------------------------------------------------------

/// A precomputed routing map from every reachable node toward the nearest
/// building of one zone type, for one transit mode (car or foot).
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
    pub fn build(sources: &[(u32, usize)], graph: &RegionGraph, flags: u8) -> Self {
        let node_count = graph.node_count();
        let mut dist = vec![f32::INFINITY; node_count];
        let mut next_node = vec![u32::MAX; node_count];
        let mut nearest_building = vec![usize::MAX; node_count];
        let mut nearest_source_node = vec![u32::MAX; node_count];

        // Keep compact incoming endpoints/costs together during Dijkstra. Direct traversal of
        // graph adjacency reads the large Edge records in search order and was slower on large
        // matched grids; this temporary table is a measured locality cache, not persistent state.
        let mut rev_adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); node_count];
        for edge in graph.edges() {
            if edge.deleted || edge.allowed_types & flags == 0 {
                continue;
            }
            let start = edge.start_node as usize;
            let end = edge.end_node as usize;
            let cost = edge.base_cost.max(1e-6);
            if edge.traversal_flags(true) & flags != 0 {
                rev_adj[end].push((edge.start_node, cost));
            }
            if edge.traversal_flags(false) & flags != 0 {
                rev_adj[start].push((edge.end_node, cost));
            }
        }

        // Seed all source nodes at cost 0.
        let mut heap = BinaryHeap::new();
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
            for &(v, cost) in &rev_adj[u as usize] {
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

        Self {
            next_node,
            nearest_building,
        }
    }

    /// Builds a path `Vec<u32>` from `from_node` to the nearest destination by
    /// following `next_node` hops. Returns `None` if `from_node` is unreachable
    /// or if the chain loops (cycle guard via `max_hops`).
    ///
    /// The path ends at the source node (a building frontage node).
    pub fn build_path(&self, from_node: u32, max_hops: usize) -> Option<Vec<u32>> {
        let from_idx = from_node as usize;
        if from_idx >= self.next_node.len() || self.next_node[from_idx] == u32::MAX {
            return None;
        }
        let mut path = Vec::with_capacity(32);
        path.push(from_node);
        let mut current = from_node;
        for _ in 0..max_hops {
            let next = self.next_node[current as usize];
            if next == u32::MAX {
                return None;
            }
            if next == current {
                // Already at a source node — path is complete.
                return Some(path);
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

fn flow_field_zone_slot(zone: ZoneType) -> Option<usize> {
    match zone {
        ZoneType::Residential => Some(0),
        ZoneType::Commercial => Some(1),
        ZoneType::Industrial => Some(2),
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => None,
    }
}

/// Manages one [`FlowField`] per zone type per transit mode, rebuilt lazily.
///
/// Attached to [`crate::simulation::network::TransitNetwork`].
/// Call [`FlowFieldSystem::mark_all_dirty`] on topology changes and
/// [`FlowFieldSystem::mark_zone_dirty`] when buildings of a zone type are added/removed.
pub struct FlowFieldSystem {
    /// `car_fields[z]` for one shipped baseline private-use family.
    pub car_fields: [Option<FlowField>; FLOW_FIELD_ZONE_COUNT],
    /// `foot_fields[z]` for one shipped baseline private-use family.
    pub foot_fields: [Option<FlowField>; FLOW_FIELD_ZONE_COUNT],
    /// Per-family dirty flags. Set to rebuild on next `rebuild_dirty` call.
    dirty: [bool; FLOW_FIELD_ZONE_COUNT],
}

impl FlowFieldSystem {
    /// Creates a new system with all fields absent and all zones dirty.
    pub fn new() -> Self {
        Self {
            car_fields: std::array::from_fn(|_| None),
            foot_fields: std::array::from_fn(|_| None),
            dirty: [true; FLOW_FIELD_ZONE_COUNT],
        }
    }

    /// Marks all zone types as dirty (called on road topology change).
    pub fn mark_all_dirty(&mut self) {
        self.dirty = [true; FLOW_FIELD_ZONE_COUNT];
    }

    /// Marks a single zone type as dirty (called on building spawn/removal).
    pub fn mark_zone_dirty(&mut self, zone: ZoneType) {
        if let Some(zone_idx) = flow_field_zone_slot(zone) {
            self.dirty[zone_idx] = true;
        }
    }

    /// Rebuilds flow fields for all dirty zone types.
    ///
    /// `sources_fn(zone, mode_flags) -> Vec<(endpoint_node, building_idx)>` is called
    /// once per dirty zone and transit mode to collect the current legal destination
    /// endpoint nodes. Passing a closure keeps this module free of a direct
    /// `BuildingAllocator` dependency.
    ///
    /// Complexity: O(dirty_zones × (V + E) log V).
    pub fn rebuild_dirty(
        &mut self,
        graph: &RegionGraph,
        sources_fn: impl Fn(ZoneType, u8) -> Vec<(u32, usize)>,
    ) {
        for (zone_idx, zone) in [
            ZoneType::Residential,
            ZoneType::Commercial,
            ZoneType::Industrial,
        ]
        .into_iter()
        .enumerate()
        {
            if !self.dirty[zone_idx] {
                continue;
            }
            let car_sources = sources_fn(zone, TransitFlags::CAR);
            let foot_sources = sources_fn(zone, TransitFlags::FOOT);
            self.car_fields[zone_idx] = if car_sources.is_empty() {
                None
            } else {
                Some(FlowField::build(&car_sources, graph, TransitFlags::CAR))
            };
            self.foot_fields[zone_idx] = if foot_sources.is_empty() {
                None
            } else {
                Some(FlowField::build(&foot_sources, graph, TransitFlags::FOOT))
            };
            self.dirty[zone_idx] = false;
        }
    }

    /// Returns the car flow field for `zone`, or `None` if not yet built or zone has no buildings.
    #[inline]
    pub fn car(&self, zone: ZoneType) -> Option<&FlowField> {
        self.car_fields[flow_field_zone_slot(zone)?].as_ref()
    }

    /// Returns the foot flow field for `zone`, or `None` if not yet built or zone has no buildings.
    #[inline]
    pub fn foot(&self, zone: ZoneType) -> Option<&FlowField> {
        self.foot_fields[flow_field_zone_slot(zone)?].as_ref()
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
            current_congestion: 0.0,
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
            println!(
                "FLOW_BUILD_BENCH {}",
                serde_json::json!({"side":side,"nodes":graph.node_count(),"edges":graph.edge_count(),"samples":samples.len(),"median_us":samples[samples.len()/2],"setup_us":setup_us,"fingerprint":format!("{expected:016x}")})
            );
        }
    }
}
