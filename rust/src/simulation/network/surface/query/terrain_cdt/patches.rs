//! Terrain render patch discovery and terrain-clip loop collection.

use super::*;

impl RoadSurfaceSystem {
    /// Returns render-patch keys covered by visible grounded road plus a seam-safe margin.
    pub(crate) fn terrain_render_patch_keys_with_visible_road_margin(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        margin_m: f32,
    ) -> Vec<(usize, usize)> {
        let mut patch_keys = HashSet::new();
        let margin_m = f64::from(margin_m.max(0.0));

        let mut span_pieces = self.compiled_visual_span_pieces.iter().collect::<Vec<_>>();
        span_pieces.sort_by_key(|(edge_idx, _)| **edge_idx);
        for (_, piece) in span_pieces {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            let Some((min, max)) = self.visual_span_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(
                (min.x - margin_m) as f32,
                (min.z - margin_m) as f32,
                (max.x + margin_m) as f32,
                (max.z + margin_m) as f32,
            ) {
                patch_keys.insert(key);
            }
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            let Some((min, max)) = self.visual_node_piece_bounds(piece, ChunkCacheKind::Surface)
            else {
                continue;
            };
            for key in terrain.render_patch_keys_for_world_bounds(
                (min.x - margin_m) as f32,
                (min.z - margin_m) as f32,
                (max.x + margin_m) as f32,
                (max.z + margin_m) as f32,
            ) {
                patch_keys.insert(key);
            }
        }

        let mut keys: Vec<(usize, usize)> = patch_keys.into_iter().collect();
        keys.sort_unstable();
        keys
    }

    pub(super) fn terrain_clip_boundary_loops_for_world_bounds(
        &self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<RoadSurfaceTerrainClipLoop> {
        let mut boundary_loops = Vec::new();

        let mut span_pieces = self.compiled_visual_span_pieces.iter().collect::<Vec<_>>();
        span_pieces.sort_by_key(|(edge_idx, _)| **edge_idx);
        for (_, piece) in span_pieces {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            Self::collect_terrain_clip_boundary_loops_from_piece(
                &piece.terrain_clip_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut boundary_loops,
            );
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            Self::collect_terrain_clip_boundary_loops_from_piece(
                &piece.terrain_clip_boundary_loops,
                min_x,
                min_z,
                max_x,
                max_z,
                &mut boundary_loops,
            );
        }

        boundary_loops
    }

    fn collect_terrain_clip_boundary_loops_from_piece(
        source: &[RoadSurfaceTerrainClipLoop],
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<RoadSurfaceTerrainClipLoop>,
    ) {
        for boundary_loop in source {
            if Self::visual_points_overlap_bounds_xz(
                boundary_loop.points_world.iter().copied(),
                min_x,
                min_z,
                max_x,
                max_z,
            ) {
                out.push(boundary_loop.clone());
            }
        }
    }

    fn visual_points_overlap_bounds_xz(
        points_world: impl IntoIterator<Item = RoadVec3>,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> bool {
        let min_x = f64::from(min_x);
        let min_z = f64::from(min_z);
        let max_x = f64::from(max_x);
        let max_z = f64::from(max_z);
        let mut polygon_min_x = f64::MAX;
        let mut polygon_max_x = f64::MIN;
        let mut polygon_min_z = f64::MAX;
        let mut polygon_max_z = f64::MIN;
        for point in points_world {
            polygon_min_x = polygon_min_x.min(point.x);
            polygon_max_x = polygon_max_x.max(point.x);
            polygon_min_z = polygon_min_z.min(point.z);
            polygon_max_z = polygon_max_z.max(point.z);
        }

        polygon_min_x <= max_x
            && polygon_max_x >= min_x
            && polygon_min_z <= max_z
            && polygon_max_z >= min_z
    }
}
