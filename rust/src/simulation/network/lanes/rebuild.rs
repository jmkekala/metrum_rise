// SPDX-License-Identifier: GPL-2.0-only

//! Full and local lane reconstruction with deterministic indexed publication.

use super::super::graph::{Edge, RegionGraph};
use super::super::roadbed::{ArmKey, EdgeEnd, NodeWalkways, RoadProfile};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::{LaneEnd, build_one_lane};
use super::pedestrian_junctions::{build_pedestrian_connections_at_node, footpath_end_insets};
use super::vehicle_junctions::build_vehicle_connections_at_node;
use super::{LaneSystem, LaneType};
use crate::config;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

// Junction-layout walkways by valid node id, for one rebuild.
type Walkways = HashMap<u32, NodeWalkways>;

// Lays out `nodes` (valid ids); nodes without a layout have no entry. The same graph-to-layout
// function node pieces use, so lanes need no compiled roadbed. O(sum of the nodes' layouts).
// A full rebuild lays out every node on the shared pool; an incremental closure holds a handful
// of nodes, which cost less to lay out than to dispatch to the pool.
fn collect_walkways(graph: &RegionGraph, nodes: &[u32], parallel: bool) -> Walkways {
    let walkways = |node: u32| Some((node, NodeWalkways::collect(graph, node)?));
    if parallel {
        crate::thread_pools::install(|| {
            nodes
                .par_iter()
                .filter_map(|&node| walkways(node))
                .collect()
        })
    } else {
        nodes.iter().filter_map(|&node| walkways(node)).collect()
    }
}

/// Lateral offset of road sidewalk lane `lane_idx` (100 or -100) in its edge's forward frame:
/// the centre of the edge profile's raised walkway, midway between the carriageway edge and the
/// sidewalk's outer edge, where the junction layout's walkways run too.
pub(super) fn sidewalk_offset(edge: &Edge, lane_idx: i8) -> f32 {
    let profile = RoadProfile::for_edge(edge);
    let centre_m = 0.5 * (profile.carriageway_half_width_m() + profile.half_width_m());
    let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
    let side = if lane_idx > 0 { -1.0 } else { 1.0 };
    side * centre_m * side_mul
}

impl LaneSystem {
    /// Completely rebuilds all physical lane geometry and connection splines for the entire graph.
    /// To be called after the road network topology and physical geometries have been updated.
    pub fn rebuild(&mut self, graph: &mut RegionGraph) {
        self.clear();

        // Maps (edge_id, is_fwd, lane_idx) -> lane_index in self.lanes
        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();
        let nodes: Vec<u32> = (0..graph.node_count().min(graph.node_adjacency_count()) as u32)
            .filter(|&node| graph.get_valid_node(node) == node)
            .collect();
        let walkways = collect_walkways(graph, &nodes, true);

        // Append in graph order because lane IDs are referenced by agents and connectors.
        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            self.append_edge_lanes(&mut lane_map, graph, &walkways, edge_idx, edge);
        }

        // Build node connections after all physical lanes exist.
        for node_id in 0..graph.node_count() {
            self.append_node_connections(&lane_map, graph, &walkways, node_id);
        }
    }

    fn append_node_connections(
        &mut self,
        lane_map: &HashMap<(usize, bool, i8), usize>,
        graph: &mut RegionGraph,
        walkways: &Walkways,
        node_id: usize,
    ) {
        build_vehicle_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
        );
        build_pedestrian_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
            &mut self.crosswalks,
            walkways.get(&(node_id as u32)),
        );
        // Connectors are pushed one at a time; keep only the ids.
        if let Some(ids) = self.node_lanes.get_mut(&node_id) {
            ids.shrink_to_fit();
        }
    }

    fn append_edge_lanes(
        &mut self,
        lane_map: &mut HashMap<(usize, bool, i8), usize>,
        graph: &RegionGraph,
        walkways: &Walkways,
        edge_idx: usize,
        edge: &Edge,
    ) {
        if edge.deleted || edge.physical_geometry.len() < 2 {
            return;
        }
        // A pure function of the junction geometry, so partial rebuilds trim footpaths alike.
        let (start_inset, end_inset) = if edge.primary_type == TransitType::Foot {
            footpath_end_insets(graph, edge_idx)
        } else {
            (0.0, 0.0)
        };
        let insets = [LaneEnd::Inset(start_inset), LaneEnd::Inset(end_inset)];
        // Road sidewalks end in their nodes' layouts; at a node without one they keep a fixed
        // inset beyond the clip, room for the walk round the node.
        let sidewalk_ends = |offset: f32| {
            [
                (edge.start_node, EdgeEnd::Start),
                (edge.end_node, EdgeEnd::End),
            ]
            .map(|(node, end)| {
                walkways
                    .get(&graph.get_valid_node(node))
                    .and_then(|walkways| {
                        let arm = ArmKey {
                            edge: edge_idx,
                            end,
                        };
                        walkways.sidewalk_end(arm, offset > 0.0)
                    })
                    .map_or(LaneEnd::Inset(config::SIDEWALK_NODE_INSET), |walk_end| {
                        let point = walk_end.point;
                        LaneEnd::Layout {
                            station_m: walk_end.station_m,
                            point: Vector3::new(point.x as f32, point.y as f32, point.z as f32),
                        }
                    })
            })
        };

        let mut edge_lane_indices = Vec::new();

        let mut build_lane =
            |is_fwd: bool, l_idx: i8, l_type: LaneType, l_off: f32, ends: [LaneEnd; 2]| {
                build_one_lane(
                    &mut self.lanes,
                    lane_map,
                    &mut edge_lane_indices,
                    edge_idx,
                    edge,
                    is_fwd,
                    l_idx,
                    l_type,
                    l_off,
                    ends,
                );
            };

        let lane_w = config::LANE_WIDTH;
        let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };

        // 1. Forward Lanes
        for l in 0..edge.fwd_lanes {
            // Lane 0 is closest to center
            let lane_offset = (l as f32 + 0.5) * lane_w * side_mul;
            build_lane(true, l as i8, LaneType::Vehicle, lane_offset, insets);
        }

        // 2. Backward Lanes
        for l in 0..edge.bkw_lanes {
            // Lane 0 is closest to center
            let lane_offset = -(l as f32 + 0.5) * lane_w * side_mul;
            build_lane(
                false,
                -(l as i8) - 1,
                LaneType::Vehicle,
                lane_offset,
                insets,
            );
        }

        // 3. Sidewalks
        if (edge.allowed_types & TransitFlags::FOOT) != 0 {
            if edge.primary_type == TransitType::Foot {
                // Dedicated Footpath: center lane
                build_lane(true, 0, LaneType::Foot, 0.0, insets);
                build_lane(false, 0, LaneType::Foot, 0.0, insets);
            } else {
                // Left (idx 100) and right (idx -100) sidewalks.
                for l_idx in [100, -100] {
                    let offset = sidewalk_offset(edge, l_idx);
                    let ends = sidewalk_ends(offset);
                    build_lane(true, l_idx, LaneType::Foot, offset, ends);
                    build_lane(false, l_idx, LaneType::Foot, offset, ends);
                }
            }
        }

        edge_lane_indices.shrink_to_fit();
        self.edge_lanes.insert(edge_idx, edge_lane_indices);
    }

    /// Returns the physical-edge closure rebuilt by an incremental lane update.
    ///
    /// Every live edge incident to an endpoint of `affected_edges` is included because a changed
    /// junction clip moves the lane mouth on all of its road arms.
    pub(crate) fn incremental_rebuild_edge_closure(
        graph: &RegionGraph,
        affected_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        let mut affected_nodes = HashSet::new();
        for &edge_id in affected_edges {
            if edge_id >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_id);
            affected_nodes.insert(edge.start_node as usize);
            affected_nodes.insert(edge.end_node as usize);
        }

        let mut rebuild_set = affected_edges.clone();
        for node_id in affected_nodes {
            if node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_id in graph.node_adjacency(node_id as u32) {
                if !graph.edge(edge_id).deleted {
                    rebuild_set.insert(edge_id);
                }
            }
        }
        rebuild_set
    }

    /// Incrementally rebuilds changed physical lanes and every connection lane touching them.
    ///
    /// Existing adjacency bounds discovery and lane-map work to local incident lanes. Sorting
    /// affected edges/nodes costs O(K log K); geometry and connector work remains per owner.
    /// Publication is ordered because appended lane IDs are part of simulation state.
    pub fn rebuild_edges_incremental(
        &mut self,
        graph: &mut RegionGraph,
        affected_edges: &HashSet<usize>,
    ) {
        if affected_edges.is_empty() {
            return;
        }
        self.revision = self.revision.wrapping_add(1);

        // Expand to every road arm whose lane mouth can move with an affected junction.
        let mut rebuild_edges: Vec<_> =
            Self::incremental_rebuild_edge_closure(graph, affected_edges)
                .into_iter()
                .collect();
        rebuild_edges.sort_unstable();

        // Connection lanes must be rebuilt at both ends of every rebuilt edge. Restricting this
        // to the original dirty endpoints leaves the far end pointing at orphaned physical lanes.
        let mut affected_nodes: HashSet<usize> = HashSet::new();
        for &edge_id in &rebuild_edges {
            if edge_id < graph.edge_count() {
                let edge = graph.edge(edge_id);
                affected_nodes.insert(edge.start_node as usize);
                affected_nodes.insert(edge.end_node as usize);
            }
        }

        let mut affected_nodes: Vec<_> = affected_nodes.into_iter().collect();
        affected_nodes.sort_unstable();
        let mut layout_nodes: Vec<u32> = affected_nodes
            .iter()
            .filter(|&&node_id| node_id < graph.node_count())
            .map(|&node_id| graph.get_valid_node(node_id as u32))
            .collect();
        layout_nodes.sort_unstable();
        layout_nodes.dedup();
        let walkways = collect_walkways(graph, &layout_nodes, false);

        // Retire old physical-lane ownership before appending replacements.
        for &e_id in &rebuild_edges {
            self.edge_lanes.remove(&e_id);
        }

        // Only incident surviving lanes are needed by the local connection builders. Clear
        // arrivals at rebuilt nodes; the opposite direction still owns its remote connections.
        let mut lane_map = HashMap::new();
        for &node_id in &affected_nodes {
            if node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_idx in graph.node_adjacency(node_id as u32) {
                let edge = graph.edge(edge_idx);
                if let Some(lane_ids) = self.edge_lanes.get(&edge_idx) {
                    for &lane_id in lane_ids {
                        let lane = &mut self.lanes[lane_id];
                        let terminal = if lane.is_fwd {
                            edge.end_node
                        } else {
                            edge.start_node
                        };
                        if terminal as usize == node_id {
                            lane.next_lanes.clear();
                        }
                        lane_map.insert((edge_idx, lane.is_fwd, lane.lane_idx), lane_id);
                    }
                }
            }
        }

        for &edge_idx in &rebuild_edges {
            if let Some(edge) = graph.edges().get(edge_idx) {
                self.append_edge_lanes(&mut lane_map, graph, &walkways, edge_idx, edge);
            }
        }

        // Rebuild connections in stable node order.
        for &node_id in &affected_nodes {
            if node_id < graph.node_count() {
                // Tombstone old connection lanes at this node so the renderer skips them, and
                // free their buffers: ids are never reused, so tombstones stay in `lanes`.
                if let Some(old_ids) = self.node_lanes.remove(&node_id) {
                    for lid in old_ids {
                        if let Some(lane) = self.lanes.get_mut(lid) {
                            self.crosswalks.remove(&lid);
                            lane.geometry = Vec::new();
                            lane.cum_dist = Vec::new();
                            lane.next_lanes.clear();
                        }
                    }
                }
                self.append_node_connections(&lane_map, graph, &walkways, node_id);
            }
        }
    }
}
