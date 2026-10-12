// SPDX-License-Identifier: GPL-2.0-only

//! Turn-aware customizable contraction hierarchy with allocation-free elimination-tree queries.

use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;
use crate::traffic_log;
use rayon::prelude::*;
use std::cell::RefCell;
#[cfg(test)]
use std::cmp::Ordering;
#[cfg(test)]
use std::collections::{BinaryHeap, HashMap};

mod bounds;
mod layout;
mod order;
mod query;
mod sweep;
mod topology;
pub(crate) use bounds::CchCarCostBounds;
use layout::{NodeSlices, VehicleTurns};
pub(crate) use query::buckets::{CchBucketSource, CchBuckets};
#[cfg(test)]
pub(crate) use query::restricted::{
    RESTRICTED_BATCH_LANES, RestrictedBatchScratch, RestrictedScratch, RestrictedTargets,
};
pub(crate) use query::{CchPathScratch, CchQueryScratch};
pub use sweep::CchCostsFrom;

thread_local! {
    static OWNED_PATH_SCRATCH: RefCell<CchPathScratch> = RefCell::default();
}

/// An undirected filled edge; each metric stores its two directions separately.
#[derive(Clone, Debug)]
pub struct CchShortcut {
    /// Lower-ranked arrival state.
    pub low: u32,
    /// Higher-ranked arrival state.
    pub high: u32,
}

#[derive(Clone, Copy)]
struct Departure {
    target: u32,
    edge: usize,
    flags: u8,
}
struct BaseArc {
    arc: usize,
    edge: usize,
    flags: u8,
    vehicle_turn: bool,
}
#[derive(Clone, Copy)]
struct Witness {
    left: usize,
    right: usize,
    edge: usize,
}
impl Default for Witness {
    fn default() -> Self {
        Self {
            left: usize::MAX,
            right: usize::MAX,
            edge: usize::MAX,
        }
    }
}
struct Metric {
    mask: u8,
    costs: Vec<f32>,
    distances: Vec<f32>,
    witnesses: Vec<Witness>,
}

/// A hierarchy over road nodes and explicit arrival states at restricted junctions.
/// Unrestricted arrivals are equivalent: their future legal transitions are identical.
/// Splitting only restricted nodes is an exact quotient of the directed-edge graph.
pub struct CchGraph {
    /// Metric-independent filled edges, in deterministic construction order.
    pub shortcuts: Vec<CchShortcut>,
    /// Changes on every rebuild, invalidating worker-local and shared route caches.
    pub build_generation: u32,
    /// Changes after customization, invalidating cached network costs.
    pub(crate) metric_revision: u64,
    up: NodeSlices<(u32, usize)>,
    parent: Vec<u32>,
    states: NodeSlices<u32>,
    departures: NodeSlices<Departure>,
    turns: VehicleTurns,
    base_arcs: Vec<BaseArc>,
    base_metric: Vec<(f32, f32)>,
    triangles: Vec<[usize; 3]>,
    profiles: Vec<Metric>,
    profile_for_mask: [u8; 256],
    node_order: Vec<u32>,
}

impl CchGraph {
    /// Creates an unbuilt hierarchy with no routes between distinct nodes.
    pub fn new(nodes: usize) -> Self {
        Self {
            shortcuts: Vec::new(),
            build_generation: 0,
            metric_revision: 0,
            up: NodeSlices::empty(0),
            parent: Vec::new(),
            states: NodeSlices::empty(nodes),
            departures: NodeSlices::empty(nodes),
            turns: VehicleTurns::empty(nodes),
            base_arcs: Vec::new(),
            base_metric: Vec::new(),
            triangles: Vec::new(),
            profiles: Vec::new(),
            profile_for_mask: [0; 256],
            node_order: Vec::new(),
        }
    }

    /// Recomputes directed metrics in O(profiles × (base transitions + lower triangles)).
    /// Independent profiles run with Rayon. Customization allocates no per-arc storage.
    pub fn customize(&mut self, graph: &RegionGraph) {
        self.metric_revision = self.metric_revision.wrapping_add(1);
        self.base_metric
            .par_iter_mut()
            .zip(graph.edges().par_iter())
            .for_each(|(metric, edge)| *metric = (edge.base_cost, edge.physical_length));
        self.profiles.par_iter_mut().for_each(|metric| {
            metric.costs.fill(f32::INFINITY);
            metric.distances.fill(0.0);
            metric.witnesses.fill(Witness::default());
            for base in &self.base_arcs {
                if base.flags & metric.mask == 0
                    || (!base.vehicle_turn && metric.mask != TransitFlags::FOOT)
                {
                    continue;
                }
                let edge = graph.edge(base.edge);
                if edge.base_cost < metric.costs[base.arc] {
                    metric.costs[base.arc] = edge.base_cost;
                    metric.distances[base.arc] = edge.physical_length;
                    metric.witnesses[base.arc].edge = base.edge;
                }
            }
            // For u < v < w, update v→w via v→u→w, and w→v via w→u→v.
            // Children have lower low-endpoint ranks and cannot change after this step.
            for &[uv, uw, vw] in &self.triangles {
                for (target, left, right) in [
                    (vw * 2, uv * 2 + 1, uw * 2),
                    (vw * 2 + 1, uw * 2 + 1, uv * 2),
                ] {
                    let cost = metric.costs[left] + metric.costs[right];
                    if cost < metric.costs[target] {
                        metric.costs[target] = cost;
                        metric.distances[target] = metric.distances[left] + metric.distances[right];
                        metric.witnesses[target] = Witness {
                            left,
                            right,
                            edge: usize::MAX,
                        };
                    }
                }
            }
        });
    }

    fn is_turn_allowed(&self, node: u32, incoming: usize, outgoing: usize, mask: u8) -> bool {
        mask == TransitFlags::FOOT
            || incoming == usize::MAX
            || outgoing == usize::MAX
            || self.turns.permits(node, incoming, outgoing)
    }

    /// Returns whether a vehicle turn from `in_edge` to `out_edge` at `node_data` is
    /// permitted under the global whitelist model.
    ///
    /// - If the node has **no** user-defined vehicle connections (lane_idx ≠ ±100) the
    ///   junction is fully open and all turns are allowed.
    /// - If **any** user vehicle connection exists anywhere on the node, the entire node
    ///   enters whitelist mode: only explicitly listed turns are permitted; all others
    ///   are blocked regardless of which arm they come from.
    ///
    /// Pedestrian lane entries (lane_idx == ±100) are always ignored so that the
    /// presence of sidewalk connections does not accidentally engage whitelist mode.
    fn vehicle_turn_allowed(
        node_data: &crate::simulation::network::graph::data::Node,
        in_edge: usize,
        out_edge: usize,
    ) -> bool {
        // Global whitelist: if the node has any user vehicle connection, all unspecified
        // turns are blocked. If the node has no user connections, all turns are open.
        if !Self::node_has_vehicle_whitelist(node_data) {
            return true; // open node
        }
        // Whitelist mode: check explicit vehicle turns only.
        for (&(e_from, lane_idx), targets) in &node_data.lane_connections {
            if lane_idx != 100 && lane_idx != -100 && e_from == in_edge {
                if targets.iter().any(|&(e_to, _)| e_to == out_edge) {
                    traffic_log!(
                        "[TURN_ALLOWED] in_edge={in_edge} out_edge={out_edge} -> TRUE (explicit connection)"
                    );
                    return true;
                }
            }
        }
        traffic_log!(
            "[TURN_BLOCKED] in_edge={in_edge} out_edge={out_edge} (whitelist active, no match)"
        );
        false
    }

    fn node_has_vehicle_whitelist(node: &crate::simulation::network::graph::data::Node) -> bool {
        node.lane_connections
            .keys()
            .any(|&(_, lane)| lane != 100 && lane != -100)
    }

    /// Returns `true` if every turn in `path` is permitted under the current vehicle
    /// restrictions. Pedestrian trips validate lane connectors separately. A path with fewer
    /// than 3 nodes has no intermediate junctions and
    /// is always valid. Paths where consecutive edge IDs cannot be resolved are skipped
    /// (treated as open).
    ///
    /// Complexity: O(path.len() × node_degree) — cheap for typical city paths.
    pub fn path_has_valid_vehicle_turns(path: &[u32], graph: &RegionGraph) -> bool {
        if path.len() < 3 {
            return true;
        }
        for i in 0..path.len() - 2 {
            let a = path[i];
            let jct = path[i + 1];
            let b = path[i + 2];
            let Some(in_e) = graph.get_edge_between_nodes(a, jct) else {
                continue;
            };
            let Some(out_e) = graph.get_edge_between_nodes(jct, b) else {
                continue;
            };
            if !Self::vehicle_turn_allowed(graph.node(jct), in_e, out_e) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
#[derive(Copy, Clone, Debug)]
struct CchState {
    cost: f32,
    node: u32,
    incoming_edge: usize,
}

#[cfg(test)]
impl Eq for CchState {}

#[cfg(test)]
impl PartialEq for CchState {
    fn eq(&self, other: &Self) -> bool {
        self.cost.to_bits() == other.cost.to_bits()
            && self.node == other.node
            && self.incoming_edge == other.incoming_edge
    }
}

#[cfg(test)]
impl Ord for CchState {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .cost
            .total_cmp(&self.cost)
            .then_with(|| other.node.cmp(&self.node))
            .then_with(|| other.incoming_edge.cmp(&self.incoming_edge))
    }
}

#[cfg(test)]
impl PartialOrd for CchState {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod ordering_tests;
#[cfg(test)]
mod state_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::graph::{Edge, Node, RegionGraph};
    use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
    use godot::prelude::Vector3;
    use std::collections::HashMap;

    fn setup_test_graph() -> RegionGraph {
        let mut graph = RegionGraph::new();

        let nodes = vec![
            Node {
                pos: Vector3::new(0.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(1200.0, 0.0, 0.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
            Node {
                pos: Vector3::new(600.0, 0.0, 600.0),
                node_type: NodeType::Junction,
                lane_connections: HashMap::new(),
                crosswalk_overrides: HashMap::new(),
            },
        ];

        let edge_defaults = Edge {
            start_node: 0,
            end_node: 0,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width: 10.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 20.0,
            base_cost: 30.0,
            physical_length: 600.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: Vec::new(),
            physical_geometry: Vec::new(),
            class: EdgeClass::Standard,
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        };

        let edges = vec![
            Edge {
                start_node: 0,
                end_node: 1,
                physical_length: 600.0,
                ..edge_defaults.clone()
            }, // 0
            Edge {
                start_node: 1,
                end_node: 2,
                physical_length: 600.0,
                ..edge_defaults.clone()
            }, // 1
            Edge {
                start_node: 0,
                end_node: 3,
                physical_length: 600.0,
                base_cost: 10.0,
                ..edge_defaults.clone()
            }, // 2
            Edge {
                start_node: 3,
                end_node: 2,
                physical_length: 600.0,
                base_cost: 10.0,
                ..edge_defaults.clone()
            }, // 3
        ];

        graph.set_nodes_edges_for_test(nodes, edges);
        graph.rebuild_adjacency_list();
        graph
    }

    #[test]
    fn test_cch_pathfinding() {
        let graph = setup_test_graph();
        let cch = CchGraph::build(&graph);
        let path_opt = cch.find_path(0, 2, usize::MAX, &graph, TransitFlags::CAR);
        assert!(path_opt.is_some());
        let (cost, _dist, sequence) = path_opt.unwrap();
        // Base cost of path 0->3->2 is 10 + 10 = 20.
        assert_eq!(cost, 20.0);
        assert_eq!(sequence, vec![0, 3, 2]);
    }
}
