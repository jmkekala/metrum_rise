// SPDX-License-Identifier: GPL-2.0-only

//! Junction mouth clips from the junction layout (`ROAD-68-C`), and incident-edge collection.
//!
//! An edge's clip at a bend or junction node is its arm's mouth station in that node's layout:
//! horizontal metres along the roadbed centreline from the node, footpaths included; a footpath
//! attached to a road node (D5) is clipped at its mouth there, a road end included. Terminals,
//! pass-through nodes and nodes whose layout fails (a placement rejection) keep a zero clip. Each layout is
//! `O(degree * arm points)`; a full rebuild lays out every node on the shared pool.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{EdgeEnd, NodeArms, NodeKind};
use crate::simulation::network::types::TransitType;
use godot::prelude::Vector3;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

impl RegionGraph {
    /// Recalculates the start/end clipping distances for all edges from their nodes' junction
    /// layouts.
    pub fn rebuild_intersection_clips(&mut self) {
        // Layouts read node adjacency, so it must be current first.
        self.rebuild_adjacency_list();
        let nodes: Vec<u32> = (0..self.nodes.len().min(self.adjacency.len()) as u32)
            .filter(|&node| self.get_valid_node(node) == node)
            .collect();
        let mouths = self.layout_mouth_stations(&nodes);
        for edge_idx in 0..self.edges.len() {
            let (start_clip, end_clip) = Self::edge_clips(edge_idx, &mouths);
            let edge = &mut self.edges[edge_idx];
            edge.start_clip = start_clip;
            edge.end_clip = end_clip;
        }

        // Re-index all roads after a massive batch clip rebuild (e.g. after terrain sync)
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
    }

    /// Rebuilds intersection clips for only the edge ends at `affected_nodes`.
    ///
    /// Equivalent to [`Self::rebuild_intersection_clips`] for those ends, scoped to the nodes
    /// touched by a single road placement: `O(affected degree * arm points)` plus the R-tree
    /// updates of the incident edges.
    pub fn rebuild_intersection_clips_for_nodes(&mut self, affected_nodes: &HashSet<u32>) {
        if affected_nodes.is_empty() {
            return;
        }

        let mut reindex_ids = Vec::new();
        self.surface_edges_touching_nodes_into(
            affected_nodes.iter().copied(),
            |node| affected_nodes.contains(&node),
            &mut reindex_ids,
        );
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let mut nodes: Vec<u32> = affected_nodes
            .iter()
            .map(|&node| self.get_valid_node(node))
            .collect();
        nodes.sort_unstable();
        nodes.dedup();
        let mouths = self.layout_mouth_stations(&nodes);
        for &edge_idx in &reindex_ids {
            let (start_clip, end_clip) = Self::edge_clips(edge_idx, &mouths);
            let (start_node, end_node) = {
                let edge = &self.edges[edge_idx];
                (
                    self.get_valid_node(edge.start_node),
                    self.get_valid_node(edge.end_node),
                )
            };
            let edge = &mut self.edges[edge_idx];
            if affected_nodes.contains(&start_node) {
                edge.start_clip = start_clip;
            }
            if affected_nodes.contains(&end_node) {
                edge.end_clip = end_clip;
            }
        }

        // Update the spatial R-tree only for the affected edges (not full rebuild).
        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        // Adjacency is unchanged — no rebuild needed.
    }

    // Mouth stations by `(edge, at_start)` at the bend and junction nodes among `nodes`.
    fn layout_mouth_stations(&self, nodes: &[u32]) -> HashMap<(usize, bool), f32> {
        let per_node: Vec<Vec<((usize, bool), f32)>> = crate::thread_pools::install(|| {
            nodes
                .par_iter()
                .map(|&node| {
                    let Some(mut arms) = NodeArms::collect(self, node) else {
                        return Vec::new();
                    };
                    // Terminals keep a zero clip on their own arm, so lanes run onto the cap;
                    // attached footpaths stop at their mouths at any road node.
                    let Ok(layout) = arms.layout() else {
                        return Vec::new();
                    };
                    let arms = layout
                        .arms
                        .iter()
                        .filter(|_| layout.kind != NodeKind::Terminal)
                        .map(|arm| (arm.key, arm.mouth_station_m));
                    let attached = layout
                        .attachments
                        .iter()
                        .map(|attachment| (attachment.key, attachment.mouth_station_m));
                    arms.chain(attached)
                        .map(|(key, mouth_m)| {
                            ((key.edge, key.end == EdgeEnd::Start), mouth_m as f32)
                        })
                        .collect()
                })
                .collect()
        });
        per_node.into_iter().flatten().collect()
    }

    fn edge_clips(edge_idx: usize, mouths: &HashMap<(usize, bool), f32>) -> (f32, f32) {
        let clip = |at_start: bool| mouths.get(&(edge_idx, at_start)).copied().unwrap_or(0.0);
        (clip(true), clip(false))
    }

    // The live road and footpath edges in the adjacency of each of `nodes` (as its valid node)
    // with an end whose valid node `affected` accepts, sorted and unique, into `edge_ids`, which
    // allocates only to grow.
    pub(super) fn surface_edges_touching_nodes_into(
        &self,
        nodes: impl IntoIterator<Item = u32>,
        affected: impl Fn(u32) -> bool,
        edge_ids: &mut Vec<usize>,
    ) {
        edge_ids.clear();
        for node_id in nodes {
            let valid_node = self.get_valid_node(node_id);
            let Some(adjacency) = self.adjacency.get(valid_node as usize) else {
                continue;
            };
            for &edge_idx in adjacency {
                let Some(edge) = self.edges.get(edge_idx) else {
                    continue;
                };
                if edge.deleted
                    || !matches!(edge.primary_type, TransitType::Road | TransitType::Foot)
                {
                    continue;
                }
                let start_node = self.get_valid_node(edge.start_node);
                let end_node = self.get_valid_node(edge.end_node);
                if affected(start_node) || affected(end_node) {
                    edge_ids.push(edge_idx);
                }
            }
        }
        edge_ids.sort_unstable();
        edge_ids.dedup();
    }

    pub(super) fn edge_profile_length_m(edge: &Edge) -> f32 {
        edge.geometry
            .windows(2)
            .map(|window| Self::edge_profile_point_distance_m(window[0], window[1]))
            .sum()
    }

    /// Measures horizontal road station separation without importing the vertical profile.
    pub(in crate::simulation::network) fn edge_profile_point_distance_m(
        a: Vector3,
        b: Vector3,
    ) -> f32 {
        (a.x - b.x).hypot(a.z - b.z)
    }

    /// Height of a graph profile polyline at the XZ projection of a point: the nearest
    /// segment's lerp. The one height a road meets another road at mid-span (`ROAD-68` open
    /// questions 16 and 30). `O(profile points)`.
    pub(crate) fn profile_height_at(profile: &[Vector3], x: f32, z: f32) -> f32 {
        let mut best = (f32::INFINITY, profile.first().map_or(0.0, |point| point.y));
        for pair in profile.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dz) = (b.x - a.x, b.z - a.z);
            let length_sq = dx * dx + dz * dz;
            let t = if length_sq > 0.0 {
                (((x - a.x) * dx + (z - a.z) * dz) / length_sq).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let (px, pz) = (a.x + dx * t, a.z + dz * t);
            let distance_sq = (x - px) * (x - px) + (z - pz) * (z - pz);
            if distance_sq < best.0 {
                best = (distance_sq, a.y + (b.y - a.y) * t);
            }
        }
        best.1
    }
}
