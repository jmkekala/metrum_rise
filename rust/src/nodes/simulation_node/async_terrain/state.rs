//! Async terrain and water request state types.

use super::super::*;

/// Async water mesh preparation state shared with Rayon jobs.
pub(crate) struct WaterPatchMeshAsyncState {
    /// Cache keys that are already being built by Rayon workers.
    pub(crate) in_flight: HashSet<WaterPatchMeshCacheKey>,
    /// Latest mesh variants requested by Godot but not yet emitted back.
    pub(crate) requested: HashSet<WaterPatchMeshCacheKey>,
    /// Ready mesh variants that Godot can poll without resubmitting request keys.
    pub(crate) ready: Vec<WaterPatchMeshCacheKey>,
    /// Deduplication set for `ready`.
    pub(crate) ready_lookup: HashSet<WaterPatchMeshCacheKey>,
    /// Completed meshes waiting to be inserted into the render cache.
    pub(crate) completed: Vec<CachedWaterPatchMesh>,
    /// Derived meshes owned entirely by the renderer job pipeline.
    pub(crate) cache: HashMap<WaterPatchMeshCacheKey, CachedWaterPatchMesh>,
}

impl WaterPatchMeshAsyncState {
    pub(in crate::nodes::simulation_node) fn new() -> Self {
        Self {
            in_flight: HashSet::new(),
            requested: HashSet::new(),
            ready: Vec::new(),
            ready_lookup: HashSet::new(),
            completed: Vec::new(),
            cache: HashMap::new(),
        }
    }

    pub(in crate::nodes::simulation_node) fn forget_requested_patch(
        &mut self,
        patch_x: usize,
        patch_z: usize,
    ) {
        self.requested
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.ready_lookup
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.ready
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
    }

    pub(in crate::nodes::simulation_node) fn queue_ready(&mut self, key: WaterPatchMeshCacheKey) {
        if self.ready_lookup.insert(key) {
            self.ready.push(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn clear(&mut self) {
        self.in_flight.clear();
        self.requested.clear();
        self.ready.clear();
        self.ready_lookup.clear();
        self.completed.clear();
        self.cache.clear();
    }

    pub(in crate::nodes::simulation_node) fn forget_patch(
        &mut self,
        patch_x: usize,
        patch_z: usize,
    ) {
        self.in_flight
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.requested
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.ready_lookup
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.ready
            .retain(|key| key.patch_x != patch_x || key.patch_z != patch_z);
        self.completed
            .retain(|entry| entry.key.patch_x != patch_x || entry.key.patch_z != patch_z);
        self.cache
            .retain(|key, _| key.patch_x != patch_x || key.patch_z != patch_z);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::nodes::simulation_node) struct TerrainPatchPayloadKey {
    pub(in crate::nodes::simulation_node) patch_x: usize,
    pub(in crate::nodes::simulation_node) patch_z: usize,
    pub(in crate::nodes::simulation_node) render_step_mm: u32,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::nodes::simulation_node) struct WaterPatchPayloadKey {
    pub(in crate::nodes::simulation_node) patch_x: usize,
    pub(in crate::nodes::simulation_node) patch_z: usize,
}

#[derive(Clone, Copy)]
pub(in crate::nodes::simulation_node) struct TerrainPatchPayloadRequest {
    pub(in crate::nodes::simulation_node) key: TerrainPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

#[derive(Clone, Copy)]
pub(in crate::nodes::simulation_node) struct WaterPatchPayloadRequest {
    pub(in crate::nodes::simulation_node) key: WaterPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) source_generation: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

pub(in crate::nodes::simulation_node) enum TerrainPatchPayloadData {
    Regular {
        patch: TerrainPatchSnapshot,
        height_bytes: Vec<u8>,
    },
    Refined {
        patch: CachedRefinedTerrainPatch,
    },
}

pub(in crate::nodes::simulation_node) struct TerrainPatchPayload {
    pub(in crate::nodes::simulation_node) key: TerrainPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
    pub(in crate::nodes::simulation_node) data: TerrainPatchPayloadData,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::nodes::simulation_node) struct TerrainPatchPayloadRequestState {
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

pub(in crate::nodes::simulation_node) enum TerrainPatchPayloadBuildJob {
    Ready(TerrainPatchPayload),
    Refined {
        request: TerrainPatchPayloadRequest,
        input: RefinedTerrainPatchBuildInput,
    },
}

pub(in crate::nodes::simulation_node) enum TerrainPatchPayloadBuildSource {
    Ready(TerrainPatchPayload),
    Refined(RefinedTerrainPatchBuildSource),
}

pub(in crate::nodes::simulation_node) struct RefinedTerrainPatchBuildSource {
    pub(in crate::nodes::simulation_node) request: TerrainPatchPayloadRequest,
    pub(in crate::nodes::simulation_node) patch: TerrainPatchSnapshot,
    pub(in crate::nodes::simulation_node) requires_engineered_refinement: bool,
    pub(in crate::nodes::simulation_node) requires_road_clipping: bool,
    pub(in crate::nodes::simulation_node) road_locked_margin_m: f32,
    pub(in crate::nodes::simulation_node) sites: BuildingSiteTerrainSnapshot,
}

pub(in crate::nodes::simulation_node) struct WaterPatchPayload {
    pub(in crate::nodes::simulation_node) key: WaterPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) source_generation: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
    pub(in crate::nodes::simulation_node) patch: WaterPatchSnapshot,
    pub(in crate::nodes::simulation_node) depth_bytes: Vec<u8>,
    pub(in crate::nodes::simulation_node) road_clip_query: RoadClipLoopQuery,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::nodes::simulation_node) struct WaterPatchPayloadRequestState {
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) source_generation: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

pub(in crate::nodes::simulation_node) struct TerrainPatchPayloadAsyncState {
    pub(in crate::nodes::simulation_node) next_request_id: u64,
    pub(in crate::nodes::simulation_node) requested:
        HashMap<TerrainPatchPayloadKey, TerrainPatchPayloadRequestState>,
    pub(in crate::nodes::simulation_node) in_flight:
        HashMap<TerrainPatchPayloadKey, TerrainPatchPayloadRequestState>,
    pub(in crate::nodes::simulation_node) ready: Vec<TerrainPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) ready_lookup: HashSet<TerrainPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) payloads:
        HashMap<TerrainPatchPayloadKey, TerrainPatchPayload>,
    pub(in crate::nodes::simulation_node) completed: Vec<TerrainPatchPayload>,
    pub(in crate::nodes::simulation_node) failed: Vec<TerrainPatchPayloadRequest>,
}

impl TerrainPatchPayloadAsyncState {
    pub(in crate::nodes::simulation_node) fn new() -> Self {
        Self {
            next_request_id: 1,
            requested: HashMap::new(),
            in_flight: HashMap::new(),
            ready: Vec::new(),
            ready_lookup: HashSet::new(),
            payloads: HashMap::new(),
            completed: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub(in crate::nodes::simulation_node) fn request(
        &mut self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) -> u64 {
        debug_assert!(
            !self.in_flight.contains_key(&key),
            "terrain patch payload requests must coalesce behind the physical in-flight job"
        );
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let state = TerrainPatchPayloadRequestState {
            request_id,
            surface_generation,
        };
        self.requested.insert(key, state);
        self.in_flight.insert(key, state);
        self.remove_ready_payload(key);
        request_id
    }

    pub(in crate::nodes::simulation_node) fn queue_ready(&mut self, key: TerrainPatchPayloadKey) {
        if self.ready_lookup.insert(key) {
            self.ready.push(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn remove_ready_payload(
        &mut self,
        key: TerrainPatchPayloadKey,
    ) {
        self.payloads.remove(&key);
        self.ready_lookup.remove(&key);
        self.ready.retain(|ready_key| *ready_key != key);
    }

    pub(in crate::nodes::simulation_node) fn drop_stale_key_for_generation(
        &mut self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) {
        let requested_stale = self
            .requested
            .get(&key)
            .is_some_and(|state| state.surface_generation != surface_generation);
        let payload_stale = self
            .payloads
            .get(&key)
            .is_some_and(|payload| payload.surface_generation != surface_generation);
        let ready_without_payload =
            self.ready_lookup.contains(&key) && !self.payloads.contains_key(&key);
        if requested_stale {
            self.requested.remove(&key);
        }
        if payload_stale || ready_without_payload {
            self.remove_ready_payload(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn has_current_request(
        &self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) -> bool {
        self.requested
            .get(&key)
            .is_some_and(|state| state.surface_generation == surface_generation)
    }

    pub(in crate::nodes::simulation_node) fn has_in_flight(
        &self,
        key: TerrainPatchPayloadKey,
    ) -> bool {
        self.in_flight.contains_key(&key)
    }

    pub(in crate::nodes::simulation_node) fn has_current_ready(
        &self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) -> bool {
        self.payloads
            .get(&key)
            .is_some_and(|payload| payload.surface_generation == surface_generation)
    }

    pub(in crate::nodes::simulation_node) fn ingest_completed(
        &mut self,
        completed: Vec<TerrainPatchPayload>,
    ) {
        for payload in completed {
            let key = payload.key;
            let completed_state = TerrainPatchPayloadRequestState {
                request_id: payload.request_id,
                surface_generation: payload.surface_generation,
            };
            if self.in_flight.get(&key).copied() == Some(completed_state) {
                self.in_flight.remove(&key);
            }
            if self.requested.get(&key).copied() != Some(completed_state) {
                continue;
            }
            self.payloads.insert(key, payload);
            self.queue_ready(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn ingest_failed(
        &mut self,
        failed: Vec<TerrainPatchPayloadRequest>,
    ) {
        for request in failed {
            let state = TerrainPatchPayloadRequestState {
                request_id: request.request_id,
                surface_generation: request.surface_generation,
            };
            if self.in_flight.get(&request.key).copied() == Some(state) {
                self.in_flight.remove(&request.key);
            }
            if self.requested.get(&request.key).copied() == Some(state) {
                self.requested.remove(&request.key);
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn clear(&mut self) {
        self.requested.clear();
        self.in_flight.clear();
        self.ready.clear();
        self.ready_lookup.clear();
        self.payloads.clear();
        self.completed.clear();
        self.failed.clear();
    }
}

pub(in crate::nodes::simulation_node) struct WaterPatchPayloadAsyncState {
    pub(in crate::nodes::simulation_node) next_request_id: u64,
    pub(in crate::nodes::simulation_node) requested:
        HashMap<WaterPatchPayloadKey, WaterPatchPayloadRequestState>,
    pub(in crate::nodes::simulation_node) in_flight:
        HashMap<WaterPatchPayloadKey, WaterPatchPayloadRequestState>,
    pub(in crate::nodes::simulation_node) ready: Vec<WaterPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) ready_lookup: HashSet<WaterPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) payloads:
        HashMap<WaterPatchPayloadKey, WaterPatchPayload>,
    pub(in crate::nodes::simulation_node) completed: Vec<WaterPatchPayload>,
    pub(in crate::nodes::simulation_node) failed: Vec<WaterPatchPayloadRequest>,
}

impl WaterPatchPayloadAsyncState {
    pub(in crate::nodes::simulation_node) fn new() -> Self {
        Self {
            next_request_id: 1,
            requested: HashMap::new(),
            in_flight: HashMap::new(),
            ready: Vec::new(),
            ready_lookup: HashSet::new(),
            payloads: HashMap::new(),
            completed: Vec::new(),
            failed: Vec::new(),
        }
    }

    pub(in crate::nodes::simulation_node) fn request(
        &mut self,
        key: WaterPatchPayloadKey,
        source_generation: u64,
        surface_generation: u64,
    ) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        let state = WaterPatchPayloadRequestState {
            request_id,
            source_generation,
            surface_generation,
        };
        self.requested.insert(key, state);
        self.in_flight.insert(key, state);
        self.remove_ready_payload(key);
        request_id
    }

    pub(in crate::nodes::simulation_node) fn drop_stale_key_for_generation(
        &mut self,
        key: WaterPatchPayloadKey,
        source_generation: u64,
        surface_generation: u64,
    ) {
        let requested_stale = self.requested.get(&key).is_some_and(|state| {
            state.source_generation != source_generation
                || state.surface_generation != surface_generation
        });
        let payload_stale = self.payloads.get(&key).is_some_and(|payload| {
            payload.source_generation != source_generation
                || payload.surface_generation != surface_generation
        });
        if requested_stale {
            self.requested.remove(&key);
        }
        if payload_stale {
            self.remove_ready_payload(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn queue_ready(&mut self, key: WaterPatchPayloadKey) {
        if self.ready_lookup.insert(key) {
            self.ready.push(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn remove_ready_payload(
        &mut self,
        key: WaterPatchPayloadKey,
    ) {
        self.payloads.remove(&key);
        self.ready_lookup.remove(&key);
        self.ready.retain(|ready_key| *ready_key != key);
    }

    pub(in crate::nodes::simulation_node) fn remove_key(&mut self, key: WaterPatchPayloadKey) {
        self.requested.remove(&key);
        self.in_flight.remove(&key);
        self.remove_ready_payload(key);
    }

    pub(in crate::nodes::simulation_node) fn drop_missing_ready_payload(
        &mut self,
        key: WaterPatchPayloadKey,
    ) {
        if self.ready_lookup.contains(&key) && !self.payloads.contains_key(&key) {
            self.remove_key(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn has_current_pending_or_ready(
        &self,
        key: WaterPatchPayloadKey,
        source_generation: u64,
        surface_generation: u64,
    ) -> bool {
        self.requested.get(&key).is_some_and(|state| {
            state.source_generation == source_generation
                && state.surface_generation == surface_generation
        }) || self.payloads.get(&key).is_some_and(|payload| {
            payload.source_generation == source_generation
                && payload.surface_generation == surface_generation
        })
    }

    pub(in crate::nodes::simulation_node) fn ingest_completed(
        &mut self,
        completed: Vec<WaterPatchPayload>,
    ) {
        for payload in completed {
            let key = payload.key;
            let state = WaterPatchPayloadRequestState {
                request_id: payload.request_id,
                source_generation: payload.source_generation,
                surface_generation: payload.surface_generation,
            };
            if self.in_flight.get(&key).copied() == Some(state) {
                self.in_flight.remove(&key);
            }
            if self.requested.get(&key).copied() != Some(state) {
                continue;
            }
            self.payloads.insert(key, payload);
            self.queue_ready(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn ingest_failed(
        &mut self,
        failed: Vec<WaterPatchPayloadRequest>,
    ) -> Vec<WaterPatchPayloadKey> {
        let mut retryable = Vec::with_capacity(failed.len());
        for request in failed {
            let key = request.key;
            let state = WaterPatchPayloadRequestState {
                request_id: request.request_id,
                source_generation: request.source_generation,
                surface_generation: request.surface_generation,
            };
            let requested_matches = self.requested.get(&key).copied() == Some(state);
            let in_flight_matches = self.in_flight.get(&key).copied() == Some(state);
            if requested_matches {
                self.requested.remove(&key);
                self.remove_ready_payload(key);
                retryable.push(key);
            }
            if in_flight_matches {
                self.in_flight.remove(&key);
            }
        }
        retryable
    }

    pub(in crate::nodes::simulation_node) fn clear(&mut self) {
        self.requested.clear();
        self.in_flight.clear();
        self.ready.clear();
        self.ready_lookup.clear();
        self.payloads.clear();
        self.completed.clear();
        self.failed.clear();
    }
}
