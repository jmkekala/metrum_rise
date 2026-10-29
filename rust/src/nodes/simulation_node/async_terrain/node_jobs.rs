// SPDX-License-Identifier: GPL-2.0-only

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
        // Patches noted since the last publication reach the road tool's terrain inputs now,
        // not at the next edit's publication (`ROAD-68-E`). `O(noted patches)`, never blocking
        // on the core.
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
        // Workers must not wait for SimCore: its owner may be waiting on Rayon.
        let Some(_core) = Self::try_lock_shared_core(core) else {
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
        (completed, failed)
    }

    pub(in crate::nodes::simulation_node) fn try_prepare_terrain_patch_payload_sources(
        core: &Mutex<SimCore>,
        requests: Vec<TerrainPatchPayloadRequest>,
    ) -> (
        Vec<TerrainPatchPayload>,
        Vec<TerrainPatchPayloadRequest>,
        Vec<(TerrainPatchPayloadRequest, &'static str)>,
    ) {
        let Some(mut core) = Self::try_lock_shared_core(core) else {
            return (Vec::new(), requests, Vec::new());
        };
        let mut sources = Vec::with_capacity(requests.len());
        let (mut failed, mut refused) = (Vec::new(), Vec::new());
        for request in requests {
            match Self::terrain_patch_payload_for_request(&mut core, request) {
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

    pub(in crate::nodes::simulation_node) fn terrain_patch_payload_for_request(
        core: &mut SimCore,
        request: TerrainPatchPayloadRequest,
    ) -> Result<TerrainPatchPayload, PayloadDeferral> {
        if core.terrain_payload_generation_for_patch(request.key.patch_x, request.key.patch_z)
            != request.surface_generation
        {
            return Err(PayloadDeferral::Retry);
        }
        if request.key.render_step_mm != 0 {
            return Err(PayloadDeferral::Refused("unsupported_terrain_payload_step"));
        }
        let patch = core
            .heightmap
            .source_patch_snapshot(request.key.patch_x, request.key.patch_z)
            .ok_or(PayloadDeferral::Refused("patch_outside_world"))?;
        let height_bytes = Self::f32_bytes_vec(&patch.height_data);
        Ok(TerrainPatchPayload {
            key: request.key,
            request_id: request.request_id,
            surface_generation: request.surface_generation,
            data: TerrainPatchPayloadData::Regular {
                patch,
                height_bytes,
            },
            mask_bytes: Vec::new(),
        })
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
        let depth_bytes = Self::f32_bytes_vec(&patch.depth_data);
        Some(WaterPatchPayload {
            key: request.key,
            request_id: request.request_id,
            source_generation: request.source_generation,
            surface_generation: request.surface_generation,
            cover_generation: core
                .terrain_payload_generation_for_patch(request.key.patch_x, request.key.patch_z),
            patch,
            depth_bytes,
        })
    }

    // The cache key of a water patch's mesh at `lod_step`, with the patch it is built from:
    // the patch's depth and its terrain payload generation, which every road or site change
    // that could drop one of its cells bumps (`docs/terrain.md` T32 (a)). None until the
    // roadbed is current.
    pub(in crate::nodes::simulation_node) fn water_patch_mesh_key_for_request(
        core: &SimCore,
        patch_x: usize,
        patch_z: usize,
        lod_step: usize,
    ) -> Option<(WaterPatchMeshCacheKey, WaterPatchSnapshot)> {
        if !core.transit_network.roadbed.is_current() {
            return None;
        }
        let patch = core.watermap.visible_patch_snapshot(patch_x, patch_z)?;
        let key = WaterPatchMeshCacheKey {
            patch_x,
            patch_z,
            lod_step,
            cover_generation: core.terrain_payload_generation_for_patch(patch_x, patch_z),
            depth_signature: water_patch_depth_signature(&patch),
        };
        Some((key, patch))
    }

    // The input of a mesh the cache misses: the cells the roads and sites drop, decided under
    // the lock, so the builder off it reads no core. A dry patch drops none. The site index
    // must be ready.
    pub(in crate::nodes::simulation_node) fn water_patch_mesh_build_input(
        core: &SimCore,
        key: WaterPatchMeshCacheKey,
        patch: WaterPatchSnapshot,
    ) -> WaterPatchMeshBuildInput {
        let dropped = if patch.depth_nonzero_count == 0 {
            Vec::new()
        } else {
            core.water_cells_under_roads_and_pads(&patch, key.lod_step)
        };
        WaterPatchMeshBuildInput {
            key,
            patch,
            dropped,
        }
    }
}
