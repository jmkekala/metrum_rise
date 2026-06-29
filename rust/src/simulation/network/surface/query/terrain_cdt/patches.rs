//! Terrain render patch discovery and terrain-clip loop collection.

use super::*;

impl RoadSurfaceSystem {
    /// Returns render-patch keys covered by visible grounded road plus a seam-safe margin.
    #[cfg(test)]
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
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                let Some((min_x, min_z, max_x, max_z)) =
                    Self::terrain_clip_boundary_loop_bounds_xz(boundary_loop)
                else {
                    continue;
                };
                for key in terrain.render_patch_keys_for_world_bounds(
                    (min_x - margin_m) as f32,
                    (min_z - margin_m) as f32,
                    (max_x + margin_m) as f32,
                    (max_z + margin_m) as f32,
                ) {
                    patch_keys.insert(key);
                }
            }
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                let Some((min_x, min_z, max_x, max_z)) =
                    Self::terrain_clip_boundary_loop_bounds_xz(boundary_loop)
                else {
                    continue;
                };
                for key in terrain.render_patch_keys_for_world_bounds(
                    (min_x - margin_m) as f32,
                    (min_z - margin_m) as f32,
                    (max_x + margin_m) as f32,
                    (max_z + margin_m) as f32,
                ) {
                    patch_keys.insert(key);
                }
            }
        }

        let mut keys: Vec<(usize, usize)> = patch_keys.into_iter().collect();
        keys.sort_unstable();
        keys
    }

    /// Returns road-locked render patches with the largest grading margin needed per patch.
    pub(crate) fn terrain_render_patch_grading_margins_for_visible_roads(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        render_step_m: f32,
    ) -> BTreeMap<(usize, usize), f32> {
        let base_margin_m = terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let mut patch_margins = BTreeMap::new();

        let mut span_pieces = self.compiled_visual_span_pieces.iter().collect::<Vec<_>>();
        span_pieces.sort_by_key(|(edge_idx, _)| **edge_idx);
        for (_, piece) in span_pieces {
            if piece.edge_class != EdgeClass::Standard {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                Self::insert_terrain_patch_grading_margins_for_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    &mut patch_margins,
                );
            }
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_standard_surface_edges(graph, node_id) {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                Self::insert_terrain_patch_grading_margins_for_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    &mut patch_margins,
                );
            }
        }

        patch_margins
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

    fn insert_terrain_patch_grading_margins_for_loop(
        terrain: &TerrainSystem,
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        render_step_m: f32,
        base_margin_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
    ) {
        let vertices = boundary_loop
            .points_world
            .iter()
            .map(|point| TerrainCdtVertex::new(point.x, point.y as f32, point.z))
            .collect::<Vec<_>>();
        if vertices.is_empty() {
            return;
        }

        let safe_step_m = render_step_m.max(f32::EPSILON);
        let base_margin_m = base_margin_m.max(0.0);
        Self::insert_terrain_patch_grading_margins_for_bounds(
            terrain,
            vertices.iter().map(|vertex| (vertex.x, vertex.z)),
            base_margin_m,
            patch_margins,
        );

        let uses_clean_grounded_tie_in = vertices.len() >= 3
            && Self::terrain_cdt_vertices_signed_area_xz(&vertices).abs() > f64::EPSILON;
        if !uses_clean_grounded_tie_in {
            let fallback_margin_m = Self::terrain_cdt_required_grading_margin_for_clip_loop(
                terrain,
                boundary_loop,
                safe_step_m,
                base_margin_m,
            );
            Self::insert_terrain_patch_grading_margins_for_bounds(
                terrain,
                vertices.iter().map(|vertex| (vertex.x, vertex.z)),
                fallback_margin_m,
                patch_margins,
            );
            return;
        }

        let loop_is_ccw = Self::terrain_cdt_vertices_signed_area_xz(&vertices) > 0.0;
        let mut edge_outward_directions = Vec::with_capacity(vertices.len());
        for index in 0..vertices.len() {
            let start = vertices[index];
            let end = vertices[(index + 1) % vertices.len()];
            let dx = end.x - start.x;
            let dz = end.z - start.z;
            let length_m = dx.hypot(dz);
            if length_m <= f64::EPSILON {
                continue;
            }
            let outward_x = if loop_is_ccw { dz } else { -dz } / length_m;
            let outward_z = if loop_is_ccw { -dx } else { dx } / length_m;
            edge_outward_directions.push((outward_x, outward_z));
            let sample_count = ((length_m as f32 / safe_step_m).ceil() as u32).max(1);
            for sample_index in 0..=sample_count {
                let t = f64::from(sample_index) / f64::from(sample_count);
                let seam_x = start.x + dx * t;
                let seam_z = start.z + dz * t;
                let seam_height_m = start.height_m + (end.height_m - start.height_m) * t as f32;
                let margin_m = Self::terrain_cdt_required_grading_margin_for_ray(
                    terrain,
                    seam_x,
                    seam_z,
                    seam_height_m,
                    outward_x,
                    outward_z,
                    safe_step_m,
                    base_margin_m,
                );
                Self::insert_terrain_patch_grading_margins_for_ray(
                    terrain,
                    seam_x,
                    seam_z,
                    outward_x,
                    outward_z,
                    margin_m,
                    safe_step_m,
                    patch_margins,
                );
            }
        }

        if edge_outward_directions.len() != vertices.len() {
            return;
        }
        for index in 0..vertices.len() {
            let (previous_outward_x, previous_outward_z) =
                edge_outward_directions[(index + vertices.len() - 1) % vertices.len()];
            let (next_outward_x, next_outward_z) = edge_outward_directions[index];
            let bisector_x = previous_outward_x + next_outward_x;
            let bisector_z = previous_outward_z + next_outward_z;
            let bisector_length_m = bisector_x.hypot(bisector_z);
            if bisector_length_m <= f64::EPSILON {
                continue;
            }
            let vertex = vertices[index];
            let margin_m = Self::terrain_cdt_required_grading_margin_for_ray(
                terrain,
                vertex.x,
                vertex.z,
                vertex.height_m,
                bisector_x / bisector_length_m,
                bisector_z / bisector_length_m,
                safe_step_m,
                base_margin_m,
            );
            Self::insert_terrain_patch_grading_margins_for_ray(
                terrain,
                vertex.x,
                vertex.z,
                bisector_x / bisector_length_m,
                bisector_z / bisector_length_m,
                margin_m,
                safe_step_m,
                patch_margins,
            );
        }
    }

    fn insert_terrain_patch_grading_margins_for_ray(
        terrain: &TerrainSystem,
        seam_x: f64,
        seam_z: f64,
        direction_x: f64,
        direction_z: f64,
        margin_m: f32,
        render_step_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
    ) {
        let margin_m = margin_m.max(0.0);
        let pad_m = render_step_m.max(terrain.cell_size_m());
        let end_x = seam_x + direction_x * f64::from(margin_m);
        let end_z = seam_z + direction_z * f64::from(margin_m);
        Self::insert_terrain_patch_grading_margins_for_rect(
            terrain,
            seam_x.min(end_x) as f32 - pad_m,
            seam_z.min(end_z) as f32 - pad_m,
            seam_x.max(end_x) as f32 + pad_m,
            seam_z.max(end_z) as f32 + pad_m,
            margin_m,
            patch_margins,
        );
    }

    fn insert_terrain_patch_grading_margins_for_bounds(
        terrain: &TerrainSystem,
        points: impl IntoIterator<Item = (f64, f64)>,
        margin_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
    ) {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for (x, z) in points {
            min_x = min_x.min(x);
            min_z = min_z.min(z);
            max_x = max_x.max(x);
            max_z = max_z.max(z);
        }
        if !min_x.is_finite() {
            return;
        }
        let margin_m = margin_m.max(0.0);
        Self::insert_terrain_patch_grading_margins_for_rect(
            terrain,
            min_x as f32 - margin_m,
            min_z as f32 - margin_m,
            max_x as f32 + margin_m,
            max_z as f32 + margin_m,
            margin_m,
            patch_margins,
        );
    }

    fn insert_terrain_patch_grading_margins_for_rect(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        margin_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
    ) {
        for key in terrain.render_patch_keys_for_world_bounds(
            min_x.min(max_x),
            min_z.min(max_z),
            min_x.max(max_x),
            min_z.max(max_z),
        ) {
            patch_margins
                .entry(key)
                .and_modify(|existing| *existing = existing.max(margin_m))
                .or_insert(margin_m);
        }
    }

    #[cfg(test)]
    fn terrain_clip_boundary_loop_bounds_xz(
        boundary_loop: &RoadSurfaceTerrainClipLoop,
    ) -> Option<(f64, f64, f64, f64)> {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for point in &boundary_loop.points_world {
            min_x = min_x.min(point.x);
            min_z = min_z.min(point.z);
            max_x = max_x.max(point.x);
            max_z = max_z.max(point.z);
        }
        min_x.is_finite().then_some((min_x, min_z, max_x, max_z))
    }
}
