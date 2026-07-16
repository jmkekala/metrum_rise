//! Junction mouth clipping and incident-edge collection.

use super::super::data::{Edge, RegionGraph};
use crate::simulation::network::types::{NodeType, TransitFlags, TransitType};
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

const CLIP_PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const CLIP_MIN_SIN: f32 = 0.0001;
const CLIP_LENGTH_RESERVE_M: f32 = 0.1;
const CLIP_WIDTH_PADDING_FACTOR: f32 = 1.1;
const CLIP_ACUTE_MAX_HALFWIDTH_FACTOR: f32 = 5.0;

#[derive(Clone, Copy)]
struct ClipIncident {
    edge_idx: usize,
    direction_xz: Vector2,
    half_width_m: f32,
}

#[derive(Default)]
struct ClipNodeStats {
    connection_count: usize,
    max_road_width_m: f32,
    min_road_width_m: f32,
    max_half_width_m: f32,
    incidents: Vec<ClipIncident>,
}

impl RegionGraph {
    /// Recalculates the start/end clipping distances for all edges meeting at junctions.
    ///
    /// This prevents road geometry from overlapping in the center of an intersection
    /// and ensures space for the junction mesh.
    pub fn rebuild_intersection_clips(&mut self) {
        let edge_indices = (0..self.edges.len()).collect::<Vec<_>>();
        let clip_stats = self.build_clip_node_stats(&edge_indices, None);
        let computed_clips = self.compute_intersection_clips(&edge_indices, &clip_stats, None);

        for (edge_idx, start_clip, end_clip) in computed_clips {
            let edge = &mut self.edges[edge_idx];
            edge.start_clip = start_clip;
            edge.end_clip = end_clip;

            // Keep physical_geometry in sync with geometry (which may have updated Y values
            // from terrain height re-interpolation). The renderer trims it via start_clip/end_clip.
            edge.physical_geometry = edge.geometry.clone();
        }

        // Re-index all roads after a massive batch clip rebuild (e.g. after terrain sync)
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }
        self.rebuild_adjacency_list();
    }

    /// Rebuilds intersection clips for only the edges incident to `affected_nodes`.
    ///
    /// Equivalent to [`Self::rebuild_intersection_clips`] but scoped to the nodes touched
    /// by a single road placement. Avoids the full O(E) resample pass and full
    /// R-tree / adjacency rebuild — only the O(K) incident edges are updated.
    ///
    /// Use this from the `AddRoad` handler. Use the full rebuild for bulk operations
    /// such as save-load restore or terrain sync.
    pub fn rebuild_intersection_clips_for_nodes(&mut self, affected_nodes: &HashSet<u32>) {
        if affected_nodes.is_empty() {
            return;
        }

        let reindex_ids = self.surface_edges_touching_nodes(affected_nodes);
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let clip_stats = self.build_clip_node_stats(&reindex_ids, Some(affected_nodes));
        let computed_clips =
            self.compute_intersection_clips(&reindex_ids, &clip_stats, Some(affected_nodes));

        for (edge_idx, start_clip, end_clip) in computed_clips {
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

            edge.physical_geometry = edge.geometry.clone();
        }

        // Update the spatial R-tree only for the affected edges (not full rebuild).
        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        // Adjacency is unchanged — no rebuild needed.
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

    fn build_clip_node_stats(
        &self,
        edge_indices: &[usize],
        affected_nodes: Option<&HashSet<u32>>,
    ) -> HashMap<u32, ClipNodeStats> {
        let mut stats: HashMap<u32, ClipNodeStats> = HashMap::new();

        for &edge_idx in edge_indices {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let endpoints = [
                (
                    self.get_valid_node(edge.start_node),
                    Self::edge_endpoint_direction_xz(edge, true),
                ),
                (
                    self.get_valid_node(edge.end_node),
                    Self::edge_endpoint_direction_xz(edge, false),
                ),
            ];
            let half_width_m = Self::roadbed_half_width_m(edge);

            for (node_id, direction_xz) in endpoints {
                if affected_nodes.is_some_and(|affected| !affected.contains(&node_id)) {
                    continue;
                }

                let node_stats = stats.entry(node_id).or_insert_with(|| ClipNodeStats {
                    min_road_width_m: f32::MAX,
                    ..Default::default()
                });
                node_stats.connection_count += 1;
                node_stats.max_road_width_m = node_stats.max_road_width_m.max(edge.width);
                node_stats.min_road_width_m = node_stats.min_road_width_m.min(edge.width);
                node_stats.max_half_width_m = node_stats.max_half_width_m.max(half_width_m);

                if let Some(direction_xz) = direction_xz {
                    node_stats.incidents.push(ClipIncident {
                        edge_idx,
                        direction_xz,
                        half_width_m,
                    });
                }
            }
        }

        for node_stats in stats.values_mut() {
            node_stats.incidents.sort_by(|a, b| {
                a.edge_idx
                    .cmp(&b.edge_idx)
                    .then(a.direction_xz.x.total_cmp(&b.direction_xz.x))
                    .then(a.direction_xz.y.total_cmp(&b.direction_xz.y))
            });
        }

        stats
    }

    fn compute_intersection_clips(
        &self,
        edge_indices: &[usize],
        clip_stats: &HashMap<u32, ClipNodeStats>,
        affected_nodes: Option<&HashSet<u32>>,
    ) -> Vec<(usize, f32, f32)> {
        let mut computed = Vec::with_capacity(edge_indices.len());

        for &edge_idx in edge_indices {
            let Some(edge) = self.edges.get(edge_idx) else {
                continue;
            };
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let start_node = self.get_valid_node(edge.start_node);
            let end_node = self.get_valid_node(edge.end_node);
            if affected_nodes.is_some_and(|affected| {
                !affected.contains(&start_node) && !affected.contains(&end_node)
            }) {
                continue;
            }

            let length_m = Self::edge_geometry_length_m(edge);
            let start_clip = if affected_nodes.is_none_or(|affected| affected.contains(&start_node))
            {
                self.clip_for_edge_at_node(edge_idx, start_node, length_m, clip_stats)
            } else {
                edge.start_clip
            };
            let end_clip = if affected_nodes.is_none_or(|affected| affected.contains(&end_node)) {
                self.clip_for_edge_at_node(edge_idx, end_node, length_m, clip_stats)
            } else {
                edge.end_clip
            };
            let (start_clip, end_clip) =
                Self::fit_clips_to_edge_length(start_clip, end_clip, length_m);
            computed.push((edge_idx, start_clip, end_clip));
        }

        computed
    }

    fn clip_for_edge_at_node(
        &self,
        edge_idx: usize,
        node_id: u32,
        edge_length_m: f32,
        clip_stats: &HashMap<u32, ClipNodeStats>,
    ) -> f32 {
        let Some(stats) = clip_stats.get(&node_id) else {
            return 0.0;
        };
        if self.nodes[node_id as usize].node_type != NodeType::Junction {
            return 0.0;
        }
        if !Self::node_requires_clip(stats) {
            return 0.0;
        }

        let mut clip_m = stats.max_half_width_m * CLIP_WIDTH_PADDING_FACTOR;
        let widths_differ = (stats.max_road_width_m - stats.min_road_width_m).abs() > 0.1;
        let acute_clip_limit_m = if widths_differ {
            edge_length_m
        } else {
            stats.max_half_width_m * CLIP_ACUTE_MAX_HALFWIDTH_FACTOR
        };
        let Some(self_incident) = stats
            .incidents
            .iter()
            .find(|incident| incident.edge_idx == edge_idx)
            .copied()
        else {
            return clip_m.min(edge_length_m);
        };

        for other in &stats.incidents {
            if other.edge_idx == edge_idx
                || Self::directions_are_pass_through(self_incident.direction_xz, other.direction_xz)
            {
                continue;
            }

            let dot = self_incident
                .direction_xz
                .dot(other.direction_xz)
                .clamp(-1.0, 1.0);
            let cross = Self::cross_xz(self_incident.direction_xz, other.direction_xz).abs();
            let required_m = if cross <= CLIP_MIN_SIN {
                edge_length_m
            } else {
                (other.half_width_m + self_incident.half_width_m * dot.abs()) / cross
            };
            if required_m.is_finite() {
                clip_m = clip_m.max(required_m.min(acute_clip_limit_m));
            } else {
                clip_m = clip_m.max(acute_clip_limit_m);
            }
        }

        clip_m.clamp(0.0, edge_length_m.max(0.0))
    }

    fn node_requires_clip(stats: &ClipNodeStats) -> bool {
        let widths_differ = (stats.max_road_width_m - stats.min_road_width_m).abs() > 0.1;
        stats.connection_count >= 3 || widths_differ || Self::node_has_non_pass_through_pair(stats)
    }

    fn node_has_non_pass_through_pair(stats: &ClipNodeStats) -> bool {
        for (index, a) in stats.incidents.iter().enumerate() {
            for b in stats.incidents.iter().skip(index + 1) {
                if !Self::directions_are_pass_through(a.direction_xz, b.direction_xz) {
                    return true;
                }
            }
        }
        false
    }

    pub(super) fn directions_are_pass_through(a: Vector2, b: Vector2) -> bool {
        a.dot(b) <= -CLIP_PASS_THROUGH_DOT_THRESHOLD
    }

    fn cross_xz(a: Vector2, b: Vector2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    fn roadbed_half_width_m(edge: &Edge) -> f32 {
        if edge.primary_type == TransitType::Foot || (edge.allowed_types & TransitFlags::CAR) == 0 {
            return edge.width.max(2.0) * 0.5;
        }

        let sidewalk_width = if edge.allowed_types & TransitFlags::FOOT != 0 {
            crate::config::SIDEWALK_WIDTH
        } else {
            0.0
        };
        edge.width.max(crate::config::LANE_WIDTH) * 0.5 + sidewalk_width
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

    fn edge_geometry_length_m(edge: &Edge) -> f32 {
        edge.geometry
            .windows(2)
            .map(|window| window[0].distance_to(window[1]))
            .sum()
    }

    pub(super) fn edge_profile_length_m(edge: &Edge) -> f32 {
        edge.geometry
            .windows(2)
            .map(|window| Self::edge_profile_point_distance_m(window[0], window[1]))
            .sum()
    }

    pub(super) fn edge_profile_point_distance_m(a: Vector3, b: Vector3) -> f32 {
        (a.x - b.x).hypot(a.z - b.z)
    }

    fn fit_clips_to_edge_length(
        start_clip_m: f32,
        end_clip_m: f32,
        edge_length_m: f32,
    ) -> (f32, f32) {
        if edge_length_m <= CLIP_LENGTH_RESERVE_M {
            return (0.0, 0.0);
        }

        let max_sum = (edge_length_m - CLIP_LENGTH_RESERVE_M).max(0.0);
        let start_clip_m = start_clip_m.clamp(0.0, edge_length_m);
        let end_clip_m = end_clip_m.clamp(0.0, edge_length_m);
        let sum = start_clip_m + end_clip_m;
        if sum <= max_sum || sum <= f32::EPSILON {
            return (start_clip_m, end_clip_m);
        }

        let scale = max_sum / sum;
        (start_clip_m * scale, end_clip_m * scale)
    }
}
