// SPDX-License-Identifier: GPL-2.0-only

//! Junction mouth clips from the junction layout (`ROAD-68-C`), and incident-edge collection.
//!
//! An edge's clip at a bend or junction node is its arm's mouth station in that node's layout:
//! horizontal metres along the roadbed centreline from the node. Terminals, pass-through nodes
//! and nodes whose layout fails (a placement rejection) keep a zero clip. Each layout is
//! `O(degree * arm points)`; a full rebuild lays out every node on the shared pool.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::roadbed::{EdgeEnd, NodeArms, NodeKind};
use crate::simulation::network::types::TransitType;
use godot::prelude::{Vector2, Vector3};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};

const CLIP_PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;

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

        let reindex_ids = self.surface_edges_touching_nodes(affected_nodes);
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
                    let Some(arms) = NodeArms::collect(self, node) else {
                        return Vec::new();
                    };
                    match arms.layout() {
                        Ok(layout) if layout.kind != NodeKind::Terminal => layout
                            .arms
                            .iter()
                            .filter(|arm| {
                                self.edges[arm.key.edge].primary_type == TransitType::Road
                            })
                            .map(|arm| {
                                let at_start = arm.key.end == EdgeEnd::Start;
                                ((arm.key.edge, at_start), arm.mouth_station_m as f32)
                            })
                            .collect(),
                        _ => Vec::new(),
                    }
                })
                .collect()
        });
        per_node.into_iter().flatten().collect()
    }

    fn edge_clips(edge_idx: usize, mouths: &HashMap<(usize, bool), f32>) -> (f32, f32) {
        let clip = |at_start: bool| mouths.get(&(edge_idx, at_start)).copied().unwrap_or(0.0);
        (clip(true), clip(false))
    }

    pub(super) fn surface_edges_touching_nodes(&self, affected_nodes: &HashSet<u32>) -> Vec<usize> {
        let mut edge_ids = HashSet::new();
        for &node_id in affected_nodes {
            let valid_node = self.get_valid_node(node_id);
            let Some(adjacency) = self.adjacency.get(valid_node as usize) else {
                continue;
            };
            for &edge_idx in adjacency {
                let Some(edge) = self.edges.get(edge_idx) else {
                    continue;
                };
                if edge.deleted || edge.primary_type != TransitType::Road {
                    continue;
                }
                let start_node = self.get_valid_node(edge.start_node);
                let end_node = self.get_valid_node(edge.end_node);
                if affected_nodes.contains(&start_node) || affected_nodes.contains(&end_node) {
                    edge_ids.insert(edge_idx);
                }
            }
        }

        let mut edge_ids = edge_ids.into_iter().collect::<Vec<_>>();
        edge_ids.sort_unstable();
        edge_ids
    }

    pub(super) fn directions_are_pass_through(a: Vector2, b: Vector2) -> bool {
        a.dot(b) <= -CLIP_PASS_THROUGH_DOT_THRESHOLD
    }

    pub(super) fn edge_endpoint_direction_xz(edge: &Edge, at_start: bool) -> Option<Vector2> {
        if edge.geometry.len() < 2 {
            return None;
        }

        if at_start {
            for window in edge.geometry.windows(2) {
                let delta = window[1] - window[0];
                let direction = Vector2::new(delta.x, delta.z);
                if direction.length_squared() > 1e-8 {
                    return Some(direction.normalized());
                }
            }
        } else {
            for window in edge.geometry.windows(2).rev() {
                let delta = window[0] - window[1];
                let direction = Vector2::new(delta.x, delta.z);
                if direction.length_squared() > 1e-8 {
                    return Some(direction.normalized());
                }
            }
        }

        None
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
}
