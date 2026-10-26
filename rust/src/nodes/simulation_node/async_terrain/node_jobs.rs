// SPDX-License-Identifier: GPL-2.0-only

//! `SimulationNode` helpers for async terrain and water jobs.

use super::super::*;
use super::state::*;
use crate::nodes::sim::core::{
    ROAD_LOCKED_TERRAIN_RENDER_STEP_M, ROAD_PREVIEW_TERRAIN_GENERATION,
    RefinedTerrainAssemblyLedger, RefinedTerrainAssemblyScope, RoadPreviewTerrain,
    RoadPreviewTerrainPatch, RoadTerrainInputs, RoadTerrainPatchInputs, RoadTerrainSourceSignature,
};
use std::collections::BTreeSet;

// What one patch's refined source reads besides the heightmap and the site query.
pub(in crate::nodes::simulation_node) struct RefinedTerrainSourceInputs<'a> {
    request: TerrainPatchPayloadRequest,
    patch: TerrainPatchSnapshot,
    previous: Option<Arc<CachedRefinedTerrainPatch>>,
    global_generation: u64,
    ledger: Option<&'a RefinedTerrainAssemblyLedger>,
    engineered_margin_m: Option<f32>,
    road_locked: bool,
    road_surface_generation: u64,
}

impl SimulationNode {
    /// Builds the actual renderer products for the edit's affected patches before acceptance.
    /// Work is limited to those patches and their dirty 64 m windows; no world snapshot is copied.
    /// When `preview` built the same sources (the same signatures for the same patches), its
    /// entries are adopted, re-stamped with the patches' generations, instead of rebuilt
    /// (`ROAD-68-E`); the second value tells which happened. The caller passes a preview only
    /// when it installed the preview's pieces into this core.
    pub(crate) fn validate_staged_road_terrain(
        core: &mut SimCore,
        patch_keys: &[(usize, usize)],
        preview: Option<&RoadPreviewTerrain>,
    ) -> Result<(Vec<Arc<CachedRefinedTerrainPatch>>, bool), String> {
        core.refresh_engineered_terrain_patch_ownership_for_keys(
            ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
            patch_keys,
        );
        let mut sources = Vec::with_capacity(patch_keys.len());
        let mut signatures = Vec::with_capacity(patch_keys.len());
        for &(patch_x, patch_z) in patch_keys {
            if !core.terrain_patch_requires_engineered_refinement(patch_x, patch_z) {
                signatures.push(((patch_x, patch_z), None));
                continue;
            }
            let request = TerrainPatchPayloadRequest {
                key: TerrainPatchPayloadKey {
                    patch_x,
                    patch_z,
                    render_step_mm: 2000,
                },
                request_id: 0,
                surface_generation: core.terrain_payload_generation_for_patch(patch_x, patch_z),
            };
            let source = Self::terrain_patch_payload_build_source_for_request(core, request)
                .map_err(|_| format!("terrain_input_unavailable patch=({patch_x},{patch_z})"))?;
            let signature = match &source {
                TerrainPatchPayloadBuildSource::Refined(source) => {
                    Some(Self::road_terrain_source_signature(source))
                }
                TerrainPatchPayloadBuildSource::Ready(_) => None,
            };
            signatures.push(((patch_x, patch_z), signature));
            sources.push(source);
        }
        if let Some(preview) = preview
            && preview.patches.len() == signatures.len()
            && preview
                .patches
                .iter()
                .zip(&signatures)
                .all(|(patch, (key, signature))| patch.key == *key && patch.source == *signature)
        {
            let entries = preview
                .patches
                .iter()
                .filter_map(|patch| patch.entry.as_ref())
                .map(|entry| {
                    let generation = core
                        .terrain_payload_generation_for_patch(entry.key.patch_x, entry.key.patch_z);
                    Arc::new(CachedRefinedTerrainPatch {
                        surface_generation: generation,
                        ..CachedRefinedTerrainPatch::clone(entry)
                    })
                })
                .collect();
            return Ok((entries, true));
        }
        let entries = Self::build_road_terrain_sources(
            &core.region_graph,
            &core.transit_network.roadbed,
            &core.heightmap,
            core.road_tool_surface_generation,
            sources,
        )?;
        Ok((entries, false))
    }

    // Builds the sources' refined entries in parallel (the caller is inside the shared pool)
    // and fails on the first typed CDT failure, as an edit must.
    fn build_road_terrain_sources(
        graph: &crate::simulation::network::graph::RegionGraph,
        roadbed: &crate::simulation::network::roadbed::RoadbedSystem,
        terrain: &TerrainSystem,
        road_surface_generation: u64,
        sources: Vec<TerrainPatchPayloadBuildSource>,
    ) -> Result<Vec<Arc<CachedRefinedTerrainPatch>>, String> {
        let jobs = sources
            .into_par_iter()
            .map(|source| {
                Self::terrain_patch_payload_build_job_for_world(
                    graph,
                    roadbed,
                    terrain,
                    road_surface_generation,
                    source,
                )
            })
            .collect();
        let (ready, _, inputs, failed) = Self::split_terrain_patch_payload_jobs(jobs);
        if !failed.is_empty() {
            return Err("terrain_input_generation_mismatch".to_owned());
        }
        let mut entries = SimCore::build_refined_terrain_patch_cache_entries(inputs);
        entries.extend(ready.into_iter().filter_map(|payload| match payload.data {
            TerrainPatchPayloadData::Refined { patch } => Some(patch),
            _ => None,
        }));
        for entry in &entries {
            if let Some(reason) = Self::cached_refined_cdt_failure_label(entry) {
                return Err(format!(
                    "{reason} patch=({},{})",
                    entry.key.patch_x, entry.key.patch_z
                ));
            }
        }
        Ok(entries)
    }

    fn road_terrain_source_signature(
        source: &RefinedTerrainPatchBuildSource,
    ) -> RoadTerrainSourceSignature {
        RoadTerrainSourceSignature {
            previous: source.previous.clone(),
            scope: match &source.assembly_scope {
                RefinedTerrainAssemblyScope::FullPatch => None,
                RefinedTerrainAssemblyScope::LocalTiles(tiles) => Some(tiles.clone()),
            },
            requires_engineered_refinement: source.requires_engineered_refinement,
            requires_road_clipping: source.requires_road_clipping,
            road_locked_margin_bits: source.road_locked_margin_m.to_bits(),
            sites: source.sites.clone(),
        }
    }

    /// The preview's terrain stage (`ROAD-68-E`): the patches and tiles `tiles` names (what a
    /// commit of the same pieces stamps), built from the road tool's terrain inputs, the edited
    /// `graph` and the preview's `roadbed` through the commit's ownership, source and build
    /// functions, in parallel on the shared pool. Entries carry
    /// [`ROAD_PREVIEW_TERRAIN_GENERATION`]. Fails as the commit's validation would.
    /// `O(touched patches * their dirty tiles' CDT)`, independent of the city.
    pub(crate) fn build_road_preview_terrain(
        terrain: &TerrainSystem,
        inputs: &RoadTerrainInputs,
        graph: &crate::simulation::network::graph::RegionGraph,
        roadbed: &crate::simulation::network::roadbed::RoadbedSystem,
        road_surface_generation: u64,
        tiles: &std::collections::BTreeMap<(usize, usize), BTreeSet<(i64, i64)>>,
    ) -> Result<RoadPreviewTerrain, String> {
        if inputs.stroke_active || !roadbed.is_current() {
            return Err("terrain_input_unavailable".to_owned());
        }
        crate::thread_pools::install(|| {
            Self::build_road_preview_terrain_on_pool(
                terrain,
                inputs,
                graph,
                roadbed,
                road_surface_generation,
                tiles,
            )
        })
    }

    // The preview's terrain stage inside the shared pool: the patches' road margins grade in
    // parallel (an indexed collect keeps them in key order), the sources follow in key order,
    // then the patches build in parallel.
    fn build_road_preview_terrain_on_pool(
        terrain: &TerrainSystem,
        inputs: &RoadTerrainInputs,
        graph: &crate::simulation::network::graph::RegionGraph,
        roadbed: &crate::simulation::network::roadbed::RoadbedSystem,
        road_surface_generation: u64,
        tiles: &std::collections::BTreeMap<(usize, usize), BTreeSet<(i64, i64)>>,
    ) -> Result<RoadPreviewTerrain, String> {
        let step_m = ROAD_LOCKED_TERRAIN_RENDER_STEP_M;
        let site_margin_m =
            crate::simulation::terrain::terrain_cdt_local_sample_margin_m(terrain, step_m);
        let none = RoadTerrainPatchInputs::default();
        let keys: Vec<(usize, usize)> = tiles.keys().copied().collect();
        let road_margins: Vec<Option<f32>> = keys
            .par_iter()
            .map(|&key| {
                SimCore::road_terrain_patch_margin_m(terrain, (graph, roadbed), key, step_m)
            })
            .collect();
        let mut patches = Vec::with_capacity(tiles.len());
        let mut sources = Vec::with_capacity(tiles.len());
        for ((&key, patch_tiles), road_margin_m) in tiles.iter().zip(road_margins) {
            let patch_inputs = inputs.patch(key).unwrap_or(&none);
            let engineered_margin_m = SimCore::engineered_terrain_margin_m(
                road_margin_m,
                patch_inputs.site_owned,
                site_margin_m,
            );
            if engineered_margin_m.is_none() {
                patches.push(RoadPreviewTerrainPatch {
                    key,
                    source: None,
                    entry: None,
                });
                continue;
            }
            let patch = terrain
                .visual_patch_snapshot(key.0, key.1)
                .ok_or_else(|| format!("terrain_input_unavailable patch=({},{})", key.0, key.1))?;
            // The ledger as the commit's stamps leave it.
            let mut ledger = patch_inputs.ledger.clone().unwrap_or_default();
            for &tile in patch_tiles {
                ledger
                    .road_tile_dirty_at
                    .insert(tile, ROAD_PREVIEW_TERRAIN_GENERATION);
            }
            let request = TerrainPatchPayloadRequest {
                key: TerrainPatchPayloadKey {
                    patch_x: key.0,
                    patch_z: key.1,
                    render_step_mm: Self::refined_patch_cache_key(key.0, key.1, step_m)
                        .render_step_mm,
                },
                request_id: 0,
                surface_generation: ROAD_PREVIEW_TERRAIN_GENERATION,
            };
            let source = Self::refined_terrain_patch_source(
                terrain,
                RefinedTerrainSourceInputs {
                    request,
                    patch,
                    previous: patch_inputs.entry.clone(),
                    global_generation: inputs.global_generation,
                    ledger: Some(&ledger),
                    engineered_margin_m,
                    road_locked: road_margin_m.is_some(),
                    road_surface_generation,
                },
                |min_x, min_z, max_x, max_z| {
                    patch_inputs.sites.within_bounds(min_x, min_z, max_x, max_z)
                },
            );
            patches.push(RoadPreviewTerrainPatch {
                key,
                source: Some(Self::road_terrain_source_signature(&source)),
                entry: None,
            });
            sources.push(TerrainPatchPayloadBuildSource::Refined(source));
        }
        let entries = Self::build_road_terrain_sources(
            graph,
            roadbed,
            terrain,
            road_surface_generation,
            sources,
        )?;
        for entry in entries {
            let key = (entry.key.patch_x, entry.key.patch_z);
            if let Some(patch) = patches.iter_mut().find(|patch| patch.key == key) {
                patch.entry = Some(entry);
            }
        }
        Ok(RoadPreviewTerrain { patches })
    }

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

    pub(in crate::nodes::simulation_node) fn clear_runtime_render_async_jobs(&mut self) {
        self.building_lods.invalidate_world();
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
        let drained =
            Self::finish_terrain_patch_payload_jobs(&self.core, &self.terrain_patch_payload_jobs);
        // The entries just inserted reach the road tool's terrain inputs now, not at the next
        // edit's publication (`ROAD-68-E`). `O(noted patches)`, never blocking on the core.
        if !drained.0.is_empty()
            && let Some(mut core) = Self::try_lock_shared_core(&self.core)
            && core.road_terrain_inputs.has_changes()
            && core.transit_network.roadbed.is_current()
        {
            let inputs = core.refresh_road_terrain_inputs();
            drop(core);
            self.road_tool.publish_terrain_inputs(inputs);
        }
        drained
    }

    pub(in crate::nodes::simulation_node) fn finish_terrain_patch_payload_jobs(
        core: &Mutex<SimCore>,
        jobs: &Mutex<TerrainPatchPayloadAsyncState>,
    ) -> (Vec<TerrainPatchPayload>, Vec<TerrainPatchPayloadRequest>) {
        // Workers must not wait for SimCore: its owner may be waiting on Rayon. Retain completed
        // payloads until polling can insert their reusable geometry without blocking either thread.
        let Some(mut core) = Self::try_lock_shared_core(core) else {
            return (Vec::new(), Vec::new());
        };
        let (completed, failed) = {
            let mut jobs = jobs
                .lock()
                .expect("terrain patch payload job lock poisoned");
            (
                std::mem::take(&mut jobs.completed),
                std::mem::take(&mut jobs.failed),
            )
        };
        let entries = completed
            .iter()
            .filter_map(|payload| match &payload.data {
                TerrainPatchPayloadData::Refined { patch } => Some(Arc::clone(patch)),
                _ => None,
            })
            .collect();
        core.insert_refined_terrain_patch_cache_entries(entries);
        (completed, failed)
    }

    pub(in crate::nodes::simulation_node) fn try_prepare_terrain_patch_payload_sources(
        core: &Mutex<SimCore>,
        requests: Vec<TerrainPatchPayloadRequest>,
    ) -> (
        Vec<TerrainPatchPayloadBuildSource>,
        Vec<TerrainPatchPayloadRequest>,
        Vec<(TerrainPatchPayloadRequest, &'static str)>,
    ) {
        let Some(mut core) = Self::try_lock_shared_core(core) else {
            return (Vec::new(), requests, Vec::new());
        };
        let mut sources = Vec::with_capacity(requests.len());
        let (mut failed, mut refused) = (Vec::new(), Vec::new());
        for request in requests {
            match Self::terrain_patch_payload_build_source_for_request(&mut core, request) {
                Ok(source) => sources.push(source),
                Err(PayloadDeferral::Retry) => failed.push(request),
                Err(PayloadDeferral::Refused(reason)) => refused.push((request, reason)),
            }
        }
        (sources, failed, refused)
    }

    pub(in crate::nodes::simulation_node) fn try_prepare_water_patch_payloads(
        core: &Mutex<SimCore>,
        requests: Vec<WaterPatchPayloadRequest>,
    ) -> (Vec<WaterPatchPayload>, Vec<WaterPatchPayloadRequest>) {
        let Some(core) = Self::try_lock_shared_core(core) else {
            return (Vec::new(), requests);
        };
        let mut built = Vec::with_capacity(requests.len());
        let mut failed = Vec::new();
        for request in requests {
            match Self::water_patch_payload_for_request(&core, request) {
                Some(payload) => built.push(payload),
                None => failed.push(request),
            }
        }
        (built, failed)
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
        for chunk in request_values.as_chunks::<3>().0 {
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
        for chunk in request_values.as_chunks::<3>().0 {
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
        for chunk in request_values.as_chunks::<2>().0 {
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
    ) -> Result<TerrainPatchPayloadBuildSource, PayloadDeferral> {
        if core.terrain_payload_generation_for_patch(request.key.patch_x, request.key.patch_z)
            != request.surface_generation
        {
            return Err(PayloadDeferral::Retry);
        }
        // The plain payload (step 0) is the patch's heightmap, served for every patch: the graded
        // tiles (`TERRAIN-07-D` item 4) draw the roads' and sites' terrain over it, road-owned
        // patches included. The exact cut asks road-owned patches at the refined step.
        if request.key.render_step_mm == 0 {
            let patch = core
                .heightmap
                .visual_patch_snapshot(request.key.patch_x, request.key.patch_z)
            .ok_or(PayloadDeferral::Refused("patch_outside_world"))?;
            let height_bytes = Self::f32_bytes_vec(&patch.height_data);
            return Ok(TerrainPatchPayloadBuildSource::Ready(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data: TerrainPatchPayloadData::Regular {
                    patch,
                    height_bytes,
                },
                mask_bytes: Vec::new(),
            }));
        }
        if !core.transit_network.roadbed.is_current() {
            return Err(PayloadDeferral::Retry);
        }
        if core.terrain_stroke_active {
            // Refined inputs sample the immutable road-query terrain snapshot. A batched stroke
            // publishes that snapshot only at finalization, so intermediate requests must retry.
            return Err(PayloadDeferral::Retry);
        }

        let cache_key = RefinedTerrainPatchCacheKey {
            patch_x: request.key.patch_x,
            patch_z: request.key.patch_z,
            render_step_mm: request.key.render_step_mm,
        };
        if let Some(cached) = core.refined_terrain_patch_cache.get(&cache_key)
            && Self::refined_terrain_patch_cache_entry_is_current(
                cached,
                request.surface_generation,
            )
            && Self::cached_refined_cdt_failure_label(cached).is_none()
        {
            return Ok(TerrainPatchPayloadBuildSource::Ready(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data: TerrainPatchPayloadData::Refined {
                    patch: cached.clone(),
                },
                mask_bytes: Vec::new(),
            }));
        }
        let previous = match core.refined_terrain_patch_cache.get(&cache_key) {
            Some(cached) if cached.contract_revision == TERRAIN_CDT_CONTRACT_REVISION => {
                Some(cached.clone())
            }
            Some(_) => {
                core.refined_terrain_patch_cache.remove(&cache_key);
                core.road_terrain_inputs
                    .note([(request.key.patch_x, request.key.patch_z)]);
                None
            }
            None => None,
        };
        let base_patch = core
            .heightmap
            .visual_patch_snapshot(request.key.patch_x, request.key.patch_z)
            .ok_or(PayloadDeferral::Refused("patch_outside_world"))?;
        let request_key = (request.key.patch_x, request.key.patch_z);
        core.allocator
            .prepare_building_site_query_index(core.config.zone_cell_m);
        let allocator = &core.allocator;
        Ok(TerrainPatchPayloadBuildSource::Refined(
            Self::refined_terrain_patch_source(
                &core.heightmap,
                RefinedTerrainSourceInputs {
                    request,
                    patch: base_patch,
                    previous,
                    global_generation: core.terrain_payload_global_generation,
                    ledger: core.refined_terrain_assembly_ledgers.get(&request_key),
                    engineered_margin_m: core
                        .engineered_terrain_patch_margins
                        .get(&request_key)
                        .copied(),
                    road_locked: core
                        .road_locked_terrain_patch_margins
                        .contains_key(&request_key),
                    road_surface_generation: core.road_tool_surface_generation,
                },
                |min_x, min_z, max_x, max_z| {
                    allocator.terrain_site_snapshot_for_world_bounds(min_x, min_z, max_x, max_z)
                },
            ),
        ))
    }

    // The refined source of one patch from what it reads: the one function the core's requests,
    // the commit's staged validation and the preview share (`ROAD-68-E`), so margins, assembly
    // scope and site bounds cannot differ between them. `sites` answers a world-bounds query.
    fn refined_terrain_patch_source(
        terrain: &TerrainSystem,
        inputs: RefinedTerrainSourceInputs<'_>,
        sites: impl FnOnce(f32, f32, f32, f32) -> BuildingSiteTerrainSnapshot,
    ) -> RefinedTerrainPatchBuildSource {
        let RefinedTerrainSourceInputs {
            request,
            patch: base_patch,
            previous,
            global_generation,
            ledger,
            engineered_margin_m,
            road_locked: requires_road_clipping,
            road_surface_generation,
        } = inputs;
        let render_step_m = (request.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON);
        let site_margin_m =
            crate::simulation::terrain::terrain_cdt_local_sample_margin_m(terrain, render_step_m);
        let requires_engineered_refinement = engineered_margin_m.is_some();
        let engineered_query_margin_m = engineered_margin_m
            .unwrap_or(site_margin_m)
            .max(site_margin_m);
        let road_locked_margin_m = if requires_road_clipping {
            crate::simulation::terrain::terrain_cdt_road_query_margin_m(
                terrain,
                render_step_m,
                engineered_query_margin_m,
            )
        } else {
            engineered_query_margin_m
        };
        let assembly_scope = Self::refined_terrain_assembly_scope(
            global_generation,
            ledger,
            &base_patch,
            previous.as_deref(),
            request.surface_generation,
        );
        let sites = sites(
            base_patch.world_origin_x - road_locked_margin_m,
            base_patch.world_origin_z - road_locked_margin_m,
            base_patch.world_origin_x + base_patch.world_size_x + road_locked_margin_m,
            base_patch.world_origin_z + base_patch.world_size_z + road_locked_margin_m,
        );
        RefinedTerrainPatchBuildSource {
            request,
            patch: base_patch,
            previous,
            assembly_scope,
            requires_engineered_refinement,
            requires_road_clipping,
            road_locked_margin_m,
            road_surface_generation,
            sites,
        }
    }

    fn refined_terrain_assembly_scope(
        global_generation: u64,
        ledger: Option<&RefinedTerrainAssemblyLedger>,
        patch: &TerrainPatchSnapshot,
        previous: Option<&CachedRefinedTerrainPatch>,
        surface_generation: u64,
    ) -> RefinedTerrainAssemblyScope {
        let Some(previous) = previous else {
            return RefinedTerrainAssemblyScope::FullPatch;
        };
        if global_generation > previous.surface_generation {
            return RefinedTerrainAssemblyScope::FullPatch;
        }
        let Some(ledger) = ledger else {
            return RefinedTerrainAssemblyScope::FullPatch;
        };
        if ledger
            .full_dirty_at
            .is_some_and(|stamp| stamp > previous.surface_generation && stamp <= surface_generation)
        {
            return RefinedTerrainAssemblyScope::FullPatch;
        }

        // The ledger holds every tile whose halo meets an edit's old or new pieces; the halo
        // also caps the grading envelope (`ROAD-68-D`).
        let mut dirty_tiles = BTreeSet::new();
        let mut has_relevant_road_tiles = false;
        for (&(x, z), &stamp) in &ledger.road_tile_dirty_at {
            if stamp <= previous.surface_generation || stamp > surface_generation {
                continue;
            }
            has_relevant_road_tiles = true;
            let tile_id = TerrainCdtTileId { x, z };
            if Self::terrain_cdt_tile_bounds(patch, tile_id).is_some() {
                dirty_tiles.insert(tile_id);
            }
        }
        if !has_relevant_road_tiles {
            return RefinedTerrainAssemblyScope::FullPatch;
        }
        let changed_tiles = dirty_tiles.iter().copied().collect::<Vec<_>>();
        for tile_id in changed_tiles {
            for (offset_x, offset_z) in TERRAIN_CDT_TILE_NEIGHBORS {
                let neighbor = TerrainCdtTileId {
                    x: tile_id.x + offset_x,
                    z: tile_id.z + offset_z,
                };
                if Self::terrain_cdt_tile_bounds(patch, neighbor).is_some() {
                    dirty_tiles.insert(neighbor);
                }
            }
        }
        RefinedTerrainAssemblyScope::LocalTiles(
            dirty_tiles
                .into_iter()
                .map(|tile_id| (tile_id.x, tile_id.z))
                .collect(),
        )
    }

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_build_job_for_source(
        query: &RoadToolQuerySnapshot,
        source: TerrainPatchPayloadBuildSource,
    ) -> TerrainPatchPayloadBuildJob {
        Self::terrain_patch_payload_build_job_for_world(
            &query.region_graph,
            &query.roadbed,
            &query.terrain,
            query.surface_generation,
            source,
        )
    }

    fn terrain_patch_payload_build_job_for_world(
        graph: &crate::simulation::network::graph::RegionGraph,
        road_surface: &crate::simulation::network::roadbed::RoadbedSystem,
        terrain: &TerrainSystem,
        surface_generation: u64,
        source: TerrainPatchPayloadBuildSource,
    ) -> TerrainPatchPayloadBuildJob {
        let source = match source {
            TerrainPatchPayloadBuildSource::Ready(payload) => {
                return TerrainPatchPayloadBuildJob::Ready(payload);
            }
            TerrainPatchPayloadBuildSource::Refined(source) => source,
        };
        let request = source.request;
        if surface_generation != source.road_surface_generation {
            return TerrainPatchPayloadBuildJob::Failed(request);
        }
        let render_step_m = (request.key.render_step_mm as f32 / 1000.0).max(f32::EPSILON);
        let (window_plan, road_clip_query) = match &source.assembly_scope {
            RefinedTerrainAssemblyScope::FullPatch => {
                let road_clip_query = Self::road_clip_loop_query_for_snapshot(
                    &source.sites,
                    source.patch.world_origin_x - source.road_locked_margin_m,
                    source.patch.world_origin_z - source.road_locked_margin_m,
                    source.patch.world_origin_x
                        + source.patch.world_size_x
                        + source.road_locked_margin_m,
                    source.patch.world_origin_z
                        + source.patch.world_size_z
                        + source.road_locked_margin_m,
                );
                let window_plan = Self::terrain_cdt_window_build_inputs_with_roads(
                    terrain,
                    &source.patch,
                    Some((graph, road_surface)),
                    &road_clip_query.cdt_road_loops,
                    render_step_m,
                    Some(TerrainCdtSiteGradingContext {
                        source: TerrainCdtSiteGradingSource::Snapshot(&source.sites),
                        roads: crate::simulation::buildings::allocator::SiteRoads::new(
                            graph,
                            road_surface,
                        ),
                    }),
                    source.previous.as_deref(),
                );
                (window_plan, road_clip_query)
            }
            RefinedTerrainAssemblyScope::LocalTiles(tile_keys) => {
                let Some(previous) = source.previous.as_ref() else {
                    return TerrainPatchPayloadBuildJob::Failed(request);
                };
                Self::terrain_cdt_incremental_window_build_inputs(
                    terrain,
                    &source.patch,
                    graph,
                    road_surface,
                    &source.sites,
                    tile_keys,
                    render_step_m,
                    source.road_locked_margin_m,
                    previous,
                )
            }
        };
        let requires_road_clipping = Self::road_clip_query_requires_road_clipping(
            &road_clip_query,
            source.requires_road_clipping,
        );
        TerrainPatchPayloadBuildJob::Refined {
            request,
            input: Box::new(RefinedTerrainPatchBuildInput {
                site_surfaces: source.sites.paving_surfaces(),
                site_foundations: source.sites.foundations(),
                key: RefinedTerrainPatchCacheKey {
                    patch_x: request.key.patch_x,
                    patch_z: request.key.patch_z,
                    render_step_mm: request.key.render_step_mm,
                },
                surface_generation: request.surface_generation,
                patch: source.patch,
                previous_patch: source.previous,
                windows: window_plan.windows,
                reused_windows: window_plan.reused_windows,
                input_clip_loop_count: window_plan.represented_road_loop_count,
                omitted_margin_clip_loop_count: window_plan.omitted_margin_loop_count,
                expected_road_clip_fingerprints: window_plan.expected_road_clip_fingerprints,
                expected_site_clip_fingerprints: window_plan.expected_site_clip_fingerprints,
                requires_engineered_refinement: source.requires_engineered_refinement,
                requires_road_clipping,
                clip_source_count: road_clip_query.source_count,
                road_clip_source_count: road_clip_query.road_source_count,
                road_clip_loop_count: road_clip_query.road_loop_count,
                site_clip_loop_count: road_clip_query.site_loop_count,
                clip_error_label: road_clip_query
                    .clip_error_label
                    .or(window_plan.road_error_label),
                clip_query_margin_m: source.road_locked_margin_m,
                // Road loops are taken per tile, so the windows' fingerprints are the counts.
                derive_clip_counts_from_windows: true,
            }),
        }
    }

    pub(in crate::nodes::simulation_node) fn split_terrain_patch_payload_jobs(
        jobs: Vec<TerrainPatchPayloadBuildJob>,
    ) -> (
        Vec<TerrainPatchPayload>,
        Vec<TerrainPatchPayloadRequest>,
        Vec<RefinedTerrainPatchBuildInput>,
        Vec<TerrainPatchPayloadRequest>,
    ) {
        let mut ready = Vec::new();
        let mut refined_requests = Vec::new();
        let mut refined_inputs = Vec::new();
        let mut failed = Vec::new();
        for job in jobs {
            match job {
                TerrainPatchPayloadBuildJob::Ready(payload) => ready.push(payload),
                TerrainPatchPayloadBuildJob::Failed(request) => failed.push(request),
                TerrainPatchPayloadBuildJob::Refined { request, input } => {
                    refined_requests.push(request);
                    refined_inputs.push(*input);
                }
            }
        }
        (ready, refined_requests, refined_inputs, failed)
    }

    pub(in crate::nodes::simulation_node) fn append_refined_terrain_patch_payloads_for_requests(
        built: &mut Vec<TerrainPatchPayload>,
        refined_requests: &[TerrainPatchPayloadRequest],
        refined_entries: &[Arc<CachedRefinedTerrainPatch>],
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
            let data = if let Some(error_label) = Self::cached_refined_cdt_failure_label(entry) {
                TerrainPatchPayloadData::RefinedFailure {
                    patch: Arc::clone(entry),
                    error_label,
                }
            } else {
                TerrainPatchPayloadData::Refined {
                    patch: Arc::clone(entry),
                }
            };
            built.push(TerrainPatchPayload {
                key: request.key,
                request_id: request.request_id,
                surface_generation: request.surface_generation,
                data,
                mask_bytes: Vec::new(),
            });
        }
    }

    /// Returns refined requests whose worker produced no current entry at all.
    pub(in crate::nodes::simulation_node) fn refined_requests_without_entries(
        refined_requests: &[TerrainPatchPayloadRequest],
        refined_entries: &[Arc<CachedRefinedTerrainPatch>],
    ) -> Vec<TerrainPatchPayloadRequest> {
        let produced_refined_keys = refined_entries
            .iter()
            .map(|entry| entry.key)
            .collect::<HashSet<_>>();
        refined_requests
            .iter()
            .copied()
            .filter(|request| {
                !produced_refined_keys.contains(&RefinedTerrainPatchCacheKey {
                    patch_x: request.key.patch_x,
                    patch_z: request.key.patch_z,
                    render_step_mm: request.key.render_step_mm,
                })
            })
            .collect()
    }

    pub(in crate::nodes::simulation_node) fn water_patch_payload_for_request(
        core: &SimCore,
        request: WaterPatchPayloadRequest,
    ) -> Option<WaterPatchPayload> {
        if core.watermap.render_generation() != request.source_generation {
            return None;
        }
        if core.cached_road_mesh_generation != request.surface_generation {
            return None;
        }
        if !core.transit_network.roadbed.is_current() {
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
        if !core.transit_network.roadbed.is_current() {
            return None;
        }
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
