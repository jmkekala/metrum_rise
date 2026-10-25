// SPDX-License-Identifier: GPL-2.0-only

//! `SimCore` refined terrain patch cache builders and invalidation helpers.

use super::super::*;
use crate::simulation::network::roadbed::{RoadSurfaceTriangleQueryIndex, RoadVec3};
use std::collections::BTreeSet;

impl SimCore {
    /// Builds one canonical CDT tile and its patch-local buffers, shared by preview and commit.
    pub(crate) fn build_refined_terrain_cdt_window(
        window: RefinedTerrainCdtWindowBuildInput,
        patch: &crate::simulation::terrain::TerrainPatchSnapshot,
    ) -> (Arc<CachedRefinedTerrainCdtWindow>, bool) {
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
        let ground_query = Arc::new(mesh_result.as_ref().map_or_else(
            |_| Default::default(),
            |mesh| {
                RoadSurfaceTriangleQueryIndex::from_ground_triangles(mesh.triangles.iter().map(
                    |indices| {
                        indices.map(|i| {
                            let p = mesh.vertices[i];
                            RoadVec3::new(p.x, f64::from(p.height_m), p.z)
                        })
                    },
                ))
            },
        ));
        let mesh_buffers = mesh_result.as_ref().ok().map(|mesh| {
            Arc::new(
                SimulationNode::prepare_cached_refined_terrain_window_mesh_buffers(patch, mesh),
            )
        });
        (
            Arc::new(CachedRefinedTerrainCdtWindow {
                ground_query,
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
    }

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
        self.refresh_terrain_patch_ownership(render_step_m, patch_keys, true)
    }

    /// Refreshes the site ownership of render patches after building-site changes and keeps their
    /// road ownership: a site change moves neither a road nor the heightmap, the only inputs of a
    /// patch's road grading margin (`ROAD-68-D`).
    pub(crate) fn refresh_site_terrain_patch_ownership_for_keys(
        &mut self,
        render_step_m: f32,
        patch_keys: &[(usize, usize)],
    ) -> usize {
        self.refresh_terrain_patch_ownership(render_step_m, patch_keys, false)
    }

    // Recomputes the road margin only with `refresh_roads` and a compiled roadbed; otherwise the
    // patch keeps its last road ownership.
    fn refresh_terrain_patch_ownership(
        &mut self,
        render_step_m: f32,
        patch_keys: &[(usize, usize)],
        refresh_roads: bool,
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
        self.allocator
            .prepare_building_site_query_index(self.config.zone_cell_m);

        let cache_key_for = |key: (usize, usize)| {
            SimulationNode::refined_patch_cache_key(key.0, key.1, safe_render_step_m)
        };
        let mut changed_keys = Vec::new();
        let mut invalidated_cache_entries = 0usize;
        // A dirty roadbed keeps the last published road ownership until it compiles. Patch road
        // margins are independent, so they are graded in parallel; the indexed collect keeps
        // them in key order.
        let refresh_roads = refresh_roads && self.transit_network.roadbed.is_current();
        let road_margins: Vec<Option<f32>> = if refresh_roads {
            let (terrain, roads) = (
                &self.heightmap,
                (&self.region_graph, &self.transit_network.roadbed),
            );
            unique_keys
                .par_iter()
                .map(|&key| {
                    Self::road_terrain_patch_margin_m(terrain, roads, key, safe_render_step_m)
                })
                .collect()
        } else {
            Vec::new()
        };
        for (index, &key) in unique_keys.iter().enumerate() {
            let old_engineered_margin = self.engineered_terrain_patch_margins.get(&key).copied();
            let road_margin = if refresh_roads {
                self.road_ownership_pending_patch_keys.remove(&key);
                road_margins[index]
            } else {
                self.road_locked_terrain_patch_margins.get(&key).copied()
            };
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

            let engineered_margin =
                Self::engineered_terrain_margin_m(road_margin, site_owned, site_margin_m);
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
                                .road_tile_dirty_at
                                .values()
                                .any(|stamp| *stamp == current_generation)
                    })
            })
            .collect::<Vec<_>>();
        // Road-only margin changes retain the already-stamped old/new query-chunk scope. Other
        // ownership transitions still get a full scope when no caller proved a local mutation.
        self.bump_terrain_payload_patch_generations(&full_dirty_keys);
        self.road_terrain_inputs.note(unique_keys.iter().copied());
        self.terrain_dirty |= !changed_keys.is_empty();
        invalidated_cache_entries
    }

    /// The engineered margin of a patch from its road margin and site ownership: what the
    /// ownership refresh stores and the preview derives (`ROAD-68-E`). `None` leaves the patch
    /// raw terrain.
    pub(crate) fn engineered_terrain_margin_m(
        road_margin_m: Option<f32>,
        site_owned: bool,
        site_margin_m: f32,
    ) -> Option<f32> {
        match (road_margin_m, site_owned) {
            (Some(road_margin_m), true) => Some(road_margin_m.max(site_margin_m)),
            (Some(road_margin_m), false) => Some(road_margin_m),
            (None, true) => Some(site_margin_m),
            (None, false) => None,
        }
    }

    /// Grading margin of the roads whose envelope reaches a render patch: the largest margin of
    /// any tile's road loops (`terrain_cdt_road_tile_loops`, the loops the build uses), `None`
    /// when no tile has one (`ROAD-68-D`). A chain that cannot close locks the patch at the
    /// envelope cap, so its build reports the typed error instead of showing raw terrain under
    /// a road. The live core and the preview's edited replica both call it (`ROAD-68-E`).
    pub(crate) fn road_terrain_patch_margin_m(
        terrain: &TerrainSystem,
        roads: TerrainCdtRoads<'_>,
        key: (usize, usize),
        render_step_m: f32,
    ) -> Option<f32> {
        use crate::simulation::terrain::grading::GRADING_ENVELOPE_MAX_MARGIN_M;
        let patch = terrain.visual_patch_snapshot(key.0, key.1)?;
        let bounds = (
            patch.world_origin_x,
            patch.world_origin_z,
            patch.world_origin_x + patch.world_size_x,
            patch.world_origin_z + patch.world_size_z,
        );
        // Tiles grade independently and `max` is order-free, so the margin does not depend on
        // the workers. `Err` is a tile whose chains cannot close.
        let larger = |a: Option<f32>, b: Option<f32>| match (a, b) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        SimulationNode::terrain_cdt_tile_ids_for_bounds(&patch, bounds)
            .into_par_iter()
            .map(|tile_id| -> Result<Option<f32>, &'static str> {
                let Some(tile_bounds) = SimulationNode::terrain_cdt_tile_bounds(&patch, tile_id)
                else {
                    return Ok(None);
                };
                let loops = SimulationNode::terrain_cdt_road_tile_loops(
                    terrain,
                    roads,
                    tile_bounds,
                    render_step_m,
                    false,
                )?;
                Ok(loops.iter().map(|tile_loop| tile_loop.margin_m).reduce(f32::max))
            })
            .try_reduce(|| None, |a, b| Ok(larger(a, b)))
            .unwrap_or(Some(GRADING_ENVELOPE_MAX_MARGIN_M))
    }

    #[cfg(test)]
    pub(crate) fn collect_refined_terrain_patch_build_inputs(
        &mut self,
        render_step_m: f32,
    ) -> Vec<RefinedTerrainPatchBuildInput> {
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
        if !self.transit_network.roadbed.is_current() {
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
            let road_clip_query = SimulationNode::road_clip_loop_query_for_snapshot(
                &sites,
                base_patch.world_origin_x - patch_query_margin_m,
                base_patch.world_origin_z - patch_query_margin_m,
                base_patch.world_origin_x + base_patch.world_size_x + patch_query_margin_m,
                base_patch.world_origin_z + base_patch.world_size_z + patch_query_margin_m,
            );
            let window_plan = SimulationNode::terrain_cdt_window_build_inputs_with_roads(
                &self.heightmap,
                &base_patch,
                Some((&self.region_graph, &self.transit_network.roadbed)),
                &road_clip_query.cdt_road_loops,
                safe_render_step_m,
                Some(TerrainCdtSiteGradingContext {
                    source: TerrainCdtSiteGradingSource::Snapshot(&sites),
                    roads: crate::simulation::buildings::allocator::SiteRoads::new(
                        &self.region_graph,
                        &self.transit_network.roadbed,
                    ),
                }),
                previous.as_deref(),
            );
            let requires_road_clipping = SimulationNode::road_clip_query_requires_road_clipping(
                &road_clip_query,
                road_locked,
            );
            inputs.push(RefinedTerrainPatchBuildInput {
                site_surfaces: sites.paving_surfaces(),
                site_foundations: sites.foundations(),
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
                clip_error_label: road_clip_query
                    .clip_error_label
                    .or(window_plan.road_error_label),
                clip_query_margin_m: patch_query_margin_m,
                derive_clip_counts_from_windows: true,
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

    /// Refreshes engineered terrain ownership and invalidates stale refined terrain cache over
    /// the whole world, after a world-scale road change (a load or a bulk road build). It
    /// subsumes every terrain change the roadbed recorded since terrain last took them: those
    /// are dropped and every patch payload takes a new generation, so the next road edit stamps
    /// and builds only its own terrain (`ROAD-68-D` part 6). `O(patches)` plus the grading of
    /// every patch with roads.
    pub(crate) fn refresh_all_engineered_terrain_patch_state(
        &mut self,
        render_step_m: f32,
    ) -> usize {
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
        drop(self.transit_network.roadbed.take_terrain_changes());
        self.bump_global_terrain_payload_generation();
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

    /// Refreshes the ownership of the patches whose road margin waits for a refresh (a compile
    /// changed pieces near them or a terrain edit changed their heights) and invalidates stale
    /// refined terrain cache. A patch the edit's validation already refreshed is not pending, and
    /// patches dirty for other reasons keep their road ownership (`ROAD-68-D`).
    pub(crate) fn refresh_road_locked_terrain_patch_state(&mut self, render_step_m: f32) -> usize {
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let safe_render_step_m = render_step_m.max(f32::EPSILON);
        self.transit_network
            .roadbed
            .compile_dirty(&self.region_graph);
        if !self.transit_network.roadbed.is_current() {
            return 0;
        }
        let pending_patches = self
            .road_ownership_pending_patch_keys
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let invalidated_refined_cache_entries = self
            .refresh_engineered_terrain_patch_ownership_for_keys(
                safe_render_step_m,
                &pending_patches,
            );

        if road_debug {
            debug_log!(
                "road",
                "refined_patch_state_refresh pending_patches={} road_owned_patches={} engineered_patches={} invalidated_cache_entries={} total_ms={:.3}",
                pending_patches.len(),
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
        let road_debug = crate::debug::Channel::Road.enabled();
        let total_start = road_debug.then(Instant::now);
        let input_count = inputs.len();
        let mut entries: Vec<Arc<CachedRefinedTerrainPatch>> = inputs
            .into_par_iter()
            .map(|input| {
                let mut cdt_ms = 0.0;
                let mut reused_windows = 0usize;
                let patch = &input.patch;
                let mut window_results = input
                    .windows
                    .into_par_iter()
                    .map(|window| Self::build_refined_terrain_cdt_window(window, patch))
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
                        && previous.site_surfaces == input.site_surfaces
                        && previous.site_foundations == input.site_foundations
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
                    site_surfaces: input.site_surfaces,
                    site_foundations: input.site_foundations,
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
                    && SimulationNode::cached_refined_cdt_window_failure_label(&entry).is_none()
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
                        let mut buffers =
                            SimulationNode::prepare_cached_refined_terrain_mesh_buffers(
                                &entry.patch,
                                &successful_windows,
                                (entry.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON),
                            );
                        SimulationNode::partition_terrain_site_paving(
                            &entry.patch,
                            &entry.site_surfaces,
                            &entry.site_foundations,
                            &mut buffers,
                        );
                        Some(Arc::new(buffers))
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
                self.road_terrain_inputs
                    .note([(entry.key.patch_x, entry.key.patch_z)]);
                self.refined_terrain_patch_cache.insert(entry.key, entry);
                inserted += 1;
            }
        }
        inserted
    }
}
