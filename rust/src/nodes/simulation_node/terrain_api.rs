//! Terrain editing and terrain-patch Godot API methods.

use super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Applies one immediate terrain sculpt edit.
    #[func]
    pub fn sculpt_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Begins one batched world-editor terrain stroke.
    #[func]
    pub fn begin_terrain_stroke(&mut self) {
        self.lock_core().start_terrain_stroke_internal();
    }

    /// Finalizes one batched world-editor terrain stroke.
    #[func]
    pub fn end_terrain_stroke(&mut self) -> bool {
        let (ended, terrain_dirty, water_dirty, network_dirty) = {
            let mut core = self.lock_core();
            let ended = core.end_terrain_stroke_internal();
            (
                ended,
                core.terrain_dirty,
                core.water_dirty,
                core.network_dirty,
            )
        };
        if ended {
            let mut snapshot = self.snapshot.write().unwrap();
            snapshot.terrain_dirty = terrain_dirty;
            snapshot.water_dirty = water_dirty;
            snapshot.network_dirty = network_dirty;
        }
        ended
    }

    /// Applies one batched terrain sculpt step during an active editor stroke.
    #[func]
    pub fn sculpt_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .sculpt_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
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
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
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
        self.lock_core()
            .level_terrain_stroke_step_internal(pos, radius, target_height_m, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Smooths terrain toward the local neighborhood average.
    #[func]
    pub fn smooth_terrain(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_internal(pos, radius, strength);
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
    }

    /// Applies one batched terrain-smooth step during an active editor stroke.
    #[func]
    pub fn smooth_terrain_stroke_step(&mut self, pos: Vector2, radius: f32, strength: f32) {
        self.lock_core()
            .smooth_terrain_stroke_step_internal(pos, radius, strength);
        self.snapshot.write().unwrap().terrain_dirty = true;
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
        let mut snapshot = self.snapshot.write().unwrap();
        snapshot.terrain_dirty = true;
        snapshot.water_dirty = true;
        snapshot.network_dirty = true;
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
        self.lock_core().slope_terrain_stroke_step_internal(
            pos,
            radius,
            start_world,
            start_height_m,
            end_world,
            end_height_m,
            strength,
        );
        self.snapshot.write().unwrap().terrain_dirty = true;
    }

    /// Returns whether the terrain mesh needs rebuilding.
    #[func]
    pub fn is_terrain_dirty(&self) -> bool {
        self.snapshot.read().unwrap().terrain_dirty
    }

    /// Clears the terrain dirty flag.
    #[func]
    pub fn clear_terrain_dirty(&mut self) {
        let (preview_context, road_query_snapshot) = {
            let mut core = self.lock_core();
            core.terrain_dirty = false;
            core.heightmap.clear_dirty_render_patches();
            road_tool_snapshots_from_core(&core)
        };
        *self.road_preview_context.write().unwrap() = preview_context;
        *self.road_tool_query_snapshot.write().unwrap() = road_query_snapshot;
        self.snapshot.write().unwrap().terrain_dirty = false;
    }

    /// Returns true if the road/rail network was mutated and the visual mesh needs a rebuild.
    ///
    /// `NetworkRenderer._process` polls this each frame. The flag stays `true` until
    /// `clear_network_dirty()` is called by GDScript after the refresh is complete,
    /// matching the same explicit-clear pattern used by `terrain_dirty` and `water_dirty`.
    #[func]
    pub fn is_network_dirty(&self) -> bool {
        self.snapshot.read().unwrap().network_dirty
    }

    /// Returns true when the background sim thread currently owns the core mutex.
    #[func]
    pub fn is_sim_core_busy(&self) -> bool {
        match self.core.try_lock() {
            Ok(_) => false,
            Err(std::sync::TryLockError::WouldBlock) => true,
            Err(std::sync::TryLockError::Poisoned(_)) => {
                panic!("simulation core lock poisoned by a failed authoritative phase")
            }
        }
    }

    /// Clears the network-dirty flag after `NetworkRenderer` has rebuilt the road/rail mesh.
    #[func]
    pub fn clear_network_dirty(&mut self) {
        self.lock_core().network_dirty = false;
        self.snapshot.write().unwrap().network_dirty = false;
    }

    /// Returns render-patch layout metadata shared by terrain and water renderers.
    #[func]
    pub fn get_terrain_patch_layout(&self) -> VarDictionary {
        let core = self.lock_core();
        let mut dict = VarDictionary::new();
        dict.set(
            "patch_cols",
            i64::try_from(core.heightmap.render_patch_cols()).unwrap_or(0),
        );
        dict.set(
            "patch_rows",
            i64::try_from(core.heightmap.render_patch_rows()).unwrap_or(0),
        );
        dict.set(
            "patch_interval_cells",
            i64::try_from(core.heightmap.render_patch_interval_cells()).unwrap_or(0),
        );
        dict.set("terrain_cell_m", f64::from(core.config.terrain_cell_m));
        dict.set("chunk_span_m", f64::from(core.heightmap.chunk_span_m()));
        dict
    }

    /// Returns the currently dirty terrain render patches as flat `(x, z)` pairs.
    #[func]
    pub fn get_dirty_terrain_patches(&self) -> PackedInt32Array {
        let core = self.lock_core();
        let mut patches: Vec<(usize, usize)> = core
            .heightmap
            .dirty_render_patches()
            .iter()
            .copied()
            .collect();
        patches.sort_unstable();
        let mut packed = PackedInt32Array::new();
        for (patch_x, patch_z) in patches {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }

    /// Returns one visual-terrain render patch, including its one-sample border ring.
    #[func]
    pub fn get_terrain_patch(&self, patch_x: i32, patch_z: i32) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        let Some(patch) = core.heightmap.visual_patch_snapshot(patch_x, patch_z) else {
            return VarDictionary::new();
        };
        Self::terrain_patch_dict(&patch)
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
        if requests.is_empty() {
            return stats;
        }

        let mut accepted_count = 0_usize;
        let mut duplicate_count = 0_usize;
        let mut in_flight_count = 0_usize;
        let mut ready_count = 0_usize;
        let mut build_queued_count = 0_usize;
        let mut build_requests = Vec::new();
        let surface_generation = self
            .road_tool_query_snapshot
            .read()
            .unwrap()
            .surface_generation;
        {
            let mut jobs = self.lock_terrain_patch_payload_jobs();
            let mut requested_key_lookup = HashSet::new();
            for key in requests {
                if !requested_key_lookup.insert(key) {
                    duplicate_count += 1;
                    continue;
                }
                jobs.drop_stale_key_for_generation(key, surface_generation);
                let in_flight = jobs.has_current_in_flight(key, surface_generation);
                let ready = jobs.has_current_ready(key, surface_generation);
                if jobs.has_current_request(key, surface_generation) || in_flight || ready {
                    duplicate_count += 1;
                    in_flight_count += usize::from(in_flight);
                    ready_count += usize::from(ready);
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
                let (mut built, refined_requests, refined_inputs) = {
                    let mut core = core
                        .lock()
                        .expect("simulation core lock poisoned during terrain payload input build");
                    let input_start = Instant::now();
                    let build_jobs = build_requests
                        .into_iter()
                        .filter_map(|request| {
                            SimulationNode::terrain_patch_payload_build_job_for_request(
                                &mut core, request,
                            )
                        })
                        .collect::<Vec<_>>();
                    let split = SimulationNode::split_terrain_patch_payload_jobs(build_jobs);
                    if perf_enabled {
                        let input_ms = input_start.elapsed().as_secs_f64() * 1000.0;
                        if input_ms >= 4.0 || refined_request_count > 0 {
                            println!(
                                "[DEBUG:perf] terrain_payload_worker_input requests={} refined_requests={} ready={} refined_inputs={} input_ms={:.3}",
                                request_count,
                                refined_request_count,
                                split.0.len(),
                                split.2.len(),
                                input_ms
                            );
                        }
                    }
                    split
                };

                let cdt_start = Instant::now();
                let refined_entries = if refined_inputs.is_empty() {
                    Vec::new()
                } else {
                    SimCore::build_refined_terrain_patch_cache_entries(refined_inputs)
                };
                let cdt_ms = cdt_start.elapsed().as_secs_f64() * 1000.0;
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
                if perf_enabled
                    && (worker_start.elapsed().as_secs_f64() * 1000.0 >= 8.0
                        || cdt_ms >= 4.0
                        || refined_request_count > 0)
                {
                    println!(
                        "[DEBUG:perf] terrain_payload_worker_done requests={} refined_requests={} refined_entries={} cache_inserted={} cdt_ms={:.3} total_ms={:.3}",
                        request_count,
                        refined_request_count,
                        refined_requests.len(),
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
        stats
    }

    /// Returns ready async terrain patch payloads without blocking on patch extraction.
    #[func]
    pub fn poll_ready_terrain_patch_payloads(&mut self, max_patches: i32) -> VarDictionary {
        let completed_payloads = self.drain_completed_terrain_patch_payload_jobs();
        let completed_count = completed_payloads.len();
        let max_patches = usize::try_from(max_patches).unwrap_or(0);
        let current_surface_generation = self
            .road_tool_query_snapshot
            .read()
            .unwrap()
            .surface_generation;
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

        let mut jobs = self.lock_terrain_patch_payload_jobs();
        if !completed_payloads.is_empty() {
            jobs.ingest_completed(completed_payloads);
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
            let payload_state = TerrainPatchPayloadRequestState {
                request_id: payload.request_id,
                surface_generation: payload.surface_generation,
            };
            if jobs.requested.get(&key).copied() != Some(payload_state) {
                stale_ready_count += 1;
                continue;
            }
            if payload.surface_generation != current_surface_generation {
                stale_ready_count += 1;
                jobs.requested.remove(&key);
                continue;
            }
            jobs.requested.remove(&key);
            patches.push(&Self::terrain_patch_payload_dict(&payload).to_variant());
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

    /// Returns one visible-terrain render patch resampled at a finer render step.
    #[func]
    pub fn get_refined_terrain_patch(
        &self,
        patch_x: i32,
        patch_z: i32,
        render_step_m: f32,
    ) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let mut core = self.lock_core();
        let cache_key = Self::refined_patch_cache_key(patch_x, patch_z, render_step_m);
        let surface_generation = core.road_tool_surface_generation;
        if core
            .refined_terrain_patch_cache
            .get(&cache_key)
            .is_some_and(|cached| {
                !Self::refined_terrain_patch_cache_entry_is_current(cached, surface_generation)
            })
        {
            core.refined_terrain_patch_cache.remove(&cache_key);
        }
        if let Some(cached) = core.refined_terrain_patch_cache.get(&cache_key) {
            let road_debug = crate::debug::category_enabled("road");
            let total_start = road_debug.then(Instant::now);
            let dict = Self::cached_refined_terrain_patch_dict(cached, false);
            if road_debug {
                debug_log!(
                    "road",
                    "refined_patch_cache_hit key=({},{}) render_step_mm={} windows={} reused_windows={} input_road_loops={} source_samples={} cdt_ms={:.3} total_ms={:.3}",
                    patch_x,
                    patch_z,
                    cache_key.render_step_mm,
                    cached.windows.len(),
                    cached.reused_windows,
                    cached.input_road_loops,
                    cached.input_source_samples,
                    cached.cdt_ms,
                    total_start
                        .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                        .unwrap_or(0.0)
                );
            }
            return dict;
        }
        if crate::debug::category_enabled("road") {
            debug_log!(
                "road",
                "refined_patch_cache_miss_fallback key=({},{}) render_step_mm={}",
                patch_x,
                patch_z,
                cache_key.render_step_mm
            );
        }
        core.heightmap
            .visual_patch_snapshot(patch_x, patch_z)
            .map(|patch| Self::terrain_patch_dict(&patch))
            .unwrap_or_else(VarDictionary::new)
    }

    /// Returns a refined terrain patch with CDT provenance sidecars for diagnostics.
    #[func]
    pub fn get_refined_terrain_patch_debug(
        &self,
        patch_x: i32,
        patch_z: i32,
        render_step_m: f32,
    ) -> VarDictionary {
        let Ok(patch_x) = usize::try_from(patch_x) else {
            return VarDictionary::new();
        };
        let Ok(patch_z) = usize::try_from(patch_z) else {
            return VarDictionary::new();
        };
        let core = self.lock_core();
        Self::refined_terrain_patch_dict(&core, patch_x, patch_z, render_step_m, true)
    }

    /// Returns the terrain-border perimeter loop as world-space top positions.
    #[func]
    pub fn get_terrain_border_loop(&self) -> PackedVector3Array {
        PackedVector3Array::from_iter(self.lock_core().heightmap.border_loop_positions())
    }

    /// Returns the dimensions of the heightmap.
    #[func]
    pub fn get_heightmap_size(&self) -> Vector2 {
        self.get_heightmap_size_internal()
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
