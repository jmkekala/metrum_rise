// SPDX-License-Identifier: GPL-3.0-or-later

//! Full and local lane reconstruction with deterministic indexed publication.

use super::super::graph::{Edge, RegionGraph};
use super::super::roadbed::{
    ArmKey, Corner, EdgeEnd, LEFT_POCKET_TAPER_M, NodeWalkways, PocketNarrowing, RoadProfile,
};
use super::super::types::{TransitFlags, TransitType};
use super::geometry::{LaneEnd, LaneTaper, build_one_lane};
use super::pedestrian_junctions::{build_pedestrian_connections_at_node, footpath_end_insets};
use super::vehicle_junctions::{
    build_vehicle_connections_at_node, vehicle_lane_end, vehicle_mouth_frame,
};
use super::identity::{LaneIdentity, RoadLaneKey};
use super::{LaneCrosswalk, LaneSystem, LaneType};
use crate::config;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

// Junction-layout products of one node for the lane rebuild: its walkways and the kerb corners
// its vehicle connectors keep clear of.
struct NodeLayoutLanes {
    walkways: Option<NodeWalkways>,
    kerb_corners: Option<Vec<Corner>>,
}

// Junction-layout products by valid node id, for one rebuild.
type Walkways = HashMap<u32, NodeLayoutLanes>;

// The ids a local rebuild retires, by the identity a rebuilt lane can claim them with: each list
// descending, so the lowest id is popped first (`PERF-28`).
type RetiredIds = HashMap<LaneIdentity, Vec<usize>>;

// Lays out `nodes` (valid ids); nodes with neither walkways nor a layout have no entry. The same
// graph-to-layout function node pieces use, so lanes need no compiled roadbed. O(sum of the
// nodes' layouts). A full rebuild lays out every node on the shared pool; an incremental closure
// holds a handful of nodes, which cost less to lay out than to dispatch to the pool.
fn collect_walkways(graph: &RegionGraph, nodes: &[u32], parallel: bool) -> Walkways {
    let walkways = |node: u32| {
        let (walkways, kerb_corners) = NodeWalkways::collect_with_kerbs(graph, node);
        (walkways.is_some() || kerb_corners.is_some()).then_some((
            node,
            NodeLayoutLanes {
                walkways,
                kerb_corners,
            },
        ))
    };
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
/// the centre of the edge profile's raised walkway on that lane's side, midway between the
/// carriageway edge and the sidewalk's outer edge, where the junction layout's walkways run too.
/// Per side, so a pocket edge's moved kerb (`ROAD-95`) moves its sidewalk lane with it.
pub(crate) fn sidewalk_offset(edge: &Edge, lane_idx: i8) -> f32 {
    let profile = RoadProfile::for_edge(edge);
    let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
    let side = if lane_idx > 0 { -1.0 } else { 1.0 };
    let (carriageway_m, outer_m) =
        profile.side_half_widths_m(sidewalk_lane_on_positive_side(lane_idx));
    let centre_m = 0.5 * (carriageway_m + outer_m);
    side * centre_m * side_mul
}

// Whether road sidewalk lane `lane_idx` (100 or -100) runs on its edge's positive lateral side,
// the side [`sidewalk_offset`] places it on: -100 under right-hand traffic.
fn sidewalk_lane_on_positive_side(lane_idx: i8) -> bool {
    (lane_idx > 0) == config::DRIVE_ON_LEFT
}

/// Lateral offset of vehicle lane `lane_idx` in `edge`'s forward frame: forward lanes
/// `0, 1, ..` and backward lanes `-1, -2, ..`, each a lane further out than the last from the
/// line between the two lane groups ([`Edge::lane_group_offset`], `ROAD-100`), on the driving
/// side, each group half the median's width off that line (`ROAD-80`). `O(1)`.
pub(crate) fn vehicle_lane_offset(edge: &Edge, lane_idx: i8) -> f32 {
    let side_mul = if config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
    let median_m = if lane_idx >= 0 {
        edge.median_half_width()
    } else {
        -edge.median_half_width()
    };
    edge.lane_group_offset()
        + ((f32::from(lane_idx) + 0.5) * config::LANE_WIDTH + median_m) * side_mul
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
        let layout = walkways.get(&(node_id as u32));
        build_vehicle_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
            &mut self.ring_entries,
            layout
                .and_then(|layout| layout.kerb_corners.as_deref())
                .unwrap_or_default(),
        );
        build_pedestrian_connections_at_node(
            &mut self.lanes,
            lane_map,
            graph,
            node_id,
            &mut self.node_lanes,
            &mut self.crosswalks,
            layout.and_then(|layout| layout.walkways.as_ref()),
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
        let ends_at = [
            (edge.start_node, EdgeEnd::Start),
            (edge.end_node, EdgeEnd::End),
        ];
        let layout_end = |walk_end: super::super::roadbed::SidewalkEnd| {
            let point = walk_end.point;
            LaneEnd::Layout {
                station_m: walk_end.station_m,
                point: Vector3::new(point.x as f32, point.y as f32, point.z as f32),
            }
        };
        let node_walkways = |node: u32| {
            walkways
                .get(&graph.get_valid_node(node))
                .and_then(|layout| layout.walkways.as_ref())
        };
        // Vehicle lanes end on their arms' mouth frames at the edge's clips (`TRAFFIC-12`).
        let frames = ends_at.map(|(_, end)| vehicle_mouth_frame(graph, edge_idx, end));
        let vehicle_ends = |lane_idx: i8| {
            [0, 1].map(|index| {
                let end = ends_at[index].1;
                frames[index].map_or(LaneEnd::Inset(0.0), |frame| LaneEnd::Mouth {
                    station_m: frame.station_m,
                    point: vehicle_lane_end(&frame, end, edge, lane_idx).point,
                })
            })
        };
        // A footpath ends at its mouth in its node's layout; elsewhere its trim is a pure
        // function of the junction geometry, so partial rebuilds trim footpaths alike.
        let insets = if edge.primary_type == TransitType::Foot {
            let (start_inset, end_inset) = footpath_end_insets(graph, edge_idx);
            let mut insets = [start_inset, end_inset].map(LaneEnd::Inset);
            for (inset, (node, end)) in insets.iter_mut().zip(ends_at) {
                let arm = ArmKey {
                    edge: edge_idx,
                    end,
                };
                if let Some(walk_end) = node_walkways(node).and_then(|ways| ways.footpath_end(arm))
                {
                    *inset = layout_end(walk_end);
                }
            }
            insets
        } else {
            [LaneEnd::Inset(0.0); 2]
        };
        // Road sidewalks end in their nodes' layouts; at a node without one they keep a fixed
        // inset beyond the clip, room for the walk round the node.
        let sidewalk_ends = |offset: f32| {
            ends_at.map(|(node, end)| {
                node_walkways(node)
                    .and_then(|walkways| {
                        let arm = ArmKey {
                            edge: edge_idx,
                            end,
                        };
                        walkways.sidewalk_end(arm, offset > 0.0)
                    })
                    .map_or(LaneEnd::Inset(config::SIDEWALK_NODE_INSET), layout_end)
            })
        };

        let mut edge_lane_indices = Vec::new();

        // A left turn pocket's lane in a narrowed median (`ROAD-95`, D3) starts on the adjacent
        // through lane's mouth point at its transition, copied so the base road's lane runs on
        // into both directly, and tapers to its own offset beside the median's narrowing. Beside
        // a kerb pocket the transition's piece narrows the median and the lane starts at its own
        // offset, a connector's fork (round R5b1a).
        let narrowing = PocketNarrowing::on_span(edge);
        let pocket_taper = |lane_idx: i8| {
            let narrowing = narrowing?;
            let fwd = narrowing.end == EdgeEnd::Start;
            let (pocket_lane, adjacent) = if fwd { (0, 1) } else { (-1, -2) };
            (lane_idx == pocket_lane).then(|| {
                let index = usize::from(narrowing.end == EdgeEnd::End);
                let mut ends = vehicle_ends(lane_idx);
                if let Some(frame) = frames[index] {
                    ends[index] = LaneEnd::Mouth {
                        station_m: frame.station_m,
                        point: vehicle_lane_end(&frame, narrowing.end, edge, adjacent).point,
                    };
                }
                let taper = LaneTaper {
                    end: narrowing.end,
                    from_offset: vehicle_lane_offset(edge, adjacent),
                    over_m: LEFT_POCKET_TAPER_M,
                };
                (ends, taper)
            })
        };

        let mut build_lane = |is_fwd: bool,
                              l_idx: i8,
                              l_type: LaneType,
                              l_off: f32,
                              ends: [LaneEnd; 2],
                              taper: Option<LaneTaper>| {
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
                taper,
            );
        };

        // 1. Forward Lanes (lane 0 is closest to the centre)
        for l in 0..edge.fwd_lanes {
            let lane_idx = l as i8;
            let offset = vehicle_lane_offset(edge, lane_idx);
            let (ends, taper) = pocket_taper(lane_idx)
                .map_or((vehicle_ends(lane_idx), None), |(ends, taper)| {
                    (ends, Some(taper))
                });
            build_lane(true, lane_idx, LaneType::Vehicle, offset, ends, taper);
        }

        // 2. Backward Lanes
        for l in 0..edge.bkw_lanes {
            let lane_idx = -(l as i8) - 1;
            let offset = vehicle_lane_offset(edge, lane_idx);
            let (ends, taper) = pocket_taper(lane_idx)
                .map_or((vehicle_ends(lane_idx), None), |(ends, taper)| {
                    (ends, Some(taper))
                });
            build_lane(false, lane_idx, LaneType::Vehicle, offset, ends, taper);
        }

        // 3. Sidewalks
        if (edge.allowed_types & TransitFlags::FOOT) != 0 {
            if edge.primary_type == TransitType::Foot {
                // Dedicated Footpath: center lane
                build_lane(true, 0, LaneType::Foot, 0.0, insets, None);
                build_lane(false, 0, LaneType::Foot, 0.0, insets, None);
            } else {
                // Left (idx 100) and right (idx -100) sidewalks, each on a side with one
                // (`ROAD-107`).
                let sides = edge.sidewalk_sides();
                for l_idx in [100, -100] {
                    if !sides.on_lateral(sidewalk_lane_on_positive_side(l_idx)) {
                        continue;
                    }
                    let offset = sidewalk_offset(edge, l_idx);
                    let ends = sidewalk_ends(offset);
                    build_lane(true, l_idx, LaneType::Foot, offset, ends, None);
                    build_lane(false, l_idx, LaneType::Foot, offset, ends, None);
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
    /// Publication is ordered because lane IDs are part of simulation state: a rebuilt lane with
    /// the identity of a lane this rebuild retires takes that lane's id (the same edge, direction
    /// and index for a road lane, the same node and road lanes joined for a connector, as a load
    /// matches them), lowest id first in build order; every other rebuilt lane is appended in
    /// build order (`PERF-28`). O(rebuilt lanes and their successors) more for the reuse.
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
        let mut retired = self.retiring_identities(graph, &rebuild_edges, &affected_nodes);
        let (old_len, old_ring_entries) = (self.lanes.len(), self.ring_entries.len());

        // Retire old physical lanes before appending replacements. A retired lane keeps its id
        // and no state (`SAVE-11`): it is the lane a load recreates at a saved hole, owned by
        // no edge or node, so nothing reads it as a live lane (`PERF-27`), until a rebuilt lane
        // with its identity takes the id below.
        for &e_id in &rebuild_edges {
            for lane_id in self.edge_lanes.remove(&e_id).into_iter().flatten() {
                if let Some(lane) = self.lanes.get_mut(lane_id) {
                    *lane = super::Lane::default();
                }
            }
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
                self.tombstone_node_lanes(node_id);
                self.append_node_connections(&lane_map, graph, &walkways, node_id);
            }
        }

        let survivors: Vec<usize> = lane_map
            .into_values()
            .filter(|&id| id < old_len)
            .collect();
        self.reuse_retired_ids(
            graph,
            &mut retired,
            old_len,
            old_ring_entries,
            &survivors,
            &rebuild_edges,
            &affected_nodes,
        );
    }

    // The identities of the lanes a local rebuild of `rebuild_edges` and `nodes` retires: the
    // edges' road lanes and the nodes' connectors (`tombstone_node_lanes` retires every
    // connector of a node below the node count). A connector is found from the road lane it
    // leaves, at a road of its node. O(the edges' lanes + the nodes' arriving lanes and their
    // successors).
    fn retiring_identities(
        &self,
        graph: &RegionGraph,
        rebuild_edges: &[usize],
        nodes: &[usize],
    ) -> RetiredIds {
        let mut retired = RetiredIds::new();
        for &edge_id in rebuild_edges {
            for &id in self.edge_lanes.get(&edge_id).into_iter().flatten() {
                let key = RoadLaneKey::of(&self.lanes[id]);
                retired.entry(LaneIdentity::Road(key)).or_default().push(id);
            }
        }
        for &node_id in nodes {
            if node_id >= graph.node_count() || node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_id in graph.node_adjacency(node_id as u32) {
                for &from in self.edge_lanes.get(&edge_id).into_iter().flatten() {
                    for (identity, id) in self.connectors_from(from, node_id) {
                        retired.entry(identity).or_default().push(id);
                    }
                }
            }
        }
        for ids in retired.values_mut() {
            // A road arriving twice (a loop) lists its lanes twice.
            ids.sort_unstable_by(|a, b| b.cmp(a));
            ids.dedup();
        }
        retired
    }

    // The connectors at `node_id` that road lane `from` leads into, with their identities.
    fn connectors_from(
        &self,
        from: usize,
        node_id: usize,
    ) -> impl Iterator<Item = (LaneIdentity, usize)> + '_ {
        let from_lane = &self.lanes[from];
        from_lane.next_lanes.iter().filter_map(move |&id| {
            let lane = &self.lanes[id];
            let &to = lane.next_lanes.first()?;
            (lane.edge_id == usize::MAX && lane.node_id == node_id).then(|| {
                let identity = LaneIdentity::Connector {
                    node_id,
                    from: RoadLaneKey::of(from_lane),
                    to: RoadLaneKey::of(&self.lanes[to]),
                };
                (identity, id)
            })
        })
    }

    // Moves the lanes a local rebuild appended from `old_len` on to the ids `retired` holds for
    // their identities, lowest first in build order, and the rest down past `old_len` in build
    // order, then points every reference to a moved lane at its id: the rebuilt lanes'
    // successors, the `survivors` (lanes kept at the rebuild's nodes, whose successors were
    // rebuilt), the edges' and nodes' lists, crossings and the ring entries added from
    // `old_ring_entries` on (`PERF-28`). A rebuilt lane takes the id only of a lane with its
    // identity, so a reference held across the rebuild names the same lane or a retired one.
    // O(rebuilt lanes + their successors + survivors' successors).
    #[allow(clippy::too_many_arguments)]
    fn reuse_retired_ids(
        &mut self,
        graph: &RegionGraph,
        retired: &mut RetiredIds,
        old_len: usize,
        old_ring_entries: usize,
        survivors: &[usize],
        rebuild_edges: &[usize],
        nodes: &[usize],
    ) {
        let built = self.lanes.len() - old_len;
        if built == 0 || retired.is_empty() {
            return;
        }
        // A rebuilt connector's identity, found from the road lane it leaves.
        let mut connectors: Vec<Option<LaneIdentity>> = vec![None; built];
        for &node_id in nodes {
            if node_id >= graph.node_count() || node_id >= graph.node_adjacency_count() {
                continue;
            }
            for &edge_id in graph.node_adjacency(node_id as u32) {
                for &from in self.edge_lanes.get(&edge_id).into_iter().flatten() {
                    for (identity, id) in self.connectors_from(from, node_id) {
                        if id >= old_len {
                            connectors[id - old_len] = Some(identity);
                        }
                    }
                }
            }
        }
        let mut target = Vec::with_capacity(built);
        let mut reused = Vec::new();
        let mut next = old_len;
        for (offset, connector) in connectors.into_iter().enumerate() {
            let lane = &self.lanes[old_len + offset];
            let identity = if lane.edge_id == usize::MAX {
                connector
            } else {
                Some(LaneIdentity::Road(RoadLaneKey::of(lane)))
            };
            match identity.and_then(|identity| retired.get_mut(&identity)?.pop()) {
                Some(id) => {
                    target.push(id);
                    reused.push(id);
                }
                None => {
                    target.push(next);
                    next += 1;
                }
            }
        }
        if reused.is_empty() {
            return;
        }
        // Each lane moves down, to an id below `old_len` (retired) or to its own rank among the
        // appended ones: a slot at or above `old_len` it moves to was emptied before it.
        for (offset, &id) in target.iter().enumerate() {
            let lane = std::mem::take(&mut self.lanes[old_len + offset]);
            self.lanes[id] = lane;
        }
        self.lanes.truncate(next);
        let map = |id: usize| id.checked_sub(old_len).map_or(id, |offset| target[offset]);
        for &id in target.iter().chain(survivors) {
            self.lanes[id].next_lanes.map_ids(map);
        }
        let owned = |ids: Option<&mut Vec<usize>>| {
            ids.into_iter().flatten().for_each(|id| *id = map(*id));
        };
        for edge_id in rebuild_edges {
            owned(self.edge_lanes.get_mut(edge_id));
        }
        for node_id in nodes {
            owned(self.node_lanes.get_mut(node_id));
        }
        let crossings: Vec<(usize, LaneCrosswalk)> = (old_len..old_len + built)
            .filter_map(|id| Some((map(id), self.crosswalks.remove(&id)?)))
            .collect();
        self.crosswalks.extend(crossings);
        for entry in &mut self.ring_entries[old_ring_entries..] {
            entry.circulating = map(entry.circulating as usize) as u32;
            entry.ring_in = map(entry.ring_in as usize) as u32;
        }
        self.mark_reused_unlifted(reused);
    }

    /// Moves every lane to the id `target` gives it and leaves the other ids of `len` as
    /// retired lanes ([`Lane::default`]): a load placing the lanes its whole rebuild made at the
    /// saved game's ids (`SAVE-11`). `target` holds one distinct id below `len` per lane. Every
    /// lane id the system holds (successors, owners' lists, crossings, ring entries) moves
    /// with it. O(lanes + connections); load only.
    ///
    /// [`Lane::default`]: super::Lane
    pub(crate) fn place_at_ids(&mut self, target: &[usize], len: usize) {
        debug_assert_eq!(target.len(), self.lanes.len());
        self.revision = self.revision.wrapping_add(1);
        let mut lanes = vec![super::Lane::default(); len];
        for (old, mut lane) in std::mem::take(&mut self.lanes).into_iter().enumerate() {
            lane.next_lanes.map_ids(|id| target[id]);
            lanes[target[old]] = lane;
        }
        self.lanes = lanes;
        for ids in self.edge_lanes.values_mut().chain(self.node_lanes.values_mut()) {
            for id in ids {
                *id = target[*id];
            }
        }
        self.crosswalks = std::mem::take(&mut self.crosswalks)
            .into_iter()
            .map(|(id, crosswalk)| (target[id], crosswalk))
            .collect();
        for entry in &mut self.ring_entries {
            entry.circulating = target[entry.circulating as usize] as u32;
            entry.ring_in = target[entry.ring_in as usize] as u32;
        }
        self.forget_lifts();
    }

    /// Tombstones the connection lanes of nodes that no longer exist, such as the nodes an undo
    /// drops past the restored node count (`ROAD-68-E`). Their incident edges are rebuilt by
    /// [`Self::rebuild_edges_incremental`]; this only retires what no surviving owner rebuilds.
    /// O(the nodes' connection lanes).
    pub fn retire_node_lanes(&mut self, nodes: impl IntoIterator<Item = usize>) {
        let mut retired = false;
        for node_id in nodes {
            retired |= self.tombstone_node_lanes(node_id);
        }
        if retired {
            self.revision = self.revision.wrapping_add(1);
        }
    }

    // Tombstones old connection lanes at a node so the renderer skips them, and frees their
    // buffers: tombstones stay in `lanes`, each a retired lane with no state ([`Lane::default`],
    // as a load recreates it, `SAVE-11`), until a local rebuild gives the id to a rebuilt lane
    // with the same identity (`PERF-28`). False if it had none.
    fn tombstone_node_lanes(&mut self, node_id: usize) -> bool {
        let Some(old_ids) = self.node_lanes.remove(&node_id) else {
            return false;
        };
        for lid in old_ids {
            if let Some(lane) = self.lanes.get_mut(lid) {
                self.crosswalks.remove(&lid);
                *lane = super::Lane::default();
            }
        }
        true
    }
}
