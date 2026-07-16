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
}

impl WaterPatchMeshAsyncState {
    pub(in crate::nodes::simulation_node) fn new() -> Self {
        Self {
            in_flight: HashSet::new(),
            requested: HashSet::new(),
            ready: Vec::new(),
            ready_lookup: HashSet::new(),
            completed: Vec::new(),
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

pub(in crate::nodes::simulation_node) struct WaterPatchPayload {
    pub(in crate::nodes::simulation_node) key: WaterPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) patch: WaterPatchSnapshot,
    pub(in crate::nodes::simulation_node) depth_bytes: Vec<u8>,
    pub(in crate::nodes::simulation_node) road_clip_query: RoadClipLoopQuery,
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
        }
    }

    pub(in crate::nodes::simulation_node) fn request(
        &mut self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) -> u64 {
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

    pub(in crate::nodes::simulation_node) fn remove_key(&mut self, key: TerrainPatchPayloadKey) {
        self.requested.remove(&key);
        self.in_flight.remove(&key);
        self.remove_ready_payload(key);
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
        let in_flight_stale = self
            .in_flight
            .get(&key)
            .is_some_and(|state| state.surface_generation != surface_generation);
        let payload_stale = self
            .payloads
            .get(&key)
            .is_some_and(|payload| payload.surface_generation != surface_generation);
        let ready_without_payload =
            self.ready_lookup.contains(&key) && !self.payloads.contains_key(&key);
        if requested_stale || in_flight_stale || payload_stale || ready_without_payload {
            self.remove_key(key);
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

    pub(in crate::nodes::simulation_node) fn has_current_in_flight(
        &self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) -> bool {
        self.in_flight
            .get(&key)
            .is_some_and(|state| state.surface_generation == surface_generation)
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

    pub(in crate::nodes::simulation_node) fn clear(&mut self) {
        self.requested.clear();
        self.in_flight.clear();
        self.ready.clear();
        self.ready_lookup.clear();
        self.payloads.clear();
        self.completed.clear();
    }
}

pub(in crate::nodes::simulation_node) struct WaterPatchPayloadAsyncState {
    pub(in crate::nodes::simulation_node) next_request_id: u64,
    pub(in crate::nodes::simulation_node) requested: HashMap<WaterPatchPayloadKey, u64>,
    pub(in crate::nodes::simulation_node) in_flight: HashMap<WaterPatchPayloadKey, u64>,
    pub(in crate::nodes::simulation_node) ready: Vec<WaterPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) ready_lookup: HashSet<WaterPatchPayloadKey>,
    pub(in crate::nodes::simulation_node) payloads:
        HashMap<WaterPatchPayloadKey, WaterPatchPayload>,
    pub(in crate::nodes::simulation_node) completed: Vec<WaterPatchPayload>,
    pub(in crate::nodes::simulation_node) failed: Vec<(WaterPatchPayloadKey, u64)>,
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

    pub(in crate::nodes::simulation_node) fn request(&mut self, key: WaterPatchPayloadKey) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.requested.insert(key, request_id);
        self.in_flight.insert(key, request_id);
        self.remove_ready_payload(key);
        request_id
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

    pub(in crate::nodes::simulation_node) fn has_pending_or_ready(
        &self,
        key: WaterPatchPayloadKey,
    ) -> bool {
        self.requested.contains_key(&key)
            || self.in_flight.contains_key(&key)
            || self.payloads.contains_key(&key)
    }

    pub(in crate::nodes::simulation_node) fn ingest_completed(
        &mut self,
        completed: Vec<WaterPatchPayload>,
    ) {
        for payload in completed {
            let key = payload.key;
            let request_id = payload.request_id;
            if self.in_flight.get(&key).copied() == Some(request_id) {
                self.in_flight.remove(&key);
            }
            if self.requested.get(&key).copied() != Some(request_id) {
                continue;
            }
            self.payloads.insert(key, payload);
            self.queue_ready(key);
        }
    }

    pub(in crate::nodes::simulation_node) fn ingest_failed(
        &mut self,
        failed: Vec<(WaterPatchPayloadKey, u64)>,
    ) -> Vec<WaterPatchPayloadKey> {
        let mut retryable = Vec::with_capacity(failed.len());
        for (key, request_id) in failed {
            let requested_matches = self.requested.get(&key).copied() == Some(request_id);
            let in_flight_matches = self.in_flight.get(&key).copied() == Some(request_id);
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
