// SPDX-License-Identifier: GPL-2.0-only

//! Terrain render patch discovery and terrain-clip loop collection.

use super::*;

const TERRAIN_CDT_MIN_PATCH_OVERLAP_M: f32 = 0.001;

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
            if piece.terrain_clip_boundary_loops.is_empty() {
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
            if !self.node_has_terrain_clip_surface_edges(graph, node_id) {
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
    #[cfg(test)]
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
            if piece.terrain_clip_boundary_loops.is_empty() {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                let _ = Self::insert_terrain_patch_grading_margins_for_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    &mut patch_margins,
                    None,
                );
            }
        }

        let mut node_pieces = self.compiled_visual_node_pieces.iter().collect::<Vec<_>>();
        node_pieces.sort_by_key(|(node_id, _)| **node_id);
        for (&node_id, piece) in node_pieces {
            if !self.node_has_terrain_clip_surface_edges(graph, node_id) {
                continue;
            }
            for boundary_loop in &piece.terrain_clip_boundary_loops {
                let _ = Self::insert_terrain_patch_grading_margins_for_loop(
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    &mut patch_margins,
                    None,
                );
            }
        }

        patch_margins
    }

    /// Resolves road grading ownership only for changed terrain patches.
    pub(crate) fn terrain_render_patch_grading_margins_for_patches(
        &self,
        graph: &RegionGraph,
        terrain: &TerrainSystem,
        render_step_m: f32,
        patch_keys: &[(usize, usize)],
    ) -> BTreeMap<(usize, usize), f32> {
        let base_margin_m = terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let query_margin_m = EARTHWORK_MAX_MARGIN_M + base_margin_m;
        let target_patches = patch_keys
            .iter()
            .filter_map(|&key| {
                terrain
                    .render_patch_world_bounds(key.0, key.1)
                    .map(|bounds| (key, bounds))
            })
            .collect::<Vec<_>>();
        if target_patches.is_empty() {
            return BTreeMap::new();
        }

        // A dirty render-patch neighborhood overlaps the same short road loops repeatedly.
        // Resolve contributors for the batch, then grade each loop once and distribute its
        // exact result to every target patch. This keeps terrain sampling proportional to the
        // changed road geometry instead of multiplying it by the number of dirty patches.
        let mut edge_indices = BTreeSet::new();
        let mut node_ids = BTreeSet::new();
        for &(_, (min_x, min_z, max_x, max_z)) in &target_patches {
            let (patch_edges, patch_nodes) = self.collect_spatial_query_contributors_for_bounds(
                f64::from(min_x - query_margin_m),
                f64::from(min_z - query_margin_m),
                f64::from(max_x + query_margin_m),
                f64::from(max_z + query_margin_m),
            );
            edge_indices.extend(patch_edges);
            node_ids.extend(patch_nodes);
        }

        let mut patch_margins = BTreeMap::new();
        let edge_count = edge_indices.len();
        let node_count = node_ids.len();
        let mut evaluated_loop_count = 0usize;
        let mut cached_loop_count = 0usize;
        let grading_cache = Arc::clone(&self.terrain_grading_cache);
        let mut grading_cache = grading_cache
            .lock()
            .expect("road terrain grading cache lock poisoned");
        for edge_idx in edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            for (loop_index, boundary_loop) in piece.terrain_clip_boundary_loops.iter().enumerate()
            {
                if let Some(cached) = Self::insert_cached_terrain_patch_grading_margins_for_targets(
                    &mut grading_cache.span_loops,
                    edge_idx,
                    loop_index,
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    query_margin_m,
                    &target_patches,
                    &mut patch_margins,
                ) {
                    evaluated_loop_count += 1;
                    cached_loop_count += usize::from(cached);
                }
            }
        }
        for node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_has_terrain_clip_surface_edges(graph, node_id) {
                continue;
            }
            for (loop_index, boundary_loop) in piece.terrain_clip_boundary_loops.iter().enumerate()
            {
                if let Some(cached) = Self::insert_cached_terrain_patch_grading_margins_for_targets(
                    &mut grading_cache.node_loops,
                    node_id,
                    loop_index,
                    terrain,
                    boundary_loop,
                    render_step_m,
                    base_margin_m,
                    query_margin_m,
                    &target_patches,
                    &mut patch_margins,
                ) {
                    evaluated_loop_count += 1;
                    cached_loop_count += usize::from(cached);
                }
            }
        }
        if crate::debug::category_enabled("road") && crate::debug::is_perf_enabled() {
            crate::debug_log!(
                "road",
                "terrain_grading_batch target_patches={} span_owners={} node_owners={} evaluated_loops={} cached_loops={} output_patches={}",
                target_patches.len(),
                edge_count,
                node_count,
                evaluated_loop_count,
                cached_loop_count,
                patch_margins.len()
            );
        }

        patch_margins
    }

    fn insert_cached_terrain_patch_grading_margins_for_targets<
        Owner: Eq + std::hash::Hash + Copy,
    >(
        owner_cache: &mut HashMap<Owner, Vec<Option<RoadSurfaceTerrainLoopGradingCacheEntry>>>,
        owner: Owner,
        loop_index: usize,
        terrain: &TerrainSystem,
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        render_step_m: f32,
        base_margin_m: f32,
        query_margin_m: f32,
        target_patches: &[((usize, usize), (f32, f32, f32, f32))],
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
    ) -> Option<bool> {
        let Some((loop_min_x, loop_min_z, loop_max_x, loop_max_z)) =
            Self::terrain_clip_boundary_loop_bounds_xz(boundary_loop)
        else {
            return None;
        };
        if !target_patches.iter().any(
            |&(_, (patch_min_x, patch_min_z, patch_max_x, patch_max_z))| {
                loop_min_x <= f64::from(patch_max_x + query_margin_m)
                    && loop_max_x >= f64::from(patch_min_x - query_margin_m)
                    && loop_min_z <= f64::from(patch_max_z + query_margin_m)
                    && loop_max_z >= f64::from(patch_min_z - query_margin_m)
            },
        ) {
            return None;
        }

        let owner_loops = owner_cache.entry(owner).or_default();
        if owner_loops.len() <= loop_index {
            owner_loops.resize_with(loop_index + 1, || None);
        }
        let cache_matches = owner_loops[loop_index].as_ref().is_some_and(|cached| {
            cached.terrain_source_generation == terrain.source_generation()
                && cached.render_step_bits == render_step_m.to_bits()
                && cached.points_world.as_slice() == boundary_loop.points_world
        });
        if !cache_matches {
            let mut loop_margins = BTreeMap::new();
            let (influence_bounds, _) = Self::insert_terrain_patch_grading_margins_for_loop(
                terrain,
                boundary_loop,
                render_step_m,
                base_margin_m,
                &mut loop_margins,
                None,
            );
            owner_loops[loop_index] = Some(RoadSurfaceTerrainLoopGradingCacheEntry {
                terrain_source_generation: terrain.source_generation(),
                render_step_bits: render_step_m.to_bits(),
                points_world: Arc::new(boundary_loop.points_world.clone()),
                influence_bounds,
                patch_margins: Arc::new(loop_margins),
            });
        }
        let cached = owner_loops[loop_index]
            .as_ref()
            .expect("terrain grading cache entry must exist after fill");
        for &(key, target_bounds) in target_patches {
            let Some(&target_margin_m) = cached.patch_margins.get(&key) else {
                continue;
            };
            let Some(target_margin_m) = Self::terrain_cdt_exact_target_patch_margin(
                cached.influence_bounds,
                target_bounds,
                Some(target_margin_m),
            ) else {
                continue;
            };
            patch_margins
                .entry(key)
                .and_modify(|current| *current = current.max(target_margin_m))
                .or_insert(target_margin_m);
        }
        Some(cache_matches)
    }

    fn terrain_cdt_exact_target_patch_margin(
        influence_bounds: Option<(f32, f32, f32, f32)>,
        target_bounds: (f32, f32, f32, f32),
        target_margin_m: Option<f32>,
    ) -> Option<f32> {
        let (influence_min_x, influence_min_z, influence_max_x, influence_max_z) =
            influence_bounds?;
        let (target_min_x, target_min_z, target_max_x, target_max_z) = target_bounds;
        let has_exact_influence = influence_max_x.min(target_max_x)
            - influence_min_x.max(target_min_x)
            > TERRAIN_CDT_MIN_PATCH_OVERLAP_M
            && influence_max_z.min(target_max_z) - influence_min_z.max(target_min_z)
                > TERRAIN_CDT_MIN_PATCH_OVERLAP_M;
        has_exact_influence.then_some(target_margin_m).flatten()
    }

    pub(super) fn terrain_clip_boundary_loops_for_world_bounds<'a>(
        &'a self,
        graph: &RegionGraph,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> Vec<&'a RoadSurfaceTerrainClipLoop> {
        let mut boundary_loops = Vec::new();
        let (edge_indices, node_ids) = self.collect_spatial_query_contributors_for_bounds(
            f64::from(min_x),
            f64::from(min_z),
            f64::from(max_x),
            f64::from(max_z),
        );

        for edge_idx in edge_indices {
            let Some(piece) = self.compiled_visual_span_pieces.get(&edge_idx) else {
                continue;
            };
            if piece.terrain_clip_boundary_loops.is_empty() {
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

        for node_id in node_ids {
            let Some(piece) = self.compiled_visual_node_pieces.get(&node_id) else {
                continue;
            };
            if !self.node_has_terrain_clip_surface_edges(graph, node_id) {
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

    fn collect_terrain_clip_boundary_loops_from_piece<'a>(
        source: &'a [RoadSurfaceTerrainClipLoop],
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        out: &mut Vec<&'a RoadSurfaceTerrainClipLoop>,
    ) {
        for boundary_loop in source {
            if Self::visual_points_overlap_bounds_xz(
                boundary_loop.points_world.iter().copied(),
                min_x,
                min_z,
                max_x,
                max_z,
            ) {
                out.push(boundary_loop);
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
        target_patch: Option<(usize, usize)>,
    ) -> (Option<(f32, f32, f32, f32)>, Option<f32>) {
        let vertices = boundary_loop
            .points_world
            .iter()
            .map(|point| TerrainCdtVertex::new(point.x, point.y as f32, point.z))
            .collect::<Vec<_>>();
        if vertices.is_empty() {
            return (None, None);
        }

        let safe_step_m = render_step_m.max(f32::EPSILON);
        let base_margin_m = base_margin_m.max(0.0);
        let mut required_margin_m = base_margin_m;
        let mut target_margin_m = Self::insert_terrain_patch_grading_margins_for_bounds(
            terrain,
            vertices.iter().map(|vertex| (vertex.x, vertex.z)),
            base_margin_m,
            patch_margins,
            target_patch,
        )
        .then_some(base_margin_m);

        let uses_clean_grounded_tie_in = vertices.len() >= 3
            && Self::terrain_cdt_vertices_signed_area_xz(&vertices).abs() > f64::EPSILON;
        if !uses_clean_grounded_tie_in {
            let fallback_margin_m = Self::terrain_cdt_required_grading_margin_for_clip_loop(
                terrain,
                boundary_loop,
                safe_step_m,
                base_margin_m,
            );
            if Self::insert_terrain_patch_grading_margins_for_bounds(
                terrain,
                vertices.iter().map(|vertex| (vertex.x, vertex.z)),
                fallback_margin_m,
                patch_margins,
                target_patch,
            ) {
                target_margin_m = Some(
                    target_margin_m
                        .map_or(fallback_margin_m, |current| current.max(fallback_margin_m)),
                );
            }
            return (
                Self::terrain_cdt_grading_influence_bounds(&vertices, fallback_margin_m),
                target_margin_m,
            );
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
                required_margin_m = required_margin_m.max(margin_m);
                if Self::insert_terrain_patch_grading_margins_for_ray(
                    terrain,
                    seam_x,
                    seam_z,
                    outward_x,
                    outward_z,
                    margin_m,
                    safe_step_m,
                    patch_margins,
                    target_patch,
                ) {
                    target_margin_m =
                        Some(target_margin_m.map_or(margin_m, |current| current.max(margin_m)));
                }
            }
        }

        if edge_outward_directions.len() != vertices.len() {
            return (
                Self::terrain_cdt_grading_influence_bounds(&vertices, required_margin_m),
                target_margin_m,
            );
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
            required_margin_m = required_margin_m.max(margin_m);
            if Self::insert_terrain_patch_grading_margins_for_ray(
                terrain,
                vertex.x,
                vertex.z,
                bisector_x / bisector_length_m,
                bisector_z / bisector_length_m,
                margin_m,
                safe_step_m,
                patch_margins,
                target_patch,
            ) {
                target_margin_m =
                    Some(target_margin_m.map_or(margin_m, |current| current.max(margin_m)));
            }
        }
        (
            Self::terrain_cdt_grading_influence_bounds(&vertices, required_margin_m),
            target_margin_m,
        )
    }

    fn terrain_cdt_grading_influence_bounds(
        vertices: &[TerrainCdtVertex],
        margin_m: f32,
    ) -> Option<(f32, f32, f32, f32)> {
        let mut min_x = f64::INFINITY;
        let mut min_z = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_z = f64::NEG_INFINITY;
        for vertex in vertices {
            min_x = min_x.min(vertex.x);
            min_z = min_z.min(vertex.z);
            max_x = max_x.max(vertex.x);
            max_z = max_z.max(vertex.z);
        }
        if !min_x.is_finite() || !min_z.is_finite() || !max_x.is_finite() || !max_z.is_finite() {
            return None;
        }
        let margin_m = f64::from(margin_m.max(0.0));
        Some((
            (min_x - margin_m) as f32,
            (min_z - margin_m) as f32,
            (max_x + margin_m) as f32,
            (max_z + margin_m) as f32,
        ))
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
        target_patch: Option<(usize, usize)>,
    ) -> bool {
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
            target_patch,
        )
    }

    fn insert_terrain_patch_grading_margins_for_bounds(
        terrain: &TerrainSystem,
        points: impl IntoIterator<Item = (f64, f64)>,
        margin_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
        target_patch: Option<(usize, usize)>,
    ) -> bool {
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
            return false;
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
            target_patch,
        )
    }

    fn insert_terrain_patch_grading_margins_for_rect(
        terrain: &TerrainSystem,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
        margin_m: f32,
        patch_margins: &mut BTreeMap<(usize, usize), f32>,
        target_patch: Option<(usize, usize)>,
    ) -> bool {
        let query_min_x = min_x.min(max_x);
        let query_min_z = min_z.min(max_z);
        let query_max_x = min_x.max(max_x);
        let query_max_z = min_z.max(max_z);
        let mut target_inserted = false;
        for key in terrain.render_patch_keys_for_world_bounds(
            query_min_x,
            query_min_z,
            query_max_x,
            query_max_z,
        ) {
            let Some((patch_min_x, patch_min_z, patch_max_x, patch_max_z)) =
                terrain.render_patch_world_bounds(key.0, key.1)
            else {
                continue;
            };
            let overlap_x = query_max_x.min(patch_max_x) - query_min_x.max(patch_min_x);
            let overlap_z = query_max_z.min(patch_max_z) - query_min_z.max(patch_min_z);
            if overlap_x <= TERRAIN_CDT_MIN_PATCH_OVERLAP_M
                || overlap_z <= TERRAIN_CDT_MIN_PATCH_OVERLAP_M
            {
                continue;
            }
            patch_margins
                .entry(key)
                .and_modify(|existing| *existing = existing.max(margin_m))
                .or_insert(margin_m);
            target_inserted |= Some(key) == target_patch;
        }
        target_inserted
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grading_rect_does_not_claim_a_patch_at_zero_area_contact() {
        let terrain = TerrainSystem::with_chunking(9, 9, 10.0, 4, 0.0);
        let (min_x, min_z, max_x, _) = terrain
            .render_patch_world_bounds(0, 0)
            .expect("first terrain patch should exist");
        let query_bounds = (min_x + 5.0, min_z + 5.0, max_x, min_z + 15.0);
        let raw_keys = terrain.render_patch_keys_for_world_bounds(
            query_bounds.0,
            query_bounds.1,
            query_bounds.2,
            query_bounds.3,
        );
        assert!(
            raw_keys.contains(&(1, 0)),
            "inclusive terrain sample ownership should expose the tangent neighbor"
        );

        let mut patch_margins = BTreeMap::new();
        RoadSurfaceSystem::insert_terrain_patch_grading_margins_for_rect(
            &terrain,
            query_bounds.0,
            query_bounds.1,
            query_bounds.2,
            query_bounds.3,
            10.0,
            &mut patch_margins,
            None,
        );

        assert_eq!(patch_margins.get(&(0, 0)), Some(&10.0));
        assert!(
            !patch_margins.contains_key(&(1, 0)),
            "a patch touched only along its boundary must not become road-locked"
        );
    }

    #[test]
    fn exact_patch_margin_cannot_mix_influence_and_pad_selection_across_loops() {
        let target_bounds = (0.0, 0.0, 10.0, 10.0);
        let influence_only = RoadSurfaceSystem::terrain_cdt_exact_target_patch_margin(
            Some((-2.0, 2.0, 2.0, 8.0)),
            target_bounds,
            None,
        );
        let pad_only = RoadSurfaceSystem::terrain_cdt_exact_target_patch_margin(
            Some((12.0, 2.0, 18.0, 8.0)),
            target_bounds,
            Some(8.0),
        );

        assert_eq!(influence_only, None);
        assert_eq!(pad_only, None);
    }
}
