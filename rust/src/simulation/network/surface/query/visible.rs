//! Visible-surface sampling, raycast, and section-range queries.

use super::super::{RoadSurfaceSection, RoadSurfaceSystem, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::surface::backend::{RoadVec2, RoadVec3, godot_vec3_to_road};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::Vector3;

const SAMPLE_EPSILON_M: f64 = 0.001;

impl RoadSurfaceSystem {
    pub(crate) fn sample_visible_surface_height(
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
        let mut top_surface_height_m: Option<f32> = None;

        self.visit_visible_top_surface_query_triangles(
            graph,
            terrain,
            &edge_indices,
            &node_ids,
            &mut |triangle| {
                if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                    keep_max_height(&mut top_surface_height_m, height_m);
                }
            },
        );

        if top_surface_height_m.is_some() {
            return top_surface_height_m;
        }

        let mut earthwork_height_m: Option<f32> = None;
        self.visit_visible_earthwork_query_triangles(
            graph,
            terrain,
            &edge_indices,
            &node_ids,
            &mut |triangle| {
                if let Some(height_m) = Self::triangle_height_at_xz(triangle, point) {
                    keep_max_height(&mut earthwork_height_m, height_m);
                }
            },
        );

        earthwork_height_m
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

    pub(crate) fn raycast_visible_surface(
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
        let terrain_hit = terrain.raycast_visual_terrain(ray_origin, ray_dir);
        let terrain_hit_road = terrain_hit.map(godot_vec3_to_road);
        let terrain_t = terrain_hit_road.map(|terrain_hit| {
            (terrain_hit - ray_origin_road).dot(ray_dir_road)
                / ray_dir_road.length_squared().max(f64::EPSILON)
        });
        let Some((min_chunk, max_chunk)) = self.raycast_visible_query_chunk_bounds(
            terrain,
            ray_origin_road,
            ray_dir_road,
            terrain_hit_road,
        ) else {
            return terrain_hit;
        };
        let (edge_indices, node_ids) = self.collect_query_contributors(min_chunk, max_chunk);

        let mut best_t = match terrain_t {
            Some(t) => t,
            None => f64::INFINITY,
        };
        let mut best_hit = None;

        self.visit_visible_top_surface_query_triangles(
            graph,
            terrain,
            &edge_indices,
            &node_ids,
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
            &edge_indices,
            &node_ids,
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

        best_hit
            .map(|hit| Vector3::new(hit.x as f32, hit.y as f32, hit.z as f32))
            .or(terrain_hit)
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
        let Some((start_handoff, end_handoff)) = self.visual_surface_handoff_range_for_edge(
            graph,
            edge_idx,
            edge,
            total_length,
            start_kind,
            end_kind,
        ) else {
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

fn road_triangle_barycentric_weights_xz(
    triangle: [RoadVec3; 3],
    point: RoadVec2,
) -> Option<(f64, f64, f64)> {
    let area = (triangle[1].x - triangle[0].x) * (triangle[2].z - triangle[0].z)
        - (triangle[1].z - triangle[0].z) * (triangle[2].x - triangle[0].x);
    if area.abs() <= SAMPLE_EPSILON_M {
        return None;
    }

    let w0 = ((triangle[1].x - point.x) * (triangle[2].z - point.y)
        - (triangle[1].z - point.y) * (triangle[2].x - point.x))
        / area;
    let w1 = ((triangle[2].x - point.x) * (triangle[0].z - point.y)
        - (triangle[2].z - point.y) * (triangle[0].x - point.x))
        / area;
    let w2 = 1.0 - w0 - w1;
    if w0 < -SAMPLE_EPSILON_M || w1 < -SAMPLE_EPSILON_M || w2 < -SAMPLE_EPSILON_M {
        return None;
    }
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
