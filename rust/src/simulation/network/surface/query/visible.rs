// SPDX-License-Identifier: GPL-2.0-only

//! Visible-surface sampling, raycast, and section-range queries.

use super::super::{
    RoadLaneSurfaceQuery, RoadSurfaceIndexedTriangle, RoadSurfaceSection, RoadSurfaceSystem,
    RoadSurfaceTriangleQueryIndex, SurfaceChunkKey,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3, godot_vec3_to_road};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

const SAMPLE_EPSILON_M: f64 = 0.001;

impl RoadSurfaceSystem {
    /// Resolves immutable owner-local acceleration data once for all samples of one lane.
    pub(crate) fn lane_owner_surface_query<'a>(
        &'a self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_id: usize,
        node_id: usize,
        carriageway_only: bool,
    ) -> RoadLaneSurfaceQuery<'a> {
        let edge_id = (edge_id < graph.edge_count()).then_some(edge_id);
        let mut node_ids = [u32::MAX; 2];
        let node_count = if let Some(edge_id) = edge_id {
            let edge = graph.edge(edge_id);
            node_ids[0] = graph.get_valid_node(edge.start_node);
            node_ids[1] = graph.get_valid_node(edge.end_node);
            usize::from(node_ids[0] != node_ids[1]) + 1
        } else if node_id < graph.node_count() {
            node_ids[0] = graph.get_valid_node(node_id as u32);
            1
        } else {
            0
        };
        let mut node_indices = [None; 2];
        for (target, &node_id) in node_indices.iter_mut().zip(&node_ids[..node_count]) {
            if self.node_uses_visible_surface(graph, terrain, node_id) {
                *target = self
                    .compiled_visual_node_pieces
                    .get(&node_id)
                    .map(|piece| piece.surface_query.as_ref());
            }
        }
        RoadLaneSurfaceQuery {
            node_indices,
            node_count,
            span_index: edge_id
                .and_then(|edge_id| self.compiled_visual_span_pieces.get(&edge_id))
                .map(|piece| piece.surface_query.as_ref()),
            carriageway_only,
        }
    }

    pub(crate) fn sample_visible_surface_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let (surface, earthwork) = self.sample_visible_surface_layers_filtered(
            graph,
            terrain,
            world_x,
            world_z,
            |_| true,
            |_| true,
        );
        surface.or(earthwork)
    }

    /// Queries selected owners without allowing earthworks to override any visible road top.
    pub(super) fn sample_visible_surface_layers_filtered(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
        include_edge: impl Fn(usize) -> bool,
        include_node: impl Fn(u32) -> bool,
    ) -> (Option<f32>, Option<f32>) {
        let world_x = f64::from(world_x);
        let world_z = f64::from(world_z);
        let point = RoadVec2::new(world_x, world_z);
        let chunk = Self::query_chunk_coords_for_world(world_x, world_z);
        let edge_indices = self.query_chunk_spans.get(&chunk);
        let node_ids = self.query_chunk_nodes.get(&chunk);
        let mut top_surface_height_m: Option<f32> = None;

        // Reuse the immutable owner-local triangle grids already built for lane queries.
        // O(owners in query chunk + triangles in their matching cells), with no allocations.
        for &node_id in node_ids.into_iter().flatten() {
            if !include_node(node_id) || !self.node_uses_visible_surface(graph, terrain, node_id) {
                continue;
            }
            if let Some(piece) = self.compiled_visual_node_pieces.get(&node_id)
                && let Some(height_m) = piece.surface_query.sample_visible_height(point)
            {
                keep_max_height(&mut top_surface_height_m, height_m);
            }
        }
        for &edge_idx in edge_indices.into_iter().flatten() {
            if !include_edge(edge_idx) {
                continue;
            }
            if let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx)
                && let Some(height_m) = piece.surface_query.sample_visible_height(point)
            {
                keep_max_height(&mut top_surface_height_m, height_m);
            }
        }

        if top_surface_height_m.is_some() {
            return (top_surface_height_m, None);
        }

        let mut earthwork_height_m: Option<f32> = None;
        self.visit_visible_earthwork_query_triangles(
            graph,
            terrain,
            edge_indices
                .into_iter()
                .flat_map(|owners| owners.iter().copied())
                .filter(|edge| include_edge(*edge)),
            node_ids
                .into_iter()
                .flat_map(|owners| owners.iter().copied())
                .filter(|node| include_node(*node)),
            &mut |triangle| {
                if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                    keep_max_height(&mut earthwork_height_m, height_m);
                }
            },
        );

        (None, earthwork_height_m)
    }

    pub(crate) fn sample_visible_carriageway_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let world_x = f64::from(world_x);
        let world_z = f64::from(world_z);
        let chunk = Self::query_chunk_coords_for_world(world_x, world_z);
        let edge_indices = self.query_chunk_spans.get(&chunk);
        let node_ids = self.query_chunk_nodes.get(&chunk);
        let point = RoadVec2::new(world_x, world_z);
        let mut road_surface_height_m: Option<f32> = None;

        for &node_id in node_ids.into_iter().flatten() {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_uses_visible_surface(graph, terrain, node_id) {
                continue;
            }
            for polygon in &piece.road_surface_polygons {
                Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                    if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                        keep_max_height(&mut road_surface_height_m, height_m);
                    }
                });
            }
        }

        for &edge_idx in edge_indices.into_iter().flatten() {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            for polygon in &piece.road_surface_polygons {
                Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                    if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                        keep_max_height(&mut road_surface_height_m, height_m);
                    }
                });
            }
        }

        road_surface_height_m
    }

    #[cfg(test)]
    pub(crate) fn sample_paved_support_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let world_x = f64::from(world_x);
        let world_z = f64::from(world_z);
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = RoadVec2::new(world_x, world_z);
        let mut best_height_m: Option<f32> = None;

        // Terrain support clearance is a lower envelope: where terminal caps, spans, or raised
        // bands overlap in XZ, terrain must remain below every road-owned top surface. Visible
        // picking uses the highest rendered surface instead.
        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_earthworks(graph, node_id, terrain) {
                continue;
            }
            let height_offset_m =
                self.node_piece_integrated_surface_offset_m(graph, node_id, terrain);

            for polygon in piece
                .road_surface_polygons
                .iter()
                .chain(&piece.curb_surface_polygons)
                .chain(&piece.sidewalk_surface_polygons)
            {
                Self::visit_visual_polygon_triangles(polygon, &mut |triangle| {
                    if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                        keep_min_height(&mut best_height_m, height_m - height_offset_m);
                    }
                });
            }
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
            self.visit_span_piece_clearance_triangles(piece, &mut |triangle| {
                if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                    keep_min_height(&mut best_height_m, height_m - height_offset_m);
                }
            });
        }

        best_height_m
    }

    #[cfg(test)]
    pub(crate) fn raycast_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        let road_hit = self.raycast_road_visible_surface(graph, terrain, ray_origin, ray_dir);
        let terrain_hit = terrain.raycast_visual_terrain(ray_origin, ray_dir);
        closest_ray_hit(ray_origin, ray_dir, road_hit, terrain_hit)
    }

    pub(crate) fn raycast_road_visible_surface(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        ray_origin: Vector3,
        ray_dir: Vector3,
    ) -> Option<Vector3> {
        if ray_dir.length_squared() <= f32::EPSILON {
            return None;
        }

        let ray_origin_road = godot_vec3_to_road(ray_origin);
        let ray_dir_road = godot_vec3_to_road(ray_dir);
        let Some((min_chunk, max_chunk)) =
            self.raycast_visible_query_chunk_bounds(terrain, ray_origin_road, ray_dir_road, None)
        else {
            return None;
        };
        let (edge_indices, node_ids) = self.collect_query_contributors(min_chunk, max_chunk);

        let mut best_t = f64::INFINITY;
        let mut best_hit = None;

        self.visit_visible_top_surface_query_triangles(
            graph,
            terrain,
            edge_indices.iter().copied(),
            node_ids.iter().copied(),
            &mut |triangle| {
                update_closest_ray_hit(
                    triangle,
                    ray_origin_road,
                    ray_dir_road,
                    &mut best_t,
                    &mut best_hit,
                );
            },
        );
        self.visit_visible_earthwork_query_triangles(
            graph,
            terrain,
            edge_indices.iter().copied(),
            node_ids.iter().copied(),
            &mut |triangle| {
                update_closest_ray_hit(
                    triangle,
                    ray_origin_road,
                    ray_dir_road,
                    &mut best_t,
                    &mut best_hit,
                );
            },
        );

        best_hit.map(|hit| Vector3::new(hit.x as f32, hit.y as f32, hit.z as f32))
    }

    pub(crate) fn visible_section_ranges_for_edge(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Vec<(usize, usize)> {
        let Some((start_index, end_index)) =
            self.visible_corridor_index_range_for_edge(graph, edge_idx, sections)
        else {
            return Vec::new();
        };
        if graph.edge(edge_idx).class != EdgeClass::Tunnel {
            return vec![(start_index, end_index)];
        }

        self.tunnel_visible_section_ranges(sections, start_index, end_index, terrain)
    }

    fn visible_corridor_index_range_for_edge(
        &self,
        graph: &RegionGraph,
        edge_idx: usize,
        sections: &[RoadSurfaceSection],
    ) -> Option<(usize, usize)> {
        if sections.len() < 2 || edge_idx >= graph.edge_count() {
            return None;
        }

        let edge = graph.edge(edge_idx);
        let total_length = sections.last()?.s_m.max(0.0);
        let start_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.start_node),
        );
        let end_kind = self.classify_surface_node_kind_from_graph_geometry(
            graph,
            graph.get_valid_node(edge.end_node),
        );
        let Some((start_handoff, end_handoff)) = self
            .visual_edge_mouth_policy_for_edge(
                graph,
                edge_idx,
                edge,
                total_length,
                start_kind,
                end_kind,
                false,
                false,
            )
            .ownership_range
        else {
            return None;
        };

        Self::section_index_range_for_s_bounds(sections, start_handoff, end_handoff)
    }

    fn triangle_height_at_xz(triangle: [RoadVec3; 3], point: RoadVec2) -> Option<f32> {
        let (wa, wb, wc) = road_triangle_barycentric_weights_xz(triangle, point)?;
        Some((triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc) as f32)
    }

    fn raycast_visible_query_chunk_bounds(
        &self,
        terrain: &TerrainSystem,
        ray_origin: RoadVec3,
        ray_dir: RoadVec3,
        terrain_hit: Option<RoadVec3>,
    ) -> Option<(SurfaceChunkKey, SurfaceChunkKey)> {
        if let Some(terrain_hit) = terrain_hit {
            return Some((
                self.chunk_coords_for_world(
                    ray_origin.x.min(terrain_hit.x),
                    ray_origin.z.min(terrain_hit.z),
                ),
                self.chunk_coords_for_world(
                    ray_origin.x.max(terrain_hit.x),
                    ray_origin.z.max(terrain_hit.z),
                ),
            ));
        }

        let (half_w, half_h) = terrain.half_world_extents();
        let half_w = f64::from(half_w);
        let half_h = f64::from(half_h);
        let (entry_t, exit_t) =
            ray_xz_interval_for_bounds(ray_origin, ray_dir, -half_w, -half_h, half_w, half_h)?;
        if !entry_t.is_finite() || !exit_t.is_finite() {
            let chunk = self.chunk_coords_for_world(ray_origin.x, ray_origin.z);
            return Some((chunk, chunk));
        }

        let start_t = entry_t.max(0.0);
        let end_t = exit_t.max(start_t);
        let start = ray_origin + ray_dir * start_t;
        let end = ray_origin + ray_dir * end_t;
        Some((
            self.chunk_coords_for_world(start.x.min(end.x), start.z.min(end.z)),
            self.chunk_coords_for_world(start.x.max(end.x), start.z.max(end.z)),
        ))
    }
}

impl RoadLaneSurfaceQuery<'_> {
    /// Samples the highest matching owner surface without graph or hash-map traversal.
    pub(crate) fn sample_height(&self, world_x: f32, world_z: f32) -> Option<f32> {
        let point = RoadVec2::new(f64::from(world_x), f64::from(world_z));
        let mut surface_height_m = None;
        for index in self.node_indices[..self.node_count].iter().flatten() {
            if let Some(height_m) = index.sample_height(point, self.carriageway_only) {
                keep_max_height(&mut surface_height_m, height_m);
            }
        }
        if let Some(index) = self.span_index
            && let Some(height_m) = index.sample_height(point, self.carriageway_only)
        {
            keep_max_height(&mut surface_height_m, height_m);
        }
        surface_height_m
    }
}

impl RoadSurfaceTriangleQueryIndex {
    fn sample_height(&self, point: RoadVec2, carriageway_only: bool) -> Option<f32> {
        self.sample_height_matching(point, |triangle| !carriageway_only || triangle.carriageway)
    }

    fn sample_visible_height(&self, point: RoadVec2) -> Option<f32> {
        self.sample_height_matching(point, |triangle| {
            RoadSurfaceSystem::top_surface_triangle_is_renderable_xz(triangle.triangle)
        })
    }

    fn sample_height_matching(
        &self,
        point: RoadVec2,
        accepts: impl Fn(&RoadSurfaceIndexedTriangle) -> bool,
    ) -> Option<f32> {
        let mut surface_height_m = None;
        for &triangle_idx in self.cell_triangle_indices(point) {
            let indexed = self.triangles[triangle_idx as usize];
            if !accepts(&indexed) {
                continue;
            }
            if let Some(height_m) =
                RoadSurfaceSystem::triangle_height_at_xz(indexed.triangle, point)
            {
                keep_max_height(&mut surface_height_m, height_m);
            }
        }
        surface_height_m
    }
}

fn ray_xz_interval_for_bounds(
    ray_origin: RoadVec3,
    ray_dir: RoadVec3,
    min_x: f64,
    min_z: f64,
    max_x: f64,
    max_z: f64,
) -> Option<(f64, f64)> {
    let mut entry_t = f64::NEG_INFINITY;
    let mut exit_t = f64::INFINITY;
    clip_ray_axis_interval(
        ray_origin.x,
        ray_dir.x,
        min_x,
        max_x,
        &mut entry_t,
        &mut exit_t,
    )?;
    clip_ray_axis_interval(
        ray_origin.z,
        ray_dir.z,
        min_z,
        max_z,
        &mut entry_t,
        &mut exit_t,
    )?;
    (exit_t >= entry_t && exit_t >= 0.0).then_some((entry_t, exit_t))
}

fn clip_ray_axis_interval(
    origin: f64,
    direction: f64,
    min: f64,
    max: f64,
    entry_t: &mut f64,
    exit_t: &mut f64,
) -> Option<()> {
    if direction.abs() <= f64::EPSILON {
        return (origin >= min && origin <= max).then_some(());
    }

    let t0 = (min - origin) / direction;
    let t1 = (max - origin) / direction;
    *entry_t = entry_t.max(t0.min(t1));
    *exit_t = exit_t.min(t0.max(t1));
    Some(())
}

fn keep_max_height(target: &mut Option<f32>, height_m: f32) {
    *target = Some(target.map_or(height_m, |best| best.max(height_m)));
}

#[cfg(test)]
fn keep_min_height(target: &mut Option<f32>, height_m: f32) {
    *target = Some(target.map_or(height_m, |best| best.min(height_m)));
}

fn update_closest_ray_hit(
    triangle: [RoadVec3; 3],
    ray_origin: RoadVec3,
    ray_dir: RoadVec3,
    best_t: &mut f64,
    best_hit: &mut Option<RoadVec3>,
) {
    let Some(t) = road_ray_triangle_intersection_t(triangle, ray_origin, ray_dir) else {
        return;
    };
    if t >= 0.0 && t <= *best_t {
        *best_t = t;
        *best_hit = Some(ray_origin + ray_dir * t);
    }
}

#[cfg(test)]
fn closest_ray_hit(
    ray_origin: Vector3,
    ray_dir: Vector3,
    left: Option<Vector3>,
    right: Option<Vector3>,
) -> Option<Vector3> {
    match (left, right) {
        (Some(left), Some(right)) => {
            let denom = ray_dir.length_squared().max(f32::EPSILON);
            let left_t = (left - ray_origin).dot(ray_dir) / denom;
            let right_t = (right - ray_origin).dot(ray_dir) / denom;
            if right_t < left_t {
                Some(right)
            } else {
                Some(left)
            }
        }
        (Some(hit), None) | (None, Some(hit)) => Some(hit),
        (None, None) => None,
    }
}

fn road_triangle_barycentric_weights_xz(
    triangle: [RoadVec3; 3],
    point: RoadVec2,
) -> Option<(f64, f64, f64)> {
    let min_x = triangle[0].x.min(triangle[1].x).min(triangle[2].x) - SAMPLE_EPSILON_M;
    let max_x = triangle[0].x.max(triangle[1].x).max(triangle[2].x) + SAMPLE_EPSILON_M;
    let min_z = triangle[0].z.min(triangle[1].z).min(triangle[2].z) - SAMPLE_EPSILON_M;
    let max_z = triangle[0].z.max(triangle[1].z).max(triangle[2].z) + SAMPLE_EPSILON_M;
    if point.x < min_x || point.x > max_x || point.y < min_z || point.y > max_z {
        return None;
    }

    let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
        - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
    if area.abs() <= SAMPLE_EPSILON_M {
        return None;
    }

    let numerator_w0 = (triangle[1].x - point.x) * (triangle[2].z - point.y)
        - (triangle[1].z - point.y) * (triangle[2].x - point.x);
    let numerator_w1 = (triangle[2].x - point.x) * (triangle[0].z - point.y)
        - (triangle[2].z - point.y) * (triangle[0].x - point.x);
    let numerator_w2 = area - numerator_w0 - numerator_w1;
    let outside = if area > 0.0 {
        let tolerance = -SAMPLE_EPSILON_M * area;
        numerator_w0 < tolerance || numerator_w1 < tolerance || numerator_w2 < tolerance
    } else {
        let tolerance = -SAMPLE_EPSILON_M * area;
        numerator_w0 > tolerance || numerator_w1 > tolerance || numerator_w2 > tolerance
    };
    if outside {
        return None;
    }
    let inverse_area = area.recip();
    let w0 = numerator_w0 * inverse_area;
    let w1 = numerator_w1 * inverse_area;
    let w2 = numerator_w2 * inverse_area;
    Some((w0, w1, w2))
}

fn road_ray_triangle_intersection_t(
    triangle: [RoadVec3; 3],
    ray_origin: RoadVec3,
    ray_dir: RoadVec3,
) -> Option<f64> {
    let edge_ab = triangle[1] - triangle[0];
    let edge_ac = triangle[2] - triangle[0];
    let pvec = ray_dir.cross(edge_ac);
    let det = edge_ab.dot(pvec);
    if det.abs() <= SAMPLE_EPSILON_M {
        return None;
    }

    let inv_det = 1.0 / det;
    let tvec = ray_origin - triangle[0];
    let u = tvec.dot(pvec) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }

    let qvec = tvec.cross(edge_ab);
    let v = ray_dir.dot(qvec) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }

    let t = edge_ac.dot(qvec) * inv_det;
    (t >= 0.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::network::build_surface_edge;
    use crate::simulation::network::types::NodeType;

    fn scanned_height(
        surface: &RoadSurfaceSystem,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        point: RoadVec2,
    ) -> Option<f32> {
        let chunk = RoadSurfaceSystem::query_chunk_coords_for_world(point.x, point.y);
        let edges = surface
            .query_chunk_spans
            .get(&chunk)
            .into_iter()
            .flatten()
            .copied();
        let nodes = surface
            .query_chunk_nodes
            .get(&chunk)
            .into_iter()
            .flatten()
            .copied();
        let mut result = None;
        surface.visit_visible_top_surface_query_triangles(
            graph,
            terrain,
            edges.clone(),
            nodes.clone(),
            &mut |triangle| {
                if let Some(height) = RoadSurfaceSystem::triangle_height_at_xz(triangle, point) {
                    keep_max_height(&mut result, height);
                }
            },
        );
        if result.is_none() {
            surface.visit_visible_earthwork_query_triangles(
                graph,
                terrain,
                edges,
                nodes,
                &mut |triangle| {
                    if let Some(height) = RoadSurfaceSystem::triangle_height_at_xz(triangle, point)
                    {
                        keep_max_height(&mut result, height);
                    }
                },
            );
        }
        result
    }

    #[test]
    fn indexed_visible_height_matches_scan_through_surface_edits() {
        let terrain = TerrainSystem::with_chunking(129, 129, 1.0, 16, 0.0);
        for class in [EdgeClass::Standard, EdgeClass::Bridge, EdgeClass::Tunnel] {
            let mut graph = RegionGraph::new();
            let height = if class == EdgeClass::Bridge { 6.0 } else { 0.0 };
            let center = graph.add_node(Vector3::new(0.0, height, 0.0), NodeType::Junction);
            for end in [
                Vector3::new(-48.0, height, 0.0),
                Vector3::new(48.0, height, 0.0),
                Vector3::new(0.0, height, 48.0),
            ] {
                let node = graph.add_node(end, NodeType::Junction);
                let middle_y = if class == EdgeClass::Tunnel {
                    -6.0
                } else {
                    height
                };
                let middle = Vector3::new(end.x * 0.5, middle_y, end.z * 0.5);
                graph.add_edge(build_surface_edge(
                    center,
                    node,
                    vec![graph.node(center).pos, middle, end],
                    1,
                    1,
                    class,
                ));
            }
            graph.rebuild_adjacency_list();
            graph.rebuild_intersection_clips();
            let mut surface = RoadSurfaceSystem::new(16.0);
            for edit in 0..3 {
                if edit == 1 {
                    graph.edge_mut(0).fwd_lanes = 2;
                    graph.edge_mut(0).width += crate::config::LANE_WIDTH;
                    surface.mark_edge_dirty(&graph, 0);
                } else if edit == 2 {
                    graph.edge_mut(0).deleted = true;
                    surface.mark_edge_dirty(&graph, 0);
                    graph.rebuild_adjacency_list();
                    graph.rebuild_intersection_clips();
                }
                assert!(surface.compile_dirty(&graph, &terrain));
                // Includes outside-road misses, chunk/grid boundaries and ±0.5 mm seam probes.
                for x in (-52..=52).step_by(2) {
                    for z in (-16..=52).step_by(2) {
                        for delta in [-0.0005_f32, 0.0, 0.0005] {
                            let (x, z) = (x as f32 + delta, z as f32 - delta);
                            let expected = scanned_height(
                                &surface,
                                &graph,
                                &terrain,
                                RoadVec2::new(x as f64, z as f64),
                            );
                            assert_eq!(
                                surface.sample_visible_surface_height(&graph, &terrain, x, z),
                                expected,
                                "{class:?} edit={edit} point=({x},{z})"
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn visible_index_keeps_highest_renderable_triangle_and_padded_cells() {
        use super::super::super::RoadSurfaceVisualPolygon;
        let triangle = [
            RoadVec3::new(0.0, 1.0, 0.0),
            RoadVec3::new(8.0, 1.0, 0.0),
            RoadVec3::new(0.0, 1.0, 8.0),
        ];
        let upper = triangle.map(|point| RoadVec3::new(point.x, 3.0, point.z));
        // Its area passes the sampler but its altitude fails the visible-render policy.
        let thin = [
            RoadVec3::new(0.0, 9.0, 0.0),
            RoadVec3::new(100.0, 9.0, 0.0),
            RoadVec3::new(0.0, 9.0, 0.001),
        ];
        let polygon = RoadSurfaceVisualPolygon::from_parts(Vec::new(), vec![triangle, upper, thin]);
        let index = RoadSurfaceTriangleQueryIndex::from_surface_polygons(&[polygon], &[], &[]);
        for point in [
            RoadVec2::new(0.0, 0.0),
            RoadVec2::new(4.0, -0.0005),
            RoadVec2::new(4.0, 0.0005),
            RoadVec2::new(4.0, 4.0),
        ] {
            assert_eq!(index.sample_visible_height(point), Some(3.0));
        }
        assert_eq!(index.sample_visible_height(RoadVec2::new(40.0, 0.0)), None);
    }
}
