// SPDX-License-Identifier: GPL-3.0-or-later

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
    /// `cache`'s keys by patch (`TERRAIN-41` B1): a patch's meshes leave the cache without a
    /// walk of it, `O(meshes of the patch)`.
    by_patch: HashMap<(usize, usize), Vec<WaterPatchMeshCacheKey>>,
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
            by_patch: HashMap::new(),
        }
    }

    /// Caches a finished mesh, indexed by its patch. `O(1)` amortised.
    pub(in crate::nodes::simulation_node) fn cache_insert(&mut self, entry: CachedWaterPatchMesh) {
        let key = entry.key;
        if self.cache.insert(key, entry).is_none() {
            self.by_patch
                .entry((key.patch_x, key.patch_z))
                .or_default()
                .push(key);
        }
    }

    /// A mesh job's finished meshes, published under the lock: each only while its key is still
    /// in flight. A key a [`Self::forget_patch`] (a released patch) or a [`Self::clear`] took out
    /// of flight while its job ran is dropped: it used to enter the cache at the next poll and stay
    /// there, a mesh of a patch no longer built (`TERRAIN-63`). A key asked again meanwhile is in
    /// flight again and is answered by whichever job publishes first. `O(entries)`.
    pub(in crate::nodes::simulation_node) fn publish(&mut self, entries: Vec<CachedWaterPatchMesh>) {
        for entry in entries {
            if self.in_flight.remove(&entry.key) {
                self.completed.push(entry);
            }
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
        self.by_patch.clear();
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
        // The cache by its index; the sets above hold work in flight only.
        for key in self.by_patch.remove(&(patch_x, patch_z)).unwrap_or_default() {
            self.cache.remove(&key);
        }
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
}

pub(in crate::nodes::simulation_node) struct TerrainPatchPayload {
    pub(in crate::nodes::simulation_node) key: TerrainPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
    pub(in crate::nodes::simulation_node) data: TerrainPatchPayloadData,
    /// Baked shading masks for the payload's height samples, filled on the payload worker.
    /// Empty until then, or for a failure payload with nothing to draw.
    pub(in crate::nodes::simulation_node) mask_bytes: Vec<u8>,
    /// The patch's land cover, built on the payload worker (`RENDER-32-M`); `None` when its
    /// inputs could not be taken with the payload.
    pub(in crate::nodes::simulation_node) land_cover: Option<super::super::vegetation_api::PatchLandCover>,
}

impl TerrainPatchPayload {
    /// Bakes this payload's shading masks; called off the main thread before publication.
    pub(in crate::nodes::simulation_node) fn bake_shading_masks(&mut self) {
        let TerrainPatchPayloadData::Regular { patch, .. } = &self.data;
        self.mask_bytes = patch.shading_mask_bytes();
        if let Some(land_cover) = self.land_cover.as_mut() {
            land_cover.bake();
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::nodes::simulation_node) struct TerrainPatchPayloadRequestState {
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

pub(in crate::nodes::simulation_node) struct WaterPatchPayload {
    pub(in crate::nodes::simulation_node) key: WaterPatchPayloadKey,
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) source_generation: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
    pub(in crate::nodes::simulation_node) patch: WaterPatchSnapshot,
    pub(in crate::nodes::simulation_node) depth_bytes: Vec<u8>,
    // The patch's terrain payload generation, which keys its mesh (`WaterPatchMeshCacheKey`).
    pub(in crate::nodes::simulation_node) cover_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::nodes::simulation_node) struct WaterPatchPayloadRequestState {
    pub(in crate::nodes::simulation_node) request_id: u64,
    pub(in crate::nodes::simulation_node) source_generation: u64,
    pub(in crate::nodes::simulation_node) surface_generation: u64,
}

/// Why a payload request produced no build source: a transient state that Godot retries (a
/// stale generation, a roadbed not yet current, a terrain stroke in progress), or a refusal by
/// contract, which is a bug: Godot logs it once and never asks again (`TERRAIN-07-D` round 4,
/// after a refusal retried silently every frame hid the graded render defect).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::nodes::simulation_node) enum PayloadDeferral {
    Retry,
    Refused(&'static str),
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
    // Requests refused by contract with the reason, for the next poll.
    pub(in crate::nodes::simulation_node) refused: Vec<(TerrainPatchPayloadRequest, &'static str)>,
    // Requests that waited behind a job for an older generation of their patch (`wait_behind`)
    // and that no job answers now that it ended, for the next poll to hand back as retries
    // (`TERRAIN-35`).
    pub(in crate::nodes::simulation_node) orphaned: Vec<TerrainPatchPayloadRequest>,
    // The payload jobs' worker times while the perf channel is on (`TERRAIN-41`), since the last
    // poll took them.
    pub(in crate::nodes::simulation_node) worker_perf: TerrainPayloadWorkerPerf,
}

/// The terrain payload jobs' worker times since the last poll (`TERRAIN-41`), kept only while the
/// perf channel is on: a job is one frame's batch of requests, run on the background pool.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(in crate::nodes::simulation_node) struct TerrainPayloadWorkerPerf {
    // Jobs spawned, started on a worker, and finished.
    pub(in crate::nodes::simulation_node) spawned: u64,
    pub(in crate::nodes::simulation_node) started: u64,
    pub(in crate::nodes::simulation_node) finished: u64,
    // Payloads built, and requests handed back for a retry (the core lock was busy, or a
    // generation moved).
    pub(in crate::nodes::simulation_node) built: u64,
    pub(in crate::nodes::simulation_node) retried: u64,
    // Microseconds summed over the finished jobs: from the spawn to the worker's start, the
    // payload sources under the core lock (heights and land cover), the shading masks' bake
    // (parallel), and the job's whole run; and the longest run.
    pub(in crate::nodes::simulation_node) queue_us: f64,
    pub(in crate::nodes::simulation_node) source_us: f64,
    pub(in crate::nodes::simulation_node) bake_us: f64,
    pub(in crate::nodes::simulation_node) run_us: f64,
    pub(in crate::nodes::simulation_node) run_max_us: f64,
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
            refused: Vec::new(),
            orphaned: Vec::new(),
            worker_perf: TerrainPayloadWorkerPerf::default(),
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

    // A request for `key` at `surface_generation` while the physical job for an older
    // generation still runs (one job a key): tracked, with no job of its own. When that job ends
    // the request is handed back as a retry (`orphaned`), so whoever asked hears of it and asks
    // again (`TERRAIN-35`; before, it was answered by nobody). `O(1)`.
    pub(in crate::nodes::simulation_node) fn wait_behind(
        &mut self,
        key: TerrainPatchPayloadKey,
        surface_generation: u64,
    ) {
        debug_assert!(self.in_flight.contains_key(&key));
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.requested.insert(
            key,
            TerrainPatchPayloadRequestState {
                request_id,
                surface_generation,
            },
        );
    }

    // After `key`'s job ended: a request still held for it that no job will answer (it waited
    // behind the job, `wait_behind`) is handed back for a retry. `O(1)`.
    fn release_orphan(&mut self, key: TerrainPatchPayloadKey) {
        if self.in_flight.contains_key(&key) {
            return;
        }
        if let Some(state) = self.requested.remove(&key) {
            self.orphaned.push(TerrainPatchPayloadRequest {
                key,
                request_id: state.request_id,
                surface_generation: state.surface_generation,
            });
        }
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
                self.release_orphan(key);
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
            } else {
                self.release_orphan(request.key);
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
        self.refused.clear();
        self.orphaned.clear();
        self.worker_perf = TerrainPayloadWorkerPerf::default();
    }

    // Drops every job and result in the same world (an undo or a bulldoze) and hands every held
    // request back as a retry for the next poll, so whoever asked hears of it and asks again
    // (`TERRAIN-35`; a bare `clear` ended them silently, and the streaming core never asked for
    // those patches again). Jobs still running are ignored when they end: nothing matches them.
    // In key order, so the retries do not follow the map's order. `O(requests log requests)`.
    pub(in crate::nodes::simulation_node) fn abandon(&mut self) {
        let mut orphaned = std::mem::take(&mut self.orphaned);
        orphaned.extend(self.requested.drain().map(|(key, state)| TerrainPatchPayloadRequest {
            key,
            request_id: state.request_id,
            surface_generation: state.surface_generation,
        }));
        orphaned.sort_unstable_by_key(|request| {
            (request.key.patch_z, request.key.patch_x, request.key.render_step_mm)
        });
        let worker_perf = self.worker_perf;
        self.clear();
        self.orphaned = orphaned;
        self.worker_perf = worker_perf;
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

    // Pops ready payloads, handing up to `max` current ones to `emit`. A payload whose
    // generations moved, or a ready key whose payload is gone, is dropped; when it held its key's
    // request, that request ended with it and the key goes to `retry`, so the renderer hears of
    // it and asks again (`TERRAIN-35`; before, such a request was dropped silently, the renderer
    // kept it pending and the streaming core never asked again). Returns the emitted, stale and
    // missing counts. `O(popped)`.
    pub(in crate::nodes::simulation_node) fn take_ready(
        &mut self,
        max: usize,
        source_generation: u64,
        surface_generation: u64,
        retry: &mut Vec<WaterPatchPayloadKey>,
        mut emit: impl FnMut(WaterPatchPayload),
    ) -> [usize; 3] {
        let [mut emitted, mut stale, mut missing] = [0; 3];
        while emitted < max {
            let Some(key) = self.ready.pop() else {
                break;
            };
            self.ready_lookup.remove(&key);
            let Some(payload) = self.payloads.remove(&key) else {
                missing += 1;
                if self.requested.remove(&key).is_some() {
                    retry.push(key);
                }
                continue;
            };
            let payload_state = WaterPatchPayloadRequestState {
                request_id: payload.request_id,
                source_generation: payload.source_generation,
                surface_generation: payload.surface_generation,
            };
            let held = self.requested.get(&key).copied() == Some(payload_state);
            if !held
                || payload.source_generation != source_generation
                || payload.surface_generation != surface_generation
            {
                stale += 1;
                if held {
                    self.requested.remove(&key);
                    retry.push(key);
                }
                continue;
            }
            self.requested.remove(&key);
            emit(payload);
            emitted += 1;
        }
        [emitted, stale, missing]
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

#[cfg(test)]
mod tests {
    use super::*;

    // `TERRAIN-41` B1: a released patch's meshes leave the cache by its index; the other
    // patches' stay.
    #[test]
    fn forgetting_a_patch_drops_exactly_its_cached_meshes() {
        let mesh = |patch_x: usize, lod_step: usize| CachedWaterPatchMesh {
            key: WaterPatchMeshCacheKey {
                patch_x,
                patch_z: 3,
                lod_step,
                cover_generation: 1,
                depth_signature: 2,
            },
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            world_size_x: 512.0,
            world_size_z: 512.0,
            stats: Default::default(),
        };
        let mut jobs = WaterPatchMeshAsyncState::new();
        for (patch_x, lod_step) in [(1, 1), (1, 2), (2, 1), (1, 2)] {
            jobs.cache_insert(mesh(patch_x, lod_step));
        }
        assert_eq!(jobs.cache.len(), 3);
        jobs.forget_patch(1, 3);
        assert_eq!(jobs.cache.keys().map(|key| key.patch_x).collect::<Vec<_>>(), vec![2]);
        jobs.forget_patch(1, 3);
        jobs.cache_insert(mesh(1, 4));
        jobs.forget_patch(2, 3);
        assert_eq!(jobs.cache.keys().map(|key| key.patch_x).collect::<Vec<_>>(), vec![1]);
    }

    // `TERRAIN-63` risk 2: a patch released (`forget_patch`) while its mesh job runs leaves no cache
    // entry behind when the job publishes, and a new request for the same mesh in the meantime is
    // answered once.
    #[test]
    fn a_patch_forgotten_while_its_mesh_job_runs_keeps_no_cache_entry() {
        let key = WaterPatchMeshCacheKey {
            patch_x: 4,
            patch_z: 3,
            lod_step: 8,
            cover_generation: 1,
            depth_signature: 2,
        };
        let mesh = || CachedWaterPatchMesh {
            key,
            vertices: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            indices: Vec::new(),
            world_size_x: 512.0,
            world_size_z: 512.0,
            stats: Default::default(),
        };
        // The poll's part: completed meshes into the cache.
        let poll = |jobs: &mut WaterPatchMeshAsyncState| {
            for entry in std::mem::take(&mut jobs.completed) {
                jobs.cache_insert(entry);
            }
        };
        let mut jobs = WaterPatchMeshAsyncState::new();
        // Requested, its job running; the patch is released; the job publishes.
        jobs.requested.insert(key);
        jobs.in_flight.insert(key);
        jobs.forget_patch(4, 3);
        jobs.publish(vec![mesh()]);
        poll(&mut jobs);
        assert!(jobs.cache.is_empty(), "a released patch's mesh must not enter the cache");
        // Forgotten, then asked again while the first job runs: the first job's mesh answers it
        // and the second's is dropped.
        jobs.requested.insert(key);
        jobs.in_flight.insert(key);
        jobs.forget_patch(4, 3);
        jobs.requested.insert(key);
        jobs.in_flight.insert(key);
        jobs.publish(vec![mesh()]);
        jobs.publish(vec![mesh()]);
        assert_eq!(jobs.completed.len(), 1);
        poll(&mut jobs);
        assert_eq!(jobs.cache.len(), 1);
        assert!(jobs.in_flight.is_empty());
    }
}
