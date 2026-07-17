//! Water rendering and water-patch Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns whether the water mesh needs rebuilding.
    #[func]
    pub fn is_water_dirty(&self) -> bool {
        self.snapshot.read().unwrap().water_dirty
    }

    /// Acknowledges rendered water patch revisions without erasing newer mutations.
    #[func]
    pub fn acknowledge_water_patches(&mut self, flat_states: PackedInt64Array) -> bool {
        let acknowledged = flat_states
            .as_slice()
            .chunks_exact(3)
            .filter_map(|state| {
                Some((
                    usize::try_from(state[0]).ok()?,
                    usize::try_from(state[1]).ok()?,
                    u64::try_from(state[2]).ok()?,
                ))
            })
            .collect::<Vec<_>>();
        let Some(mut core) = self.try_lock_core() else {
            return false;
        };
        let acknowledged_keys = acknowledged
            .iter()
            .filter_map(|&(patch_x, patch_z, generation)| {
                (core.watermap.render_patch_generation(patch_x, patch_z) == generation)
                    .then_some((patch_x, patch_z))
            })
            .collect::<Vec<_>>();
        for (patch_x, patch_z, generation) in acknowledged {
            core.watermap
                .acknowledge_render_patch(patch_x, patch_z, generation);
        }
        core.water_dirty = !core.watermap.dirty_render_patches().is_empty();
        let water_dirty = core.water_dirty;
        let states = core.watermap.dirty_render_patch_states();
        let generation = core.watermap.render_generation();
        drop(core);

        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.water_dirty = water_dirty;
        snapshot.water_dirty_patch_states = Arc::new(states);
        snapshot.water_payload_generation = generation;
        drop(snapshot);
        let mut mesh_jobs = self.lock_water_patch_mesh_jobs();
        for (patch_x, patch_z) in acknowledged_keys {
            mesh_jobs.forget_patch(patch_x, patch_z);
        }
        true
    }

    /// Returns the currently dirty water render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_water_patches(&self) -> PackedInt32Array {
        let snapshot = self.snapshot.read().unwrap();
        let mut packed = PackedInt32Array::new();
        for &(patch_x, patch_z, _) in snapshot.water_dirty_patch_states.iter() {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns dirty water patches as flat `(x, z, payload_generation)` triples.
    #[func]
    pub fn get_dirty_water_patch_payload_states(&self) -> PackedInt64Array {
        let snapshot = self.snapshot.read().unwrap();
        let mut packed = PackedInt64Array::new();
        for &(patch_x, patch_z, generation) in snapshot.water_dirty_patch_states.iter() {
            packed.push(i64::try_from(patch_x).unwrap_or(i64::MAX));
            packed.push(i64::try_from(patch_z).unwrap_or(i64::MAX));
            packed.push(i64::try_from(generation).unwrap_or(i64::MAX));
        }
        packed
    }

    /// Requests async water patch payload preparation for flat `(patch_x, patch_z)` pairs.
    #[func]
    pub fn request_water_patch_payloads(
        &mut self,
        flat_requests: PackedInt32Array,
    ) -> VarDictionary {
        let requests = Self::water_patch_payload_keys_from_flat(&flat_requests);
        let raw_count = requests.len();
        let mut stats = VarDictionary::new();
        stats.set("raw_count", i64::try_from(raw_count).unwrap_or(i64::MAX));
        stats.set("accepted_count", 0_i64);
        stats.set("duplicate_count", 0_i64);
        stats.set("in_flight_count", 0_i64);
        stats.set("build_queued_count", 0_i64);
        if requests.is_empty() {
            return stats;
        }

        let (source_generation, surface_generation) = {
            let snapshot = self.snapshot.read().unwrap();
            (
                snapshot.water_payload_generation,
                snapshot.network_generation,
            )
        };
        let mut accepted_count = 0_usize;
        let mut duplicate_count = 0_usize;
        let mut in_flight_count = 0_usize;
        let mut build_queued_count = 0_usize;
        let mut build_requests = Vec::new();
        {
            let mut jobs = self.lock_water_patch_payload_jobs();
            let mut requested_key_lookup = HashSet::new();
            for key in requests {
                if !requested_key_lookup.insert(key) {
                    duplicate_count += 1;
                    continue;
                }
                jobs.drop_stale_key_for_generation(key, source_generation, surface_generation);
                jobs.drop_missing_ready_payload(key);
                let in_flight = jobs.in_flight.contains_key(&key);
                if jobs.has_current_pending_or_ready(key, source_generation, surface_generation) {
                    duplicate_count += 1;
                    in_flight_count += usize::from(in_flight);
                    continue;
                }
                let request_id = jobs.request(key, source_generation, surface_generation);
                build_requests.push(WaterPatchPayloadRequest {
                    key,
                    request_id,
                    source_generation,
                    surface_generation,
                });
                accepted_count += 1;
                build_queued_count += 1;
            }
        }

        if !build_requests.is_empty() {
            let core = Arc::clone(&self.core);
            let jobs = Arc::clone(&self.water_patch_payload_jobs);
            rayon::spawn(move || {
                let mut built = Vec::with_capacity(build_requests.len());
                let mut failed = Vec::new();
                {
                    let core = core
                        .lock()
                        .expect("simulation core lock poisoned during water payload build");
                    for request in build_requests {
                        match SimulationNode::water_patch_payload_for_request(&core, request) {
                            Some(payload) => built.push(payload),
                            None => failed.push(request),
                        }
                    }
                }
                let mut job_state = jobs
                    .lock()
                    .expect("water payload job lock poisoned during result publication");
                job_state.completed.extend(built);
                job_state.failed.extend(failed);
            });
        }

        stats.set(
            "accepted_count",
            i64::try_from(accepted_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "duplicate_count",
            i64::try_from(duplicate_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "in_flight_count",
            i64::try_from(in_flight_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "build_queued_count",
            i64::try_from(build_queued_count).unwrap_or(i64::MAX),
        );
        stats
    }

    /// Returns ready async water patch payloads without blocking on patch extraction.
    ///
    /// Failed request keys are returned as flat `(patch_x, patch_z)` pairs and become
    /// eligible for a later retry after their matching request state is cleared.
    #[func]
    pub fn poll_ready_water_patch_payloads(&mut self, max_patches: i32) -> VarDictionary {
        let (completed_payloads, failed_payloads) = self.drain_completed_water_patch_payload_jobs();
        let completed_count = completed_payloads.len();
        let max_patches = usize::try_from(max_patches).unwrap_or(0);
        let mut patches = VarArray::new();
        let mut stats = VarDictionary::new();
        stats.set("patches", patches.to_variant());
        stats.set(
            "completed_count",
            i64::try_from(completed_count).unwrap_or(i64::MAX),
        );
        stats.set("ready_before_count", 0_i64);
        stats.set("emitted_count", 0_i64);
        stats.set("stale_ready_count", 0_i64);
        stats.set("missing_ready_count", 0_i64);
        stats.set("failed_count", 0_i64);
        stats.set("failed_patch_keys", PackedInt32Array::new());
        stats.set("requested_before_count", 0_i64);
        stats.set("requested_after_count", 0_i64);

        let (source_generation, surface_generation) = {
            let snapshot = self.snapshot.read().unwrap();
            (
                snapshot.water_payload_generation,
                snapshot.network_generation,
            )
        };
        let mut jobs = self.lock_water_patch_payload_jobs();
        if !completed_payloads.is_empty() {
            jobs.ingest_completed(completed_payloads);
        }
        let failed_keys = jobs.ingest_failed(failed_payloads);
        let mut failed_patch_keys = PackedInt32Array::new();
        for key in &failed_keys {
            failed_patch_keys.push(i32::try_from(key.patch_x).unwrap_or(i32::MAX));
            failed_patch_keys.push(i32::try_from(key.patch_z).unwrap_or(i32::MAX));
        }
        stats.set(
            "failed_count",
            i64::try_from(failed_keys.len()).unwrap_or(i64::MAX),
        );
        stats.set("failed_patch_keys", failed_patch_keys);
        let ready_before_count = jobs.ready.len();
        let requested_before_count = jobs.requested.len();
        if max_patches == 0 {
            stats.set(
                "ready_before_count",
                i64::try_from(ready_before_count).unwrap_or(i64::MAX),
            );
            stats.set(
                "requested_before_count",
                i64::try_from(requested_before_count).unwrap_or(i64::MAX),
            );
            stats.set(
                "requested_after_count",
                i64::try_from(jobs.requested.len()).unwrap_or(i64::MAX),
            );
            return stats;
        }

        let mut emitted_count = 0_usize;
        let mut stale_ready_count = 0_usize;
        let mut missing_ready_count = 0_usize;
        while emitted_count < max_patches {
            let Some(key) = jobs.ready.pop() else {
                break;
            };
            jobs.ready_lookup.remove(&key);
            let Some(payload) = jobs.payloads.remove(&key) else {
                missing_ready_count += 1;
                jobs.requested.remove(&key);
                continue;
            };
            let payload_state = WaterPatchPayloadRequestState {
                request_id: payload.request_id,
                source_generation: payload.source_generation,
                surface_generation: payload.surface_generation,
            };
            if payload.source_generation != source_generation
                || payload.surface_generation != surface_generation
                || jobs.requested.get(&key).copied() != Some(payload_state)
            {
                stale_ready_count += 1;
                if jobs.requested.get(&key).copied() == Some(payload_state) {
                    jobs.requested.remove(&key);
                }
                continue;
            }
            jobs.requested.remove(&key);
            patches.push(&Self::water_patch_payload_dict(&payload).to_variant());
            emitted_count += 1;
        }

        stats.set("patches", patches.to_variant());
        stats.set(
            "ready_before_count",
            i64::try_from(ready_before_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "emitted_count",
            i64::try_from(emitted_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "stale_ready_count",
            i64::try_from(stale_ready_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "missing_ready_count",
            i64::try_from(missing_ready_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "requested_before_count",
            i64::try_from(requested_before_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "requested_after_count",
            i64::try_from(jobs.requested.len()).unwrap_or(i64::MAX),
        );
        stats
    }

    /// Clears cached water mesh variants for flat `(patch_x, patch_z)` patch keys.
    #[func]
    pub fn clear_water_patch_mesh_cache(&mut self, flat_patch_keys: PackedInt32Array) {
        let values = flat_patch_keys.as_slice();
        let mut patch_keys = Vec::new();
        for chunk in values.chunks_exact(2) {
            let Ok(patch_x) = usize::try_from(chunk[0]) else {
                continue;
            };
            let Ok(patch_z) = usize::try_from(chunk[1]) else {
                continue;
            };
            patch_keys.push((patch_x, patch_z));
        }
        if !patch_keys.is_empty() {
            let mut jobs = self.lock_water_patch_mesh_jobs();
            for (patch_x, patch_z) in patch_keys {
                jobs.forget_patch(patch_x, patch_z);
            }
        }
    }

    /// Requests async water patch mesh preparation for flat `(patch_x, patch_z, lod_step)` tuples.
    #[func]
    pub fn request_water_patch_meshes(&mut self, flat_requests: PackedInt32Array) -> VarDictionary {
        let requests = Self::water_patch_mesh_requests_from_flat(&flat_requests);
        let raw_count = requests.len();
        let mut stats = VarDictionary::new();
        stats.set("raw_count", i64::try_from(raw_count).unwrap_or(i64::MAX));
        stats.set("accepted_count", 0_i64);
        stats.set("invalid_count", 0_i64);
        stats.set("duplicate_count", 0_i64);
        stats.set("cache_hit_count", 0_i64);
        stats.set("in_flight_count", 0_i64);
        stats.set("build_queued_count", 0_i64);
        stats.set("core_busy", false);
        if requests.is_empty() {
            return stats;
        }
        let mut accepted_count = 0_usize;
        let mut invalid_count = 0_usize;
        let mut duplicate_count = 0_usize;
        let mut cache_hit_count = 0_usize;
        let mut in_flight_count = 0_usize;
        let mut build_queued_count = 0_usize;
        let mut build_inputs = Vec::new();
        {
            let Some(core) = self.try_lock_core() else {
                stats.set("core_busy", true);
                return stats;
            };
            let mut jobs = self.lock_water_patch_mesh_jobs();
            let mut requested_key_lookup = HashSet::new();
            for (patch_x, patch_z, lod_step) in requests {
                let Some((key, build_input)) = Self::water_patch_mesh_build_input_for_request(
                    &core, patch_x, patch_z, lod_step,
                ) else {
                    invalid_count += 1;
                    continue;
                };
                if !requested_key_lookup.insert(key) {
                    duplicate_count += 1;
                    continue;
                }
                jobs.forget_requested_patch(patch_x, patch_z);
                jobs.requested.insert(key);
                accepted_count += 1;
                if jobs.cache.contains_key(&key) {
                    jobs.queue_ready(key);
                    cache_hit_count += 1;
                    continue;
                }
                if jobs.in_flight.contains(&key) {
                    in_flight_count += 1;
                    continue;
                }
                jobs.in_flight.insert(key);
                build_inputs.push(build_input);
                build_queued_count += 1;
            }
        }

        if !build_inputs.is_empty() {
            let jobs = Arc::clone(&self.water_patch_mesh_jobs);
            rayon::spawn(move || {
                let entries = SimCore::build_water_patch_mesh_cache_entries(build_inputs);
                let mut job_state = jobs
                    .lock()
                    .expect("water mesh job lock poisoned during result publication");
                for entry in entries {
                    job_state.in_flight.remove(&entry.key);
                    job_state.completed.push(entry);
                }
            });
        }
        stats.set(
            "accepted_count",
            i64::try_from(accepted_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "invalid_count",
            i64::try_from(invalid_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "duplicate_count",
            i64::try_from(duplicate_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "cache_hit_count",
            i64::try_from(cache_hit_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "in_flight_count",
            i64::try_from(in_flight_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "build_queued_count",
            i64::try_from(build_queued_count).unwrap_or(i64::MAX),
        );
        stats
    }

    /// Returns ready cached water patch meshes without requiring Godot to resubmit pending keys.
    #[func]
    pub fn poll_ready_water_patch_meshes(&mut self, max_meshes: i32) -> VarDictionary {
        let completed_entries = self.drain_completed_water_patch_mesh_jobs();
        let completed_count = completed_entries.len();
        let completed_keys = completed_entries
            .iter()
            .map(|entry| entry.key)
            .collect::<Vec<_>>();
        let max_meshes = usize::try_from(max_meshes).unwrap_or(0);
        let mut meshes = VarArray::new();
        let mut stats = VarDictionary::new();
        stats.set("meshes", meshes.to_variant());
        stats.set(
            "completed_count",
            i64::try_from(completed_count).unwrap_or(i64::MAX),
        );
        stats.set("ready_before_count", 0_i64);
        stats.set("emitted_count", 0_i64);
        stats.set("stale_ready_count", 0_i64);
        stats.set("missing_ready_count", 0_i64);
        stats.set("requested_before_count", 0_i64);
        stats.set("requested_after_count", 0_i64);
        if max_meshes == 0 {
            let mut jobs = self.lock_water_patch_mesh_jobs();
            for entry in completed_entries {
                jobs.cache.insert(entry.key, entry);
            }
            return stats;
        }

        let mut jobs = self.lock_water_patch_mesh_jobs();
        for entry in completed_entries {
            jobs.cache.insert(entry.key, entry);
        }
        for key in completed_keys {
            if jobs.requested.contains(&key) {
                jobs.queue_ready(key);
            }
        }

        let ready_before_count = jobs.ready.len();
        let requested_before_count = jobs.requested.len();
        let mut emitted_count = 0_usize;
        let mut stale_ready_count = 0_usize;
        let mut missing_ready_count = 0_usize;
        while emitted_count < max_meshes {
            let Some(key) = jobs.ready.pop() else {
                break;
            };
            jobs.ready_lookup.remove(&key);
            if !jobs.requested.contains(&key) {
                stale_ready_count += 1;
                continue;
            }
            let mesh_variant = {
                let Some(mesh) = jobs.cache.get(&key) else {
                    jobs.requested.remove(&key);
                    missing_ready_count += 1;
                    continue;
                };
                Self::water_patch_mesh_dict(mesh).to_variant()
            };
            jobs.requested.remove(&key);
            meshes.push(&mesh_variant);
            emitted_count += 1;
        }
        stats.set("meshes", meshes.to_variant());
        stats.set(
            "ready_before_count",
            i64::try_from(ready_before_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "emitted_count",
            i64::try_from(emitted_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "stale_ready_count",
            i64::try_from(stale_ready_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "missing_ready_count",
            i64::try_from(missing_ready_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "requested_before_count",
            i64::try_from(requested_before_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "requested_after_count",
            i64::try_from(jobs.requested.len()).unwrap_or(i64::MAX),
        );
        stats
    }

    /// Returns debug-only baseline/dynamic/combined water stats for one render patch.
    #[func]
    pub fn get_water_patch_debug(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let Some(core) = self.try_lock_core() else {
            return VarDictionary::new();
        };
        let Some(stats) = core.watermap.visible_patch_layer_stats(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::water_patch_layer_debug_dict(&stats)
    }

    /// Returns debug-only authored baseline-water fill contributors for one render patch.
    #[func]
    pub fn get_water_patch_authored_fill_debug(&self, patch_x: i32, patch_z: i32) -> VarArray {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarArray::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarArray::new();
        };
        let Some(core) = self.try_lock_core() else {
            return VarArray::new();
        };
        let contributors = core.authored_water_patch_fill_debug_internal(patch_x, patch_z);
        let mut array = VarArray::new();
        for contributor in contributors {
            let dict = Self::authored_water_patch_fill_debug_dict(&contributor);
            array.push(&dict.to_variant());
        }
        array
    }

    /// Returns the visible water depth along the world-edge perimeter loop.
    #[func]
    pub fn get_water_border_depths(&self) -> PackedFloat32Array {
        PackedFloat32Array::from_iter(
            self.snapshot
                .read()
                .unwrap()
                .water_border_depths
                .iter()
                .copied(),
        )
    }
}
