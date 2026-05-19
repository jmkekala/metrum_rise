//! Visible-surface sampling, raycast, and section-range queries.

use super::super::{RoadSurfaceSection, RoadSurfaceSystem, SurfaceChunkKey};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;
use godot::prelude::{Vector2, Vector3};

impl RoadSurfaceSystem {
    pub(crate) fn sample_visible_surface_height(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        world_x: f32,
        world_z: f32,
    ) -> Option<f32> {
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
        let mut top_surface_height_m: Option<f32> = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    top_surface_height_m =
                        Some(top_surface_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                top_surface_height_m =
                    Some(top_surface_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        if top_surface_height_m.is_some() {
            return top_surface_height_m;
        }

        let mut earthwork_height_m: Option<f32> = None;
        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                earthwork_height_m =
                    Some(earthwork_height_m.map_or(height_m, |best| best.max(height_m)));
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc;
                    earthwork_height_m =
                        Some(earthwork_height_m.map_or(height_m, |best| best.max(height_m)));
                },
            );
        }

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
        let chunk = self.chunk_coords_for_world(world_x, world_z);
        let (edge_indices, node_ids) = self.collect_query_contributors(chunk, chunk);
        let point = Vector2::new(world_x, world_z);
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
                    let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                    else {
                        return;
                    };
                    let height_m = triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc
                        - height_offset_m;
                    best_height_m = Some(best_height_m.map_or(height_m, |best| best.min(height_m)));
                });
            }
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            let height_offset_m = self.span_piece_integrated_surface_offset_m(piece);
            self.visit_span_piece_clearance_triangles(piece, &mut |triangle| {
                let Some((wa, wb, wc)) = Self::triangle_barycentric_weights_xz(triangle, point)
                else {
                    return;
                };
                let height_m =
                    triangle[0].y * wa + triangle[1].y * wb + triangle[2].y * wc - height_offset_m;
                best_height_m = Some(best_height_m.map_or(height_m, |best| best.min(height_m)));
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

        let terrain_hit = terrain.raycast_visual_terrain(ray_origin, ray_dir);
        let terrain_t = terrain_hit.map(|terrain_hit| {
            (terrain_hit - ray_origin).dot(ray_dir) / ray_dir.length_squared().max(f32::EPSILON)
        });
        let Some((min_chunk, max_chunk)) =
            self.raycast_visible_query_chunk_bounds(terrain, ray_origin, ray_dir, terrain_hit)
        else {
            return terrain_hit;
        };
        let (edge_indices, node_ids) = self.collect_query_contributors(min_chunk, max_chunk);

        let mut best_t = terrain_t.unwrap_or(f32::INFINITY);
        let mut best_hit = None;

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            self.visit_visible_node_piece_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            self.visit_visible_span_piece_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &edge_idx in &edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if !self.span_piece_uses_visible_earthwork(piece) {
                continue;
            }
            self.visit_span_piece_earthwork_triangles(piece, &mut |triangle| {
                let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                else {
                    return;
                };
                if t >= 0.0 && t <= best_t {
                    best_t = t;
                    best_hit = Some(ray_origin + ray_dir * t);
                }
            });
        }

        for &node_id in &node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_piece_uses_visible_earthwork(graph, node_id, terrain) {
                continue;
            }
            self.visit_node_piece_earthwork_triangles(
                graph,
                terrain,
                node_id,
                piece,
                &mut |triangle| {
                    let Some(t) = Self::ray_triangle_intersection_t(triangle, ray_origin, ray_dir)
                    else {
                        return;
                    };
                    if t >= 0.0 && t <= best_t {
                        best_t = t;
                        best_hit = Some(ray_origin + ray_dir * t);
                    }
                },
            );
        }

        best_hit.or(terrain_hit)
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

    fn raycast_visible_query_chunk_bounds(
        &self,
        terrain: &TerrainSystem,
        ray_origin: Vector3,
        ray_dir: Vector3,
        terrain_hit: Option<Vector3>,
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
    ray_origin: Vector3,
    ray_dir: Vector3,
    min_x: f32,
    min_z: f32,
    max_x: f32,
    max_z: f32,
) -> Option<(f32, f32)> {
    let mut entry_t = f32::NEG_INFINITY;
    let mut exit_t = f32::INFINITY;
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
    origin: f32,
    direction: f32,
    min: f32,
    max: f32,
    entry_t: &mut f32,
    exit_t: &mut f32,
) -> Option<()> {
    if direction.abs() <= f32::EPSILON {
        return (origin >= min && origin <= max).then_some(());
    }

    let t0 = (min - origin) / direction;
    let t1 = (max - origin) / direction;
    *entry_t = entry_t.max(t0.min(t1));
    *exit_t = exit_t.min(t0.max(t1));
    Some(())
}
