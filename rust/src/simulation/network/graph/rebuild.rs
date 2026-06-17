//! Logic for rebuilding graph metadata (adjacency, clips, and terrain synchronization).

use super::super::types::*;
use super::data::RegionGraph;
use godot::prelude::{Vector2, Vector3};
use std::collections::{HashMap, HashSet};

const CLIP_PASS_THROUGH_DOT_THRESHOLD: f32 = 0.98;
const CLIP_MIN_SIN: f32 = 0.0001;
const CLIP_LENGTH_RESERVE_M: f32 = 0.1;
const CLIP_WIDTH_PADDING_FACTOR: f32 = 1.2;
const JUNCTION_PROFILE_HARD_ZONE_MIN_M: f32 = 1.0;
const JUNCTION_PROFILE_HARD_ZONE_MAX_M: f32 = 2.0;
const JUNCTION_PROFILE_BLEND_ZONE_M: f32 = 32.0;
const JUNCTION_PROFILE_SOLVE_SAMPLE_M: f32 = 12.0;
const JUNCTION_PROFILE_MIN_SAMPLE_M: f32 = 1.0;
const JUNCTION_PROFILE_MOUTH_MAX_GRADE: f32 = 0.16;
const JUNCTION_PROFILE_LIMIT_MAX_SAMPLE_DELTA_M: f32 = 0.5;
const JUNCTION_PROFILE_REJECT_MAX_GRADE: f32 = 0.5;
const JUNCTION_PROFILE_PLANE_DET_EPS: f32 = 1.0e-5;
const JUNCTION_PROFILE_SUPPORT_EPS_M: f32 = 0.01;
const JUNCTION_PROFILE_SUPPORT_STEP_M: f32 = 4.0;

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

#[derive(Clone, Copy)]
struct JunctionProfileIncident {
    edge_idx: usize,
    at_start: bool,
}

#[derive(Clone, Copy)]
enum JunctionProfileLimitMode {
    ConservativeSourceFit,
    RegradePlatform,
}

/// Node-local profile plane used to make incident Bend/JunctionN mouth rails height-compatible.
#[derive(Clone, Copy)]
pub(crate) struct JunctionEndpointProfilePlane {
    origin: Vector3,
    grade_x: f32,
    grade_z: f32,
}

impl JunctionEndpointProfilePlane {
    /// Evaluates the solved endpoint profile height at an arbitrary world XZ coordinate.
    pub(crate) fn height_at_xz(&self, x: f32, z: f32) -> f32 {
        self.origin.y + self.grade_x * (x - self.origin.x) + self.grade_z * (z - self.origin.z)
    }

    fn grade(&self) -> f32 {
        self.grade_x.hypot(self.grade_z)
    }

    fn grade_limited(
        origin: Vector3,
        grade_x: f32,
        grade_z: f32,
        sample_offsets: &[(f32, f32)],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<Self> {
        let grade = grade_x.hypot(grade_z);
        if !grade.is_finite()
            || matches!(limit_mode, JunctionProfileLimitMode::ConservativeSourceFit)
                && grade > JUNCTION_PROFILE_REJECT_MAX_GRADE
        {
            return None;
        }
        let mut limited_grade_x = grade_x;
        let mut limited_grade_z = grade_z;
        if grade > JUNCTION_PROFILE_MOUTH_MAX_GRADE {
            let scale = JUNCTION_PROFILE_MOUTH_MAX_GRADE / grade;
            let candidate_grade_x = grade_x * scale;
            let candidate_grade_z = grade_z * scale;
            match limit_mode {
                JunctionProfileLimitMode::ConservativeSourceFit => {
                    let max_sample_delta_m = sample_offsets
                        .iter()
                        .map(|&(dx, dz)| {
                            ((grade_x - candidate_grade_x) * dx
                                + (grade_z - candidate_grade_z) * dz)
                                .abs()
                        })
                        .fold(0.0_f32, f32::max);
                    if max_sample_delta_m <= JUNCTION_PROFILE_LIMIT_MAX_SAMPLE_DELTA_M {
                        limited_grade_x = candidate_grade_x;
                        limited_grade_z = candidate_grade_z;
                    }
                }
                JunctionProfileLimitMode::RegradePlatform => {
                    limited_grade_x = candidate_grade_x;
                    limited_grade_z = candidate_grade_z;
                }
            }
        };
        Some(Self {
            origin,
            grade_x: limited_grade_x,
            grade_z: limited_grade_z,
        })
    }
}

impl RegionGraph {
    /// Rebuilds the adjacency list from the current set of non-deleted edges.
    pub fn rebuild_adjacency_list(&mut self) {
        self.adjacency.clear();
        self.adjacency.resize(self.nodes.len(), Vec::new());
        for (i, e) in self.edges.iter().enumerate() {
            if e.deleted {
                continue;
            }
            self.adjacency[e.start_node as usize].push(i);
            self.adjacency[e.end_node as usize].push(i);
        }
    }

    /// Removes all edges marked as `deleted` and remaps graph-local edge references.
    ///
    /// This is a low-level canonicalization helper for tests and future persistence code.
    /// Live gameplay/editor code keeps soft-deleted edge slots in place and skips them.
    /// Returns a mapping from [Old Edge Index] -> [New Edge Index].
    pub fn compact_edges(&mut self) -> HashMap<usize, usize> {
        let mut old_to_new = HashMap::new();
        let mut new_edges = Vec::new();

        for (old_idx, edge) in self.edges.iter().enumerate() {
            if !edge.deleted {
                let new_idx = new_edges.len();
                old_to_new.insert(old_idx, new_idx);
                new_edges.push(edge.clone());
            }
        }

        // If no edges were deleted, we're already compacted.
        if new_edges.len() == self.edges.len() {
            return HashMap::new();
        }

        self.edges = new_edges;

        // 1. Rebuild Adjacency List (Fastest way to update indices)
        self.rebuild_adjacency_list();

        // 2. Rebuild Spatial Index
        self.spatial_edge_rt = rstar::RTree::new();
        for i in 0..self.edges.len() {
            self.add_to_spatial_index(i);
        }

        // 3. Update Lane Connection rules inside each Node
        for node in &mut self.nodes {
            let mut new_lane_conns = HashMap::new();
            for (src, targets) in node.lane_connections.drain() {
                // If the source edge still exists, remap it
                if let Some(&new_src_idx) = old_to_new.get(&src.0) {
                    let mut new_targets = Vec::new();
                    for mut tgt in targets {
                        // If the target edge still exists, remap it
                        if let Some(&new_tgt_idx) = old_to_new.get(&tgt.0) {
                            tgt.0 = new_tgt_idx;
                            new_targets.push(tgt);
                        }
                    }
                    if !new_targets.is_empty() {
                        new_lane_conns.insert((new_src_idx, src.1), new_targets);
                    }
                }
            }
            node.lane_connections = new_lane_conns;
        }

        old_to_new
    }

    /// Returns the number of disconnected components (islands) in the network
    pub fn get_island_count(&self) -> usize {
        if self.nodes.is_empty() {
            return 0;
        }

        // Disjoint Set Union (DSU)
        let mut parent: Vec<usize> = (0..self.nodes.len()).collect();

        fn find(i: usize, parent: &mut Vec<usize>) -> usize {
            if parent[i] == i {
                return i;
            }
            parent[i] = find(parent[i], parent);
            parent[i]
        }

        fn unite(i: usize, j: usize, parent: &mut Vec<usize>) {
            let root_i = find(i, parent);
            let root_j = find(j, parent);
            if root_i != root_j {
                parent[root_i] = root_j;
            }
        }

        // Unite nodes connected by edges
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            unite(
                edge.start_node as usize,
                edge.end_node as usize,
                &mut parent,
            );
        }

        // Count unique roots (only for nodes that are part of an edge to avoid counting "floating" preview nodes)
        let mut active_nodes = std::collections::HashSet::new();
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            active_nodes.insert(edge.start_node as usize);
            active_nodes.insert(edge.end_node as usize);
        }

        let mut roots = std::collections::HashSet::new();
        for &node_idx in &active_nodes {
            roots.insert(find(node_idx, &mut parent));
        }

        roots.len()
    }

    /// Synchronizes all road nodes and intermediate geometries to the terrain heightmap.
    ///
    /// Applies Laplacian smoothing to road grades to ensure smooth vertical transitions.
    pub fn sync_to_terrain(&mut self, terrain: &crate::simulation::terrain::TerrainSystem) {
        // 0. Pre-calculate which nodes are snappable (Standard only)
        let mut node_snappable = vec![true; self.nodes.len()];
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                node_snappable[edge.start_node as usize] = false;
                node_snappable[edge.end_node as usize] = false;
            }
        }

        // 1. Sync Nodes Only if snappable
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !node_snappable[i] {
                continue;
            }
            node.pos.y =
                terrain.sample_height_world(node.pos.x, node.pos.z) * crate::config::HEIGHT_SCALE;
        }

        // 2. Re-interpolate Edge Geometry (Smooth Grades)
        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                continue;
            }

            let count = edge.geometry.len();
            if count < 2 {
                continue;
            }

            // Snap endpoints to nodes
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;

            // HARMONIC CONFORMANCE (Laplacian Smoothing)
            // 1. Re-sample raw terrain for all intermediate points so road follows new hills
            for j in 1..count - 1 {
                edge.geometry[j].y = terrain
                    .sample_height_world(edge.geometry[j].x, edge.geometry[j].z)
                    * crate::config::HEIGHT_SCALE;
            }

            // 2. Taubin Smoothing to iron out bumps without volume shrinkage
            let iters = 50;
            if count > 2 {
                let mut temp_h = vec![0.0; count];
                let lambda = 0.5;
                let mu = -0.53;
                for _ in 0..iters {
                    // Positive Pass (Shrink/Smooth)
                    for j in 1..count - 1 {
                        let laplacian = 0.5 * (edge.geometry[j - 1].y + edge.geometry[j + 1].y)
                            - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + lambda * laplacian;
                    }
                    for j in 1..count - 1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                    // Negative Pass (Inflate/Restore Volume)
                    for j in 1..count - 1 {
                        let laplacian = 0.5 * (edge.geometry[j - 1].y + edge.geometry[j + 1].y)
                            - edge.geometry[j].y;
                        temp_h[j] = edge.geometry[j].y + mu * laplacian;
                    }
                    for j in 1..count - 1 {
                        edge.geometry[j].y = temp_h[j];
                    }
                }
            }
        }
        self.rebuild_intersection_clips();
    }

    /// Recalculates the start/end clipping distances for all edges meeting at junctions.
    ///
    /// This prevents road geometry from overlapping in the center of an intersection
    /// and ensures space for the junction mesh.
    pub fn rebuild_intersection_clips(&mut self) {
        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let clip_stats = self.build_clip_node_stats(&valid_node_ids, None);
        let computed_clips = self.compute_intersection_clips(&valid_node_ids, &clip_stats, None);

        for (edge_idx, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            if let Some((start_clip, end_clip)) = computed_clips[edge_idx] {
                edge.start_clip = start_clip;
                edge.end_clip = end_clip;
            }

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
    /// Equivalent to [`rebuild_intersection_clips`] but scoped to the nodes touched
    /// by a single road placement. Avoids the full O(E) resample pass and full
    /// R-tree / adjacency rebuild — only the O(K) incident edges are updated.
    ///
    /// Use this from the `AddRoad` handler. Use the full rebuild for bulk operations
    /// such as save-load restore or terrain sync.
    pub fn rebuild_intersection_clips_for_nodes(&mut self, affected_nodes: &HashSet<u32>) {
        if affected_nodes.is_empty() {
            return;
        }

        // Pre-compute valid node IDs so we can use them during the mutable edge pass.
        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();

        let mut reindex_ids = self.surface_edges_touching_nodes(&valid_node_ids, affected_nodes);
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let clip_stats = self.build_clip_node_stats(&valid_node_ids, Some(affected_nodes));
        let computed_clips =
            self.compute_intersection_clips(&valid_node_ids, &clip_stats, Some(affected_nodes));

        // Pass 2 (mutable): resample only edges that touch an affected node.
        for (edge_idx, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }
            let s = valid_node_ids[edge.start_node as usize];
            let e = valid_node_ids[edge.end_node as usize];

            if !affected_nodes.contains(&s) && !affected_nodes.contains(&e) {
                continue;
            }

            if let Some((start_clip, end_clip)) = computed_clips[edge_idx] {
                edge.start_clip = if affected_nodes.contains(&s) {
                    start_clip
                } else {
                    edge.start_clip
                };
                edge.end_clip = if affected_nodes.contains(&e) {
                    end_clip
                } else {
                    edge.end_clip
                };
            }

            edge.physical_geometry = edge.geometry.clone();
        }

        // Update the spatial R-tree only for the affected edges (not full rebuild).
        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        // Adjacency is unchanged — no rebuild needed.
    }

    /// Adapts newly authored edge endpoints to existing Bend/JunctionN grade/profile anchors.
    ///
    /// The node compiler consumes the resulting edge profiles as source authority; it still
    /// rejects any contradictory mouth heights that remain after this edit-stage solve.
    pub(in crate::simulation::network) fn solve_junction_endpoint_profiles_for_edges(
        &mut self,
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() || adaptable_edges.is_empty() {
            return HashSet::new();
        }

        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let mut reindex_ids = adaptable_edges
            .iter()
            .copied()
            .filter(|&edge_idx| edge_idx < self.edges.len() && !self.edges[edge_idx].deleted)
            .collect::<Vec<_>>();
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let changed_edges = self.solve_junction_endpoint_profiles(
            &valid_node_ids,
            affected_nodes,
            adaptable_edges,
            JunctionProfileLimitMode::ConservativeSourceFit,
        );

        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    /// Regrades every road edge incident to affected over-limit Bend/JunctionN nodes.
    ///
    /// This is the stronger road-edit path: only nodes whose conservative profile still exceeds
    /// the grade cap are selected, then all touching edge mouths receive real support vertices so
    /// later section compiles sample that platform.
    pub(in crate::simulation::network) fn regrade_junction_endpoint_profiles_for_nodes(
        &mut self,
        affected_nodes: &HashSet<u32>,
    ) -> HashSet<usize> {
        if affected_nodes.is_empty() {
            return HashSet::new();
        }

        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let regrade_nodes = self.junction_profile_regrade_nodes(&valid_node_ids, affected_nodes);
        if regrade_nodes.is_empty() {
            return HashSet::new();
        }

        let mut reindex_ids = self.surface_edges_touching_nodes(&valid_node_ids, &regrade_nodes);
        reindex_ids.sort_unstable();
        reindex_ids.dedup();
        if reindex_ids.is_empty() {
            return HashSet::new();
        }

        for &edge_idx in &reindex_ids {
            self.remove_from_spatial_index(edge_idx);
        }

        let adaptable_edges = reindex_ids.iter().copied().collect::<HashSet<_>>();
        let changed_edges = self.solve_junction_endpoint_profiles(
            &valid_node_ids,
            &regrade_nodes,
            &adaptable_edges,
            JunctionProfileLimitMode::RegradePlatform,
        );

        for edge_idx in reindex_ids {
            self.add_to_spatial_index(edge_idx);
        }
        changed_edges
    }

    fn junction_profile_regrade_nodes(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: &HashSet<u32>,
    ) -> HashSet<u32> {
        let incidents_by_node =
            self.build_junction_profile_incidents(valid_node_ids, Some(affected_nodes));
        incidents_by_node
            .iter()
            .filter_map(|(&node_id, incidents)| {
                if incidents.len() < 2 {
                    return None;
                }

                let conservative = self.solve_junction_profile_plane(
                    node_id,
                    incidents,
                    JunctionProfileLimitMode::ConservativeSourceFit,
                );
                if conservative
                    .is_some_and(|plane| plane.grade() > JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4)
                    || conservative.is_none()
                        && self
                            .solve_junction_profile_plane(
                                node_id,
                                incidents,
                                JunctionProfileLimitMode::RegradePlatform,
                            )
                            .is_some()
                {
                    Some(node_id)
                } else {
                    None
                }
            })
            .collect()
    }

    fn solve_junction_endpoint_profiles(
        &mut self,
        valid_node_ids: &[u32],
        affected_nodes: &HashSet<u32>,
        adaptable_edges: &HashSet<usize>,
        limit_mode: JunctionProfileLimitMode,
    ) -> HashSet<usize> {
        let incidents_by_node =
            self.build_junction_profile_incidents(valid_node_ids, Some(affected_nodes));
        let mut edge_solves: Vec<(usize, bool, JunctionEndpointProfilePlane)> = Vec::new();

        let mut node_ids: Vec<u32> = incidents_by_node.keys().copied().collect();
        node_ids.sort_unstable();
        for node_id in node_ids {
            let incidents = &incidents_by_node[&node_id];
            if incidents.len() < 2 {
                continue;
            }
            let stable_incidents = incidents
                .iter()
                .copied()
                .filter(|incident| !adaptable_edges.contains(&incident.edge_idx))
                .collect::<Vec<_>>();
            let plane = if stable_incidents.len() >= 2 {
                self.solve_junction_profile_plane(node_id, &stable_incidents, limit_mode)
                    .or_else(|| self.solve_junction_profile_plane(node_id, incidents, limit_mode))
            } else {
                self.solve_junction_profile_plane(node_id, incidents, limit_mode)
            };
            let Some(plane) = plane else {
                continue;
            };
            for incident in incidents {
                if !adaptable_edges.contains(&incident.edge_idx) {
                    continue;
                }
                edge_solves.push((incident.edge_idx, incident.at_start, plane));
            }
        }

        edge_solves.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut changed_edges = Vec::new();
        for (edge_idx, at_start, plane) in edge_solves {
            if edge_idx >= self.edges.len() || self.edges[edge_idx].deleted {
                continue;
            }
            Self::apply_junction_profile_plane_to_edge(
                &mut self.edges[edge_idx],
                at_start,
                plane,
                matches!(limit_mode, JunctionProfileLimitMode::RegradePlatform),
            );
            changed_edges.push(edge_idx);
        }
        changed_edges.sort_unstable();
        changed_edges.dedup();
        for &edge_idx in &changed_edges {
            let (cost, length) = crate::simulation::pathing::cost::CostCalculator::calculate_costs(
                &self.edges[edge_idx],
            );
            self.edges[edge_idx].base_cost = cost;
            self.edges[edge_idx].physical_length = length;
        }
        changed_edges.into_iter().collect()
    }

    fn build_junction_profile_incidents(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: Option<&HashSet<u32>>,
    ) -> HashMap<u32, Vec<JunctionProfileIncident>> {
        let mut incidents_by_node: HashMap<u32, Vec<JunctionProfileIncident>> = HashMap::new();

        for (edge_idx, edge) in self.edges.iter().enumerate() {
            if edge.deleted
                || edge.primary_type != TransitType::Road
                || edge.geometry.len() < 2
                || edge.start_node as usize >= valid_node_ids.len()
                || edge.end_node as usize >= valid_node_ids.len()
            {
                continue;
            }

            let start_node = valid_node_ids[edge.start_node as usize];
            let end_node = valid_node_ids[edge.end_node as usize];
            for (node_id, at_start) in [(start_node, true), (end_node, false)] {
                if affected_nodes.is_some_and(|affected| !affected.contains(&node_id)) {
                    continue;
                }
                if self.nodes[node_id as usize].node_type != NodeType::Junction {
                    continue;
                }
                incidents_by_node
                    .entry(node_id)
                    .or_default()
                    .push(JunctionProfileIncident { edge_idx, at_start });
            }
        }

        for (&node_id, incidents) in incidents_by_node.iter_mut() {
            incidents.sort_by(|a, b| {
                self.junction_profile_incident_angle(node_id, *a)
                    .total_cmp(&self.junction_profile_incident_angle(node_id, *b))
                    .then(a.edge_idx.cmp(&b.edge_idx))
                    .then(a.at_start.cmp(&b.at_start))
            });
        }
        incidents_by_node
    }

    fn junction_profile_incident_angle(
        &self,
        node_id: u32,
        incident: JunctionProfileIncident,
    ) -> f32 {
        let Some(edge) = self.edges.get(incident.edge_idx) else {
            return 0.0;
        };
        let Some(origin) = self.nodes.get(node_id as usize).map(|node| node.pos) else {
            return 0.0;
        };
        let away = if incident.at_start {
            edge.geometry.get(1).copied()
        } else {
            edge.geometry
                .len()
                .checked_sub(2)
                .and_then(|index| edge.geometry.get(index).copied())
        };
        let Some(away) = away else {
            return 0.0;
        };
        (away.z - origin.z).atan2(away.x - origin.x)
    }

    /// Builds the canonical endpoint profile plane for a Bend/JunctionN node from incident edge mouths.
    pub(crate) fn junction_endpoint_profile_plane(
        &self,
        node_id: u32,
    ) -> Option<JunctionEndpointProfilePlane> {
        if self.nodes.get(node_id as usize)?.node_type != NodeType::Junction {
            return None;
        }
        let valid_node_ids: Vec<u32> = (0..self.nodes.len())
            .map(|i| self.get_valid_node(i as u32))
            .collect();
        let affected_nodes = HashSet::from([node_id]);
        let incidents_by_node =
            self.build_junction_profile_incidents(&valid_node_ids, Some(&affected_nodes));
        let incidents = incidents_by_node.get(&node_id)?;
        (incidents.len() >= 2)
            .then(|| {
                self.solve_junction_profile_plane(
                    node_id,
                    incidents,
                    JunctionProfileLimitMode::ConservativeSourceFit,
                )
            })
            .flatten()
    }

    fn solve_junction_profile_plane(
        &self,
        node_id: u32,
        incidents: &[JunctionProfileIncident],
        limit_mode: JunctionProfileLimitMode,
    ) -> Option<JunctionEndpointProfilePlane> {
        let origin = self.nodes.get(node_id as usize)?.pos;
        let mut xx = 0.0;
        let mut xz = 0.0;
        let mut zz = 0.0;
        let mut xy = 0.0;
        let mut zy = 0.0;
        let mut sample_count = 0;
        let mut sample_offsets = Vec::with_capacity(incidents.len());

        for incident in incidents {
            let edge = self.edges.get(incident.edge_idx)?;
            let total_length_m = Self::edge_profile_length_m(edge);
            if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let sample_distance_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(total_length_m * 0.5);
            if sample_distance_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            let Some(sample) = Self::sample_edge_geometry_from_endpoint(
                edge,
                incident.at_start,
                sample_distance_m,
            ) else {
                continue;
            };
            let dx = sample.x - origin.x;
            let dz = sample.z - origin.z;
            let dy = sample.y - origin.y;
            if dx * dx + dz * dz <= JUNCTION_PROFILE_MIN_SAMPLE_M * JUNCTION_PROFILE_MIN_SAMPLE_M {
                continue;
            }
            xx += dx * dx;
            xz += dx * dz;
            zz += dz * dz;
            xy += dx * dy;
            zy += dz * dy;
            sample_count += 1;
            sample_offsets.push((dx, dz));
        }

        if sample_count < 2 {
            return None;
        }
        let det = xx * zz - xz * xz;
        if det.abs() <= JUNCTION_PROFILE_PLANE_DET_EPS {
            return None;
        }

        let grade_x = (xy * zz - zy * xz) / det;
        let grade_z = (xx * zy - xz * xy) / det;
        JunctionEndpointProfilePlane::grade_limited(
            origin,
            grade_x,
            grade_z,
            &sample_offsets,
            limit_mode,
        )
    }

    fn apply_junction_profile_plane_to_edge(
        edge: &mut super::data::Edge,
        at_start: bool,
        plane: JunctionEndpointProfilePlane,
        materialize_supports: bool,
    ) {
        let total_length_m = Self::edge_profile_length_m(edge);
        if total_length_m <= JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        let hard_zone_m = Self::junction_profile_hard_zone_m(edge, total_length_m);
        let blend_end_m =
            (hard_zone_m + JUNCTION_PROFILE_BLEND_ZONE_M).min(total_length_m.max(hard_zone_m));
        if hard_zone_m < JUNCTION_PROFILE_MIN_SAMPLE_M {
            return;
        }
        if materialize_supports {
            Self::materialize_edge_endpoint_profile_supports(edge, at_start, blend_end_m);
        }

        let solve_sample_m =
            materialize_supports.then_some(JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m));
        let distances = Self::edge_endpoint_distances(edge, at_start);
        for (point, distance_m) in edge.geometry.iter_mut().zip(distances.iter().copied()) {
            if distance_m > blend_end_m {
                continue;
            }
            let target_y = plane.origin.y
                + plane.grade_x * (point.x - plane.origin.x)
                + plane.grade_z * (point.z - plane.origin.z);
            let weight = if distance_m <= hard_zone_m
                || solve_sample_m.is_some_and(|sample_m| {
                    (distance_m - sample_m).abs() <= JUNCTION_PROFILE_SUPPORT_EPS_M
                })
                || blend_end_m <= hard_zone_m
            {
                1.0
            } else {
                let t = ((distance_m - hard_zone_m) / (blend_end_m - hard_zone_m)).clamp(0.0, 1.0);
                1.0 - Self::smootherstep(t)
            };
            point.y = point.y * (1.0 - weight) + target_y * weight;
        }
        edge.physical_geometry = edge.geometry.clone();
    }

    fn materialize_edge_endpoint_profile_supports(
        edge: &mut super::data::Edge,
        at_start: bool,
        blend_end_m: f32,
    ) {
        let solve_sample_m = JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(blend_end_m);
        Self::ensure_edge_endpoint_profile_support(edge, at_start, solve_sample_m);

        let mut distance_m = solve_sample_m + JUNCTION_PROFILE_SUPPORT_STEP_M;
        while distance_m < blend_end_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            Self::ensure_edge_endpoint_profile_support(edge, at_start, distance_m);
            distance_m += JUNCTION_PROFILE_SUPPORT_STEP_M;
        }
        Self::ensure_edge_endpoint_profile_support(edge, at_start, blend_end_m);
    }

    fn junction_profile_hard_zone_m(edge: &super::data::Edge, total_length_m: f32) -> f32 {
        (edge.width * 0.25)
            .max(JUNCTION_PROFILE_HARD_ZONE_MIN_M)
            .min(JUNCTION_PROFILE_HARD_ZONE_MAX_M)
            .min(total_length_m * 0.25)
    }

    fn smootherstep(t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
    }

    fn ensure_edge_endpoint_profile_support(
        edge: &mut super::data::Edge,
        at_start: bool,
        distance_m: f32,
    ) {
        if !distance_m.is_finite() || distance_m <= JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if edge.geometry.len() < 2 {
            return;
        }
        let distances = Self::edge_endpoint_distances(edge, at_start);
        let Some(&total_length_m) = distances.iter().max_by(|a, b| a.total_cmp(b)) else {
            return;
        };
        if distance_m >= total_length_m - JUNCTION_PROFILE_SUPPORT_EPS_M {
            return;
        }
        if distances
            .iter()
            .any(|existing| (*existing - distance_m).abs() <= JUNCTION_PROFILE_SUPPORT_EPS_M)
        {
            return;
        }

        if at_start {
            for index in 0..edge.geometry.len() - 1 {
                let start_d = distances[index];
                let end_d = distances[index + 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = edge.geometry[index].lerp(edge.geometry[index + 1], t);
                edge.geometry.insert(index + 1, point);
                return;
            }
        } else {
            for index in (1..edge.geometry.len()).rev() {
                let start_d = distances[index];
                let end_d = distances[index - 1];
                if distance_m < start_d - JUNCTION_PROFILE_SUPPORT_EPS_M
                    || distance_m > end_d + JUNCTION_PROFILE_SUPPORT_EPS_M
                {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                let point = edge.geometry[index].lerp(edge.geometry[index - 1], t);
                edge.geometry.insert(index, point);
                return;
            }
        }
    }

    fn sample_edge_geometry_from_endpoint(
        edge: &super::data::Edge,
        at_start: bool,
        distance_m: f32,
    ) -> Option<Vector3> {
        let distances = Self::edge_endpoint_distances(edge, at_start);
        let points = &edge.geometry;
        if points.is_empty() {
            return None;
        }
        if points.len() == 1 {
            return Some(points[0]);
        }

        if at_start {
            for index in 0..points.len() - 1 {
                let start_d = distances[index];
                let end_d = distances[index + 1];
                if distance_m > end_d && index + 2 < points.len() {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                return Some(points[index].lerp(points[index + 1], t));
            }
        } else {
            for index in (1..points.len()).rev() {
                let start_d = distances[index];
                let end_d = distances[index - 1];
                if distance_m > end_d && index > 1 {
                    continue;
                }
                let segment_m = (end_d - start_d).max(f32::EPSILON);
                let t = ((distance_m - start_d) / segment_m).clamp(0.0, 1.0);
                return Some(points[index].lerp(points[index - 1], t));
            }
        }

        if at_start {
            points.last().copied()
        } else {
            points.first().copied()
        }
    }

    fn edge_endpoint_distances(edge: &super::data::Edge, at_start: bool) -> Vec<f32> {
        let mut distances = vec![0.0; edge.geometry.len()];
        if edge.geometry.len() < 2 {
            return distances;
        }

        if at_start {
            for index in 1..edge.geometry.len() {
                distances[index] = distances[index - 1]
                    + Self::edge_profile_point_distance_m(
                        edge.geometry[index - 1],
                        edge.geometry[index],
                    );
            }
        } else {
            for index in (0..edge.geometry.len() - 1).rev() {
                distances[index] = distances[index + 1]
                    + Self::edge_profile_point_distance_m(
                        edge.geometry[index + 1],
                        edge.geometry[index],
                    );
            }
        }
        distances
    }

    fn surface_edges_touching_nodes(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: &HashSet<u32>,
    ) -> Vec<usize> {
        self.edges
            .iter()
            .enumerate()
            .filter_map(|(edge_idx, edge)| {
                if edge.deleted
                    || edge.primary_type != TransitType::Road
                    || edge.start_node as usize >= valid_node_ids.len()
                    || edge.end_node as usize >= valid_node_ids.len()
                {
                    return None;
                }
                let start_node = valid_node_ids[edge.start_node as usize];
                let end_node = valid_node_ids[edge.end_node as usize];
                (affected_nodes.contains(&start_node) || affected_nodes.contains(&end_node))
                    .then_some(edge_idx)
            })
            .collect()
    }

    fn build_clip_node_stats(
        &self,
        valid_node_ids: &[u32],
        affected_nodes: Option<&HashSet<u32>>,
    ) -> HashMap<u32, ClipNodeStats> {
        let mut stats: HashMap<u32, ClipNodeStats> = HashMap::new();

        for (edge_idx, edge) in self.edges.iter().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let endpoints = [
                (
                    valid_node_ids[edge.start_node as usize],
                    Self::edge_endpoint_direction_xz(edge, true),
                ),
                (
                    valid_node_ids[edge.end_node as usize],
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
        valid_node_ids: &[u32],
        clip_stats: &HashMap<u32, ClipNodeStats>,
        affected_nodes: Option<&HashSet<u32>>,
    ) -> Vec<Option<(f32, f32)>> {
        let mut computed = vec![None; self.edges.len()];

        for (edge_idx, edge) in self.edges.iter().enumerate() {
            if edge.deleted || edge.primary_type != TransitType::Road {
                continue;
            }

            let start_node = valid_node_ids[edge.start_node as usize];
            let end_node = valid_node_ids[edge.end_node as usize];
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
            computed[edge_idx] = Some(Self::fit_clips_to_edge_length(
                start_clip, end_clip, length_m,
            ));
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
                clip_m = clip_m.max(required_m);
            } else {
                clip_m = edge_length_m;
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

    fn directions_are_pass_through(a: Vector2, b: Vector2) -> bool {
        a.dot(b) <= -CLIP_PASS_THROUGH_DOT_THRESHOLD
    }

    fn cross_xz(a: Vector2, b: Vector2) -> f32 {
        a.x * b.y - a.y * b.x
    }

    fn roadbed_half_width_m(edge: &super::data::Edge) -> f32 {
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

    fn edge_endpoint_direction_xz(edge: &super::data::Edge, at_start: bool) -> Option<Vector2> {
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

    fn edge_geometry_length_m(edge: &super::data::Edge) -> f32 {
        edge.geometry
            .windows(2)
            .map(|window| window[0].distance_to(window[1]))
            .sum()
    }

    fn edge_profile_length_m(edge: &super::data::Edge) -> f32 {
        edge.geometry
            .windows(2)
            .map(|window| Self::edge_profile_point_distance_m(window[0], window[1]))
            .sum()
    }

    fn edge_profile_point_distance_m(a: Vector3, b: Vector3) -> f32 {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_test_edge(points: Vec<Vector3>) -> super::super::data::Edge {
        super::super::data::Edge {
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            geometry: points.clone(),
            physical_geometry: points,
            ..Default::default()
        }
    }

    #[test]
    fn junction_profile_sampling_uses_requested_distance_from_edge_end() {
        let edge = profile_test_edge(vec![
            Vector3::new(0.0, 0.0, 0.0),
            Vector3::new(10.0, 0.0, 0.0),
            Vector3::new(20.0, 0.0, 0.0),
        ]);

        let sample = RegionGraph::sample_edge_geometry_from_endpoint(&edge, false, 5.0)
            .expect("edge-end profile sample should exist");

        assert!((sample.x - 15.0).abs() <= f32::EPSILON);
        assert!((sample.y - 0.0).abs() <= f32::EPSILON);
        assert!((sample.z - 0.0).abs() <= f32::EPSILON);
    }

    #[test]
    fn regrade_junction_profile_caps_steep_platform_and_materializes_supports() {
        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::ZERO, NodeType::Junction);
        let east = graph.add_node(Vector3::new(48.0, 24.0, 0.0), NodeType::Junction);
        let north = graph.add_node(Vector3::new(0.0, 0.0, 48.0), NodeType::Junction);

        let mut east_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(48.0, 24.0, 0.0)]);
        east_edge.start_node = center;
        east_edge.end_node = east;
        graph.add_edge(east_edge);

        let mut north_edge = profile_test_edge(vec![Vector3::ZERO, Vector3::new(0.0, 0.0, 48.0)]);
        north_edge.start_node = center;
        north_edge.end_node = north;
        graph.add_edge(north_edge);

        graph.rebuild_adjacency_list();
        let changed_edges =
            graph.regrade_junction_endpoint_profiles_for_nodes(&HashSet::from([center]));

        assert_eq!(changed_edges.len(), 2);
        let plane = graph
            .junction_endpoint_profile_plane(center)
            .expect("regraded support vertices should define a readable platform plane");
        let grade = plane.grade_x.hypot(plane.grade_z);
        assert!(
            grade <= JUNCTION_PROFILE_MOUTH_MAX_GRADE + 1.0e-4,
            "platform grade should be capped after regrade, got {grade:.3}"
        );

        let east_geometry = &graph.edge(0).geometry;
        assert!(
            east_geometry.len() >= 8,
            "regrade should insert vertical-curve support vertices outside the protected mouth"
        );
        let solve_sample_m =
            JUNCTION_PROFILE_SOLVE_SAMPLE_M.min(graph.edge(0).physical_length * 0.5);
        let solve_sample =
            RegionGraph::sample_edge_geometry_from_endpoint(graph.edge(0), true, solve_sample_m)
                .expect("solve-distance support sample should exist");
        let expected_y = plane.height_at_xz(solve_sample.x, solve_sample.z);
        let solve_sample_delta_m = (solve_sample.y - expected_y).abs();
        assert!(
            solve_sample_delta_m <= 0.05,
            "solve-distance support should sit on the capped profile plane: delta={solve_sample_delta_m:.6} sample={solve_sample:?} expected_y={expected_y:.6}"
        );

        let transition_sample = RegionGraph::sample_edge_geometry_from_endpoint(
            graph.edge(0),
            true,
            solve_sample_m + JUNCTION_PROFILE_SUPPORT_STEP_M * 2.0,
        )
        .expect("vertical-curve transition sample should exist");
        let transition_plane_y = plane.height_at_xz(transition_sample.x, transition_sample.z);
        assert!(
            (transition_sample.y - transition_plane_y).abs() > 0.05,
            "post-hard-zone sample should have started easing away from the platform plane"
        );
    }
}
