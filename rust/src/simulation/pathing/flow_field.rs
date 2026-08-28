// ========================================================================
//  MANIFEST
// ========================================================================
//  script_name: flow_field.rs
//  script_path: rust/src/simulation/pathing/flow_field.rs
//  module_name: flow_field
//  version: 0.1.0
//  description: Multi-source reverse Dijkstra per zone type, answering
//  kind: module
//  spec: none
//  internal_dependencies: [graph, zoning]
//  external_dependencies: []
//  features: [flow-field, dijkstra, zone-routing, lazy-rebuild]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// ========================================================================

//! Flow-field routing: multi-source reverse Dijkstra per zone type.
//!
//! A [`FlowField`] answers "from any node V, which node should I move to next
//! in order to reach the nearest building of a given zone type?" in O(1) per
//! agent per step. This replaces per-agent CCH `find_path` calls for routine
//! work/shop trips, reducing per-tick pathfinding cost from O(A × CCH) to
//! O(M × (V+E) log V) where M ≤ 6 (zone types) and A ≫ V.
//!
//! Flow fields are rebuilt lazily when their zone type is marked dirty.
//! Dirty flags are set by topology changes and by building spawns/removals.
//!
//! ## IDM hook
//!
//! [`FlowField::look_ahead`] returns the next N nodes from any position by
//! chaining `next_node` lookups. IDM can call this cheaply to anticipate the
//! upcoming lane sequence for gap-acceptance and merge decisions.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::simulation::zoning::ZoneType;
use std::collections::BinaryHeap;

// ========================================================================
// HEAP ORDERING
// ========================================================================

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

// ========================================================================
// ONE FIELD
// ========================================================================

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
    /// Complexity: O((V + E) log V).
    pub fn build(sources: &[(u32, usize)], graph: &RegionGraph, flags: u8) -> Self {
        let node_count = graph.node_count();
        let mut dist = vec![f32::INFINITY; node_count];
        let mut next_node = vec![u32::MAX; node_count];
        let mut nearest_building = vec![usize::MAX; node_count];
        let mut nearest_source_node = vec![u32::MAX; node_count];

        // Reverse adjacency: rev_adj[u] = [(v, cost)] meaning
        // "the original graph has edge v → u with this cost, so in the
        //  reverse graph u → v costs the same."
        // When we process u from the heap, we update v: from v, go to u.
        let mut rev_adj: Vec<Vec<(u32, f32)>> = vec![Vec::new(); node_count];
        for edge in graph.edges() {
            if edge.deleted {
                continue;
            }
            if (edge.allowed_types & flags) == 0 {
                continue;
            }
            let cost = edge.base_cost.max(1e-6);
            let s = edge.start_node as usize;
            let e = edge.end_node as usize;
            if edge.fwd_lane_count() > 0 && e < node_count && s < node_count {
                // Original forward edge: s → e.  Reverse: from e, update s.
                rev_adj[e].push((edge.start_node, cost));
            }
            if edge.bkw_lane_count() > 0 && s < node_count && e < node_count {
                // Original backward edge: e → s.  Reverse: from s, update e.
                rev_adj[s].push((edge.end_node, cost));
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
                || (dist[fn_idx] == 0.0
                    && (building_idx < nearest_building[fn_idx]
                        || (building_idx == nearest_building[fn_idx]
                            && source_node < nearest_source_node[fn_idx])));
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

    /// Returns the next node to move to from `node`, or `u32::MAX` if unreachable.
    #[inline]
    pub fn next_hop(&self, node: u32) -> u32 {
        self.next_node
            .get(node as usize)
            .copied()
            .unwrap_or(u32::MAX)
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

    /// Returns up to `hops` next nodes from `node` by chaining `next_node` lookups.
    ///
    /// Used by IDM for multi-hop lane lookahead: pass the result to the car-following
    /// model to determine which edges the agent will traverse in the next few seconds.
    /// Returns an empty Vec if the agent is at a source node or `node` is unreachable.
    pub fn look_ahead(&self, node: u32, hops: usize) -> Vec<u32> {
        let mut result = Vec::with_capacity(hops);
        let mut current = node;
        for _ in 0..hops {
            let next = self
                .next_node
                .get(current as usize)
                .copied()
                .unwrap_or(u32::MAX);
            if next == u32::MAX || next == current {
                break;
            }
            result.push(next);
            current = next;
        }
        result
    }
}

// ========================================================================
// ZONE SLOTS
// ========================================================================

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

// ========================================================================
// THE FIELD SET
// ========================================================================

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

// ========================================================================
// TESTS
// ========================================================================

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
            lanes: crate::simulation::network::graph::LaneLayout::from_counts(1, 1),
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
            frontage_class: Default::default(),
        }
    }

    /// Linear graph: n0 — n1 — n2 — n3 (dest)
    /// Flow field from n3 should route: n0→n1→n2→n3, n1→n2→n3, n2→n3.
    #[test]
    fn test_flow_field_linear_chain() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = g.add_node(Vector3::new(150.0, 0.0, 0.0), NodeType::Junction);
        g.add_edge(make_edge(n0, n1, 1.0));
        g.add_edge(make_edge(n1, n2, 1.0));
        g.add_edge(make_edge(n2, n3, 1.0));
        g.rebuild_adjacency_list();

        let sources = vec![(n3, 0usize)];
        let ff = FlowField::build(&sources, &g, TransitFlags::CAR);

        assert_eq!(ff.next_hop(n0), n1, "n0 should route to n1");
        assert_eq!(ff.next_hop(n1), n2, "n1 should route to n2");
        assert_eq!(ff.next_hop(n2), n3, "n2 should route to n3");
        assert_eq!(ff.nearest_building[n0 as usize], 0);
    }

    /// Build path from n0 should give [n0, n1, n2, n3].
    #[test]
    fn test_flow_field_build_path() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = g.add_node(Vector3::new(150.0, 0.0, 0.0), NodeType::Junction);
        g.add_edge(make_edge(n0, n1, 1.0));
        g.add_edge(make_edge(n1, n2, 1.0));
        g.add_edge(make_edge(n2, n3, 1.0));
        g.rebuild_adjacency_list();

        let ff = FlowField::build(&[(n3, 0)], &g, TransitFlags::CAR);
        let path = ff.build_path(n0, 100).expect("path should exist");
        assert_eq!(path, vec![n0, n1, n2, n3]);
    }

    /// Multi-source: two destinations n2 and n3. From n0 via equal-cost paths,
    /// it should route to the nearer one (n2 via n0→n1→n2 costs 2, n3 via n0→n1→n2→n3 costs 3).
    #[test]
    fn test_flow_field_multi_source_picks_nearest() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = g.add_node(Vector3::new(150.0, 0.0, 0.0), NodeType::Junction);
        g.add_edge(make_edge(n0, n1, 1.0));
        g.add_edge(make_edge(n1, n2, 1.0));
        g.add_edge(make_edge(n2, n3, 1.0));
        g.rebuild_adjacency_list();

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

        let ff = FlowField::build(&[(n1, 1usize), (n2, 0usize)], &g, TransitFlags::CAR);
        assert_eq!(ff.nearest_building[n0 as usize], 0);
        let path = ff.build_path(n0, 100).unwrap();
        assert_eq!(*path.last().unwrap(), n2);
    }

    /// look_ahead from n0 with 3 hops should return [n1, n2, n3].
    #[test]
    fn test_flow_field_look_ahead() {
        let mut g = RegionGraph::new();
        let n0 = g.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n1 = g.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        let n2 = g.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n3 = g.add_node(Vector3::new(150.0, 0.0, 0.0), NodeType::Junction);
        g.add_edge(make_edge(n0, n1, 1.0));
        g.add_edge(make_edge(n1, n2, 1.0));
        g.add_edge(make_edge(n2, n3, 1.0));
        g.rebuild_adjacency_list();

        let ff = FlowField::build(&[(n3, 0)], &g, TransitFlags::CAR);
        let ahead = ff.look_ahead(n0, 3);
        assert_eq!(ahead, vec![n1, n2, n3]);
    }

    /// Timing test: FlowField on a 1000-node linear chain must build in < 5 ms.
    #[test]
    fn test_flow_field_build_under_5ms_on_1000_nodes() {
        let mut g = RegionGraph::new();
        let mut prev = g.add_node(Vector3::ZERO, NodeType::Junction);
        for i in 1..1000u32 {
            let n = g.add_node(Vector3::new(i as f32 * 10.0, 0.0, 0.0), NodeType::Junction);
            g.add_edge(make_edge(prev, n, 1.0));
            prev = n;
        }
        g.rebuild_adjacency_list();

        let sources = vec![(prev, 0usize)]; // dest = last node
        let start = std::time::Instant::now();
        let _ff = FlowField::build(&sources, &g, TransitFlags::CAR);
        let elapsed = start.elapsed();
        assert!(
            elapsed.as_millis() < 5,
            "FlowField build on 1000-node graph took {}ms, expected < 5ms",
            elapsed.as_millis()
        );
    }
}
