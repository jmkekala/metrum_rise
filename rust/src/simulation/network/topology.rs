//! Network topology modification and intersection processing.
//!
//! Handles the logic for detecting road crossings, splitting edges at
//! intersections, and migrating associated data (zoning, buildings).

use super::TransitNetwork;
use super::graph::{Edge, Node, RegionGraph};
use super::interaction;
use super::types::*;
use crate::config;
use godot::prelude::*;
use std::collections::HashMap;

const INTERSECTION_NODE_CAPTURE_EPSILON: f32 = 0.05;

/// Maximum distance between consecutive geometry points used for the O(segs²)
/// crossing-detection inner loop. Mouse-tracked polylines can have a point
/// every ~0.5 m; 5 m is accurate enough for roads wider than 7 m and gives a
/// ~100× reduction in comparisons.
const ISECT_MIN_STEP: f32 = 5.0;

/// Returns a geometry vec downsampled so consecutive points are ≥ `min_step` apart.
/// Always keeps the first and last point.
fn downsample(geo: &[Vector3], min_step: f32) -> Vec<Vector3> {
    if geo.len() < 2 {
        return geo.to_vec();
    }
    let mut out = Vec::with_capacity(geo.len());
    out.push(geo[0]);
    let mut acc = 0.0_f32;
    for w in geo.windows(2) {
        acc += w[0].distance_to(w[1]);
        if acc >= min_step {
            out.push(w[1]);
            acc = 0.0;
        }
    }
    if out.last() != geo.last() {
        out.push(*geo.last().unwrap());
    }
    out
}

/// Returns the float segment-index factor (seg + t) in `geo` closest to `pos`.
///
/// Used to convert an approximate 3D intersection position (from downsampled
/// detection) back to an exact factor in the original full-resolution geometry,
/// so `split_edge` receives the correct segment index.
fn find_geo_factor(geo: &[Vector3], pos: Vector3) -> f32 {
    let mut best_factor = 0.0_f32;
    let mut best_dist = f32::MAX;
    for (i, w) in geo.windows(2).enumerate() {
        let ab = w[1] - w[0];
        let ab_sq = ab.dot(ab);
        let t = if ab_sq > 1e-10 {
            ((pos - w[0]).dot(ab) / ab_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let closest = w[0] + ab * t;
        let dist = pos.distance_to(closest);
        if dist < best_dist {
            best_dist = dist;
            best_factor = i as f32 + t;
        }
    }
    best_factor
}

impl RegionGraph {
    /// Returns the canonical node ID for a given ID, following any merge aliases.
    pub fn get_valid_node(&self, mut id: u32) -> u32 {
        while let Some(&alias) = self.node_aliases.get(&id) {
            id = alias;
        }
        id
    }

    /// Returns the index of the edge connecting two nodes, if one exists.
    pub fn get_edge_between_nodes(&self, from: u32, to: u32) -> Option<usize> {
        if (from as usize) < self.node_adjacency_count() {
            for &idx in self.node_adjacency(from) {
                let e = self.edge(idx);
                if !e.deleted
                    && ((e.start_node == from && e.end_node == to)
                        || (e.start_node == to && e.end_node == from))
                {
                    return Some(idx);
                }
            }
        }
        None
    }

    /// Adds a new node to the graph at the specified position.
    pub fn add_node(&mut self, pos: Vector3, node_type: NodeType) -> u32 {
        let id = self.node_count() as u32;
        self.nodes.push(Node {
            pos,
            node_type,
            lane_connections: std::collections::HashMap::new(),
            crosswalk_overrides: std::collections::HashMap::new(),
        });
        self.adjacency.push(Vec::new());
        self.add_node_to_spatial_index(id);
        id
    }

    /// Finds an existing node within `threshold` distance or adds a new one.
    pub fn find_or_add_node(&mut self, pos: Vector3, threshold: f32, node_type: NodeType) -> u32 {
        let chunk_coords = Self::get_node_chunk_coords(pos);

        // Search current and adjacent chunks
        for dx in -1..=1 {
            for dz in -1..=1 {
                if let Some(chunk) = self
                    .spatial_node_grid
                    .get(&(chunk_coords.0 + dx, chunk_coords.1 + dz))
                {
                    for &node_id in chunk {
                        if self.node(node_id).pos.distance_to(pos) < threshold {
                            let id = self.get_valid_node(node_id);
                            return id;
                        }
                    }
                }
            }
        }
        self.add_node(pos, node_type)
    }

    /// Adds a new edge to the graph and updates adjacency and spatial indices.
    pub fn add_edge(&mut self, mut edge: Edge) -> usize {
        edge.deleted = false;
        let start = edge.start_node;
        let end = edge.end_node;
        let id = self.edge_count();
        self.edges.push(edge);
        self.add_to_spatial_index(id);

        // Update Adjacency
        self.adjacency[start as usize].push(id);
        self.adjacency[end as usize].push(id);

        id
    }

    /// Calculates the total physical length of a polyline.
    pub fn calculate_length(&self, pts: &[Vector3]) -> f32 {
        let mut l = 0.0;
        for i in 0..pts.len().saturating_sub(1) {
            l += pts[i].distance_to(pts[i + 1]);
        }
        l
    }

    /// Removes a node and merges its two connected edges if they are compatible.
    pub fn remove_node_and_merge_edges(&mut self, node_id: u32) -> Option<(usize, usize)> {
        if (node_id as usize) >= self.node_count() {
            return None;
        }

        // Find edges connected to this node
        let mut e1_idx = None;
        let mut e2_idx = None;

        for (i, edge) in self.edges().iter().enumerate() {
            if edge.deleted {
                continue;
            } // Important: Skip already deleted edges
            if edge.start_node == node_id || edge.end_node == node_id {
                if e1_idx.is_none() {
                    e1_idx = Some(i);
                } else if e2_idx.is_none() {
                    e2_idx = Some(i);
                } else {
                    // More than 2 edges? This node is likely a real intersection now.
                    // DO NOT MERGE.
                    return None;
                }
            }
        }

        if let (Some(i1), Some(i2)) = (e1_idx, e2_idx) {
            // Check if they are compatible for merging
            let (_target_end_node, mid_node, target_start_node) = {
                let e1 = self.edge(i1);
                let e2 = self.edge(i2);
                if e1.primary_type != e2.primary_type
                    || e1.width != e2.width
                    || e1.vehicle_frontage_access != e2.vehicle_frontage_access
                {
                    return None;
                }

                // Determine the flow: A -> node_id -> B
                let a = if e1.start_node == node_id {
                    e1.end_node
                } else {
                    e1.start_node
                };
                let b = if e2.start_node == node_id {
                    e2.end_node
                } else {
                    e2.start_node
                };

                (a, node_id, b)
            };

            // Combine geometry
            let mut new_geom = Vec::new();
            let (first_edge_idx, second_edge_idx) = {
                if self.edge(i1).end_node == mid_node {
                    (i1, i2)
                } else {
                    (i2, i1)
                }
            };

            for p in &self.edge(first_edge_idx).geometry {
                new_geom.push(*p);
            }
            // Skip the first point of the second edge as it's the same as the last point of the first
            for i in 1..self.edge(second_edge_idx).geometry.len() {
                new_geom.push(self.edge(second_edge_idx).geometry[i]);
            }

            // Update the first edge to span the whole distance
            self.edges[first_edge_idx].end_node = target_start_node;
            self.edges[first_edge_idx].geometry = new_geom;
            // Also combine physical geometry to keep lengths and rendering stable until next rebuild
            let mut new_phys = self.edge(first_edge_idx).physical_geometry.clone();
            if !self.edge(second_edge_idx).physical_geometry.is_empty() {
                new_phys.extend_from_slice(&self.edge(second_edge_idx).physical_geometry[1..]);
            }
            self.edges[first_edge_idx].physical_geometry = new_phys;
            self.edges[first_edge_idx].physical_length =
                self.calculate_length(&self.edges[first_edge_idx].physical_geometry);

            // Mark the second edge as deleted instead of removing it to keep indices stable
            let to_remove = second_edge_idx;
            self.edges[to_remove].deleted = true;
            self.remove_from_spatial_index(to_remove);

            // Re-index the first edge as its geometry changed
            self.remove_from_spatial_index(first_edge_idx);
            self.add_to_spatial_index(first_edge_idx);

            return Some((first_edge_idx, second_edge_idx));
        }
        None
    }

    /// Merges two nodes into one, updating all connected edges.
    pub fn unite_nodes(&mut self, id1: u32, id2: u32) {
        if id1 == id2 {
            return;
        }
        // Ensure we always map to the ultimate valid parent and don't loop
        let keep = self.get_valid_node(id1.min(id2));
        let remove = self.get_valid_node(id1.max(id2));
        if keep == remove {
            return;
        }

        let new_pos = self.node(keep).pos;
        self.node_aliases.insert(remove, keep);

        // Merging two network pieces transforms any restrictive node type into a Junction
        self.nodes[keep as usize].node_type = NodeType::Junction;
        self.nodes[keep as usize].lane_connections.clear();

        // Update all edges using the 'remove' node to use 'keep' node instead
        let mut affected_edges = Vec::new();
        for (i, edge) in self.edges.iter_mut().enumerate() {
            if edge.deleted {
                continue;
            }
            let mut changed = false;
            if edge.start_node == remove {
                edge.start_node = keep;
                if !edge.geometry.is_empty() {
                    edge.geometry[0] = new_pos;
                }
                changed = true;
            }
            if edge.end_node == remove {
                edge.end_node = keep;
                if !edge.geometry.is_empty() {
                    let last = edge.geometry.len() - 1;
                    edge.geometry[last] = new_pos;
                }
                changed = true;
            }
            if changed {
                affected_edges.push(i);
            }
        }

        for i in affected_edges {
            self.remove_from_spatial_index(i);
            self.add_to_spatial_index(i);
        }
    }

    /// Moves a node to a new position, smoothly deforming all connected edges.
    ///
    /// Uses the adjacency list for O(degree) edge lookup instead of an O(E) full scan.
    /// Does NOT rebuild intersection clips — callers that need visual clip updates
    /// (e.g. `move_network_node_internal`) must call `rebuild_intersection_clips` explicitly.
    /// The topology path (`process_intersections` → `add_road`) calls it after all splits.
    pub fn move_node(&mut self, node_id: u32, new_pos: Vector3) {
        let old_pos = self.node(node_id).pos;
        let delta = new_pos - old_pos;

        self.remove_node_from_spatial_index(node_id, old_pos);
        self.nodes[node_id as usize].pos = new_pos;
        self.add_node_to_spatial_index(node_id);

        // Collect connected edges via adjacency list — O(degree), not O(E).
        let connected: Vec<usize> = self
            .node_adjacency(node_id)
            .iter()
            .copied()
            .filter(|&i| !self.edge(i).deleted)
            .collect();

        // Pre-remove from spatial index while geometry/physical_geometry still have old values.
        for &i in &connected {
            self.remove_from_spatial_index(i);
        }

        // Update geometry and physical_geometry with smoothstep deformation.
        for &i in &connected {
            let edge = &mut self.edges[i];
            let count = edge.geometry.len();
            if count < 2 {
                continue;
            }
            let is_start = edge.start_node == node_id;
            let is_end = edge.end_node == node_id;
            if is_start && is_end {
                for pt in edge
                    .geometry
                    .iter_mut()
                    .chain(edge.physical_geometry.iter_mut())
                {
                    *pt += delta;
                }
            } else if is_start {
                for idx in 0..count {
                    let w = 1.0 - (idx as f32 / (count - 1) as f32);
                    let w_smooth = w * w * (3.0 - 2.0 * w);
                    let d = delta * w_smooth;
                    edge.geometry[idx] += d;
                    edge.physical_geometry[idx] += d;
                }
            } else {
                // is_end
                for idx in 0..count {
                    let w = idx as f32 / (count - 1) as f32;
                    let w_smooth = w * w * (3.0 - 2.0 * w);
                    let d = delta * w_smooth;
                    edge.geometry[idx] += d;
                    edge.physical_geometry[idx] += d;
                }
            }
        }

        // Re-add to spatial index with updated geometry.
        for &i in &connected {
            self.add_to_spatial_index(i);
        }
    }
}

fn find_or_add_intersection_node(graph: &mut RegionGraph, pos: Vector3) -> u32 {
    graph.find_or_add_node(pos, INTERSECTION_NODE_CAPTURE_EPSILON, NodeType::Junction)
}

fn snap_new_edge_endpoint_to_intersection(
    graph: &mut RegionGraph,
    edge_id: usize,
    at_start: bool,
    pos: Vector3,
) -> u32 {
    let node_id = graph.get_valid_node(if at_start {
        graph.edge(edge_id).start_node
    } else {
        graph.edge(edge_id).end_node
    });
    let active_degree = graph
        .node_adjacency(node_id)
        .iter()
        .filter(|&&incident_edge| !graph.edge(incident_edge).deleted)
        .count();
    if active_degree == 1 {
        graph.move_node(node_id, pos);
        node_id
    } else {
        find_or_add_intersection_node(graph, pos)
    }
}

/// Finds all edges whose AABB overlaps the new edge's padded AABB.
pub(crate) fn scan_intersection_candidates(graph: &RegionGraph, edge_id: usize) -> Vec<usize> {
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_z = f32::MAX;
    let mut max_z = f32::MIN;
    for p in &graph.edge(edge_id).geometry {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_z = min_z.min(p.z);
        max_z = max_z.max(p.z);
    }
    let pad = config::SNAP_TOLERANCE + 1.0;
    graph.get_edges_near_aabb(
        godot::prelude::Vector3::new(min_x - pad, 0.0, min_z - pad),
        godot::prelude::Vector3::new(max_x + pad, 0.0, max_z + pad),
    )
}

/// Identifies all physical road crossings where edges intersect in 2D.
fn collect_crossing_splits(
    graph: &mut RegionGraph,
    edge_id: usize,
    candidates: &[usize],
    all_splits: &mut HashMap<usize, Vec<(f32, u32)>>,
) {
    let edge1_geo_full = graph.edge(edge_id).geometry.clone();
    let edge1_geo_ds = downsample(&edge1_geo_full, ISECT_MIN_STEP);

    for &other_id in candidates {
        if graph.edge(other_id).deleted {
            continue;
        }

        let edge2_geo_full = graph.edge(other_id).geometry.clone();
        let edge2_geo_ds = downsample(&edge2_geo_full, ISECT_MIN_STEP);

        for i in 0..edge1_geo_ds.len() - 1 {
            let d1 = edge1_geo_ds[i + 1] - edge1_geo_ds[i];
            if d1.length() < 0.001 {
                continue;
            }

            for j in 0..edge2_geo_ds.len() - 1 {
                if edge_id == other_id && (i as i32 - j as i32).abs() < 5 {
                    continue;
                }

                let d2 = edge2_geo_ds[j + 1] - edge2_geo_ds[j];
                if d2.length() < 0.001 {
                    continue;
                }

                // The old `v1.dot(v2).abs() > 0.98` guard is intentionally removed.
                // Truly parallel roads return None from find_intersection_2d anyway
                // (denom ≈ 0). Keeping the guard caused shallow-angle intersections
                // (< ~11.5°) between *distinct* edges to be silently skipped.

                if let Some((t, u)) = interaction::find_intersection_2d(
                    edge1_geo_ds[i],
                    edge1_geo_ds[i + 1],
                    edge2_geo_ds[j],
                    edge2_geo_ds[j + 1],
                ) {
                    let p1 = edge1_geo_ds[i].lerp(edge1_geo_ds[i + 1], t);
                    let p2 = edge2_geo_ds[j].lerp(edge2_geo_ds[j + 1], u);

                    if (p1.y - p2.y).abs() > 4.5 {
                        continue;
                    }

                    let pos = p1;
                    let factor_t = find_geo_factor(&edge1_geo_full, pos);
                    let factor_u = find_geo_factor(&edge2_geo_full, pos);

                    let junction_id = find_or_add_intersection_node(graph, pos);
                    all_splits
                        .entry(edge_id)
                        .or_default()
                        .push((factor_t, junction_id));
                    all_splits
                        .entry(other_id)
                        .or_default()
                        .push((factor_u, junction_id));
                }
            }
        }
    }
}

/// Checks endpoints of the new edge for snapping to existing edges or nodes.
fn collect_endpoint_snap_splits(
    graph: &mut RegionGraph,
    edge_id: usize,
    candidates: &[usize],
    all_splits: &mut HashMap<usize, Vec<(f32, u32)>>,
) {
    let edge1_geo_full = graph.edge(edge_id).geometry.clone();
    let edge1_geo_ds = downsample(&edge1_geo_full, ISECT_MIN_STEP);
    let original_len = edge1_geo_full.len();

    let snap_guard = config::INTERSECTION_TOLERANCE + ISECT_MIN_STEP;
    let endpoints = [edge1_geo_ds[0], *edge1_geo_ds.last().unwrap()];

    for &other_id in candidates {
        if edge_id == other_id || graph.edge(other_id).deleted {
            continue;
        }

        let edge2_geo_full = graph.edge(other_id).geometry.clone();
        let edge2_geo_ds = downsample(&edge2_geo_full, ISECT_MIN_STEP);

        for (idx, &p) in endpoints.iter().enumerate() {
            let factor_t = if idx == 0 {
                0.0
            } else {
                (original_len - 1) as f32
            };

            let mut best_dist = f32::MAX;
            let mut best_closest = godot::prelude::Vector3::ZERO;
            for j in 0..edge2_geo_ds.len() - 1 {
                let closest = interaction::get_closest_point_on_segment(
                    p,
                    edge2_geo_ds[j],
                    edge2_geo_ds[j + 1],
                );
                let dist = Vector2::new(p.x - closest.x, p.z - closest.z).length();
                if dist < best_dist {
                    best_dist = dist;
                    best_closest = closest;
                }
            }

            if best_dist < snap_guard && (p.y - best_closest.y).abs() < 4.5 {
                let factor_u = find_geo_factor(&edge2_geo_full, best_closest);
                let seg = (factor_u.floor() as usize).min(edge2_geo_full.len() - 2);
                let t = factor_u.fract();
                let refined = edge2_geo_full[seg].lerp(edge2_geo_full[seg + 1], t);

                if Vector2::new(p.x - refined.x, p.z - refined.z).length()
                    < config::INTERSECTION_TOLERANCE
                {
                    let junction_id = snap_new_edge_endpoint_to_intersection(
                        graph,
                        edge_id,
                        idx == 0,
                        best_closest,
                    );
                    all_splits
                        .entry(edge_id)
                        .or_default()
                        .push((factor_t, junction_id));
                    all_splits
                        .entry(other_id)
                        .or_default()
                        .push((factor_u, junction_id));
                }
            }
        }
    }
}

/// Applies all identified splits to the graph, handling node unification and edge splitting.
fn apply_splits(
    all_splits: HashMap<usize, Vec<(f32, u32)>>,
    network: &mut TransitNetwork,
    graph: &mut RegionGraph,
    zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
    allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
) {
    for (eid, mut splits) in all_splits {
        let geo_len = graph.edge(eid).geometry.len();
        splits.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        splits.dedup_by(|a, b| a.1 == b.1);

        for (factor, junction_id) in splits {
            let seg_idx = factor.floor() as usize;
            let sub_t = factor.fract();

            if factor < 0.1 {
                let start_node = graph.edge(eid).start_node;
                network.mark_point_dirty(graph.node(start_node).pos);
                graph.unite_nodes(junction_id, start_node);
                continue;
            }
            if factor > (geo_len - 1) as f32 - 0.1 {
                let end_node = graph.edge(eid).end_node;
                network.mark_point_dirty(graph.node(end_node).pos);
                graph.unite_nodes(junction_id, end_node);
                continue;
            }
            let valid_junction_id = graph.get_valid_node(junction_id);
            split_edge(
                network,
                graph,
                eid,
                seg_idx,
                sub_t,
                valid_junction_id,
                zoning,
                allocator,
            );
        }
    }
}

/// Migrates zoning occupancy and buildings when an edge is split.
fn migrate_split_dependents(
    edge_id: usize,
    new_edge_id: usize,
    split_x: usize,
    new_len_first: f32,
    new_len_second: f32,
    zoning: &crate::simulation::grid::zoning::ZoningSystem,
    allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
) {
    let cell_size = zoning.config.zone_cell_m;

    // Migrate buildings
    for b in &mut allocator.buildings {
        if b.edge_idx == edge_id {
            if b.cell_x >= split_x {
                b.edge_idx = new_edge_id;
                b.cell_x = b.cell_x.saturating_sub(split_x);
                let half_cells = b.width_cells as f32 * 0.5;
                b.frontage_t =
                    (b.cell_x as f32 + half_cells) * cell_size / new_len_second.max(0.001);
            } else {
                let half_cells = b.width_cells as f32 * 0.5;
                b.frontage_t =
                    (b.cell_x as f32 + half_cells) * cell_size / new_len_first.max(0.001);
            }
        }
    }

    // Migrate edge occupancy
    if let Some(old_occ) = allocator.edge_occupancy.remove(&edge_id) {
        let part1_len = split_x.min(old_occ.cells_long);
        let part2_len = old_occ.cells_long.saturating_sub(split_x);
        allocator.edge_occupancy.insert(
            edge_id,
            crate::simulation::buildings::allocator::EdgeOccupancy {
                cells_long: part1_len,
                left: old_occ.left[..part1_len].to_vec(),
                right: old_occ.right[..part1_len].to_vec(),
            },
        );
        if part2_len > 0 {
            allocator.edge_occupancy.insert(
                new_edge_id,
                crate::simulation::buildings::allocator::EdgeOccupancy {
                    cells_long: part2_len,
                    left: old_occ.left[split_x..].to_vec(),
                    right: old_occ.right[split_x..].to_vec(),
                },
            );
        }
    }
}

/// Checks every junction node reachable from candidate edges against the interior of
/// `edge_id`'s geometry and records a split wherever one lies within
/// [`config::INTERSECTION_TOLERANCE`] of the road centreline.
///
/// This catches the case where the new road passes exactly through an existing junction
/// that belongs only to parallel roads: `collect_crossing_splits` skips parallel pairs
/// (and `find_intersection_2d` returns `None` for them regardless), while
/// `collect_endpoint_snap_splits` only tests the *new* edge's own endpoints.
fn collect_interior_node_splits(
    graph: &mut RegionGraph,
    edge_id: usize,
    candidates: &[usize],
    all_splits: &mut HashMap<usize, Vec<(f32, u32)>>,
) {
    let edge_geo = graph.edge(edge_id).geometry.clone();
    let geo_len = edge_geo.len();
    if geo_len < 2 {
        return;
    }
    let new_start = graph.get_valid_node(graph.edge(edge_id).start_node);
    let new_end = graph.get_valid_node(graph.edge(edge_id).end_node);

    // Collect unique canonical junction nodes from all candidate edges.
    let mut seen: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for &other_id in candidates {
        if other_id == edge_id || graph.edge(other_id).deleted {
            continue;
        }
        let e = graph.edge(other_id);
        for &raw in &[e.start_node, e.end_node] {
            seen.insert(graph.get_valid_node(raw));
        }
    }

    for node_id in seen {
        // Skip the new edge's own terminal nodes (handled by endpoint snap).
        if node_id == new_start || node_id == new_end {
            continue;
        }
        // Skip if this node is already scheduled for a split on this edge.
        if all_splits
            .get(&edge_id)
            .map_or(false, |v| v.iter().any(|&(_, jid)| jid == node_id))
        {
            continue;
        }
        let node_pos = graph.node(node_id).pos;

        // Find the closest point on the new edge's centreline to this node (XZ plane).
        let mut best_dist_xz = f32::MAX;
        let mut best_factor = 0.0_f32;
        let mut best_y = 0.0_f32;
        for (seg_idx, w) in edge_geo.windows(2).enumerate() {
            let closest = interaction::get_closest_point_on_segment(node_pos, w[0], w[1]);
            let dist_xz =
                godot::prelude::Vector2::new(node_pos.x - closest.x, node_pos.z - closest.z)
                    .length();
            if dist_xz < best_dist_xz {
                best_dist_xz = dist_xz;
                best_y = closest.y;
                let ab = w[1] - w[0];
                let t = if ab.length_squared() > 1e-10 {
                    ((node_pos - w[0]).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                best_factor = seg_idx as f32 + t;
            }
        }

        if best_dist_xz >= config::INTERSECTION_TOLERANCE {
            continue;
        }
        if (node_pos.y - best_y).abs() > 4.5 {
            continue; // different elevation (bridge / tunnel)
        }
        // Endpoint cases are handled by collect_endpoint_snap_splits.
        let at_start = best_factor < 0.1;
        let at_end = best_factor > (geo_len - 1) as f32 - 0.1;
        if at_start || at_end {
            continue;
        }

        all_splits
            .entry(edge_id)
            .or_default()
            .push((best_factor, node_id));
    }
}

/// Scans for and processes all intersections for a given edge.
pub fn process_intersections(
    network: &mut TransitNetwork,
    graph: &mut RegionGraph,
    edge_id: usize,
    zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
    allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
) {
    let t0 = std::time::Instant::now();

    // 1. Scan for candidates
    let candidates = scan_intersection_candidates(graph, edge_id);
    let mut all_splits: HashMap<usize, Vec<(f32, u32)>> = HashMap::new();

    // 2. Collect all splits: geometric crossings, endpoint snaps, interior-node snaps.
    collect_crossing_splits(graph, edge_id, &candidates, &mut all_splits);
    collect_endpoint_snap_splits(graph, edge_id, &candidates, &mut all_splits);
    collect_interior_node_splits(graph, edge_id, &candidates, &mut all_splits);

    // 3. Emit per-split details before apply_splits consumes the map.
    if crate::debug::is_traffic_enabled() {
        let total_splits: usize = all_splits.values().map(|v| v.len()).sum();
        crate::traffic_log!(
            "[ROAD] place e{}: candidates={} splits={} elapsed={}µs",
            edge_id,
            candidates.len(),
            total_splits,
            t0.elapsed().as_micros()
        );
        for (&eid, splits) in &all_splits {
            for &(factor, jid) in splits {
                if jid < graph.node_count() as u32 {
                    let p = graph.node(jid).pos;
                    crate::traffic_log!(
                        "[ROAD]   split e{} factor={:.3} junction=N{} ({:.1},{:.1})",
                        eid,
                        factor,
                        jid,
                        p.x,
                        p.z
                    );
                }
            }
        }
    }

    // 4. Apply splits to the graph and dependent systems
    apply_splits(all_splits, network, graph, zoning, allocator);

    let dt_total_us = t0.elapsed().as_micros();
    crate::debug_log!(
        "isect",
        "e{} candidates={} total={}µs",
        edge_id,
        candidates.len(),
        dt_total_us
    );
}

/// Splits an existing edge at a specific segment and junction node.
///
/// Handles the geometric split, re-indexing, and migration of all dependent
/// simulation data including zoning cells and buildings.
pub fn split_edge(
    network: &mut TransitNetwork,
    graph: &mut RegionGraph,
    edge_id: usize,
    segment_idx: usize,
    _t: f32,
    junction_node_id: u32,
    zoning: &mut crate::simulation::grid::zoning::ZoningSystem,
    allocator: &mut crate::simulation::buildings::allocator::BuildingAllocator,
) {
    let old_edge = graph.edge(edge_id);
    let geometry = &old_edge.geometry;
    let _length = old_edge.physical_length;
    let split_pos = graph.node(junction_node_id).pos;

    // Physical distance guard: Don't split if too close to either end (e.g. < 0.2m)
    let start_pos = geometry[0];
    let end_pos = *geometry.last().unwrap();
    if split_pos.distance_to(start_pos) < 0.2 || split_pos.distance_to(end_pos) < 0.2 {
        return;
    }

    let end_node = old_edge.end_node;

    let mut part2_geo = vec![split_pos];
    part2_geo.extend_from_slice(&old_edge.geometry[segment_idx + 1..]);

    let mut part1_geo = old_edge.geometry[..=segment_idx].to_vec();
    if part1_geo.last().unwrap().distance_to(split_pos) > 0.001 {
        part1_geo.push(split_pos);
    }

    let primary_type = old_edge.primary_type;
    let allowed_types = old_edge.allowed_types;
    let width = old_edge.width;
    let fwd_lanes = old_edge.fwd_lanes;
    let bkw_lanes = old_edge.bkw_lanes;
    let speed_limit = old_edge.speed_limit;
    let current_congestion = old_edge.current_congestion;
    let class = old_edge.class;
    let no_building_spawn = old_edge.no_building_spawn;
    let vehicle_frontage_access = old_edge.vehicle_frontage_access;

    let old_end_node = graph.edges[edge_id].end_node;

    // Remove from spatial index BEFORE updating geometry so the AABB still matches
    // the current entry in the R-tree. Updating geometry first causes a AABB mismatch
    // and the remove silently fails, leaving a stale entry.
    graph.remove_from_spatial_index(edge_id);

    graph.edges[edge_id].end_node = junction_node_id;
    graph.edges[edge_id].geometry = part1_geo.clone();
    graph.edges[edge_id].physical_geometry = part1_geo;
    let (cost, length) =
        crate::simulation::pathing::cost::CostCalculator::calculate_costs(&graph.edges[edge_id]);
    graph.edges[edge_id].base_cost = cost;
    graph.edges[edge_id].physical_length = length;

    graph.add_to_spatial_index(edge_id);

    graph.adjacency[old_end_node as usize].retain(|&i| i != edge_id);
    graph.adjacency[junction_node_id as usize].push(edge_id);

    let mut new_edge = Edge {
        start_node: junction_node_id,
        end_node,
        primary_type,
        allowed_types,
        width,
        fwd_lanes,
        bkw_lanes,
        speed_limit,
        base_cost: 0.0,
        physical_length: 0.0,
        current_congestion,
        start_clip: 0.0,
        end_clip: 0.0,
        geometry: part2_geo.clone(),
        physical_geometry: part2_geo.clone(),
        class,
        deleted: false,
        no_building_spawn,
        vehicle_frontage_access,
    };
    let (cost_new, length_new) =
        crate::simulation::pathing::cost::CostCalculator::calculate_costs(&new_edge);
    new_edge.base_cost = cost_new;
    new_edge.physical_length = length_new;

    let new_edge_id = graph.add_edge(new_edge);

    // --- MIGRATION LOGIC ---
    let new_len_first = graph.edge(edge_id).physical_length;
    let new_len_second = graph.edge(new_edge_id).physical_length;
    let split_x = (new_len_first / zoning.config.zone_cell_m).floor() as usize;

    migrate_split_dependents(
        edge_id,
        new_edge_id,
        split_x,
        new_len_first,
        new_len_second,
        zoning,
        allocator,
    );

    // distance_to_road grid does not need updating per split; it is recomputed
    // once after the full road placement in add_road / finalize_bulk_load.
    network.mark_point_dirty(split_pos);
    if network.bulk_load {
        network.bulk_dirty_edges.insert(edge_id);
        network.bulk_dirty_edges.insert(new_edge_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{BuildingData, LodEntry, PlacementMode, ZoneClass};
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::grid::zoning::ZoningSystem;
    use crate::simulation::network::TransitNetwork;
    use godot::prelude::Vector3;

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
        let (household_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        allocator.registry.register(
            pack_id,
            AssetManifest {
                asset_id: asset_id.to_owned(),
                display_name: "Test".to_owned(),
                asset_set: None,
                tags: vec![],
                thumbnail: None,
                lods: vec![LodEntry {
                    file: "lod0.glb".to_owned(),
                    distance_min_m: 0.0,
                    distance_max_m: None,
                }],
                anchors: vec![],
                building: Some(BuildingData { flat_size_m2: None,
                    placement_mode: PlacementMode::ZonedPrivate,
                    zone_type: Some(zone),
                    density: Some("low".to_owned()),
                    lot_width_cells: 3,
                    lot_depth_cells: 3,
                    min_zone_width_cells: None,
                    min_zone_depth_cells: None,
                    level: 1,
                    household_capacity,
                    worker_capacity,
                    service_class: None,
                    economy_profile: None,
                    preview_scale: None,
                }),
                prop: None,
                vehicle: None,
                character: None,
                pivot_offset: None,
            },
            String::new(),
        );
        format!("{pack_id}:{asset_id}")
    }

    #[test]
    fn test_topology_t_junction() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();
        // straight road
        net.add_road(
            &mut graph,
            vec![Vector3::new(-10.0, 0.0, 0.0), Vector3::new(10.0, 0.0, 0.0)].into(),
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        // side road connecting to the middle
        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 10.0), Vector3::new(0.0, 0.0, 0.0)].into(),
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
    }
    #[test]
    fn test_split_edge_recalculates_building_frontage() {
        let mut net = TransitNetwork::new();
        let mut graph = RegionGraph::new();
        let config = MapConfig::default();
        let mut zoning = ZoningSystem::new(&config);
        let mut allocator = BuildingAllocator::new();

        net.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)].into(),
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        let edge_id = graph.edges.len() - 1;
        let residential_asset = register_test_asset(
            &mut allocator,
            "test",
            "split_edge_residential",
            ZoneClass::Residential,
        );

        // Force a mock building at cell_x = 8, which is at 80m.
        allocator
            .buildings
            .push(crate::simulation::buildings::allocator::Building {
                center_x: 80.0,
                center_y: 10.0,
                width_cells: 3,
                depth_cells: 3,
                zone_profile_runtime_id: 0,
                zone_type: crate::simulation::grid::zoning::ZoneType::Residential,
                facing_dir: godot::prelude::Vector2::new(0.0, 1.0),
                frontage_t: 0.85, // Pre-split frontage_t
                side_offset: 1.0,
                is_deserted: false,
                budget_distress: false,
                edge_idx: edge_id,
                side: 1,
                cell_x: 8,
                cell_y: 0,
                occupancy: 0,
                worker_count: 0,
                asset_id: residential_asset,
                level: 1,
                broken: false,
                economy_profile_runtime_id: 0,
                economy_broken: false,
                resource_inventory: Vec::new(),
                revenue: 0.0,
                operating_budget: 500.0,
                shipment_cooldown_hours: 0,
                daily_owa_input_value: 0.0,
                daily_local_input_value: 0.0,
                pending_redevelopment: false,
                rezone_grace_days_remaining: 0,
            });

        // Split the road exactly at 50m (cell 5).
        let junction_id = graph.add_node(Vector3::new(50.0, 0.0, 0.0), NodeType::Junction);
        split_edge(
            &mut net,
            &mut graph,
            edge_id,
            0,
            0.5, // Used to interpolate inside split_edge (dummy)
            junction_id,
            &mut zoning,
            &mut allocator,
        );

        let b = &allocator.buildings[0];
        // The edge should have split. The building was at cell 8.
        // It migrated to the new edge, so its cell_x should be 8 - 5 = 3.
        assert_eq!(b.cell_x, 3);
        assert_ne!(b.edge_idx, edge_id);

        // original length = 100m. cell = 10m.
        // New edge length = 50m.
        // frontage_t should be updated to a percentage along the new 50m edge.
        // formula: (3 + 1.5) * 10 / 50 = 45 / 50 = 0.90!
        assert!(
            (b.frontage_t - 0.90).abs() < 0.01,
            "Expected 0.90, got {}",
            b.frontage_t
        );
    }
}
