// SPDX-License-Identifier: GPL-2.0-only

//! `SimCore` refined terrain patch cache builders and invalidation helpers.

use super::super::*;
use std::collections::BTreeSet;

impl SimCore {
    pub(crate) fn terrain_patch_requires_engineered_refinement(
        &self,
        patch_x: usize,
        patch_z: usize,
    ) -> bool {
        let key = (patch_x, patch_z);
        self.engineered_terrain_patch_margins.contains_key(&key)
            || self.road_locked_terrain_patch_margins.contains_key(&key)
            || self.building_site_owned_terrain_patch_keys.contains(&key)
    }

    fn set_sorted_patch_membership(
        keys: &mut Vec<(usize, usize)>,
        key: (usize, usize),
        present: bool,
    ) {
        match (keys.binary_search(&key), present) {
            (Err(index), true) => keys.insert(index, key),
            (Ok(index), false) => {
                keys.remove(index);
            }
            _ => {}
        }
    }

    pub(crate) fn refresh_engineered_terrain_patch_ownership_for_keys(
        &mut self,
        render_step_m: f32,
        patch_keys: &[(usize, usize)],
    ) -> usize {
        if patch_keys.is_empty() {
            return 0;
        }
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        let site_margin_m = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
            &self.heightmap,
            safe_render_step_m,
        );
        let mut unique_keys = patch_keys.to_vec();
        unique_keys.sort_unstable();
        unique_keys.dedup();
        let road_margins = self
            .transit_network
            .road_surface
            .published_generation_matches_source()
            .then(|| {
                self.transit_network
                    .road_surface
                    .terrain_render_patch_grading_margins_for_patches(
                        &self.region_graph,
                        &self.heightmap,
                        safe_render_step_m,
                        &unique_keys,
                    )
            });
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);

        let cache_key_for = |key: (usize, usize)| {
            SimulationNode::refined_patch_cache_key(key.0, key.1, safe_render_step_m)
        };
        let mut changed_keys = Vec::new();
        let mut invalidated_cache_entries = 0usize;
        for key in unique_keys {
            let old_engineered_margin = self.engineered_terrain_patch_margins.get(&key).copied();
            let road_margin = road_margins
                .as_ref()
                .and_then(|margins| margins.get(&key).copied())
                .or_else(|| {
                    road_margins
                        .is_none()
                        .then(|| self.road_locked_terrain_patch_margins.get(&key).copied())
                        .flatten()
                });
            if let Some(margin_m) = road_margin {
                self.road_locked_terrain_patch_margins.insert(key, margin_m);
            } else {
                self.road_locked_terrain_patch_margins.remove(&key);
            }
            Self::set_sorted_patch_membership(
                &mut self.road_locked_terrain_patch_keys,
                key,
                road_margin.is_some(),
            );

            let site_owned = self
                .heightmap
                .render_patch_world_bounds(key.0, key.1)
                .is_some_and(|(min_x, min_z, max_x, max_z)| {
                    self.allocator.has_building_site_for_world_bounds(
                        min_x - site_margin_m,
                        min_z - site_margin_m,
                        max_x + site_margin_m,
                        max_z + site_margin_m,
                    )
                });
            if site_owned {
                self.building_site_owned_terrain_patch_keys.insert(key);
            } else {
                self.building_site_owned_terrain_patch_keys.remove(&key);
            }

            let engineered_margin = match (road_margin, site_owned) {
                (Some(road_margin_m), true) => Some(road_margin_m.max(site_margin_m)),
                (Some(road_margin_m), false) => Some(road_margin_m),
                (None, true) => Some(site_margin_m),
                (None, false) => None,
            };
            if let Some(margin_m) = engineered_margin {
                self.engineered_terrain_patch_margins.insert(key, margin_m);
            } else {
                self.engineered_terrain_patch_margins.remove(&key);
            }
            Self::set_sorted_patch_membership(
                &mut self.engineered_terrain_patch_keys,
                key,
                engineered_margin.is_some(),
            );

            if old_engineered_margin != engineered_margin {
                self.heightmap.mark_render_patch_dirty(key.0, key.1);
                if engineered_margin.is_none()
                    && self
                        .refined_terrain_patch_cache
                        .remove(&cache_key_for(key))
                        .is_some()
                {
                    invalidated_cache_entries += 1;
                }
                changed_keys.push(key);
            }
        }
        let full_dirty_keys = changed_keys
            .iter()
            .copied()
            .filter(|key| {
                let current_generation = self.terrain_payload_generation_for_patch(key.0, key.1);
                !self
                    .refined_terrain_assembly_ledgers
                    .get(key)
                    .is_some_and(|ledger| {
                        ledger
                            .full_dirty_at
                            .is_some_and(|stamp| stamp <= current_generation)
                            || ledger
                                .road_query_chunk_dirty_at
                                .values()
                                .any(|stamp| *stamp == current_generation)
                    })
            })
            .collect::<Vec<_>>();
        // Road-only margin changes retain the already-stamped old/new query-chunk scope. Other
        // ownership transitions still get a full scope when no caller proved a local mutation.
        self.bump_terrain_payload_patch_generations(&full_dirty_keys);
        self.terrain_dirty |= !changed_keys.is_empty();
        invalidated_cache_entries
    }

    #[cfg(test)]
    pub(crate) fn collect_refined_terrain_patch_build_inputs(
        &mut self,
        render_step_m: f32,
    ) -> Vec<RefinedTerrainPatchBuildInput> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::TerrainEarthwork,
        );
        if !self
            .transit_network
            .road_surface
            .published_generation_matches_source()
        {
            return Vec::new();
        }
        let mut dirty_patches: Vec<(usize, usize)> = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        dirty_patches.sort_unstable();
        self.refresh_engineered_terrain_patch_ownership_for_keys(
            safe_render_step_m,
            &dirty_patches,
        );
        if dirty_patches.is_empty() {
            return Vec::new();
        }

        let mut inputs = Vec::new();
        for (patch_x, patch_z) in dirty_patches.iter().copied() {
            if !self.terrain_patch_requires_engineered_refinement(patch_x, patch_z) {
                continue;
            }
            let Some(base_patch) = self.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
                continue;
            };
            let patch_margin_m = self
                .engineered_terrain_patch_margins
                .get(&(patch_x, patch_z))
                .copied()
                .unwrap_or_default();
            let road_locked = self
                .road_locked_terrain_patch_margins
                .contains_key(&(patch_x, patch_z));
            let patch_query_margin_m = if road_locked {
                crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                    &self.heightmap,
                    safe_render_step_m,
                    patch_margin_m,
                )
            } else {
                patch_margin_m
            };
            let road_clip_query = SimulationNode::road_clip_loop_query_for_bounds(
                self,
                base_patch.world_origin_x - patch_query_margin_m,
                base_patch.world_origin_z - patch_query_margin_m,
                base_patch.world_origin_x + base_patch.world_size_x + patch_query_margin_m,
                base_patch.world_origin_z + base_patch.world_size_z + patch_query_margin_m,
            );
            let key = SimulationNode::refined_patch_cache_key(patch_x, patch_z, safe_render_step_m);
            let surface_generation = self.terrain_payload_generation_for_patch(patch_x, patch_z);
            if self
                .refined_terrain_patch_cache
                .get(&key)
                .is_some_and(|cached| cached.contract_revision != TERRAIN_CDT_CONTRACT_REVISION)
            {
                self.refined_terrain_patch_cache.remove(&key);
            }
            let previous = self.refined_terrain_patch_cache.get(&key).cloned();
            let sites = self.allocator.terrain_site_snapshot_for_world_bounds(
                base_patch.world_origin_x - patch_query_margin_m,
                base_patch.world_origin_z - patch_query_margin_m,
                base_patch.world_origin_x + base_patch.world_size_x + patch_query_margin_m,
                base_patch.world_origin_z + base_patch.world_size_z + patch_query_margin_m,
            );
            let window_plan = SimulationNode::terrain_cdt_window_build_inputs(
                &self.heightmap,
                &base_patch,
                &road_clip_query.cdt_road_loops,
                safe_render_step_m,
                Some(TerrainCdtSiteGradingContext {
                    source: TerrainCdtSiteGradingSource::Snapshot(&sites),
                    graph: &self.region_graph,
                    road_surface: &self.transit_network.road_surface,
                }),
                previous.as_deref(),
            );
            let requires_road_clipping = SimulationNode::road_clip_query_requires_road_clipping(
                &road_clip_query,
                road_locked,
            );
            inputs.push(RefinedTerrainPatchBuildInput {
                key,
                surface_generation,
                patch: base_patch,
                previous_patch: previous,
                windows: window_plan.windows,
                reused_windows: window_plan.reused_windows,
                input_clip_loop_count: window_plan.represented_road_loop_count,
                omitted_margin_clip_loop_count: window_plan.omitted_margin_loop_count,
                expected_road_clip_fingerprints: window_plan.expected_road_clip_fingerprints,
                expected_site_clip_fingerprints: window_plan.expected_site_clip_fingerprints,
                requires_engineered_refinement: self
                    .terrain_patch_requires_engineered_refinement(patch_x, patch_z),
                requires_road_clipping,
                clip_source_count: road_clip_query.source_count,
                road_clip_source_count: road_clip_query.road_source_count,
                road_clip_loop_count: road_clip_query.road_loop_count,
                site_clip_loop_count: road_clip_query.site_loop_count,
                clip_error_label: road_clip_query.clip_error_label,
                clip_query_margin_m: patch_query_margin_m,
                derive_clip_counts_from_windows: false,
            });
        }

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_precompute_inputs dirty_patches={} road_owned_patches={} engineered_patches={} inputs={} total_ms={:.3}",
                dirty_patches.len(),
                self.road_locked_terrain_patch_keys.len(),
                self.engineered_terrain_patch_keys.len(),
                inputs.len(),
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }

        inputs
    }

    /// Refreshes engineered terrain ownership and invalidates stale refined terrain cache.
    pub(crate) fn refresh_all_engineered_terrain_patch_state(
        &mut self,
        render_step_m: f32,
    ) -> usize {
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::TerrainEarthwork,
        );
        let patch_cols = self.heightmap.render_patch_cols();
        let patch_rows = self.heightmap.render_patch_rows();
        let mut patch_keys = Vec::with_capacity(patch_cols.saturating_mul(patch_rows));
        for patch_z in 0..patch_rows {
            for patch_x in 0..patch_cols {
                patch_keys.push((patch_x, patch_z));
            }
        }
        self.refresh_engineered_terrain_patch_ownership_for_keys(render_step_m, &patch_keys)
    }

    /// Refreshes engineered terrain ownership and invalidates stale refined terrain cache.
    pub(crate) fn refresh_road_locked_terrain_patch_state(&mut self, render_step_m: f32) -> usize {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::TerrainEarthwork,
        );
        if !self
            .transit_network
            .road_surface
            .published_generation_matches_source()
        {
            return 0;
        }
        let mut dirty_patches: Vec<(usize, usize)> = self
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        dirty_patches.sort_unstable();
        let invalidated_refined_cache_entries = self
            .refresh_engineered_terrain_patch_ownership_for_keys(
                safe_render_step_m,
                &dirty_patches,
            );

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_state_refresh dirty_patches={} road_owned_patches={} engineered_patches={} invalidated_cache_entries={} total_ms={:.3}",
                dirty_patches.len(),
                self.road_locked_terrain_patch_keys.len(),
                self.engineered_terrain_patch_keys.len(),
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
    ) -> Vec<Arc<CachedRefinedTerrainPatch>> {
        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let input_count = inputs.len();
        let mut entries: Vec<Arc<CachedRefinedTerrainPatch>> = inputs
            .into_par_iter()
            .map(|input| {
                let mut cdt_ms = 0.0;
                let mut reused_windows = 0usize;
                let patch = &input.patch;
                let boundary_step_m = (input.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON);
                let mut window_results = input
                    .windows
                    .into_par_iter()
                    .map(|window| {
                        if let Some(previous) = window
                            .previous
                            .filter(|previous| previous.mesh_result.is_ok())
                        {
                            return (previous, true);
                        }
                        let input_road_loops = window.cdt_input.road_loops.len();
                        let input_source_samples = window.cdt_input.source_samples.len();
                        let cdt_patch = window.cdt_input.patch;
                        let cdt_start = Instant::now();
                        let mesh_result = build_road_touched_terrain_patch(window.cdt_input);
                        let cdt_ms = cdt_start.elapsed().as_secs_f64() * 1000.0;
                        let mesh_buffers = mesh_result.as_ref().ok().map(|mesh| {
                            Arc::new(
                                SimulationNode::prepare_cached_refined_terrain_window_mesh_buffers(
                                    patch,
                                    cdt_patch,
                                    boundary_step_m,
                                    mesh,
                                ),
                            )
                        });
                        (
                            Arc::new(CachedRefinedTerrainCdtWindow {
                                key: window.key,
                                input_road_loops,
                                input_source_samples,
                                cdt_patch,
                                road_input: window.road_input,
                                mesh_result,
                                mesh_buffers,
                                cdt_ms,
                                has_engineered_contributor: window.has_engineered_contributor,
                                road_clip_fingerprints: window.road_clip_fingerprints,
                                site_clip_fingerprints: window.site_clip_fingerprints,
                            }),
                            false,
                        )
                    })
                    .collect::<Vec<_>>();
                window_results.extend(
                    input
                        .reused_windows
                        .into_iter()
                        .map(|window| (window, true)),
                );
                window_results.sort_by_key(|(window, _)| window.key);
                for (window, reused) in &window_results {
                    if *reused {
                        reused_windows += 1;
                    } else {
                        cdt_ms += window.cdt_ms;
                    }
                }
                let input_source_samples = window_results
                    .iter()
                    .map(|(window, _)| window.input_source_samples)
                    .sum::<usize>();
                let windows = window_results
                    .into_iter()
                    .map(|(window, _)| window)
                    .collect::<Vec<_>>();
                let road_clip_fingerprints = windows
                    .iter()
                    .flat_map(|window| window.road_clip_fingerprints.iter().copied())
                    .collect::<BTreeSet<_>>();
                let site_clip_fingerprints = windows
                    .iter()
                    .flat_map(|window| window.site_clip_fingerprints.iter().copied())
                    .collect::<BTreeSet<_>>();
                let expected_road_clip_fingerprints = input
                    .expected_road_clip_fingerprints
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                let expected_site_clip_fingerprints = input
                    .expected_site_clip_fingerprints
                    .iter()
                    .copied()
                    .collect::<BTreeSet<_>>();
                let reusable_mesh_buffers = input.previous_patch.as_ref().and_then(|previous| {
                    (previous.contract_revision == TERRAIN_CDT_CONTRACT_REVISION
                        && previous.key == input.key
                        && previous.patch == input.patch
                        && previous.windows.len() == windows.len()
                        && previous
                            .windows
                            .iter()
                            .zip(&windows)
                            .all(|(left, right)| Arc::ptr_eq(left, right)))
                    .then(|| previous.mesh_buffers.as_ref())
                    .flatten()
                    .filter(|buffers| buffers.variant_payload_valid)
                    .cloned()
                });
                let clip_manifest_error_label = (road_clip_fingerprints
                    != expected_road_clip_fingerprints
                    || site_clip_fingerprints != expected_site_clip_fingerprints)
                    .then_some("incomplete_terrain_clip_windows");
                let (
                    input_clip_loop_count,
                    clip_source_count,
                    road_clip_source_count,
                    road_clip_loop_count,
                    site_clip_loop_count,
                    omitted_margin_clip_loop_count,
                ) = if input.derive_clip_counts_from_windows {
                    let input_road_count = road_clip_fingerprints.len();
                    let input_site_count = site_clip_fingerprints.len();
                    let expected_road_count = expected_road_clip_fingerprints.len();
                    let expected_site_count = expected_site_clip_fingerprints.len();
                    (
                        input_road_count.saturating_add(input_site_count),
                        expected_road_count.saturating_add(expected_site_count),
                        expected_road_count,
                        expected_road_count,
                        expected_site_count,
                        0,
                    )
                } else {
                    (
                        input.input_clip_loop_count,
                        input.clip_source_count,
                        input.road_clip_source_count,
                        input.road_clip_loop_count,
                        input.site_clip_loop_count,
                        input.omitted_margin_clip_loop_count,
                    )
                };
                let mut entry = CachedRefinedTerrainPatch {
                    key: input.key,
                    contract_revision: TERRAIN_CDT_CONTRACT_REVISION,
                    surface_generation: input.surface_generation,
                    patch: input.patch,
                    input_road_loops: input_clip_loop_count,
                    input_source_samples,
                    windows,
                    mesh_buffers: None,
                    requires_engineered_refinement: input.requires_engineered_refinement,
                    requires_road_clipping: input.requires_road_clipping,
                    clip_source_count,
                    road_clip_source_count,
                    road_clip_loop_count,
                    site_clip_loop_count,
                    omitted_margin_clip_loop_count,
                    clip_error_label: input.clip_error_label.or(clip_manifest_error_label),
                    clip_query_margin_m: input.clip_query_margin_m,
                    cdt_ms,
                    reused_windows,
                };
                if entry.input_road_loops > 0
                    && SimulationNode::cached_refined_cdt_failure_label(&entry).is_none()
                {
                    entry.mesh_buffers = reusable_mesh_buffers.or_else(|| {
                        let successful_windows = entry
                            .windows
                            .iter()
                            .filter_map(|window| {
                                window
                                    .mesh_result
                                    .as_ref()
                                    .ok()
                                    .map(|mesh| (window.as_ref(), mesh))
                            })
                            .collect::<Vec<_>>();
                        debug_assert_eq!(successful_windows.len(), entry.windows.len());
                        Some(Arc::new(
                            SimulationNode::prepare_cached_refined_terrain_mesh_buffers(
                                &entry.patch,
                                &successful_windows,
                                (entry.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON),
                            ),
                        ))
                    });
                }
                Arc::new(entry)
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
                let error_label = SimulationNode::cached_refined_cdt_failure_label(entry);
                let mut max_face_y_delta_m = 0.0_f32;
                let mut max_face_slope_ratio = 0.0_f32;
                let mut longest_triangle_edge_m = 0.0_f32;
                let mut tie_in_widened_source_samples = 0usize;
                let mut retaining_wall_faces = 0usize;
                let successful_windows = entry
                    .windows
                    .iter()
                    .filter_map(|window| {
                        window
                            .mesh_result
                            .as_ref()
                            .ok()
                            .map(|mesh| (window.as_ref(), mesh))
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
                let final_buffer_summary = entry
                    .mesh_buffers
                    .as_deref()
                    .map(SimulationNode::cached_refined_terrain_mesh_buffer_summary);
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
                let status = if let Some(error_label) = error_label {
                    error_label
                } else if entry.windows.is_empty() {
                    "empty"
                } else if has_conflicts {
                    "conflicted"
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
                    "refined_patch_precompute key=({},{}) render_step_mm={} status={} windows={} failed_windows={} reused_windows={} requires_road_clipping={} road_sources={} road_loops={} site_loops={} input_clip_loops={} source_samples={} max_face_y_delta_m={:.3} max_face_slope={:.3} tie_in_widened_samples={} retaining_wall_faces={} longest_triangle_edge_m={:.3} cdt_ms={:.3}",
                    entry.key.patch_x,
                    entry.key.patch_z,
                    entry.key.render_step_mm,
                    status,
                    entry.windows.len(),
                    entry
                        .windows
                        .iter()
                        .filter(|window| window.mesh_result.is_err())
                        .count(),
                    entry.reused_windows,
                    entry.requires_road_clipping,
                    entry.road_clip_source_count,
                    entry.road_clip_loop_count,
                    entry.site_clip_loop_count,
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
        entries: Vec<Arc<CachedRefinedTerrainPatch>>,
    ) -> usize {
        let mut inserted = 0usize;
        for entry in entries {
            let surface_generation =
                self.terrain_payload_generation_for_patch(entry.key.patch_x, entry.key.patch_z);
            if SimulationNode::refined_terrain_patch_cache_entry_is_current(
                &entry,
                surface_generation,
            ) && SimulationNode::cached_refined_cdt_failure_label(&entry).is_none()
            {
                self.refined_terrain_patch_cache.insert(entry.key, entry);
                inserted += 1;
            }
        }
        inserted
    }
}
