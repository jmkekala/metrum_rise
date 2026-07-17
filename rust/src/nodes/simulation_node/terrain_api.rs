//! Terrain editing and terrain-patch Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Applies one immediate terrain sculpt edit.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_internal(pos, radius, strength);
        self.refresh_snapshot_from_core();
    }

    /// Begins one batched world-editor terrain stroke.
    #[func]
    pub fn begin_terrain_stroke(&mut self) {
        self.lock_core().start_terrain_stroke_internal();
    }

    /// Finalizes one batched world-editor terrain stroke.
    #[func]
    pub fn end_terrain_stroke(&mut self) -> bool {
        let ended = {
            let mut core = self.lock_core();
            core.end_terrain_stroke_internal()
        };
        if ended {
            self.refresh_snapshot_from_core();
        }
        ended
    }

    /// Applies one batched terrain sculpt step during an active editor stroke.
    #[func]
    pub fn sculpt_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        let core = {
            let mut core = self.lock_core();
            core.sculpt_terrain_stroke_step_internal(pos, radius, strength);
            core
        };
        Self::publish_terrain_render_state(&mut self.snapshot.write().unwrap(), &core);
    }

    /// Moves terrain toward a clicked rendered heightmap level.
    #[func]
    pub fn level_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        self.lock_core()
            .level_terrain_internal(pos, radius, target_height_m, strength);
        self.refresh_snapshot_from_core();
    }

    /// Applies one batched terrain-level step during an active editor stroke.
    #[func]
    pub fn level_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        target_height_m: f32,
        strength: f32,
    ) {
        let core = {
            let mut core = self.lock_core();
            core.level_terrain_stroke_step_internal(pos, radius, target_height_m, strength);
            core
        };
        Self::publish_terrain_render_state(&mut self.snapshot.write().unwrap(), &core);
    }

    /// Smooths terrain toward the local neighborhood average.
    #[func]
    pub fn smooth_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_internal(pos, radius, strength);
        self.refresh_snapshot_from_core();
    }

    /// Applies one batched terrain-smooth step during an active editor stroke.
    #[func]
    pub fn smooth_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        let core = {
            let mut core = self.lock_core();
            core.smooth_terrain_stroke_step_internal(pos, radius, strength);
            core
        };
        Self::publish_terrain_render_state(&mut self.snapshot.write().unwrap(), &core);
    }

    /// Moves terrain toward a slope defined by two clicked rendered anchor points.
    #[func]
    pub fn slope_terrain(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        self.lock_core().slope_terrain_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        self.refresh_snapshot_from_core();
    }

    /// Applies one batched terrain-slope step during an active editor stroke.
    #[func]
    pub fn slope_terrain_stroke_step(
        &mut self,
        pos: Vector2,
        radius: f32,
        start_world: Vector2,
        start_height_m: f32,
        end_world: Vector2,
        end_height_m: f32,
        strength: f32,
    ) {
        let core = {
            let mut core = self.lock_core();
            core.slope_terrain_stroke_step_internal(
                pos,
                radius,
                start_world,
                start_height_m,
                end_world,
                end_height_m,
                strength,
            );
            core
        };
        Self::publish_terrain_render_state(&mut self.snapshot.write().unwrap(), &core);
    }

    fn publish_terrain_render_state(snapshot: &mut RenderSnapshot, core: &SimCore) {
        snapshot.terrain_dirty = core.terrain_dirty;
        snapshot.terrain_dirty_patch_states = Arc::new(core.terrain_dirty_patch_states());
        snapshot.terrain_payload_global_generation = core.terrain_payload_global_generation;
        snapshot.terrain_payload_patch_generations =
            Arc::new(core.terrain_payload_patch_generations.clone());
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool {
        self.snapshot.read().unwrap().terrain_dirty
    }

    /// Acknowledges rendered terrain patch revisions without erasing newer mutations.
    #[func]
    pub fn acknowledge_terrain_patches(&mut self, flat_states: PackedInt64Array) -> bool {
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
        for (patch_x, patch_z, generation) in acknowledged {
            core.acknowledge_terrain_render_patch(patch_x, patch_z, generation);
        }
        core.terrain_dirty = !core.heightmap.dirty_render_patches().is_empty();
        let terrain_dirty = core.terrain_dirty;
        let states = core.terrain_dirty_patch_states();
        let global_generation = core.terrain_payload_global_generation;
        let patch_generations = core.terrain_payload_patch_generations.clone();
        drop(core);

        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = terrain_dirty;
        snapshot.terrain_dirty_patch_states = Arc::new(states);
        snapshot.terrain_payload_global_generation = global_generation;
        snapshot.terrain_payload_patch_generations = Arc::new(patch_generations);
        true
    }

    /// Returns true if the road/rail network was mutated and the visual mesh needs a rebuild.
    ///
    /// `NetworkRenderer._process` polls this each frame. The flag stays `true` until
    /// Godot acknowledges the exact revision it uploaded.
    #[func]
    pub fn is_network_dirty(&self) -> bool {
        self.snapshot.read().unwrap().network_dirty
    }

    /// Returns the exact network mesh revision currently published to the renderer.
    #[func]
    pub fn get_network_render_generation(&self) -> i64 {
        i64::try_from(self.snapshot.read().unwrap().network_generation).unwrap_or(i64::MAX)
    }

    /// Acknowledges one published network revision without erasing a newer road edit.
    #[func]
    pub fn acknowledge_network_render(&mut self, generation: i64) -> bool {
        let Ok(generation) = u64::try_from(generation) else {
            return false;
        };
        let Some(mut core) = self.try_lock_core() else {
            return false;
        };
        core.acknowledge_network_render_generation(generation);
        let network_dirty = core.network_dirty;
        let current_generation = core.road_tool_surface_generation;
        drop(core);

        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.network_dirty = network_dirty;
        snapshot.network_generation = current_generation;
        true
    }

    /// Returns render-patch layout metadata shared by terrain and water renderers.
    #[func]
    pub fn get_terrain_patch_layout(&self) -> VarDictionary {
        let snapshot = self.snapshot.read().unwrap();
        let mut dict = VarDictionary::new();
        dict.set(
            "patch_cols",
            i64::try_from(snapshot.terrain_patch_cols).unwrap_or(0),
        );
        dict.set(
            "patch_rows",
            i64::try_from(snapshot.terrain_patch_rows).unwrap_or(0),
        );
        dict.set(
            "patch_interval_cells",
            i64::try_from(snapshot.terrain_patch_interval_cells).unwrap_or(0),
        );
        dict.set("terrain_cell_m", f64::from(snapshot.terrain_cell_m));
        dict.set("chunk_span_m", f64::from(snapshot.terrain_chunk_span_m));
        dict
    }

    /// Returns the currently dirty terrain render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_terrain_patches(&self) -> PackedInt32Array {
        let snapshot = self.snapshot.read().unwrap();
        let mut packed = PackedInt32Array::new();
        for &(patch_x, patch_z, _) in snapshot.terrain_dirty_patch_states.iter() {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns dirty terrain patches as flat `(x, z, payload_generation)` triples.
    #[func]
    pub fn get_dirty_terrain_patch_payload_states(&self) -> PackedInt64Array {
        let snapshot = self.snapshot.read().unwrap();
        let mut packed = PackedInt64Array::new();
        for &(patch_x, patch_z, generation) in snapshot.terrain_dirty_patch_states.iter() {
            packed.push(i64::try_from(patch_x).unwrap_or(i64::MAX));
            packed.push(i64::try_from(patch_z).unwrap_or(i64::MAX));
            packed.push(i64::try_from(generation).unwrap_or(i64::MAX));
        }
        packed
    }

    fn packed_terrain_patch_payload_states(
        states: impl IntoIterator<Item = (usize, usize, u32, u64)>,
    ) -> PackedInt64Array {
        let mut packed = PackedInt64Array::new();
        for (patch_x, patch_z, render_step_mm, generation) in states {
            packed.push(i64::try_from(patch_x).unwrap_or(i64::MAX));
            packed.push(i64::try_from(patch_z).unwrap_or(i64::MAX));
            packed.push(i64::from(render_step_mm));
            packed.push(i64::try_from(generation).unwrap_or(i64::MAX));
        }
        packed
    }

    /// Requests async terrain patch payload preparation for flat `(patch_x, patch_z, render_step_mm)` tuples.
    #[func]
    pub fn request_terrain_patch_payloads(
        &mut self,
        flat_requests: PackedInt32Array,
    ) -> VarDictionary {
        let requests = Self::terrain_patch_payload_keys_from_flat(&flat_requests);
        let raw_count = requests.len();
        let mut stats = VarDictionary::new();
        stats.set("raw_count", i64::try_from(raw_count).unwrap_or(i64::MAX));
        stats.set("accepted_count", 0_i64);
        stats.set("duplicate_count", 0_i64);
        stats.set("in_flight_count", 0_i64);
        stats.set("ready_count", 0_i64);
        stats.set("build_queued_count", 0_i64);
        stats.set("tracked_requests", PackedInt64Array::new().to_variant());
        if requests.is_empty() {
            return stats;
        }

        let requests = {
            let snapshot = self.snapshot.read().unwrap();
            requests
                .into_iter()
                .map(|key| {
                    let generation =
                        snapshot.terrain_payload_generation_for_patch(key.patch_x, key.patch_z);
                    (key, generation)
                })
                .collect::<Vec<_>>()
        };
        let road_query_snapshot = self.road_tool_query_snapshot.read().unwrap().clone();

        let mut accepted_count = 0_usize;
        let mut duplicate_count = 0_usize;
        let mut in_flight_count = 0_usize;
        let mut ready_count = 0_usize;
        let mut build_queued_count = 0_usize;
        let mut build_requests = Vec::new();
        let mut tracked_requests = Vec::new();
        {
            let mut jobs = self.lock_terrain_patch_payload_jobs();
            let mut requested_key_lookup = HashSet::new();
            for (key, surface_generation) in requests {
                if !requested_key_lookup.insert(key) {
                    duplicate_count += 1;
                    continue;
                }
                jobs.drop_stale_key_for_generation(key, surface_generation);
                let in_flight = jobs.has_in_flight(key);
                let ready = jobs.has_current_ready(key, surface_generation);
                let current_request = jobs.has_current_request(key, surface_generation);
                if current_request || ready {
                    duplicate_count += 1;
                    in_flight_count += usize::from(in_flight);
                    ready_count += usize::from(ready);
                    tracked_requests.push((key, surface_generation));
                    continue;
                }
                if in_flight {
                    duplicate_count += 1;
                    in_flight_count += 1;
                    continue;
                }
                let request_id = jobs.request(key, surface_generation);
                build_requests.push(TerrainPatchPayloadRequest {
                    key,
                    request_id,
                    surface_generation,
                });
                accepted_count += 1;
                build_queued_count += 1;
                tracked_requests.push((key, surface_generation));
            }
        }

        if !build_requests.is_empty() {
            let request_count = build_requests.len();
            let refined_request_count = build_requests
                .iter()
                .filter(|request| request.key.render_step_mm > 0)
                .count();
            let core = Arc::clone(&self.core);
            let jobs = Arc::clone(&self.terrain_patch_payload_jobs);
            rayon::spawn(move || {
                let worker_start = Instant::now();
                let perf_enabled = crate::debug::is_perf_enabled();
                let snapshot_start = Instant::now();
                let (sources, mut failed_requests) = {
                    let mut core = core
                        .lock()
                        .expect("simulation core lock poisoned during terrain payload snapshot");
                    let mut sources = Vec::with_capacity(build_requests.len());
                    let mut failed = Vec::new();
                    for request in build_requests {
                        if let Some(source) =
                            SimulationNode::terrain_patch_payload_build_source_for_request(
                                &mut core, request,
                            )
                        {
                            sources.push(source);
                        } else {
                            failed.push(request);
                        }
                    }
                    (sources, failed)
                };
                let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;

                let input_start = Instant::now();
                let build_jobs = sources
                    .into_par_iter()
                    .map(|source| {
                        SimulationNode::terrain_patch_payload_build_job_for_source(
                            &road_query_snapshot,
                            source,
                        )
                    })
                    .collect::<Vec<_>>();
                let (mut built, refined_requests, refined_inputs) =
                    SimulationNode::split_terrain_patch_payload_jobs(build_jobs);
                let input_ms = input_start.elapsed().as_secs_f64() * 1000.0;
                if perf_enabled
                    && (snapshot_ms >= 4.0 || input_ms >= 4.0 || refined_request_count > 0)
                {
                    println!(
                        "[DEBUG:perf] terrain_payload_worker_input requests={} refined_requests={} ready={} refined_inputs={} failed={} snapshot_lock_ms={:.3} input_ms={:.3}",
                        request_count,
                        refined_request_count,
                        built.len(),
                        refined_inputs.len(),
                        failed_requests.len(),
                        snapshot_ms,
                        input_ms
                    );
                }

                let cdt_start = Instant::now();
                let refined_entries = if refined_inputs.is_empty() {
                    Vec::new()
                } else {
                    SimCore::build_refined_terrain_patch_cache_entries(refined_inputs)
                };
                let cdt_ms = cdt_start.elapsed().as_secs_f64() * 1000.0;
                let refined_entry_count = refined_entries.len();
                let refined_window_count = refined_entries
                    .iter()
                    .map(|entry| entry.windows.len())
                    .sum::<usize>();
                let refined_failed_window_count = refined_entries
                    .iter()
                    .flat_map(|entry| &entry.windows)
                    .filter(|window| window.mesh_result.is_err())
                    .count();
                let refined_invalid_entry_count = refined_entries
                    .iter()
                    .filter(|entry| {
                        SimulationNode::cached_refined_cdt_failure_label(entry).is_some()
                    })
                    .count();
                let refined_road_loop_count = refined_entries
                    .iter()
                    .map(|entry| entry.road_clip_loop_count)
                    .sum::<usize>();
                let refined_site_loop_count = refined_entries
                    .iter()
                    .map(|entry| entry.site_clip_loop_count)
                    .sum::<usize>();
                let first_refined_error = refined_entries
                    .iter()
                    .find_map(SimulationNode::cached_refined_cdt_failure_label)
                    .unwrap_or("none");
                Self::append_refined_terrain_patch_payloads_for_requests(
                    &mut built,
                    &refined_requests,
                    &refined_entries,
                );
                let mut cache_inserted = false;
                if !refined_entries.is_empty() {
                    match core.try_lock() {
                        Ok(mut core) => {
                            core.insert_refined_terrain_patch_cache_entries(refined_entries);
                            cache_inserted = true;
                        }
                        Err(std::sync::TryLockError::WouldBlock) => {}
                        Err(std::sync::TryLockError::Poisoned(_)) => panic!(
                            "simulation core lock poisoned during refined terrain cache insertion"
                        ),
                    }
                }
                let mut job_state = jobs
                    .lock()
                    .expect("terrain payload job lock poisoned during result publication");
                job_state.completed.extend(built);
                job_state.failed.append(&mut failed_requests);
                if perf_enabled
                    && (worker_start.elapsed().as_secs_f64() * 1000.0 >= 8.0
                        || cdt_ms >= 4.0
                        || refined_request_count > 0)
                {
                    println!(
                        "[DEBUG:perf] terrain_payload_worker_done requests={} refined_requests={} refined_entries={} refined_windows={} failed_windows={} invalid_entries={} road_loops={} site_loops={} first_error={} cache_inserted={} cdt_ms={:.3} total_ms={:.3}",
                        request_count,
                        refined_request_count,
                        refined_entry_count,
                        refined_window_count,
                        refined_failed_window_count,
                        refined_invalid_entry_count,
                        refined_road_loop_count,
                        refined_site_loop_count,
                        first_refined_error,
                        cache_inserted,
                        cdt_ms,
                        worker_start.elapsed().as_secs_f64() * 1000.0
                    );
                }
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
            "ready_count",
            i64::try_from(ready_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "build_queued_count",
            i64::try_from(build_queued_count).unwrap_or(i64::MAX),
        );
        let packed_tracked_requests =
            Self::packed_terrain_patch_payload_states(tracked_requests.into_iter().map(
                |(key, generation)| (key.patch_x, key.patch_z, key.render_step_mm, generation),
            ));
        stats.set("tracked_requests", packed_tracked_requests.to_variant());
        stats
    }

    /// Returns ready async terrain patch payloads without blocking on patch extraction.
    #[func]
    pub fn poll_ready_terrain_patch_payloads(&mut self, max_patches: i32) -> VarDictionary {
        let (completed_payloads, failed_requests) =
            self.drain_completed_terrain_patch_payload_jobs();
        let completed_count = completed_payloads.len();
        let failed_count = failed_requests.len();
        let mut retry_requests = failed_requests.clone();
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
        stats.set("requested_before_count", 0_i64);
        stats.set("requested_after_count", 0_i64);
        stats.set(
            "failed_count",
            i64::try_from(failed_count).unwrap_or(i64::MAX),
        );
        stats.set(
            "retry_requests",
            Self::packed_terrain_patch_payload_states(retry_requests.iter().map(|request| {
                (
                    request.key.patch_x,
                    request.key.patch_z,
                    request.key.render_step_mm,
                    request.surface_generation,
                )
            }))
            .to_variant(),
        );

        let (global_generation, patch_generations) = {
            let snapshot = self.snapshot.read().unwrap();
            (
                snapshot.terrain_payload_global_generation,
                Arc::clone(&snapshot.terrain_payload_patch_generations),
            )
        };
        let mut jobs = self.lock_terrain_patch_payload_jobs();
        if !completed_payloads.is_empty() {
            jobs.ingest_completed(completed_payloads);
        }
        if !failed_requests.is_empty() {
            jobs.ingest_failed(failed_requests);
        }
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
        let mut emitted_payloads = Vec::with_capacity(max_patches.min(jobs.ready.len()));
        while emitted_count < max_patches {
            let Some(key) = jobs.ready.pop() else {
                break;
            };
            jobs.ready_lookup.remove(&key);
            let Some(payload) = jobs.payloads.remove(&key) else {
                missing_ready_count += 1;
                if let Some(state) = jobs.requested.remove(&key) {
                    retry_requests.push(TerrainPatchPayloadRequest {
                        key,
                        request_id: state.request_id,
                        surface_generation: state.surface_generation,
                    });
                }
                continue;
            };
            let payload_state = TerrainPatchPayloadRequestState {
                request_id: payload.request_id,
                surface_generation: payload.surface_generation,
            };
            if jobs.requested.get(&key).copied() != Some(payload_state) {
                stale_ready_count += 1;
                continue;
            }
            let current_generation = patch_generations
                .get(&(key.patch_x, key.patch_z))
                .copied()
                .unwrap_or(0)
                .max(global_generation);
            if payload.surface_generation != current_generation {
                stale_ready_count += 1;
                jobs.requested.remove(&key);
                retry_requests.push(TerrainPatchPayloadRequest {
                    key,
                    request_id: payload.request_id,
                    surface_generation: payload.surface_generation,
                });
                continue;
            }
            jobs.requested.remove(&key);
            emitted_payloads.push(payload);
            emitted_count += 1;
        }
        let requested_after_count = jobs.requested.len();
        drop(jobs);
        for payload in emitted_payloads {
            patches.push(&Self::terrain_patch_payload_dict(&payload).to_variant());
        }
        let packed_retry_requests =
            Self::packed_terrain_patch_payload_states(retry_requests.into_iter().map(|request| {
                (
                    request.key.patch_x,
                    request.key.patch_z,
                    request.key.render_step_mm,
                    request.surface_generation,
                )
            }));

        stats.set("patches", patches.to_variant());
        stats.set("retry_requests", packed_retry_requests.to_variant());
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
            i64::try_from(requested_after_count).unwrap_or(i64::MAX),
        );
        stats
    }

    /// Returns the terrain-border perimeter loop as world-space top positions.
    #[func]
    pub fn get_terrain_border_loop(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(
            self.snapshot
                .read()
                .unwrap()
                .terrain_border_loop
                .iter()
                .copied(),
        )
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        let snapshot = self.snapshot.read().unwrap();
        Vector2::new(
            snapshot.heightmap_width as f32,
            snapshot.heightmap_height as f32,
        )
    }

    /// Returns the terrain world extent in metres.
    #[func]
    pub fn get_terrain_world_size(&self) -> Vector2 {
        self.snapshot.read().unwrap().terrain_world_size
    }

    // ── Terrain Queries ──

    /// Returns the terrain heightmap elevation at a world X/Z position.
    #[func]
    pub fn get_height_at(&self, pos: Vector2) -> f32 {
        self.lock_core().get_height_at_internal(pos)
    }

    /// Returns the visible world-surface height at a position.
    ///
    /// This reads the already compiled roadbed when a road surface owns the queried XZ location
    /// and otherwise falls back to the current visual terrain.
    #[func]
    pub fn get_world_surface_height(&self, pos: Vector2) -> f32 {
        self.lock_core().get_world_surface_height_internal(pos)
    }

    /// Raycasts against the terrain heightmap.
    ///
    /// Uses `try_lock` so this never stalls the Godot main thread if the sim thread
    /// is currently holding the mutex (e.g. during `add_road_internal`). Returns
    /// `null` when contended; GDScript already handles null from this call gracefully.
    #[func]
    pub fn intersect_terrain(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.intersect_terrain_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Raycasts against the visible world surface.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain`. The combined surface prefers
    /// compiled roadbed ownership and otherwise falls back to the visible terrain surface.
    #[func]
    pub fn intersect_world_surface(&self, ray_origin: Vector3, ray_dir: Vector3) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => match core.intersect_world_surface_internal(ray_origin, ray_dir) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }
}
