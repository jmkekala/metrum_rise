//! `SimulationNode` helpers for async terrain and water jobs.

use super::super::*;
use super::state::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn lock_water_patch_mesh_jobs(
        &self,
    ) -> std::sync::MutexGuard<'_, WaterPatchMeshAsyncState> {
        self.water_patch_mesh_jobs
            .lock()
            .expect("water mesh job lock poisoned")
    }

    pub(in crate::nodes::simulation_node) fn lock_terrain_patch_payload_jobs(
        &self,
    ) -> std::sync::MutexGuard<'_, TerrainPatchPayloadAsyncState> {
        self.terrain_patch_payload_jobs
            .lock()
            .expect("terrain patch payload job lock poisoned")
    }

    pub(in crate::nodes::simulation_node) fn clear_terrain_patch_payload_jobs(&self) {
        self.lock_terrain_patch_payload_jobs().clear();
    }

    pub(in crate::nodes::simulation_node) fn clear_runtime_render_async_jobs(&self) {
        self.clear_terrain_patch_payload_jobs();
        self.lock_water_patch_payload_jobs().clear();
        self.lock_water_patch_mesh_jobs().clear();
    }

    pub(in crate::nodes::simulation_node) fn lock_water_patch_payload_jobs(
        &self,
    ) -> std::sync::MutexGuard<'_, WaterPatchPayloadAsyncState> {
        self.water_patch_payload_jobs
            .lock()
            .expect("water patch payload job lock poisoned")
    }

    pub(in crate::nodes::simulation_node) fn drain_completed_water_patch_mesh_jobs(
        &self,
    ) -> Vec<CachedWaterPatchMesh> {
        let mut jobs = self.lock_water_patch_mesh_jobs();
        std::mem::take(&mut jobs.completed)
    }

    pub(in crate::nodes::simulation_node) fn drain_completed_terrain_patch_payload_jobs(
        &self,
    ) -> (Vec<TerrainPatchPayload>, Vec<TerrainPatchPayloadRequest>) {
        let mut jobs = self.lock_terrain_patch_payload_jobs();
        (
            std::mem::take(&mut jobs.completed),
            std::mem::take(&mut jobs.failed),
        )
    }

    pub(in crate::nodes::simulation_node) fn drain_completed_water_patch_payload_jobs(
        &self,
    ) -> (Vec<WaterPatchPayload>, Vec<WaterPatchPayloadRequest>) {
        let mut jobs = self.lock_water_patch_payload_jobs();
        (
            std::mem::take(&mut jobs.completed),
            std::mem::take(&mut jobs.failed),
        )
    }

    pub(in crate::nodes::simulation_node) fn water_patch_mesh_requests_from_flat(
        flat_requests: &PackedInt32Array,
    ) -> Vec<(usize, usize, usize)> {
        let request_values = flat_requests.as_slice();
        if request_values.len() < 3 {
            return Vec::new();
        }
        let mut requests = Vec::new();
        for chunk in request_values.chunks_exact(3) {
            let Ok(patch_x) = usize::try_from(chunk[0]) else {
                continue;
            };
            let Ok(patch_z) = usize::try_from(chunk[1]) else {
                continue;
            };
            let lod_step = usize::try_from(chunk[2]).unwrap_or(1).max(1);
            requests.push((patch_x, patch_z, lod_step));
        }
        requests
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_keys_from_flat(
        flat_requests: &PackedInt32Array,
    ) -> Vec<TerrainPatchPayloadKey> {
        let request_values = flat_requests.as_slice();
        if request_values.len() < 3 {
            return Vec::new();
        }
        let mut requests = Vec::new();
        for chunk in request_values.chunks_exact(3) {
            let Ok(patch_x) = usize::try_from(chunk[0]) else {
                continue;
            };
            let Ok(patch_z) = usize::try_from(chunk[1]) else {
                continue;
            };
            let render_step_mm = u32::try_from(chunk[2]).unwrap_or(0);
            requests.push(TerrainPatchPayloadKey {
                patch_x,
                patch_z,
                render_step_mm,
            });
        }
        requests
    }

    pub(in crate::nodes::simulation_node) fn water_patch_payload_keys_from_flat(
        flat_requests: &PackedInt32Array,
    ) -> Vec<WaterPatchPayloadKey> {
        let request_values = flat_requests.as_slice();
        if request_values.len() < 2 {
            return Vec::new();
        }
        let mut requests = Vec::new();
        for chunk in request_values.chunks_exact(2) {
            let Ok(patch_x) = usize::try_from(chunk[0]) else {
                continue;
            };
            let Ok(patch_z) = usize::try_from(chunk[1]) else {
                continue;
            };
            requests.push(WaterPatchPayloadKey { patch_x, patch_z });
        }
        requests
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_build_source_for_request(
        core: &mut SimCore,
        request: TerrainPatchPayloadRequest,
    ) -> Option<TerrainPatchPayloadBuildSource> {
        if core.terrain_payload_generation_for_patch(request.key.patch_x, request.key.patch_z)
            != request.surface_generation
        {
            return None;
        }
        if request.key.render_step_mm == 0 {
            if core.terrain_patch_requires_engineered_refinement(
                request.key.patch_x,
                request.key.patch_z,
            ) {
                return None;
            }
            let patch = core
                .heightmap
                .visual_patch_snapshot(request.key.patch_x, request.key.patch_z)?;
            let height_bytes = Self::f32_bytes_vec(&patch.height_data);
            return Some(TerrainPatchPayloadBuildSource::Ready(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data: TerrainPatchPayloadData::Regular {
                    patch,
                    height_bytes,
                },
            }));
        }

        let cache_key = RefinedTerrainPatchCacheKey {
            patch_x: request.key.patch_x,
            patch_z: request.key.patch_z,
            render_step_mm: request.key.render_step_mm,
        };
        if core
            .refined_terrain_patch_cache
            .get(&cache_key)
            .is_some_and(|cached| {
                !Self::refined_terrain_patch_cache_entry_is_current(
                    cached,
                    request.surface_generation,
                )
            })
        {
            core.refined_terrain_patch_cache.remove(&cache_key);
        }
        if let Some(cached) = core.refined_terrain_patch_cache.get(&cache_key) {
            return Some(TerrainPatchPayloadBuildSource::Ready(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data: TerrainPatchPayloadData::Refined {
                    patch: cached.clone(),
                },
            }));
        }

        let render_step_m = (request.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON);
        let base_patch = core
            .heightmap
            .visual_patch_snapshot(request.key.patch_x, request.key.patch_z)?;
        let site_margin_m = crate::simulation::terrain::terrain_cdt_local_sample_margin_m(
            &core.heightmap,
            render_step_m,
        );
        let request_key = (request.key.patch_x, request.key.patch_z);
        let engineered_margin = core
            .engineered_terrain_patch_margins
            .get(&request_key)
            .copied();
        let requires_engineered_refinement = engineered_margin.is_some();
        let requires_road_clipping = core
            .road_locked_terrain_patch_margins
            .contains_key(&request_key);
        let engineered_query_margin_m = engineered_margin
            .unwrap_or(site_margin_m)
            .max(site_margin_m);
        let road_locked_margin_m = if requires_road_clipping {
            crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                &core.heightmap,
                render_step_m,
                engineered_query_margin_m,
            )
        } else {
            engineered_query_margin_m
        };
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let sites = core.allocator.terrain_site_snapshot_for_world_bounds(
            base_patch.world_origin_x - road_locked_margin_m,
            base_patch.world_origin_z - road_locked_margin_m,
            base_patch.world_origin_x + base_patch.world_size_x + road_locked_margin_m,
            base_patch.world_origin_z + base_patch.world_size_z + road_locked_margin_m,
        );
        Some(TerrainPatchPayloadBuildSource::Refined(
            RefinedTerrainPatchBuildSource {
                request,
                patch: base_patch,
                requires_engineered_refinement,
                requires_road_clipping,
                road_locked_margin_m,
                sites,
            },
        ))
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_build_job_for_source(
        query: &RoadToolQuerySnapshot,
        source: TerrainPatchPayloadBuildSource,
    ) -> TerrainPatchPayloadBuildJob {
        let source = match source {
            TerrainPatchPayloadBuildSource::Ready(payload) => {
                return TerrainPatchPayloadBuildJob::Ready(payload);
            }
            TerrainPatchPayloadBuildSource::Refined(source) => source,
        };
        let request = source.request;
        let render_step_m = (request.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON);
        let road_clip_query = Self::road_clip_loop_query_for_snapshot(
            query.region_graph.as_ref(),
            query.road_surface.as_ref(),
            &source.sites,
            source.patch.world_origin_x - source.road_locked_margin_m,
            source.patch.world_origin_z - source.road_locked_margin_m,
            source.patch.world_origin_x + source.patch.world_size_x + source.road_locked_margin_m,
            source.patch.world_origin_z + source.patch.world_size_z + source.road_locked_margin_m,
        );
        let windows = Self::terrain_cdt_window_build_inputs(
            query.terrain.as_ref(),
            &source.patch,
            &road_clip_query.cdt_road_loops,
            render_step_m,
            Some(TerrainCdtSiteGradingContext {
                source: TerrainCdtSiteGradingSource::Snapshot(&source.sites),
                graph: query.region_graph.as_ref(),
                road_surface: query.road_surface.as_ref(),
            }),
            None,
        );
        let requires_road_clipping = Self::road_clip_query_requires_road_clipping(
            &road_clip_query,
            source.requires_road_clipping,
        );
        TerrainPatchPayloadBuildJob::Refined {
            request,
            input: RefinedTerrainPatchBuildInput {
                key: RefinedTerrainPatchCacheKey {
                    patch_x: request.key.patch_x,
                    patch_z: request.key.patch_z,
                    render_step_mm: request.key.render_step_mm,
                },
                surface_generation: request.surface_generation,
                patch: source.patch,
                windows,
                requires_engineered_refinement: source.requires_engineered_refinement,
                requires_road_clipping,
                clip_source_count: road_clip_query.source_count,
                road_clip_source_count: road_clip_query.road_source_count,
                road_clip_loop_count: road_clip_query.road_loop_count,
                site_clip_loop_count: road_clip_query.site_loop_count,
                clip_error_label: road_clip_query.clip_error_label,
            },
        }
    }

    pub(in crate::nodes::simulation_node) fn split_terrain_patch_payload_jobs(
        jobs: Vec<TerrainPatchPayloadBuildJob>,
    ) -> (
        Vec<TerrainPatchPayload>,
        Vec<TerrainPatchPayloadRequest>,
        Vec<RefinedTerrainPatchBuildInput>,
    ) {
        let mut ready = Vec::new();
        let mut refined_requests = Vec::new();
        let mut refined_inputs = Vec::new();
        for job in jobs {
            match job {
                TerrainPatchPayloadBuildJob::Ready(payload) => ready.push(payload),
                TerrainPatchPayloadBuildJob::Refined { request, input } => {
                    refined_requests.push(request);
                    refined_inputs.push(input);
                }
            }
        }
        (ready, refined_requests, refined_inputs)
    }

    pub(in crate::nodes::simulation_node) fn append_refined_terrain_patch_payloads_for_requests(
        built: &mut Vec<TerrainPatchPayload>,
        refined_requests: &[TerrainPatchPayloadRequest],
        refined_entries: &[CachedRefinedTerrainPatch],
    ) {
        let entries_by_key = refined_entries
            .iter()
            .map(|entry| (entry.key, entry))
            .collect::<HashMap<_, _>>();
        for request in refined_requests {
            let cache_key = RefinedTerrainPatchCacheKey {
                patch_x: request.key.patch_x,
                patch_z: request.key.patch_z,
                render_step_mm: request.key.render_step_mm,
            };
            let Some(entry) = entries_by_key.get(&cache_key) else {
                continue;
            };
            built.push(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data: TerrainPatchPayloadData::Refined {
                    patch: (**entry).clone(),
                },
            });
        }
    }

    pub(in crate::nodes::simulation_node) fn water_patch_payload_for_request(
        core: &SimCore,
        request: WaterPatchPayloadRequest,
    ) -> Option<WaterPatchPayload> {
        if core.watermap.render_generation() != request.source_generation {
            return None;
        }
        if core.road_tool_surface_generation != request.surface_generation {
            return None;
        }
        let patch = core
            .watermap
            .visible_patch_snapshot(request.key.patch_x, request.key.patch_z)?;
        let road_clip_query = Self::road_clip_loop_query_for_bounds(
            core,
            patch.world_origin_x,
            patch.world_origin_z,
            patch.world_origin_x + patch.world_size_x,
            patch.world_origin_z + patch.world_size_z,
        );
        let depth_bytes = Self::f32_bytes_vec(&patch.depth_data);
        Some(WaterPatchPayload {
            key: request.key,
            request_id: request.request_id,
            source_generation: request.source_generation,
            surface_generation: request.surface_generation,
            patch,
            depth_bytes,
            road_clip_query,
        })
    }

    pub(in crate::nodes::simulation_node) fn water_patch_mesh_build_input_for_request(
        core: &SimCore,
        patch_x: usize,
        patch_z: usize,
        lod_step: usize,
    ) -> Option<(WaterPatchMeshCacheKey, WaterPatchMeshBuildInput)> {
        let patch = core.watermap.visible_patch_snapshot(patch_x, patch_z)?;
        let road_clip_query = Self::road_clip_loop_query_for_bounds(
            core,
            patch.world_origin_x,
            patch.world_origin_z,
            patch.world_origin_x + patch.world_size_x,
            patch.world_origin_z + patch.world_size_z,
        );
        let key = WaterPatchMeshCacheKey {
            patch_x,
            patch_z,
            lod_step,
            road_clip_signature: Self::road_clip_query_signature(&road_clip_query),
            depth_signature: water_patch_depth_signature(&patch),
        };
        Some((
            key,
            WaterPatchMeshBuildInput {
                key,
                patch,
                road_clip_loops: road_clip_query.cdt_road_loops,
                clip_failed: road_clip_query.clip_error_label.is_some(),
            },
        ))
    }
}
