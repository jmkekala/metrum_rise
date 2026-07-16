//! `SimCore` refined terrain patch cache builders and invalidation helpers.

use super::super::*;

impl SimCore {
    pub(crate) fn collect_refined_terrain_patch_build_inputs(
        &mut self,
        render_step_m: f32,
    ) -> Vec<RefinedTerrainPatchBuildInput> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let road_locked_start = road_debug.then(Instant::now);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::TerrainEarthwork,
        );
        let max_road_locked_margin_m = self
            .transit_network
            .road_surface
            .terrain_cdt_required_grading_margin_for_visible_roads(
                &self.region_graph,
                &self.heightmap,
                safe_render_step_m,
            );
        let site_margin_m = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
            &self.heightmap,
            safe_render_step_m,
        );
        let mut road_locked_patch_margins = self
            .transit_network
            .road_surface
            .terrain_render_patch_grading_margins_for_visible_roads(
                &self.region_graph,
                &self.heightmap,
                safe_render_step_m,
            );
        for key in self
            .allocator
            .terrain_render_patch_keys_with_building_site_margin(&self.heightmap, site_margin_m)
        {
            road_locked_patch_margins
                .entry(key)
                .and_modify(|existing| *existing = existing.max(site_margin_m))
                .or_insert(site_margin_m);
        }
        let mut road_locked_key_vec = road_locked_patch_margins
            .keys()
            .copied()
            .collect::<Vec<_>>();
        road_locked_key_vec.sort_unstable();
        road_locked_key_vec.dedup();
        let old_road_locked_keys: HashSet<(usize, usize)> = self
            .road_locked_terrain_patch_keys
            .iter()
            .copied()
            .collect();
        let road_locked_keys: HashSet<(usize, usize)> =
            road_locked_key_vec.iter().copied().collect();
        let mut road_locked_changed_patches = 0usize;
        for key in old_road_locked_keys.symmetric_difference(&road_locked_keys) {
            self.heightmap.mark_render_patch_dirty(key.0, key.1);
            self.refined_terrain_patch_cache
                .remove(&SimulationNode::refined_patch_cache_key(
                    key.0,
                    key.1,
                    safe_render_step_m,
                ));
            road_locked_changed_patches += 1;
        }
        self.road_locked_terrain_patch_keys = road_locked_key_vec;
        let road_locked_ms = road_locked_start
            .map(|start| start.elapsed().as_secs_f64() * 1000.0)
            .unwrap_or(0.0);

        let dirty_patches: Vec<(usize, usize)> = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        if dirty_patches.is_empty() {
            return Vec::new();
        }

        let surface_generation = self.road_tool_surface_generation;
        let mut inputs = Vec::new();
        for (patch_x, patch_z) in dirty_patches.iter().copied() {
            if !road_locked_keys.contains(&(patch_x, patch_z)) {
                continue;
            }
            let Some(base_patch) = self.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
                continue;
            };
            let patch_margin_m = road_locked_patch_margins
                .get(&(patch_x, patch_z))
                .copied()
                .unwrap_or(site_margin_m)
                .max(site_margin_m);
            let road_clip_query = SimulationNode::road_clip_loop_query_for_bounds(
                self,
                base_patch.world_origin_x - patch_margin_m,
                base_patch.world_origin_z - patch_margin_m,
                base_patch.world_origin_x + base_patch.world_size_x + patch_margin_m,
                base_patch.world_origin_z + base_patch.world_size_z + patch_margin_m,
            );
            let key = SimulationNode::refined_patch_cache_key(patch_x, patch_z, safe_render_step_m);
            if self
                .refined_terrain_patch_cache
                .get(&key)
                .is_some_and(|cached| {
                    !SimulationNode::refined_terrain_patch_cache_entry_is_current(
                        cached,
                        surface_generation,
                    )
                })
            {
                self.refined_terrain_patch_cache.remove(&key);
            }
            let previous = self.refined_terrain_patch_cache.get(&key);
            let windows = SimulationNode::terrain_cdt_window_build_inputs(
                &self.heightmap,
                &base_patch,
                &road_clip_query.cdt_road_loops,
                safe_render_step_m,
                Some(TerrainCdtSiteGradingContext {
                    allocator: &self.allocator,
                    graph: &self.region_graph,
                    road_surface: &self.transit_network.road_surface,
                }),
                previous,
            );
            inputs.push(RefinedTerrainPatchBuildInput {
                key,
                surface_generation,
                patch: base_patch,
                windows,
                road_clip_source_count: road_clip_query.source_count,
                clip_error_label: road_clip_query.clip_error_label,
            });
        }

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_precompute_inputs dirty_patches={} road_locked_patches={} changed_locked_patches={} inputs={} max_road_locked_margin_m={:.3} site_margin_m={:.3} road_locked_ms={:.3} total_ms={:.3}",
                dirty_patches.len(),
                road_locked_keys.len(),
                road_locked_changed_patches,
                inputs.len(),
                max_road_locked_margin_m,
                site_margin_m,
                road_locked_ms,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        inputs
    }

    /// Refreshes road-locked terrain patch membership and invalidates stale refined terrain cache.
    pub(crate) fn refresh_road_locked_terrain_patch_state(&mut self, render_step_m: f32) -> usize {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::TerrainEarthwork,
        );
        let site_margin_m = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
            &self.heightmap,
            safe_render_step_m,
        );
        let mut road_locked_patch_margins = self
            .transit_network
            .road_surface
            .terrain_render_patch_grading_margins_for_visible_roads(
                &self.region_graph,
                &self.heightmap,
                safe_render_step_m,
            );
        for key in self
            .allocator
            .terrain_render_patch_keys_with_building_site_margin(&self.heightmap, site_margin_m)
        {
            road_locked_patch_margins
                .entry(key)
                .and_modify(|existing| *existing = existing.max(site_margin_m))
                .or_insert(site_margin_m);
        }
        let mut road_locked_key_vec = road_locked_patch_margins
            .keys()
            .copied()
            .collect::<Vec<_>>();
        road_locked_key_vec.sort_unstable();
        road_locked_key_vec.dedup();
        let old_road_locked_keys: HashSet<(usize, usize)> = self
            .road_locked_terrain_patch_keys
            .iter()
            .copied()
            .collect();
        let road_locked_keys: HashSet<(usize, usize)> =
            road_locked_key_vec.iter().copied().collect();
        let mut road_locked_changed_patches = 0usize;
        let mut invalidated_refined_cache_entries = 0usize;
        for key in old_road_locked_keys.symmetric_difference(&road_locked_keys) {
            self.heightmap.mark_render_patch_dirty(key.0, key.1);
            if self
                .refined_terrain_patch_cache
                .remove(&SimulationNode::refined_patch_cache_key(
                    key.0,
                    key.1,
                    safe_render_step_m,
                ))
                .is_some()
            {
                invalidated_refined_cache_entries += 1;
            }
            road_locked_changed_patches += 1;
        }
        self.road_locked_terrain_patch_keys = road_locked_key_vec;

        let dirty_patches: Vec<(usize, usize)> = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        for (patch_x, patch_z) in dirty_patches.iter().copied() {
            if !road_locked_keys.contains(&(patch_x, patch_z)) {
                continue;
            }
            if self
                .refined_terrain_patch_cache
                .remove(&SimulationNode::refined_patch_cache_key(
                    patch_x,
                    patch_z,
                    safe_render_step_m,
                ))
                .is_some()
            {
                invalidated_refined_cache_entries += 1;
            }
        }

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_state_refresh dirty_patches={} road_locked_patches={} changed_locked_patches={} invalidated_cache_entries={} total_ms={:.3}",
                dirty_patches.len(),
                road_locked_keys.len(),
                road_locked_changed_patches,
                invalidated_refined_cache_entries,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        invalidated_refined_cache_entries
    }

    pub(crate) fn build_refined_terrain_patch_cache_entries(
        inputs: Vec<RefinedTerrainPatchBuildInput>,
    ) -> Vec<CachedRefinedTerrainPatch> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let input_count = inputs.len();
        let mut entries: Vec<CachedRefinedTerrainPatch> = inputs
            .into_par_iter()
            .map(|input| {
                let mut cdt_ms = 0.0;
                let mut reused_windows = 0usize;
                let mut windows = input
                    .windows
                    .into_par_iter()
                    .map(|window| {
                        if let Some(mut previous) = window.previous {
                            previous.reused = true;
                            return previous;
                        }
                        let input_road_loops = window.cdt_input.road_loops.len();
                        let input_source_samples = window.cdt_input.source_samples.len();
                        let cdt_patch = window.cdt_input.patch;
                        let cdt_start = Instant::now();
                        let mesh_result = build_road_touched_terrain_patch(window.cdt_input);
                        let cdt_ms = cdt_start.elapsed().as_secs_f64() * 1000.0;
                        CachedRefinedTerrainCdtWindow {
                            key: window.key,
                            input_road_loops,
                            input_source_samples,
                            cdt_patch,
                            mesh_result,
                            cdt_ms,
                            reused: false,
                        }
                    })
                    .collect::<Vec<_>>();
                windows.sort_by_key(|window| window.key);
                for window in &windows {
                    if window.reused {
                        reused_windows += 1;
                    } else {
                        cdt_ms += window.cdt_ms;
                    }
                }
                let input_road_loops = windows
                    .iter()
                    .map(|window| window.input_road_loops)
                    .sum::<usize>();
                let input_source_samples = windows
                    .iter()
                    .map(|window| window.input_source_samples)
                    .sum::<usize>();
                CachedRefinedTerrainPatch {
                    key: input.key,
                    contract_revision: TERRAIN_CDT_CONTRACT_REVISION,
                    surface_generation: input.surface_generation,
                    patch: input.patch,
                    input_road_loops,
                    input_source_samples,
                    windows,
                    road_clip_source_count: input.road_clip_source_count,
                    clip_error_label: input.clip_error_label,
                    cdt_ms,
                    reused_windows,
                }
            })
            .collect();
        entries.sort_by_key(|entry| (entry.key.patch_x, entry.key.patch_z));

        if road_debug {
            for entry in &entries {
                let has_conflicts = entry.windows.iter().any(|window| {
                    window.mesh_result.as_ref().is_ok_and(|mesh| {
                        SimulationNode::terrain_cdt_stats_have_constraint_conflicts(mesh.stats)
                    })
                });
                let error_label = entry
                    .windows
                    .iter()
                    .find_map(|window| window.mesh_result.as_ref().err())
                    .map(SimulationNode::terrain_cdt_error_label);
                let mut max_face_y_delta_m = 0.0_f32;
                let mut max_face_slope_ratio = 0.0_f32;
                let mut longest_triangle_edge_m = 0.0_f32;
                let mut tie_in_widened_source_samples = 0usize;
                let mut retaining_wall_faces = 0usize;
                let successful_windows = entry
                    .windows
                    .iter()
                    .filter_map(|window| {
                        window.mesh_result.as_ref().ok().map(|mesh| (window, mesh))
                    })
                    .collect::<Vec<_>>();
                for (_, mesh) in &successful_windows {
                    max_face_y_delta_m = max_face_y_delta_m.max(mesh.stats.max_face_y_delta_m);
                    max_face_slope_ratio =
                        max_face_slope_ratio.max(mesh.stats.max_face_slope_ratio);
                    longest_triangle_edge_m =
                        longest_triangle_edge_m.max(mesh.stats.longest_triangle_edge_m);
                    tie_in_widened_source_samples += mesh.stats.tie_in_widened_source_samples;
                    retaining_wall_faces += mesh.stats.retaining_wall_faces;
                }
                let final_buffer_summary = if successful_windows.is_empty() {
                    None
                } else {
                    Some(SimulationNode::terrain_cdt_window_final_buffer_stats(
                        &entry.patch,
                        &successful_windows,
                        (entry.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON),
                    ))
                };
                if let Some(summary) = final_buffer_summary {
                    max_face_y_delta_m = summary.max_face_y_delta_m;
                    max_face_slope_ratio = summary.max_face_slope_ratio;
                    longest_triangle_edge_m = summary.longest_triangle_edge_m;
                };
                let pathological_terrain_slope_ratio = final_buffer_summary
                    .map(|summary| summary.terrain_max_face_slope_ratio)
                    .unwrap_or(max_face_slope_ratio);
                let pathological_terrain_longest_edge_m = final_buffer_summary
                    .map(|summary| summary.terrain_longest_triangle_edge_m)
                    .unwrap_or(longest_triangle_edge_m);
                let status = if entry.windows.is_empty() {
                    "empty"
                } else if has_conflicts {
                    "conflicted"
                } else if let Some(error_label) = error_label {
                    error_label
                } else {
                    SimulationNode::terrain_cdt_output_status(
                        false,
                        pathological_terrain_slope_ratio,
                        pathological_terrain_longest_edge_m,
                    )
                };
                SimulationNode::debug_log_pathological_terrain_cdt_output(
                    "refined_patch_precompute_pathological_output",
                    entry.key.patch_x,
                    entry.key.patch_z,
                    status,
                    pathological_terrain_slope_ratio,
                    pathological_terrain_longest_edge_m,
                    entry.input_road_loops,
                    entry.input_source_samples,
                );
                debug_log!(
                    "road",
                    "refined_patch_precompute key=({},{}) render_step_mm={} status={} windows={} reused_windows={} road_loops={} source_samples={} max_face_y_delta_m={:.3} max_face_slope={:.3} tie_in_widened_samples={} retaining_wall_faces={} longest_triangle_edge_m={:.3} cdt_ms={:.3}",
                    entry.key.patch_x,
                    entry.key.patch_z,
                    entry.key.render_step_mm,
                    status,
                    entry.windows.len(),
                    entry.reused_windows,
                    entry.input_road_loops,
                    entry.input_source_samples,
                    max_face_y_delta_m,
                    max_face_slope_ratio,
                    tie_in_widened_source_samples,
                    retaining_wall_faces,
                    longest_triangle_edge_m,
                    entry.cdt_ms
                );
            }
            debug_log!(
                "road",
                "refined_patch_precompute_total inputs={} built={} total_ms={:.3}",
                input_count,
                entries.len(),
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        entries
    }

    pub(crate) fn insert_refined_terrain_patch_cache_entries(
        &mut self,
        entries: Vec<CachedRefinedTerrainPatch>,
    ) {
        let surface_generation = self.road_tool_surface_generation;
        for entry in entries {
            if SimulationNode::refined_terrain_patch_cache_entry_is_current(
                &entry,
                surface_generation,
            ) {
                self.refined_terrain_patch_cache.insert(entry.key, entry);
            }
        }
    }
}
