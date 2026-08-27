// =========================================================================
//  MANIFEST
// =========================================================================
//  script_name: rebuild.rs
//  script_path: rust/src/simulation/network/lanes/rebuild.rs
//  module_name: rebuild
//  version: 0.1.0
//  description: Rebuilds all physical lane geometry and junction
//           connection splines from the graph. Runs as a full clear and
//           regenerate rather than an incremental update, because a lane's
//           identity is its (edge, direction, index) triple and any
//           topology edit renumbers enough of those that patching is more
//           error-prone than rebuilding. Straight lanes are built first,
//           then vehicle and pedestrian connections are woven at each node
//           from the resulting map.
//  kind: module
//  spec: none
//  internal_dependencies: [graph, geometry]
//  external_dependencies: []
//  features: [lane-rebuild, junction-connections, lane-geometry]
//  api_version: metrum-v1.0.0
//  last_updated: 2026-08-24
// =========================================================================

use super::super::graph::{LaneDirection, RegionGraph};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::build_one_lane;
use super::pedestrian_junctions::build_pedestrian_connections_at_node;
use super::vehicle_junctions::build_vehicle_connections_at_node;
use super::{LaneSystem, LaneType};
use crate::config;
use std::collections::{HashMap, HashSet};

impl LaneSystem {
    /// Completely rebuilds all physical lane geometry and connection splines for the entire graph.
    /// To be called after the road network topology and physical geometries have been updated.
    pub fn rebuild(&mut self, graph: &mut RegionGraph) {
        self.clear();

        // Maps (edge_id, is_fwd, lane_idx) -> lane_index in self.lanes
        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // 1. Build Straight Lanes for all active edges
        for (edge_idx, edge) in graph.edges().iter().enumerate() {
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }

            let mut edge_lane_indices = Vec::new();

            // Helper to build a lane (using the one from geometry module for consistency)
            let mut build_lane = |is_fwd: bool, l_idx: i8, l_type: LaneType, l_off: f32| {
                build_one_lane(
                    &mut self.lanes,
                    &mut lane_map,
                    &mut edge_lane_indices,
                    edge_idx,
                    edge,
                    is_fwd,
                    l_idx,
                    l_type,
                    l_off,
                );
            };

            let sidewalk_w = config::SIDEWALK_WIDTH;
            let layout = edge.lane_layout();
            let asphalt_width = layout.asphalt_width();
            let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };

            // Lane offsets come from the layout, which accumulates each band's
            // real width. The old form was (index + 0.5) * LANE_WIDTH, correct
            // only while every lane was the same width, and it is a wide truck
            // lane or a median that breaks it. For a layout built from counts
            // the two agree exactly.
            //
            // Lane indices keep their existing meaning for everything
            // downstream: forward lanes count outward from the centre from 0,
            // backward lanes from -1. Medians and other non-travel bands take
            // width and receive no lane of their own.
            // Backward lanes are stored outermost-first but indexed
            // innermost-first, so they are walked in reverse to keep index -1
            // on the lane nearest the centre, which is what every consumer
            // downstream already assumes.
            let mut fwd_seen: i8 = 0;
            for (band, lane) in layout.lanes().iter().enumerate() {
                if lane.direction != LaneDirection::Forward || !lane.carries(TransitFlags::CAR) {
                    continue;
                }
                if let Some(offset) = layout.centre_offset(band) {
                    build_lane(true, fwd_seen, LaneType::Vehicle, offset * side_mul);
                    fwd_seen += 1;
                }
            }
            let mut bkw_seen: i8 = 0;
            for (band, lane) in layout.lanes().iter().enumerate().rev() {
                if lane.direction != LaneDirection::Backward || !lane.carries(TransitFlags::CAR) {
                    continue;
                }
                if let Some(offset) = layout.centre_offset(band) {
                    build_lane(false, -bkw_seen - 1, LaneType::Vehicle, offset * side_mul);
                    bkw_seen += 1;
                }
            }

            // 3. Sidewalks
            if (edge.allowed_types & TransitFlags::FOOT) != 0 {
                if edge.primary_type == TransitType::Foot {
                    // Dedicated Footpath: center lane
                    build_lane(true, 0, LaneType::Foot, 0.0);
                    build_lane(false, 0, LaneType::Foot, 0.0);
                } else {
                    // Left Sidewalk (idx 100)
                    let left_offset = -(asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_lane(true, 100, LaneType::Foot, left_offset);
                    build_lane(false, 100, LaneType::Foot, left_offset);

                    // Right Sidewalk (idx -100)
                    let right_offset = (asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_lane(true, -100, LaneType::Foot, right_offset);
                    build_lane(false, -100, LaneType::Foot, right_offset);
                }
            }

            self.edge_lanes.insert(edge_idx, edge_lane_indices);
        }

        // 2. Build Connection Lanes (Intersections)
        for node_id in 0..graph.node_count() {
            build_vehicle_connections_at_node(
                &mut self.lanes,
                &lane_map,
                graph,
                node_id,
                &mut self.node_lanes,
            );
            build_pedestrian_connections_at_node(
                &mut self.lanes,
                &lane_map,
                graph,
                node_id,
                &mut self.node_lanes,
            );
        }

        // Crossing movements are pure geometry over the connectors just built,
        // so they are computed here rather than per tick.
        for node_id in 0..graph.node_count() {
            self.rebuild_node_conflicts(node_id);
        }
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
    pub fn rebuild_edges_incremental(
        &mut self,
        graph: &mut RegionGraph,
        affected_edges: &HashSet<usize>,
    ) {
        if affected_edges.is_empty() {
            return;
        }

        // 1. Expand to every road arm whose lane mouth can move with an affected junction.
        let rebuild_set = Self::incremental_rebuild_edge_closure(graph, affected_edges);

        // Connection lanes must be rebuilt at both ends of every rebuilt edge. Restricting this
        // to the original dirty endpoints leaves the far end pointing at orphaned physical lanes.
        let mut affected_nodes: HashSet<usize> = HashSet::new();
        for &edge_id in &rebuild_set {
            if edge_id < graph.edge_count() {
                let edge = graph.edge(edge_id);
                affected_nodes.insert(edge.start_node as usize);
                affected_nodes.insert(edge.end_node as usize);
            }
        }

        // 2. Orphan old road lanes for every edge in rebuild_set.
        for &e_id in &rebuild_set {
            self.edge_lanes.remove(&e_id);
        }

        // 3. Clear next_lanes on non-orphaned lanes at affected nodes.
        for &node_id in &affected_nodes {
            if node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &e_id in graph.node_adjacency(node_id as u32) {
                if let Some(lane_ids) = self.edge_lanes.get(&e_id) {
                    let ids: Vec<usize> = lane_ids.clone();
                    for lid in ids {
                        self.lanes[lid].next_lanes.clear();
                    }
                }
            }
        }

        let mut lane_map: HashMap<(usize, bool, i8), usize> = HashMap::new();

        // 4. Pre-populate lane_map from surviving (non-rebuilt) edges so that connection
        //    builders at affected nodes can route through arms that weren't touched.
        for (&edge_idx, lane_ids) in &self.edge_lanes {
            for &lid in lane_ids {
                let lane = &self.lanes[lid];
                lane_map.insert((edge_idx, lane.is_fwd, lane.lane_idx), lid);
            }
        }

        // 5. Append new straight lanes for every edge in rebuild_set.
        for &edge_idx in &rebuild_set {
            if edge_idx >= graph.edge_count() {
                continue;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || edge.physical_geometry.len() < 2 {
                continue;
            }

            let mut edge_lane_indices = Vec::new();
            let sidewalk_w = config::SIDEWALK_WIDTH;
            let layout = edge.lane_layout();
            let asphalt_width = layout.asphalt_width();
            let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };

            // The same layout walk as the full rebuild above, and it has to
            // stay the same. When this path computed offsets from lane counts
            // instead, a road drawn from scratch and the same road after an
            // edit produced different geometry, and every band the counts
            // cannot express (a median, a bus lane, a cycle track, a turn
            // pocket) was silently dropped the moment anything touched it.
            let mut fwd_seen: i8 = 0;
            for (band, lane) in layout.lanes().iter().enumerate() {
                if lane.direction != LaneDirection::Forward || !lane.carries(TransitFlags::CAR) {
                    continue;
                }
                if let Some(offset) = layout.centre_offset(band) {
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        true,
                        fwd_seen,
                        LaneType::Vehicle,
                        offset * side_mul,
                    );
                    fwd_seen += 1;
                }
            }
            let mut bkw_seen: i8 = 0;
            for (band, lane) in layout.lanes().iter().enumerate().rev() {
                if lane.direction != LaneDirection::Backward || !lane.carries(TransitFlags::CAR) {
                    continue;
                }
                if let Some(offset) = layout.centre_offset(band) {
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        false,
                        -bkw_seen - 1,
                        LaneType::Vehicle,
                        offset * side_mul,
                    );
                    bkw_seen += 1;
                }
            }

            if (edge.allowed_types & TransitFlags::FOOT) != 0 {
                if edge.primary_type == TransitType::Foot {
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        true,
                        0,
                        LaneType::Foot,
                        0.0,
                    );
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        false,
                        0,
                        LaneType::Foot,
                        0.0,
                    );
                } else {
                    let left_off = -(asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        true,
                        100,
                        LaneType::Foot,
                        left_off,
                    );
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        false,
                        100,
                        LaneType::Foot,
                        left_off,
                    );
                    let right_off = (asphalt_width * 0.5 + sidewalk_w * 0.5) * side_mul;
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        true,
                        -100,
                        LaneType::Foot,
                        right_off,
                    );
                    build_one_lane(
                        &mut self.lanes,
                        &mut lane_map,
                        &mut edge_lane_indices,
                        edge_idx,
                        edge,
                        false,
                        -100,
                        LaneType::Foot,
                        right_off,
                    );
                }
            }
            self.edge_lanes.insert(edge_idx, edge_lane_indices);
        }

        // 6. Rebuild connections for every affected node.
        for &node_id in &affected_nodes {
            if node_id < graph.node_count() {
                // Tombstone old connection lanes at this node so the renderer skips them.
                if let Some(old_ids) = self.node_lanes.remove(&node_id) {
                    for lid in old_ids {
                        if lid < self.lanes.len() {
                            self.lanes[lid].crosswalk_edge_id = None;
                            self.lanes[lid].crosswalk_marking = None;
                            self.lanes[lid].geometry.clear();
                            self.lanes[lid].next_lanes.clear();
                        }
                    }
                }
                build_vehicle_connections_at_node(
                    &mut self.lanes,
                    &lane_map,
                    graph,
                    node_id,
                    &mut self.node_lanes,
                );
                build_pedestrian_connections_at_node(
                    &mut self.lanes,
                    &lane_map,
                    graph,
                    node_id,
                    &mut self.node_lanes,
                );
                self.rebuild_node_conflicts(node_id);
            }
        }
    }
}
